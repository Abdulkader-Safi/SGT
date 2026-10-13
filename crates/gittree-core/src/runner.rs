use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::{GitError, GitErrorKind, classify};
use crate::model::ActivityRecord;

/// Config overrides that stop a repository from running its own programs.
const UNTRUSTED_OVERRIDES: [&str; 8] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.sshCommand=ssh",
    "-c",
    "credential.helper=",
];

const ACTIVITY_LIMIT: usize = 500;
const STDERR_LIMIT: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Changes nothing. Allowed in every repository.
    Read,
    /// Changes the repository. Refused unless it is trusted.
    Write,
}

/// Starts `git` in one folder with a fixed environment, and is the only way
/// the app runs Git. It enforces trust: an untrusted repository gets no write
/// calls and cannot run hooks or helper programs.
#[derive(Debug, Clone)]
pub struct GitRunner {
    git: PathBuf,
    dir: PathBuf,
    trusted: bool,
    /// Shared by clones, so a repository has one activity list.
    activity: Arc<Mutex<VecDeque<ActivityRecord>>>,
}

impl GitRunner {
    pub fn new(git: impl Into<PathBuf>, dir: impl Into<PathBuf>, trusted: bool) -> Self {
        Self {
            git: git.into(),
            dir: dir.into(),
            trusted,
            activity: Default::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn trusted(&self) -> bool {
        self.trusted
    }

    /// The last 500 calls, oldest first.
    pub fn activity(&self) -> Vec<ActivityRecord> {
        self.activity.lock().unwrap().iter().cloned().collect()
    }

    /// Runs a read call and returns its stdout.
    pub fn read<I, S>(&self, args: I) -> Result<Vec<u8>, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run(Access::Read, args, None, None, read_all)
    }

    /// Runs a write call and returns its stdout. `Untrusted` if the
    /// repository is not trusted.
    pub fn write<I, S>(&self, args: I) -> Result<Vec<u8>, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run(Access::Write, args, None, None, read_all)
    }

    /// Runs Git and hands its stdout to `read_stdout` as it arrives. `input`
    /// goes to stdin. Setting `cancel` kills the process and gives
    /// `Cancelled`. An error from `read_stdout` also kills it.
    pub fn run<I, S, T>(
        &self,
        access: Access,
        args: I,
        input: Option<&[u8]>,
        cancel: Option<&AtomicBool>,
        read_stdout: impl FnOnce(&mut ChildStdout) -> io::Result<T>,
    ) -> Result<T, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let shown: Vec<String> = args.iter().map(|a| redact(&a.to_string_lossy())).collect();
        let error = |kind, exit_code, stderr: String| GitError {
            kind,
            args: shown.clone(),
            exit_code,
            stderr,
        };
        if access == Access::Write && !self.trusted {
            return Err(error(GitErrorKind::Untrusted, None, String::new()));
        }

        let mut command = Command::new(&self.git);
        command.current_dir(&self.dir);
        if !self.trusted {
            command.args(UNTRUSTED_OVERRIDES);
        }
        let mut rest = args.iter();
        command.args(rest.next());
        if !self.trusted && args.first().is_some_and(|name| shows_diffs(name)) {
            // An empty textconv or external diff command cannot be set
            // through config: Git would try to run it. These flags can.
            command.args(["--no-ext-diff", "--no-textconv"]);
        }
        command
            .args(rest)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_PAGER", "cat")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if access == Access::Read {
            command.env("GIT_OPTIONAL_LOCKS", "0");
        }
        if !self.trusted {
            let overrides = self.program_overrides();
            command.env("GIT_CONFIG_COUNT", overrides.len().to_string());
            for (n, (key, value)) in overrides.iter().enumerate() {
                command.env(format!("GIT_CONFIG_KEY_{n}"), key);
                command.env(format!("GIT_CONFIG_VALUE_{n}"), value);
            }
        }

        let started = Instant::now();
        let mut child = command.spawn().map_err(|e| {
            // A missing folder also fails the spawn, and is not Git's fault.
            let kind = if self.dir.is_dir() {
                GitErrorKind::GitNotFound
            } else {
                GitErrorKind::Other
            };
            error(kind, None, e.to_string())
        })?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let child = Mutex::new(child);
        let done = AtomicBool::new(false);

        // stderr and stdin get their own threads, so a full pipe on one side
        // can never block the other.
        let (result, stderr) = thread::scope(|scope| {
            let stderr_reader = scope.spawn(move || {
                let mut text = Vec::new();
                let _ = stderr.read_to_end(&mut text);
                text
            });
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                scope.spawn(move || {
                    let _ = stdin.write_all(input);
                });
            }
            let watcher = cancel.map(|cancel| {
                scope.spawn(|| {
                    while !done.load(Ordering::Relaxed) {
                        if cancel.load(Ordering::Relaxed) {
                            let _ = child.lock().unwrap().kill();
                            break;
                        }
                        thread::park_timeout(Duration::from_millis(20));
                    }
                })
            });

            let result = read_stdout(&mut stdout);
            if result.is_err() {
                let _ = child.lock().unwrap().kill();
            }
            // Let Git finish if the reader stopped early.
            let _ = io::copy(&mut stdout, &mut io::sink());
            let stderr = stderr_reader.join().unwrap_or_default();
            done.store(true, Ordering::Relaxed);
            if let Some(watcher) = watcher {
                watcher.thread().unpark();
            }
            (result, stderr)
        });

        let status = child.into_inner().unwrap().wait();
        let exit_code = status.as_ref().ok().and_then(|status| status.code());
        let stderr = capped(&String::from_utf8_lossy(&stderr));
        self.record(&shown, started, exit_code, &stderr);

        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Err(error(GitErrorKind::Cancelled, exit_code, stderr));
        }
        match (result, status) {
            (Ok(value), Ok(status)) if status.success() => Ok(value),
            (Err(e), _) | (_, Err(e)) => Err(error(GitErrorKind::Other, exit_code, e.to_string())),
            (Ok(_), Ok(_)) => Err(error(classify(&stderr), exit_code, stderr)),
        }
    }

    /// Overrides for every setting in the repository's own config that names
    /// a program: filters, merge drivers, pagers and the rest. (Diff drivers
    /// are handled by flags, see `shows_diffs`.) The
    /// fixed overrides cannot cover these, because their names are chosen by
    /// the repository. Settings from the user's global and system config are
    /// left alone: those are the user's own programs.
    // ponytail: this is one extra `git config` per call in an untrusted
    // repository. Cache it per runner if that ever shows up in a profile.
    fn program_overrides(&self) -> Vec<(String, &'static str)> {
        let listed = Command::new(&self.git)
            .current_dir(&self.dir)
            .args(["config", "--list", "--show-scope", "--name-only", "-z"])
            .env("LC_ALL", "C")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        // Outside a repository there is no local config to defuse.
        let Ok(listed) = listed else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&listed.stdout);
        let mut fields = text.split('\0');
        let mut overrides = Vec::new();
        while let (Some(scope), Some(key)) = (fields.next(), fields.next()) {
            let own = !matches!(scope, "system" | "global");
            if let Some(value) = own.then(|| program_override(key)).flatten() {
                overrides.push((key.to_owned(), value));
            }
        }
        overrides
    }

    fn record(&self, args: &[String], started: Instant, exit_code: Option<i32>, stderr: &str) {
        let duration_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let exit_code = exit_code.unwrap_or(-1);
        tracing::info!(?args, duration_ms, exit_code, stderr, "git");
        let mut activity = self.activity.lock().unwrap();
        if activity.len() == ACTIVITY_LIMIT {
            activity.pop_front();
        }
        activity.push_back(ActivityRecord {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs() as i64),
            args: args.to_vec(),
            duration_ms,
            exit_code,
            stderr: stderr.to_owned(),
        });
    }
}

/// The harmless value for a config key that can make Git run a program, or
/// `None` for any other key. `key` is as `git config --list` prints it: the
/// section and the name in lower case, with any subsection between them.
fn program_override(key: &str) -> Option<&'static str> {
    let (section, rest) = key.split_once('.')?;
    let name = rest.rsplit_once('.').map_or(rest, |(_, name)| name);
    let runs_a_program = match section {
        "filter" => matches!(name, "clean" | "smudge" | "process"),
        "merge" => name == "driver",
        "core" => matches!(
            name,
            "fsmonitor"
                | "hookspath"
                | "sshcommand"
                | "pager"
                | "editor"
                | "askpass"
                | "gitproxy"
                | "alternaterefscommand"
        ),
        "gpg" => name == "program",
        "sequence" => name == "editor",
        "credential" => name == "helper",
        "pager" | "alias" => true,
        _ => false,
    };
    match (section, name) {
        // Without this a required filter with no command is an error.
        ("filter", "required") => Some("false"),
        ("core", "hookspath") => Some("/dev/null"),
        ("core", "fsmonitor") => Some("false"),
        _ if runs_a_program => Some(""),
        _ => None,
    }
}

/// Git commands that can run a diff driver or a textconv program.
fn shows_diffs(command: &OsStr) -> bool {
    [
        "diff",
        "show",
        "log",
        "diff-tree",
        "diff-index",
        "diff-files",
        "blame",
    ]
    .iter()
    .any(|name| command == *name)
}

fn read_all(stdout: &mut ChildStdout) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    stdout.read_to_end(&mut out)?;
    Ok(out)
}

fn capped(text: &str) -> String {
    let mut end = text.len().min(STDERR_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Removes the password from a URL like `https://user:secret@host/path`, so
/// it never reaches a log or an error message.
fn redact(arg: &str) -> String {
    let Some(scheme_end) = arg.find("://") else {
        return arg.to_owned();
    };
    let authority_start = scheme_end + 3;
    let authority_end = arg[authority_start..]
        .find('/')
        .map_or(arg.len(), |slash| authority_start + slash);
    let authority = &arg[authority_start..authority_end];
    let (Some(at), Some(colon)) = (authority.rfind('@'), authority.find(':')) else {
        return arg.to_owned();
    };
    if colon > at {
        return arg.to_owned();
    }
    format!(
        "{}***{}",
        &arg[..authority_start + colon + 1],
        &arg[authority_start + at..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_passwords_from_urls() {
        let cases = [
            (
                "https://ada:secret@example.com/r.git",
                "https://ada:***@example.com/r.git",
            ),
            (
                "https://ada:p@ss:word@example.com",
                "https://ada:***@example.com",
            ),
            (
                "https://ada@example.com/r.git",
                "https://ada@example.com/r.git",
            ),
            (
                "https://example.com:8443/r.git",
                "https://example.com:8443/r.git",
            ),
            ("https://example.com/a:b@c", "https://example.com/a:b@c"),
            ("git@example.com:team/r.git", "git@example.com:team/r.git"),
            ("--format=%H", "--format=%H"),
        ];
        for (arg, shown) in cases {
            assert_eq!(redact(arg), shown);
        }
    }

    #[test]
    fn finds_the_config_keys_that_run_programs() {
        for key in [
            "filter.lfs.clean",
            "filter.a.b.smudge",
            "filter.x.process",
            "merge.ours.driver",
            "core.pager",
            "core.sshcommand",
            "pager.log",
            "alias.st",
            "gpg.program",
            "gpg.ssh.program",
            "credential.helper",
            "credential.https://example.com.helper",
        ] {
            assert_eq!(program_override(key), Some(""), "{key}");
        }
        assert_eq!(program_override("filter.lfs.required"), Some("false"));
        assert_eq!(program_override("core.hookspath"), Some("/dev/null"));
        assert_eq!(program_override("core.fsmonitor"), Some("false"));
        for key in [
            "user.name",
            "core.bare",
            "remote.origin.url",
            "diff.renames",
            "nodot",
        ] {
            assert_eq!(program_override(key), None, "{key}");
        }
    }

    #[test]
    fn caps_long_text_on_a_character_boundary() {
        let text = "é".repeat(STDERR_LIMIT);
        let capped = capped(&text);
        assert!(capped.len() <= STDERR_LIMIT);
        assert!(text.starts_with(&capped));
    }
}
