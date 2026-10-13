//! The undo journal: one entry per undoable action, and a snapshot of what
//! an action could destroy. Kept under `<common_dir>/gittree/`.
//!
//! M3 writes entries for commits and discards and can undo those. Redo, the
//! snapshot list and pruning come with M7.

use std::io::Write;
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
    /// What was checked out, in words for the label.
    Checkout {
        target: String,
    },
    CreateBranch {
        name: String,
        at: String,
        checkout: bool,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    DeleteBranch {
        name: String,
        force: bool,
    },
    CreateTag {
        name: String,
        at: String,
        message: Option<String>,
    },
    DeleteTag {
        name: String,
    },
    StashPush {
        message: String,
        include_untracked: bool,
    },
    /// The dropped stash commit, so undo can store it again.
    StashDrop {
        oid: Oid,
        message: String,
    },
    Merge {
        branch: String,
        no_ff: bool,
    },
    Rebase {
        onto: String,
    },
    CherryPick {
        oid: String,
        mainline: Option<u32>,
    },
    Revert {
        oid: String,
        mainline: Option<u32>,
    },
    Reset {
        target: String,
        mode: crate::ops::merge::ResetMode,
    },
    /// A force-push. It cannot be undone from here, but the commit the
    /// remote branch had is kept as a snapshot.
    ForcePush {
        remote: String,
        branch: String,
        replaced: Oid,
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
    /// The action changed a remote, which cannot be taken back from here.
    NotUndoable,
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
    getrandom::fill(&mut b).map_err(|error| std::io::Error::other(error.to_string()))?;
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
        let head_ref = |name: &String| format!("refs/heads/{name}");
        let touched: Vec<String> = match &action {
            Action::Commit { .. } => head_before.symbolic.iter().cloned().collect(),
            Action::DiscardFiles { .. } | Action::DiscardLines { .. } | Action::Checkout { .. } => {
                Vec::new()
            }
            Action::CreateBranch { name, .. } | Action::DeleteBranch { name, .. } => {
                vec![head_ref(name)]
            }
            Action::RenameBranch { old, new } => vec![head_ref(old), head_ref(new)],
            Action::CreateTag { name, .. } | Action::DeleteTag { name } => {
                vec![format!("refs/tags/{name}")]
            }
            Action::StashPush { .. } | Action::StashDrop { .. } => vec!["refs/stash".into()],
            Action::Merge { .. }
            | Action::Rebase { .. }
            | Action::CherryPick { .. }
            | Action::Revert { .. }
            | Action::Reset { .. } => head_before.symbolic.iter().cloned().collect(),
            Action::ForcePush { .. } => Vec::new(),
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
            Action::Commit { .. }
            | Action::Checkout { .. }
            | Action::CreateBranch { .. }
            | Action::RenameBranch { .. }
            | Action::DeleteBranch { .. }
            | Action::CreateTag { .. }
            | Action::DeleteTag { .. }
            | Action::StashPush { .. }
            | Action::StashDrop { .. }
            | Action::Merge { .. }
            | Action::CherryPick { .. }
            | Action::Revert { .. } => None,
            Action::ForcePush {
                remote,
                branch,
                replaced,
            } => {
                // A ref of its own keeps the replaced commit from being
                // cleaned up.
                let name = format!("refs/gittree/snapshots/{id}");
                runner.write(["update-ref", &name, replaced])?;
                Some(Snapshot {
                    remote_ref: Some((format!("{remote}/{branch}"), replaced.clone())),
                    ..Default::default()
                })
            }
            // Both can throw away or rewrite work, so the index and the
            // tracked changes are saved first.
            Action::Rebase { .. } | Action::Reset { .. } => {
                let stash = text(runner.write(["stash", "create"])?);
                if stash.is_empty() {
                    None
                } else {
                    let name = format!("refs/gittree/snapshots/{id}");
                    runner.write(["update-ref", &name, &stash])?;
                    Some(Snapshot {
                        tree_ref: Some(name),
                        ..Default::default()
                    })
                }
            }
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

    fn parked_file(&self) -> PathBuf {
        self.dir.join("in-progress.json")
    }

    /// Keeps the entry of an action that stopped on a conflict. It is
    /// finished when the operation is continued to its end.
    pub fn park(&self, pending: PendingEntry) -> Result<(), GitError> {
        std::fs::create_dir_all(&self.dir).map_err(io_error)?;
        let json = serde_json::to_vec(&pending.0).map_err(|e| io_error(e.into()))?;
        std::fs::write(self.parked_file(), json).map_err(io_error)
    }

    /// The parked entry, if there is one. It is removed from disk.
    pub fn take_parked(&self) -> Option<PendingEntry> {
        let json = std::fs::read(self.parked_file()).ok()?;
        let _ = std::fs::remove_file(self.parked_file());
        serde_json::from_slice(&json).ok().map(PendingEntry)
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
        if matches!(entry.action, Action::ForcePush { .. }) {
            return Err(Blocked::NotUndoable);
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
            Action::Checkout { .. } => self.switch_back(runner, entry)?,
            Action::CreateBranch { name, checkout, .. } => {
                if *checkout {
                    self.switch_back(runner, entry)?;
                }
                runner.write(["branch", "-D", name]).map(drop)?;
            }
            Action::RenameBranch { old, new } => {
                runner.write(["branch", "-m", new, old]).map(drop)?
            }
            // Back to what each ref pointed at before.
            Action::DeleteBranch { .. } | Action::DeleteTag { .. } => {
                for (name, before) in &entry.refs_before {
                    if let Some(oid) = before {
                        runner.write(["update-ref", name, oid]).map(drop)?;
                    }
                }
            }
            Action::CreateTag { name, .. } => runner.write(["tag", "-d", name]).map(drop)?,
            Action::StashPush { .. } => runner.write(["stash", "pop", "-q"]).map(drop)?,
            Action::StashDrop { oid, message } => runner
                .write(["stash", "store", "-m", message, oid])
                .map(drop)?,
            Action::ForcePush { .. } => {}
            Action::Merge { .. }
            | Action::Rebase { .. }
            | Action::CherryPick { .. }
            | Action::Revert { .. }
            | Action::Reset { .. } => {
                if let Some(oid) = &entry.head_before.oid {
                    runner.write(["reset", "-q", "--hard", oid]).map(drop)?;
                }
                if let Some(tree) = &snapshot.tree_ref {
                    runner
                        .write(["stash", "apply", "-q", "--index", tree])
                        .map(drop)?;
                }
            }
        }
        self.mark_undone(entry)
    }

    /// Where HEAD was before the action: back on its branch, or detached.
    fn switch_back(&self, runner: &GitRunner, entry: &JournalEntry) -> Result<(), GitError> {
        let before = &entry.head_before;
        match (&before.symbolic, &before.oid) {
            (Some(name), _) => {
                let short = name.strip_prefix("refs/heads/").unwrap_or(name);
                runner.write(["switch", "-q", short]).map(drop)
            }
            (None, Some(oid)) => runner.write(["switch", "-q", "--detach", oid]).map(drop),
            (None, None) => Ok(()),
        }
    }

    fn mark_undone(&self, entry: &JournalEntry) -> Result<(), GitError> {
        self.mark(entry, true)
    }

    fn mark(&self, entry: &JournalEntry, undone: bool) -> Result<(), GitError> {
        let mut entries = self.entries();
        for saved in entries.iter_mut().filter(|saved| saved.id == entry.id) {
            saved.undone = undone;
        }
        self.write_all(&entries)
    }

    /// The entry Undo would take back: the newest one not undone yet.
    pub fn next_undo(&self) -> Option<JournalEntry> {
        self.entries().into_iter().rev().find(|entry| !entry.undone)
    }

    /// The entry Redo would run again: the oldest of the undone entries at
    /// the end of the journal. A new action after an undo ends that run.
    pub fn next_redo(&self) -> Option<JournalEntry> {
        let entries = self.entries();
        let run = entries
            .iter()
            .rev()
            .take_while(|entry| entry.undone)
            .count();
        entries.into_iter().rev().nth(run.checked_sub(1)?)
    }

    /// Redo is allowed only while the repository is where the undo left it.
    pub fn can_redo(&self, runner: &GitRunner, entry: &JournalEntry) -> bool {
        entry.undone
            && head_state(runner) == entry.head_before
            && entry
                .refs_before
                .iter()
                .all(|(name, value)| &ref_value(runner, name) == value)
    }

    /// Runs an undone action again from its recorded inputs.
    pub fn redo(
        &self,
        runner: &GitRunner,
        git_dir: &Path,
        entry: &JournalEntry,
    ) -> Result<(), GitError> {
        use crate::ops::{branch, merge, stage};
        let has_commits = entry.head_before.oid.is_some();
        match &entry.action {
            // The commit the action made still exists, so redo points the
            // branch back at it. Running the action again would make a new
            // commit with a new time and ID.
            Action::Commit { message, amend } => match &entry.head_after.oid {
                Some(oid) => runner.write(["reset", "-q", "--soft", oid]).map(drop)?,
                None => stage::commit(runner, message, *amend).map(drop)?,
            },
            Action::DiscardFiles { tracked, untracked } => {
                stage::discard_files(runner, tracked, untracked, has_commits)?
            }
            Action::DiscardLines { .. } => {
                let patch = std::fs::read(self.backup_dir(&entry.id).join("discard.patch"))
                    .map_err(io_error)?;
                stage::apply_patch(runner, &patch, stage::Target::Discard)?;
            }
            // Back to where the checkout ended up.
            Action::Checkout { .. } => match (&entry.head_after.symbolic, &entry.head_after.oid) {
                (Some(name), _) => {
                    let short = name.strip_prefix("refs/heads/").unwrap_or(name);
                    runner.write(["switch", "-q", short]).map(drop)?
                }
                (None, Some(oid)) => runner.write(["switch", "-q", "--detach", oid]).map(drop)?,
                (None, None) => {}
            },
            Action::CreateBranch { name, at, checkout } => {
                branch::create_branch(runner, name, at, *checkout)?
            }
            Action::RenameBranch { old, new } => branch::rename_branch(runner, old, new)?,
            // It was confirmed the first time, unmerged commits and all.
            Action::DeleteBranch { name, .. } => branch::delete_branch(runner, name, true)?,
            Action::CreateTag { name, at, message } => {
                branch::create_tag(runner, name, at, message.as_deref())?
            }
            Action::DeleteTag { name } => branch::delete_tag(runner, name)?,
            Action::StashPush {
                message,
                include_untracked,
            } => branch::stash_push(runner, message, *include_untracked)?,
            Action::StashDrop { oid, .. } => {
                let stashes = branch::list_stashes(runner)?;
                if let Some(stash) = stashes.iter().find(|stash| &stash.oid == oid) {
                    branch::stash_drop(runner, stash.index)?;
                }
            }
            // The same holds here, and it also spares a second round of
            // conflicts.
            Action::Merge { .. }
            | Action::Rebase { .. }
            | Action::CherryPick { .. }
            | Action::Revert { .. }
                if entry.head_after.oid.is_some() =>
            {
                let oid = entry.head_after.oid.as_deref().unwrap_or_default();
                runner.write(["reset", "-q", "--hard", oid]).map(drop)?
            }
            Action::Merge { branch, no_ff } => merge::merge(runner, git_dir, branch, *no_ff)?,
            Action::Rebase { onto } => merge::rebase(runner, git_dir, onto)?,
            Action::CherryPick { oid, mainline } => {
                merge::cherry_pick(runner, git_dir, oid, *mainline)?
            }
            Action::Revert { oid, mainline } => merge::revert(runner, git_dir, oid, *mainline)?,
            Action::Reset { target, mode } => merge::reset(runner, target, *mode)?,
            Action::ForcePush { .. } => {}
        }
        self.mark(entry, false)
    }

    /// The entries that saved something, newest first, for the snapshot
    /// list.
    pub fn snapshots(&self) -> Vec<JournalEntry> {
        let mut entries: Vec<JournalEntry> = self
            .entries()
            .into_iter()
            .filter(|entry| entry.snapshot.is_some())
            .collect();
        entries.reverse();
        entries
    }

    /// Brings back what a snapshot saved. With `as_branch` the saved state
    /// becomes a new branch and the working tree is left alone, which is
    /// the only way to restore a commit a force-push replaced.
    pub fn restore(
        &self,
        runner: &GitRunner,
        entry: &JournalEntry,
        as_branch: Option<&str>,
    ) -> Result<(), GitError> {
        let snapshot = entry.snapshot.clone().unwrap_or_default();
        let backup = self.backup_dir(&entry.id);
        let saved = snapshot
            .tree_ref
            .clone()
            .or_else(|| snapshot.remote_ref.as_ref().map(|(_, oid)| oid.to_string()));
        if let Some(name) = as_branch {
            let Some(saved) = saved else {
                return Err(GitError {
                    kind: GitErrorKind::Other,
                    args: Vec::new(),
                    exit_code: None,
                    stderr: "This snapshot holds no commit to make a branch from.".into(),
                });
            };
            return crate::ops::branch::create_branch(runner, name, &saved, false);
        }
        if let Some(tree) = &snapshot.tree_ref {
            runner
                .write(["stash", "apply", "-q", "--index", tree])
                .map(drop)?;
        }
        if snapshot.patch_file.is_some() {
            let patch = std::fs::read(backup.join("discard.patch")).map_err(io_error)?;
            apply_forward(runner, &patch)?;
        }
        if snapshot.untracked_dir.is_some() {
            restore_files(&backup.join("untracked"), runner.dir()).map_err(io_error)?;
        }
        Ok(())
    }

    /// Removes entries older than 30 days or beyond the newest 200, with
    /// their snapshot refs and backup folders. Run when a repository opens.
    pub fn prune(&self, runner: &GitRunner, now: i64) -> Result<usize, GitError> {
        const KEEP: usize = 200;
        const MAX_AGE: i64 = 30 * 24 * 60 * 60;
        let entries = self.entries();
        let cut = entries.len().saturating_sub(KEEP);
        let (mut kept, mut dropped) = (Vec::new(), Vec::new());
        for (ix, entry) in entries.into_iter().enumerate() {
            if ix < cut || now - entry.time > MAX_AGE {
                dropped.push(entry);
            } else {
                kept.push(entry);
            }
        }
        if dropped.is_empty() {
            return Ok(0);
        }
        for entry in &dropped {
            let _ = std::fs::remove_dir_all(self.backup_dir(&entry.id));
            let name = format!("refs/gittree/snapshots/{}", entry.id);
            if runner.trusted() && ref_value(runner, &name).is_some() {
                let _ = runner.write(["update-ref", "-d", &name]);
            }
        }
        self.write_all(&kept)?;
        Ok(dropped.len())
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
        let target = std::fs::read_link(from)?;
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, to);
        // ponytail: a link to a folder is recreated as a file link, and
        // Windows refuses both without developer mode.
        #[cfg(windows)]
        return std::os::windows::fs::symlink_file(target, to);
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
