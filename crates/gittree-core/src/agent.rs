//! Agent sessions: a command-line coding agent the user has installed,
//! run on a task in its own worktree (FR-AGENT-04).
//!
//! The app starts the agent's own program with an argument vector and no
//! shell. It sends nothing anywhere itself and stores no key: the agent
//! uses whatever sign-in it already has.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    /// A command the user typed.
    Other,
}

impl AgentKind {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Gemini, Self::Other];

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::Other => "Other",
        }
    }
}

/// The program and arguments that run `task` without a terminal. Each
/// known agent is told it may edit files in its folder and nothing wider.
/// A custom command is split at spaces and gets the task as its last
/// argument.
// ponytail: no quoting in a custom command, so a program path with a space
// in it cannot be used. Parse quotes if someone needs that.
pub fn command(kind: AgentKind, custom: &str, task: &str) -> Result<Vec<String>, String> {
    let task = task.trim();
    if task.is_empty() {
        return Err("The task is empty.".into());
    }
    let mut argv: Vec<String> = match kind {
        AgentKind::Claude => vec!["claude", "--permission-mode", "acceptEdits", "-p"],
        AgentKind::Codex => vec!["codex", "exec", "--sandbox", "workspace-write"],
        AgentKind::Gemini => vec!["gemini", "--approval-mode", "auto_edit", "-p"],
        AgentKind::Other => custom.split_whitespace().collect(),
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if argv.is_empty() {
        return Err("The command is empty.".into());
    }
    argv.push(task.to_owned());
    Ok(argv)
}

/// Finds a program the way a terminal would, then in the folders agents
/// are usually installed to. An app started from the Finder gets a short
/// `PATH` that leaves those out.
fn find_program(name: &str) -> PathBuf {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return name.into();
    }
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        folders.extend([home.join(".local/bin"), home.join(".cargo/bin")]);
    }
    folders.extend(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]);
    let names = [
        name.to_owned(),
        format!("{name}.exe"),
        format!("{name}.cmd"),
    ];
    folders
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|path| path.is_file())
        .unwrap_or_else(|| name.into())
}

/// Runs the agent in `folder` until it ends or `cancel` is set. `on_line`
/// gets each line it prints. Returns whether it ended well.
pub fn run(
    argv: &[String],
    folder: &Path,
    cancel: &AtomicBool,
    mut on_line: impl FnMut(&str),
) -> Result<bool, String> {
    let (program, args) = argv.split_first().ok_or("The command is empty.")?;
    let mut child = Command::new(find_program(program))
        .args(args)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {program}: {error}"))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            let mut lines = Vec::new();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                lines.push(line);
            }
            lines
        });
        // ponytail: lines are handed over when the agent ends, since the
        // known agents print their answer only then.
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.success()),
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => break Err(error.to_string()),
            }
        };
        for line in reader.join().unwrap_or_default() {
            on_line(&line);
        }
        status
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Working,
    Done,
    Failed,
    /// The app closed, or the user stopped it, before the agent ended.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub agent: String,
    pub task: String,
    /// The session's own branch. It names the session.
    pub branch: String,
    /// The branch it started from, and merges back into.
    pub base: String,
    pub path: PathBuf,
    pub state: SessionState,
    /// The last thing the agent printed.
    pub note: String,
}

fn file(common_dir: &Path) -> PathBuf {
    common_dir.join("gittree").join("sessions.json")
}

/// The saved sessions. One that was working when the app closed has lost
/// its process, so it comes back as stopped. A missing or broken file
/// gives none.
pub fn load(common_dir: &Path) -> Vec<Session> {
    let mut sessions: Vec<Session> = std::fs::read(file(common_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for session in &mut sessions {
        if session.state == SessionState::Working {
            session.state = SessionState::Stopped;
        }
    }
    sessions
}

pub fn save(common_dir: &Path, sessions: &[Session]) -> std::io::Result<()> {
    let path = file(common_dir);
    std::fs::create_dir_all(path.parent().expect("has a parent"))?;
    std::fs::write(path, serde_json::to_vec_pretty(sessions)?)
}

/// A branch name made from a task: "agent/" and its first few words.
pub fn branch_for(task: &str) -> String {
    let words: Vec<String> = task
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect();
    format!(
        "agent/{}",
        if words.is_empty() {
            "task".into()
        } else {
            words.join("-")
        }
    )
}

/// Where a session's worktree goes by default: next to the repository, in
/// "<repository>.worktrees", named after the branch.
pub fn folder_for(repository: &Path, branch: &str) -> PathBuf {
    let name = repository.file_name().unwrap_or_default().to_string_lossy();
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    repository
        .with_file_name(format!("{name}.worktrees"))
        .join(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_end_with_the_task_and_never_go_through_a_shell() {
        let argv = command(AgentKind::Claude, "", " fix; rm -rf / ").unwrap();
        assert_eq!(argv[0], "claude");
        // The task is one argument, whatever is in it.
        assert_eq!(argv.last().unwrap(), "fix; rm -rf /");
        assert_eq!(
            command(AgentKind::Other, "my-agent --go", "task").unwrap(),
            ["my-agent", "--go", "task"]
        );
        assert!(command(AgentKind::Other, "  ", "task").is_err());
        assert!(command(AgentKind::Codex, "", "  ").is_err());
    }

    #[test]
    fn names_come_from_the_task_and_the_branch() {
        assert_eq!(
            branch_for("Add j and k as down, please!"),
            "agent/add-j-and-k"
        );
        assert_eq!(branch_for("!!"), "agent/task");
        assert_eq!(
            folder_for(Path::new("/code/SGT"), "agent/vim-keys"),
            Path::new("/code/SGT.worktrees/vim-keys")
        );
    }

    #[test]
    fn a_working_session_is_stopped_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session {
            agent: "Codex".into(),
            task: "t".into(),
            branch: "agent/t".into(),
            base: "main".into(),
            path: "/wt".into(),
            state: SessionState::Working,
            note: String::new(),
        };
        save(dir.path(), std::slice::from_ref(&session)).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded[0].state, SessionState::Stopped);
        assert_eq!(loaded[0].branch, "agent/t");
        assert!(load(&dir.path().join("missing")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_collects_output_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = Vec::new();
        let argv: Vec<String> = ["/bin/sh", "-c", "pwd; echo done"].map(String::from).into();
        let ok = run(&argv, dir.path(), &AtomicBool::new(false), |line| {
            lines.push(line.to_owned())
        });
        assert_eq!(ok, Ok(true));
        assert_eq!(lines.last().unwrap(), "done");
        let fail: Vec<String> = ["/bin/sh", "-c", "exit 3"].map(String::from).into();
        assert_eq!(
            run(&fail, dir.path(), &AtomicBool::new(false), |_| {}),
            Ok(false)
        );
        let missing = vec!["/no/such/agent".to_owned()];
        assert!(run(&missing, dir.path(), &AtomicBool::new(false), |_| {}).is_err());
        // A stopped agent ends at once.
        let slow: Vec<String> = ["/bin/sh", "-c", "sleep 30"].map(String::from).into();
        let started = std::time::Instant::now();
        assert_eq!(
            run(&slow, dir.path(), &AtomicBool::new(true), |_| {}),
            Ok(false)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
