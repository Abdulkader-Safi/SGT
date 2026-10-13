//! An MCP server over stdio, so a coding agent can use this app's Git path:
//! the same runner, trust rules and journal as the app itself.
//!
//! Messages are JSON-RPC 2.0, one per line. Only tools are offered.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::error::{GitError, GitErrorKind};
use crate::journal::{Action, Journal};
use crate::model::{Head, RefKind, RepoPath};
use crate::ops::{read, stage};
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
}

/// A tool call that could not run, in words for the agent.
type Refusal = String;

impl Server {
    pub fn new(runner: GitRunner, common_dir: &Path, access: McpAccess) -> Self {
        Self {
            runner,
            journal: Journal::new(common_dir),
            access,
        }
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
            Some("initialize") => json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "safi-git-tree", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Git for the repository this server was started in. \
                    Changing tools work only in a repository the user trusts, with access \
                    set to read and change. Commits made here can be undone in the app.",
            }),
            Some("ping") => json!({}),
            Some("tools/list") => json!({ "tools": tools() }),
            Some("tools/call") => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let (text, failed) = match self.call(name, &args) {
                    Ok(text) => (text, false),
                    Err(refusal) => (refusal, true),
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn call(&self, name: &str, args: &Value) -> Result<String, Refusal> {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
        let action = text("action");
        match name {
            "git_status" => self.status(),
            "git_log_or_diff" if action == "diff" => self.diff(args),
            "git_log_or_diff" => self.log(args),
            "git_blame" => self.blame(args),
            "git_branch" => self.branches(),
            "git_add_or_commit" => {
                self.may_change()?;
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
    fn may_change(&self) -> Result<(), Refusal> {
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
        Ok(())
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
            "description": "Local and remote branches, with the current one marked.",
            "inputSchema": { "type": "object", "properties": {} },
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
