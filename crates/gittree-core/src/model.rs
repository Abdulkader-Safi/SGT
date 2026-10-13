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

/// A path inside a repository, as the raw bytes Git stores. Git paths need
/// not be UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(pub Vec<u8>);

impl RepoPath {
    pub fn as_os_str(&self) -> &std::ffi::OsStr {
        std::os::unix::ffi::OsStrExt::from_bytes(&self.0)
    }
}

impl std::fmt::Display for RepoPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.0).fmt(f)
    }
}

impl From<&str> for RepoPath {
    fn from(path: &str) -> Self {
        Self(path.as_bytes().to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// Full name, for example `refs/heads/main`.
    pub name: String,
    pub kind: RefKind,
    /// The commit, also for an annotated tag.
    pub target: Oid,
    /// Full name of the upstream. Local branches only.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub is_current: bool,
}

impl Ref {
    /// The name without `refs/heads/`, `refs/remotes/` or `refs/tags/`.
    pub fn short_name(&self) -> &str {
        ["refs/heads/", "refs/remotes/", "refs/tags/", "refs/"]
            .iter()
            .find_map(|prefix| self.name.strip_prefix(prefix))
            .unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub oid: Oid,
    pub parents: Vec<Oid>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub commit_time: i64,
    /// The whole message.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Untracked,
}

/// One file changed by a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub change: Change,
    pub path: RepoPath,
    /// Where a renamed or copied file came from.
    pub orig_path: Option<RepoPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Text,
    Binary {
        old_size: u64,
        new_size: u64,
    },
    /// Only the file mode changed.
    ModeOnly,
    /// More lines than the app shows at once. `lines` is the full count and
    /// `hunks` holds the first part.
    TooLarge {
        lines: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The line with its ending, so CRLF files survive a rebuild.
    pub text: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// The text after the second `@@`, usually a function name.
    pub section: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: Option<RepoPath>,
    pub new_path: Option<RepoPath>,
    pub kind: DiffKind,
    /// The raw lines before the first hunk, kept to rebuild patches.
    pub header: Vec<u8>,
    pub hunks: Vec<Hunk>,
    /// "No newline at end of file" on the old side.
    pub no_newline_old: bool,
    pub no_newline_new: bool,
}
