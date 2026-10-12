use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Starts `git` for one repository with a fixed environment.
///
/// M0 only reads, and treats every repository as untrusted. The trust list,
/// write calls, activity records and cancellation of any call come in M1.
#[derive(Debug, Clone)]
pub struct GitRunner {
    git: PathBuf,
    repo: PathBuf,
}

impl GitRunner {
    /// Uses `GITTREE_GIT` when set, otherwise the `git` found on `PATH`.
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        let git = std::env::var_os("GITTREE_GIT").map_or_else(|| "git".into(), PathBuf::from);
        Self {
            git,
            repo: repo.into(),
        }
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    /// A read-only Git call. The caller sets up stdio and spawns it.
    pub fn read_command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.git);
        command
            .current_dir(&self.repo)
            .args(UNTRUSTED_OVERRIDES)
            .args(args)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_PAGER", "cat")
            .env("GIT_OPTIONAL_LOCKS", "0");
        command
    }
}
