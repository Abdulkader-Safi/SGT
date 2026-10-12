use std::path::PathBuf;
use std::sync::Arc;

/// A Git object ID as lowercase hex.
pub type Oid = Arc<str>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: Oid,
    /// First parent first.
    pub parents: Vec<Oid>,
    pub author_name: Arc<str>,
    pub author_email: Arc<str>,
    /// Unix seconds, UTC.
    pub author_time: i64,
    pub summary: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// On a branch, by short name.
    Branch(String),
    Detached(Oid),
    /// On a branch that has no commit yet.
    Unborn(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Working tree root, canonical.
    pub path: PathBuf,
    pub git_dir: PathBuf,
    /// Shared by all worktrees. The journal lives under it.
    pub common_dir: PathBuf,
    pub trusted: bool,
    pub head: Head,
    pub object_format: ObjectFormat,
}

impl Repository {
    /// The folder name, for titles and the recent list.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map_or_else(
                || self.path.to_string_lossy(),
                |name| name.to_string_lossy(),
            )
            .into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitVersion {
    pub path: PathBuf,
    pub major: u32,
    pub minor: u32,
    /// What `git --version` printed, for example "git version 2.55.0".
    pub text: String,
}

/// One finished Git call, for the activity log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRecord {
    /// Unix seconds, UTC.
    pub time: i64,
    /// Passwords in URLs are removed.
    pub args: Vec<String>,
    pub duration_ms: u32,
    /// -1 when the process was killed.
    pub exit_code: i32,
    /// Capped at 8 KB.
    pub stderr: String,
}
