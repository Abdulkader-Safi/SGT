//! The undo journal: one entry per undoable action, and a snapshot of what
//! an action could destroy. Kept under `<common_dir>/gittree/`.
//!
//! M3 writes entries for commits and discards and can undo those. Redo, the
//! snapshot list and pruning come with M7.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{GitError, GitErrorKind};
use crate::model::{Oid, RepoPath};
use crate::runner::GitRunner;

/// An undoable action, with the inputs needed to run it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Commit {
        message: String,
        amend: bool,
    },
    /// Whole files put back to HEAD, and untracked files deleted.
    DiscardFiles {
        tracked: Vec<RepoPath>,
        untracked: Vec<RepoPath>,
    },
    /// A hunk or some lines taken out of the working tree. The patch is not
    /// written here: the snapshot's patch file holds it.
    DiscardLines {
        path: RepoPath,
        #[serde(skip)]
        patch: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadState {
    /// The full branch ref, unless HEAD is detached.
    pub symbolic: Option<String>,
    /// None before the first commit.
    pub oid: Option<Oid>,
}

/// What an action could destroy, saved before it ran. The files live in
/// `backups/<entry id>/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// `refs/gittree/snapshots/<id>`, a `git stash create` commit.
    pub tree_ref: Option<String>,
    /// `backups/<id>/discard.patch`.
    pub patch_file: Option<String>,
    /// `backups/<id>/untracked/`.
    pub untracked_dir: Option<String>,
    /// The remote branch and its old commit, for force-push.
    pub remote_ref: Option<(String, Oid)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    /// Unix seconds, UTC.
    pub time: i64,
    pub action: Action,
    /// Text for the undo tooltip.
    pub label: String,
    pub head_before: HeadState,
    pub head_after: HeadState,
    /// Only the refs the action touched. None means the ref did not exist.
    pub refs_before: Vec<(String, Option<Oid>)>,
    pub refs_after: Vec<(String, Option<Oid>)>,
    pub snapshot: Option<Snapshot>,
    pub undone: bool,
}

/// An entry whose action has not finished yet.
#[derive(Debug)]
pub struct PendingEntry(JournalEntry);

/// Why an entry cannot be undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    AlreadyUndone,
    /// HEAD or a ref moved since the action, outside the app.
    RepositoryChanged,
}

#[derive(Debug, Clone)]
pub struct Journal {
    dir: PathBuf,
}

fn io_error(error: std::io::Error) -> GitError {
    GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: error.to_string(),
    }
}

fn text(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).trim_end().to_owned()
}

fn ref_value(runner: &GitRunner, name: &str) -> Option<Oid> {
    runner
        .read(["rev-parse", "-q", "--verify", name])
        .ok()
        .map(|out| text(out).into())
}

fn head_state(runner: &GitRunner) -> HeadState {
    HeadState {
        symbolic: runner.read(["symbolic-ref", "-q", "HEAD"]).ok().map(text),
        oid: ref_value(runner, "HEAD"),
    }
}

/// A random version 4 UUID.
fn new_id() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        hex(&b[..4]),
        hex(&b[4..6]),
        hex(&b[6..8]),
        hex(&b[8..10]),
        hex(&b[10..])
    ))
}

impl Journal {
    pub fn new(common_dir: &Path) -> Self {
        Self {
            dir: common_dir.join("gittree"),
        }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("journal.jsonl")
    }

    fn backup_dir(&self, id: &str) -> PathBuf {
        self.dir.join("backups").join(id)
    }

    /// Every entry, oldest first. A line that does not parse is skipped. So
    /// is one whose ID is not a UUID, since the ID names a folder.
    pub fn entries(&self) -> Vec<JournalEntry> {
        let Ok(text) = std::fs::read_to_string(self.file()) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let entry: Option<JournalEntry> = serde_json::from_str(line).ok();
                let entry = entry.filter(|entry| {
                    entry.id.len() == 36
                        && entry.id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                });
                if entry.is_none() {
                    tracing::warn!("skipped a journal line that does not parse");
                }
                entry
            })
            .collect()
    }

    fn write_all(&self, entries: &[JournalEntry]) -> Result<(), GitError> {
        let mut out = Vec::new();
        for entry in entries {
            serde_json::to_writer(&mut out, entry).map_err(|e| io_error(e.into()))?;
            out.push(b'\n');
        }
        let temp = self.dir.join("journal.jsonl.tmp");
        std::fs::write(&temp, out)
            .and_then(|()| std::fs::rename(&temp, self.file()))
            .map_err(io_error)
    }

    /// Records the state before an action and saves whatever the action
    /// could destroy. Call it right before running the action.
    pub fn begin(
        &self,
        runner: &GitRunner,
        action: Action,
        label: String,
    ) -> Result<PendingEntry, GitError> {
        // A snapshot writes into the repository, so trust comes first.
        if !runner.trusted() {
            return Err(GitError::untrusted());
        }
        let id = new_id().map_err(io_error)?;
        let head_before = head_state(runner);
        let touched: Vec<String> = match &action {
            Action::Commit { .. } => head_before.symbolic.iter().cloned().collect(),
            Action::DiscardFiles { .. } | Action::DiscardLines { .. } => Vec::new(),
        };
        let refs_before = touched
            .into_iter()
            .map(|name| {
                let value = ref_value(runner, &name);
                (name, value)
            })
            .collect();

        let backup = self.backup_dir(&id);
        let snapshot = match &action {
            Action::Commit { .. } => None,
            Action::DiscardLines { patch, .. } => {
                std::fs::create_dir_all(&backup).map_err(io_error)?;
                std::fs::write(backup.join("discard.patch"), patch).map_err(io_error)?;
                Some(Snapshot {
                    patch_file: Some(format!("backups/{id}/discard.patch")),
                    ..Default::default()
                })
            }
            Action::DiscardFiles { tracked, untracked } => {
                let mut snapshot = Snapshot::default();
                let mut copied: Vec<&RepoPath> = untracked.iter().collect();
                if head_before.oid.is_none() {
                    // No commit yet, so there is nothing to stash against.
                    // The files come back untracked.
                    copied.extend(tracked);
                } else if !tracked.is_empty() {
                    let stash = text(runner.write(["stash", "create"])?);
                    if !stash.is_empty() {
                        let name = format!("refs/gittree/snapshots/{id}");
                        runner.write(["update-ref", &name, &stash])?;
                        snapshot.tree_ref = Some(name);
                    }
                }
                if !copied.is_empty() {
                    let dir = backup.join("untracked");
                    for path in copied {
                        copy_file(
                            &runner.dir().join(path.as_os_str()),
                            &dir.join(path.as_os_str()),
                        )
                        .map_err(io_error)?;
                    }
                    snapshot.untracked_dir = Some(format!("backups/{id}/untracked"));
                }
                Some(snapshot)
            }
        };

        Ok(PendingEntry(JournalEntry {
            id,
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
            action,
            label,
            head_after: head_before.clone(),
            head_before,
            refs_after: Vec::new(),
            refs_before,
            snapshot,
            undone: false,
        }))
    }

    /// Records the state after the action and appends the entry.
    pub fn finish(
        &self,
        runner: &GitRunner,
        pending: PendingEntry,
    ) -> Result<JournalEntry, GitError> {
        let mut entry = pending.0;
        entry.head_after = head_state(runner);
        entry.refs_after = entry
            .refs_before
            .iter()
            .map(|(name, _)| (name.clone(), ref_value(runner, name)))
            .collect();
        std::fs::create_dir_all(&self.dir).map_err(io_error)?;
        let mut line = serde_json::to_vec(&entry).map_err(|e| io_error(e.into()))?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.file())
            .and_then(|mut file| file.write_all(&line))
            .map_err(io_error)?;
        Ok(entry)
    }

    /// Drops the snapshot of an action that failed.
    pub fn abandon(&self, runner: &GitRunner, pending: PendingEntry) {
        let _ = std::fs::remove_dir_all(self.backup_dir(&pending.0.id));
        if let Some(name) = pending.0.snapshot.and_then(|snapshot| snapshot.tree_ref) {
            let _ = runner.write(["update-ref", "-d", &name]);
        }
    }

    /// Undo is allowed only while HEAD and every ref the action touched are
    /// where the action left them.
    pub fn can_undo(&self, runner: &GitRunner, entry: &JournalEntry) -> Result<(), Blocked> {
        if entry.undone {
            return Err(Blocked::AlreadyUndone);
        }
        let same = head_state(runner) == entry.head_after
            && entry
                .refs_after
                .iter()
                .all(|(name, value)| &ref_value(runner, name) == value);
        if same {
            Ok(())
        } else {
            Err(Blocked::RepositoryChanged)
        }
    }

    /// Takes an action back and marks its entry undone. Check `can_undo`
    /// first.
    pub fn undo(&self, runner: &GitRunner, entry: &JournalEntry) -> Result<(), GitError> {
        let backup = self.backup_dir(&entry.id);
        let snapshot = entry.snapshot.clone().unwrap_or_default();
        match &entry.action {
            Action::Commit { .. } => match &entry.head_before.oid {
                Some(oid) => runner.write(["reset", "--soft", oid]).map(drop)?,
                // The first commit: back to a branch with no commit.
                None => runner.write(["update-ref", "-d", "HEAD"]).map(drop)?,
            },
            Action::DiscardLines { .. } => {
                let patch = std::fs::read(backup.join("discard.patch")).map_err(io_error)?;
                // Reversing the discard is the same as unstaging, but in the
                // working tree, so the patch is applied forward there.
                apply_forward(runner, &patch)?;
            }
            Action::DiscardFiles { tracked, .. } => {
                if let Some(tree) = &snapshot.tree_ref {
                    // The stash commit holds the working tree. Its second
                    // parent holds the index.
                    let index = format!("--source={tree}^2");
                    let worktree = format!("--source={tree}");
                    for path in tracked {
                        let args = |source: &'_ str, flags: &[&'static str]| {
                            let mut args: Vec<std::ffi::OsString> =
                                vec!["restore".into(), source.into()];
                            args.extend(flags.iter().map(Into::into));
                            args.push("--".into());
                            args.push(path.as_os_str().into());
                            args
                        };
                        runner.write(args(&worktree, &["--staged", "--worktree"]))?;
                        runner.write(args(&index, &["--staged"]))?;
                    }
                }
                if snapshot.untracked_dir.is_some() {
                    restore_files(&backup.join("untracked"), runner.dir()).map_err(io_error)?;
                }
            }
        }
        let mut entries = self.entries();
        for saved in entries.iter_mut().filter(|saved| saved.id == entry.id) {
            saved.undone = true;
        }
        self.write_all(&entries)
    }
}

fn apply_forward(runner: &GitRunner, patch: &[u8]) -> Result<(), GitError> {
    for check in [true, false] {
        let mut args = vec!["apply", "--whitespace=nowarn"];
        args.extend(check.then_some("--check"));
        args.push("-");
        runner.run(crate::Access::Write, args, Some(patch), None, |_| Ok(()))?;
    }
    Ok(())
}

/// Copies one file, or recreates a symbolic link, making folders as needed.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::symlink_metadata(from)?.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
    } else {
        std::fs::copy(from, to).map(drop)
    }
}

/// Copies a backup folder back into the working tree. A file whose folder
/// resolves to somewhere outside the working tree is refused.
fn restore_files(from: &Path, root: &Path) -> std::io::Result<()> {
    let root = root.canonicalize()?;
    let mut folders = vec![from.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for item in std::fs::read_dir(&folder)? {
            let item = item?.path();
            if item.is_dir() && !item.is_symlink() {
                folders.push(item);
                continue;
            }
            let to = root.join(item.strip_prefix(from).map_err(std::io::Error::other)?);
            let parent = to.parent().unwrap_or(&root);
            std::fs::create_dir_all(parent)?;
            if !parent.canonicalize()?.starts_with(&root) {
                return Err(std::io::Error::other("backup path leaves the working tree"));
            }
            let _ = std::fs::remove_file(&to);
            copy_file(&item, &to)?;
        }
    }
    Ok(())
}
