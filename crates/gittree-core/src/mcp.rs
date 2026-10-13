//! An MCP server over stdio, so a coding agent can use this app's Git path:
//! the same runner, trust rules and journal as the app itself.
//!
//! Messages are JSON-RPC 2.0, one per line. Only tools are offered.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{GitError, GitErrorKind};
use crate::journal::{Action, Journal};
use crate::model::{Head, RefKind, RepoPath};
use crate::ops::branch::{self, Target};
use crate::ops::{read, stage, worktree};
use crate::runner::GitRunner;
use crate::store::McpAccess;

/// The protocol version answered when the client names none.
const PROTOCOL: &str = "2025-06-18";
/// Longer tool output is cut, so one call cannot flood the agent.
const OUTPUT_LIMIT: usize = 200_000;

pub struct Server {
    runner: GitRunner,
    journal: Journal,
    access: McpAccess,
    /// Tools the user switched off in Settings.
    off: Vec<String>,
    /// The folder the server and the app share: `gittree` in the Git folder.
    link: PathBuf,
    /// Whether the app must allow each change first.
    ask: bool,
    ask_wait: Duration,
    /// The client's name, from its `initialize` message.
    client: Mutex<String>,
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Failed,
    /// The user said no, or nobody answered.
    Denied,
}

/// One tool call an agent made, as the app lists it (FR-AGENT-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// Seconds since 1970.
    pub time: u64,
    pub client: String,
    pub tool: String,
    pub detail: String,
    pub outcome: Outcome,
}

/// A change an agent is waiting to make, until the app allows or denies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub time: u64,
    pub client: String,
    pub tool: String,
    pub detail: String,
}

/// How long an agent waits for the user's answer.
pub const ASK_WAIT: Duration = Duration::from_secs(120);
/// The calls kept in the list.
const CALLS_KEPT: usize = 200;
const NO_ANSWER: &str = "The user did not allow this change in Safi Git Tree, so it was not \
    made. The app must be open on this repository to answer.";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn link(common_dir: &Path) -> PathBuf {
    common_dir.join("gittree")
}

/// The calls agents made in this repository, oldest first.
pub fn calls(common_dir: &Path) -> Vec<Call> {
    let text = std::fs::read_to_string(link(common_dir).join("mcp-calls.jsonl"));
    let mut calls: Vec<Call> = text
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    calls.drain(..calls.len().saturating_sub(CALLS_KEPT));
    calls
}

/// The changes waiting for an answer, oldest first. A request nobody is
/// waiting on any more is removed.
pub fn requests(common_dir: &Path) -> Vec<Request> {
    let Ok(folder) = std::fs::read_dir(link(common_dir)) else {
        return Vec::new();
    };
    let mut waiting = Vec::new();
    for entry in folder.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("mcp-request-") {
            continue;
        }
        let request = std::fs::read(entry.path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok());
        match request {
            Some(request) if now() < request.time + ASK_WAIT.as_secs() => waiting.push(request),
            _ => drop(std::fs::remove_file(entry.path())),
        }
    }
    waiting.sort_by_key(|request| request.time);
    waiting
}

/// The user's answer to a request. The waiting server picks it up.
pub fn answer(common_dir: &Path, id: &str, allow: bool) -> std::io::Result<()> {
    // The id becomes part of a file name, and it was read from a file.
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let folder = link(common_dir);
    // The agent gave up waiting, so there is nobody to tell.
    if !folder.join(format!("mcp-request-{id}.json")).exists() {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    // Written whole, then renamed, so the server never reads half of it.
    let draft = folder.join(format!("mcp-answer-{id}.tmp"));
    std::fs::write(&draft, if allow { "allow" } else { "deny" })?;
    std::fs::rename(draft, folder.join(format!("mcp-answer-{id}")))
}

/// What a call is about, in a few words for the list and the banner.
fn describe(args: &Value) -> String {
    let mut words = Vec::new();
    for key in ["action", "name", "branch", "path", "revision", "message"] {
        if let Some(word) = args.get(key).and_then(Value::as_str) {
            words.push(word.lines().next().unwrap_or_default());
        }
    }
    if let Some(paths) = args.get("paths").and_then(Value::as_array) {
        words.extend(paths.iter().filter_map(Value::as_str));
    }
    words.join(" ").chars().take(120).collect()
}

/// Every tool by name, and whether it can change the repository.
pub const TOOLS: [(&str, bool); 8] = [
    ("git_status", false),
    ("git_log_or_diff", false),
    ("git_blame", false),
    ("git_branch", true),
    ("git_checkout", true),
    ("git_stash", true),
    ("git_worktree", true),
    ("git_add_or_commit", true),
];

/// The command that adds this server to Claude Code. `program` is this
/// app's own binary.
pub fn client_command(program: &Path) -> String {
    format!("claude mcp add sgt -- \"{}\" mcp", program.display())
}

/// A tool call that could not run, in words for the agent.
type Refusal = String;

impl Server {
    pub fn new(runner: GitRunner, common_dir: &Path, access: McpAccess) -> Self {
        Self {
            runner,
            journal: Journal::new(common_dir),
            access,
            off: Vec::new(),
            link: link(common_dir),
            ask: false,
            ask_wait: ASK_WAIT,
            client: Mutex::new("An agent".into()),
        }
    }

    /// Makes every change wait for the user's answer in the app, for at
    /// most `wait`.
    pub fn asking(mut self, wait: Duration) -> Self {
        self.ask = true;
        self.ask_wait = wait;
        self
    }

    fn client(&self) -> String {
        self.client
            .lock()
            .map(|name| name.clone())
            .unwrap_or_default()
    }

    /// Adds a call to the list the app shows. The list is for the user:
    /// if it cannot be written, the call still stands. Nothing is listed
    /// in an untrusted repository.
    fn record(&self, tool: &str, detail: String, outcome: Outcome) {
        // An untrusted repository is read-only, this file included.
        if !self.runner.trusted() {
            return;
        }
        let call = Call {
            time: now(),
            client: self.client(),
            tool: tool.to_owned(),
            detail,
            outcome,
        };
        let path = self.link.join("mcp-calls.jsonl");
        // ponytail: the file is cut back to the last 200 calls only when it
        // passes 256 KB, so most calls are one append.
        if std::fs::metadata(&path).is_ok_and(|file| file.len() > 256 * 1024)
            && let Some(common_dir) = self.link.parent()
        {
            let kept: Vec<String> = calls(common_dir)
                .iter()
                .filter_map(|call| serde_json::to_string(call).ok())
                .collect();
            let _ = std::fs::write(&path, kept.join("\n") + "\n");
        }
        let _ = std::fs::create_dir_all(&self.link);
        if let (Ok(mut file), Ok(line)) = (
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path),
            serde_json::to_string(&call),
        ) {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Asks the user in the app and waits for the answer.
    fn approve(&self, tool: &str, detail: &str) -> Result<(), Refusal> {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).map_err(|error| error.to_string())?;
        let id: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let request = Request {
            id: id.clone(),
            time: now(),
            client: self.client(),
            tool: tool.to_owned(),
            detail: detail.to_owned(),
        };
        let (asked, answered) = (
            self.link.join(format!("mcp-request-{id}.json")),
            self.link.join(format!("mcp-answer-{id}")),
        );
        std::fs::create_dir_all(&self.link)
            .and_then(|()| std::fs::write(&asked, serde_json::to_vec(&request)?))
            .map_err(|error| format!("Could not ask the user: {error}"))?;
        let until = std::time::Instant::now() + self.ask_wait;
        let allowed = loop {
            if let Ok(answer) = std::fs::read_to_string(&answered) {
                break answer == "allow";
            }
            if std::time::Instant::now() >= until {
                break false;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let _ = std::fs::remove_file(asked);
        let _ = std::fs::remove_file(answered);
        if allowed {
            Ok(())
        } else {
            Err(NO_ANSWER.into())
        }
    }

    /// Leaves out the named tools: they are not listed and cannot be called.
    pub fn without(mut self, tools: Vec<String>) -> Self {
        self.off = tools;
        self
    }

    fn is_off(&self, tool: &str) -> bool {
        self.off.iter().any(|off| off == tool)
    }

    /// Reads requests until the input ends, answering each on one line.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(message) => self.handle(&message),
                Err(_) => Some(error(Value::Null, -32700, "Parse error")),
            };
            if let Some(reply) = reply {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Answers one message. A notification gets no answer.
    pub fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                let name = params["clientInfo"]["name"].as_str().unwrap_or("").trim();
                if !name.is_empty()
                    && let Ok(mut client) = self.client.lock()
                {
                    *client = name.chars().take(40).collect();
                }
                json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "safi-git-tree", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Git for the repository this server was started in. \
                    Changing tools work only in a repository the user trusts, with access \
                    set to read and change. Commits made here can be undone in the app.",
                })
            }
            Some("ping") => json!({}),
            Some("tools/list") => {
                let mut list = tools();
                if let Some(list) = list.as_array_mut() {
                    list.retain(|tool| !self.is_off(tool["name"].as_str().unwrap_or("")));
                }
                json!({ "tools": list })
            }
            Some("tools/call") => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let (text, failed) = match self.call(name, &args) {
                    Ok(text) => (text, false),
                    Err(refusal) => (refusal, true),
                };
                let outcome = match failed {
                    false => Outcome::Done,
                    true if text == NO_ANSWER => Outcome::Denied,
                    true => Outcome::Failed,
                };
                self.record(name, describe(&args), outcome);
                json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn call(&self, name: &str, args: &Value) -> Result<String, Refusal> {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
        let action = text("action");
        let (tool, about) = (name, describe(args));
        if self.is_off(name) {
            return Err(format!(
                "The user has switched {name} off in Safi Git Tree's settings."
            ));
        }
        match name {
            "git_status" => self.status(),
            "git_log_or_diff" if action == "diff" => self.diff(args),
            "git_log_or_diff" => self.log(args),
            "git_blame" => self.blame(args),
            "git_branch" => match action {
                "" | "list" => self.branches(),
                "create" => {
                    self.may_change(tool, &about)?;
                    let (name, at) = (text("name"), or(text("at"), "HEAD"));
                    let action = Action::CreateBranch {
                        name: name.to_owned(),
                        at: at.to_owned(),
                        checkout: false,
                    };
                    self.journaled(
                        action,
                        format!("Create branch {name} (by an agent)"),
                        || branch::create_branch(&self.runner, name, at, false),
                    )?;
                    Ok(format!("Created branch {name} at {at}."))
                }
                "delete" => {
                    self.may_change(tool, &about)?;
                    let name = text("name");
                    // Never forced: Git refuses a branch with unmerged commits.
                    let action = Action::DeleteBranch {
                        name: name.to_owned(),
                        force: false,
                    };
                    self.journaled(
                        action,
                        format!("Delete branch {name} (by an agent)"),
                        || branch::delete_branch(&self.runner, name, false),
                    )?;
                    Ok(format!("Deleted branch {name}."))
                }
                _ => Err("git_branch takes action \"list\", \"create\" or \"delete\".".into()),
            },
            "git_checkout" => {
                self.may_change(tool, &about)?;
                let name = text("branch");
                branch::check_name(&self.runner, name).map_err(told)?;
                let action = Action::Checkout {
                    target: name.to_owned(),
                };
                self.journaled(action, format!("Checkout {name} (by an agent)"), || {
                    branch::checkout(&self.runner, &Target::Branch(name.to_owned()))
                })?;
                Ok(format!("Switched to {name}."))
            }
            "git_stash" => {
                let index = args.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                match action {
                    "" | "list" => {
                        let stashes = branch::list_stashes(&self.runner).map_err(told)?;
                        let lines: Vec<String> = stashes
                            .iter()
                            .map(|stash| format!("{} {}", stash.index, stash.message))
                            .collect();
                        Ok(if lines.is_empty() {
                            "There are no stashes.".into()
                        } else {
                            lines.join("\n")
                        })
                    }
                    "push" => {
                        self.may_change(tool, &about)?;
                        let message = or(text("message"), "Stashed by an agent");
                        let action = Action::StashPush {
                            message: message.to_owned(),
                            include_untracked: true,
                        };
                        self.journaled(action, "Stash changes (by an agent)".into(), || {
                            branch::stash_push(&self.runner, message, true)
                        })?;
                        Ok("Stashed the changes, untracked files included.".into())
                    }
                    "apply" | "pop" => {
                        self.may_change(tool, &about)?;
                        let run = if action == "pop" {
                            branch::stash_pop
                        } else {
                            branch::stash_apply
                        };
                        run(&self.runner, index).map_err(told)?;
                        Ok(format!("Stash {index}: {action} done."))
                    }
                    // Dropping a stash destroys work, so agents cannot.
                    _ => Err(
                        "git_stash takes action \"list\", \"push\", \"apply\" or \"pop\".".into(),
                    ),
                }
            }
            "git_worktree" => {
                let path = Path::new(text("path"));
                match action {
                    "" | "list" => {
                        let trees = worktree::list(&self.runner).map_err(told)?;
                        let lines: Vec<String> = trees
                            .iter()
                            .map(|tree| {
                                let branch = tree.branch.as_deref().unwrap_or("(detached)");
                                format!("{} {branch}", tree.path.display())
                            })
                            .collect();
                        Ok(lines.join("\n"))
                    }
                    "add" => {
                        self.may_change(tool, &about)?;
                        let (name, start) = (text("branch"), or(text("start"), "HEAD"));
                        worktree::add(&self.runner, path, name, start).map_err(told)?;
                        Ok(format!(
                            "Added a worktree at {} on new branch {name}.",
                            path.display()
                        ))
                    }
                    "remove" => {
                        self.may_change(tool, &about)?;
                        // Never forced: Git refuses a worktree with changes.
                        worktree::remove(&self.runner, path).map_err(told)?;
                        Ok(format!("Removed the worktree at {}.", path.display()))
                    }
                    _ => Err("git_worktree takes action \"list\", \"add\" or \"remove\".".into()),
                }
            }
            "git_add_or_commit" => {
                self.may_change(tool, &about)?;
                if action == "commit" {
                    self.commit(text("message"))
                } else {
                    self.add(args)
                }
            }
            _ => Err(format!("There is no tool named {name}.")),
        }
    }

    /// Changes need a trusted repository and the user's say-so in Settings.
    /// With "Ask before each change" on, the user also allows this one.
    fn may_change(&self, tool: &str, about: &str) -> Result<(), Refusal> {
        if !self.runner.trusted() {
            return Err("This repository is not trusted, so it is read-only. \
                The user can trust it in Safi Git Tree."
                .into());
        }
        if self.access != McpAccess::ReadWrite {
            return Err("Agents may only read. The user can allow changes in \
                Settings, under MCP server."
                .into());
        }
        if self.ask {
            self.approve(tool, about)?;
        }
        Ok(())
    }

    /// Runs a change with a journal entry, so the app can undo it.
    fn journaled(
        &self,
        action: Action,
        label: String,
        run: impl FnOnce() -> Result<(), GitError>,
    ) -> Result<(), Refusal> {
        let pending = self
            .journal
            .begin(&self.runner, action, label)
            .map_err(told)?;
        match run() {
            Ok(()) => self
                .journal
                .finish(&self.runner, pending)
                .map(drop)
                .map_err(told),
            Err(error) => {
                self.journal.abandon(&self.runner, pending);
                Err(told(error))
            }
        }
    }

    fn status(&self) -> Result<String, Refusal> {
        let head = match read::head(&self.runner).map_err(told)? {
            Head::Branch(name) => format!("On branch {name}"),
            Head::Unborn(name) => format!("On branch {name}, with no commit yet"),
            Head::Detached(oid) => format!("Detached at {}", &oid[..7.min(oid.len())]),
        };
        let entries = read::status(&self.runner).map_err(told)?;
        if entries.is_empty() {
            return Ok(format!(
                "{head}\nNothing to commit, the working tree is clean."
            ));
        }
        let mut out = format!("{head}\n");
        for (title, staged) in [("Staged", true), ("Not staged", false)] {
            let mut lines = entries
                .iter()
                .filter(|entry| {
                    if staged {
                        entry.staged()
                    } else {
                        entry.unstaged()
                    }
                })
                .map(|entry| {
                    let change = if staged { entry.index } else { entry.worktree };
                    let what = if entry.conflict.is_some() {
                        "conflicted".to_owned()
                    } else {
                        format!("{change:?}").to_lowercase()
                    };
                    format!("  {what}: {}", entry.path)
                })
                .peekable();
            if lines.peek().is_some() {
                out.push_str(&format!("{title}:\n"));
                for line in lines {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    fn log(&self, args: &Value) -> Result<String, Refusal> {
        let count = args
            .get("max_count")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 200)
            .to_string();
        let mut call = vec![
            "log".to_owned(),
            "-n".into(),
            count,
            "--date=iso-strict".into(),
            "--format=%h%x09%an%x09%ad%x09%s".into(),
        ];
        call.extend(revision(args)?);
        call.extend(path_args(args));
        self.read(call)
    }

    fn diff(&self, args: &Value) -> Result<String, Refusal> {
        let mut call: Vec<String> = ["diff", "--no-color", "--no-ext-diff", "--no-textconv"]
            .map(String::from)
            .into();
        if args.get("staged").and_then(Value::as_bool) == Some(true) {
            call.push("--cached".into());
        }
        call.extend(revision(args)?);
        call.extend(path_args(args));
        self.read(call)
    }

    fn blame(&self, args: &Value) -> Result<String, Refusal> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        if path.is_empty() {
            return Err("git_blame needs a path.".into());
        }
        let mut call = vec!["blame".to_owned(), "--date=short".into()];
        let line = |key: &str| args.get(key).and_then(Value::as_u64);
        if let (Some(start), Some(end)) = (line("start_line"), line("end_line")) {
            call.extend(["-L".to_owned(), format!("{start},{end}")]);
        }
        call.extend(["--".to_owned(), path.to_owned()]);
        self.read(call)
    }

    fn branches(&self) -> Result<String, Refusal> {
        let refs = read::list_refs(&self.runner).map_err(told)?;
        let lines: Vec<String> = refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Local | RefKind::Remote))
            .map(|r| {
                let mark = if r.is_current { "*" } else { " " };
                let track = match &r.upstream {
                    Some(_) => format!("  ahead {} behind {}", r.ahead, r.behind),
                    None => String::new(),
                };
                let oid = &r.target[..7.min(r.target.len())];
                format!("{mark} {} {oid}{track}", r.short_name())
            })
            .collect();
        Ok(lines.join("\n"))
    }

    fn add(&self, args: &Value) -> Result<String, Refusal> {
        let paths: Vec<RepoPath> = args
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(RepoPath::from)
            .collect();
        if paths.is_empty() {
            return Err("git_add_or_commit with action \"add\" needs paths.".into());
        }
        stage::stage(&self.runner, &paths).map_err(told)?;
        Ok(format!("Staged {} paths.", paths.len()))
    }

    /// Commits what is staged, with a journal entry so the app can undo it.
    fn commit(&self, message: &str) -> Result<String, Refusal> {
        if message.trim().is_empty() {
            return Err("git_add_or_commit with action \"commit\" needs a message.".into());
        }
        let summary = message.lines().next().unwrap_or_default();
        let action = Action::Commit {
            message: message.to_owned(),
            amend: false,
        };
        let label = format!("Commit \"{summary}\" (by an agent)");
        let pending = self
            .journal
            .begin(&self.runner, action, label)
            .map_err(told)?;
        match stage::commit(&self.runner, message, false) {
            Ok(oid) => {
                self.journal.finish(&self.runner, pending).map_err(told)?;
                Ok(format!("Committed {}.", &oid[..7.min(oid.len())]))
            }
            Err(error) => {
                self.journal.abandon(&self.runner, pending);
                Err(told(error))
            }
        }
    }

    fn read(&self, call: Vec<String>) -> Result<String, Refusal> {
        let out = self.runner.read(call).map_err(told)?;
        let mut text = String::from_utf8_lossy(&out).into_owned();
        if text.len() > OUTPUT_LIMIT {
            let mut cut = OUTPUT_LIMIT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n(The output was cut here. Narrow the call to see the rest.)");
        }
        Ok(text)
    }
}

/// The optional `revision` argument. One that starts with a dash would be
/// read by Git as an option, so it is refused.
fn revision(args: &Value) -> Result<Option<String>, Refusal> {
    match args.get("revision").and_then(Value::as_str) {
        None | Some("") => Ok(None),
        Some(revision) if revision.starts_with('-') => {
            Err("A revision cannot start with a dash.".into())
        }
        Some(revision) => Ok(Some(revision.to_owned())),
    }
}

/// The optional `path` argument, behind `--` so it is never an option.
fn path_args(args: &Value) -> Vec<String> {
    match args.get("path").and_then(Value::as_str) {
        None | Some("") => Vec::new(),
        Some(path) => vec!["--".to_owned(), path.to_owned()],
    }
}

/// `value`, or `fallback` when it is empty.
fn or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

fn told(error: GitError) -> Refusal {
    match error.kind {
        GitErrorKind::Untrusted => "This repository is not trusted, so it is read-only.".into(),
        _ if error.stderr.trim().is_empty() => format!("Git failed ({:?}).", error.kind),
        _ => error.stderr.trim().to_owned(),
    }
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tools() -> Value {
    let text = |description: &str| json!({ "type": "string", "description": description });
    json!([
        {
            "name": "git_status",
            "description": "The current branch and every changed, staged or untracked file.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "git_log_or_diff",
            "description": "History, or a diff. With action \"log\" it lists commits. With \
                action \"diff\" it shows changes: of the working tree, of what is staged, or \
                between commits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["log", "diff"] },
                    "revision": text("A commit, branch or range such as main..feature."),
                    "path": text("Limit to this file or folder."),
                    "max_count": { "type": "integer", "description": "Commits to list, up to 200. Default 20." },
                    "staged": { "type": "boolean", "description": "For diff: what is staged, against HEAD." },
                },
                "required": ["action"],
            },
        },
        {
            "name": "git_blame",
            "description": "Who last changed each line of a file, and in which commit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": text("The file."),
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" },
                },
                "required": ["path"],
            },
        },
        {
            "name": "git_branch",
            "description": "With action \"list\" (the default): local and remote branches, \
                the current one marked. \"create\" makes a branch without switching to it. \
                \"delete\" removes one whose commits are merged. Both can be undone in the app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create", "delete"] },
                    "name": text("For create and delete: the branch."),
                    "at": text("For create: where it starts. Default HEAD."),
                },
            },
        },
        {
            "name": "git_checkout",
            "description": "Switches to a local branch. Git refuses when local changes would \
                be overwritten: stash them first. Can be undone in the app.",
            "inputSchema": {
                "type": "object",
                "properties": { "branch": text("The local branch to switch to.") },
                "required": ["branch"],
            },
        },
        {
            "name": "git_stash",
            "description": "With action \"list\" (the default): the stashes. \"push\" stashes \
                all changes, untracked files included. \"apply\" and \"pop\" bring one back. \
                Stashes cannot be dropped here.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "push", "apply", "pop"] },
                    "message": text("For push."),
                    "index": { "type": "integer", "description": "For apply and pop. Default 0, the newest." },
                },
            },
        },
        {
            "name": "git_worktree",
            "description": "With action \"list\" (the default): the worktrees. \"add\" makes \
                one at a full path on a new branch. \"remove\" removes one that has no \
                changed or untracked files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "add", "remove"] },
                    "path": text("For add and remove: the folder, as a full path."),
                    "branch": text("For add: the new branch."),
                    "start": text("For add: where the branch starts. Default HEAD."),
                },
            },
        },
        {
            "name": "git_add_or_commit",
            "description": "Stages files, or commits what is staged. Works only in a trusted \
                repository when the user allows changes. A commit can be undone in the app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "commit"] },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "For add." },
                    "message": text("For commit."),
                },
                "required": ["action"],
            },
        },
    ])
}
