//! Merge, rebase, cherry-pick, revert and reset, the operation in progress,
//! and resolving conflicted files.

use std::path::Path;

use crate::error::{GitError, GitErrorKind};
use crate::model::RepoPath;
use crate::ops::read;
use crate::runner::GitRunner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// A merge, rebase, cherry-pick or revert that has stopped part way, also
/// one started outside the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InProgressOp {
    pub kind: OpKind,
    /// Current and total step. Rebase only.
    pub step: Option<(u32, u32)>,
    /// What is being merged, or rebased onto, for the banner.
    pub onto: Option<String>,
    /// Files still in conflict.
    pub conflicted: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

fn check_rev(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(GitError {
            kind: GitErrorKind::Other,
            args: Vec::new(),
            exit_code: None,
            stderr: "A revision cannot be empty or start with a dash.".into(),
        });
    }
    Ok(())
}

/// Git reports a conflict on stdout and with exit code 1, which the runner
/// cannot tell from other failures. If the repository is left with an
/// operation in progress, the failure was a conflict.
fn or_conflict(
    runner: &GitRunner,
    git_dir: &Path,
    result: Result<Vec<u8>, GitError>,
) -> Result<(), GitError> {
    match result {
        Ok(_) => Ok(()),
        Err(mut error) => {
            if in_progress(runner, git_dir).is_some() {
                error.kind = GitErrorKind::Conflict;
            }
            Err(error)
        }
    }
}

pub fn merge(
    runner: &GitRunner,
    git_dir: &Path,
    branch: &str,
    no_ff: bool,
) -> Result<(), GitError> {
    check_rev(branch)?;
    let mut args = vec!["merge", "--no-edit"];
    args.extend(no_ff.then_some("--no-ff"));
    args.push(branch);
    or_conflict(runner, git_dir, runner.write(args))
}

pub fn rebase(runner: &GitRunner, git_dir: &Path, onto: &str) -> Result<(), GitError> {
    check_rev(onto)?;
    or_conflict(runner, git_dir, runner.write(["rebase", onto]))
}

/// Skips the commit a rebase stopped on.
pub fn rebase_skip(runner: &GitRunner, git_dir: &Path) -> Result<(), GitError> {
    or_conflict(runner, git_dir, runner.write(["rebase", "--skip"]))
}

fn pick(
    runner: &GitRunner,
    git_dir: &Path,
    verb: &str,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    check_rev(oid)?;
    let parent = mainline.map(|n| n.to_string());
    let mut args = vec![verb, "--no-edit"];
    if let Some(parent) = &parent {
        args.extend(["-m", parent]);
    }
    args.push(oid);
    or_conflict(runner, git_dir, runner.write(args))
}

/// Copies a commit onto HEAD. A merge commit needs `mainline`: which parent
/// to diff against, counted from 1.
pub fn cherry_pick(
    runner: &GitRunner,
    git_dir: &Path,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    pick(runner, git_dir, "cherry-pick", oid, mainline)
}

pub fn revert(
    runner: &GitRunner,
    git_dir: &Path,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    pick(runner, git_dir, "revert", oid, mainline)
}

pub fn reset(runner: &GitRunner, target: &str, mode: ResetMode) -> Result<(), GitError> {
    check_rev(target)?;
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    runner.write(["reset", "-q", flag, target]).map(drop)
}

/// The operation that is stopped part way, read from the Git folder.
pub fn in_progress(runner: &GitRunner, git_dir: &Path) -> Option<InProgressOp> {
    let read = |name: &str| std::fs::read_to_string(git_dir.join(name)).ok();
    let number = |name: &str| read(name)?.trim().parse::<u32>().ok();
    let short = |text: String| {
        let text = text.trim();
        text.strip_prefix("refs/heads/").unwrap_or(text).to_owned()
    };
    let (kind, step, onto) = if git_dir.join("rebase-merge").is_dir() {
        (
            OpKind::Rebase,
            number("rebase-merge/msgnum").zip(number("rebase-merge/end")),
            read("rebase-merge/onto").map(|oid| oid.trim().chars().take(7).collect()),
        )
    } else if git_dir.join("rebase-apply").is_dir() {
        (
            OpKind::Rebase,
            number("rebase-apply/next").zip(number("rebase-apply/last")),
            None,
        )
    } else if git_dir.join("MERGE_HEAD").exists() {
        // "Merge branch 'x'" names what is coming in.
        let name = read("MERGE_MSG").and_then(|message| {
            let line = message.lines().next()?.to_owned();
            Some(line.split('\'').nth(1)?.to_owned())
        });
        (OpKind::Merge, None, name.map(short))
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        (OpKind::CherryPick, None, None)
    } else if git_dir.join("REVERT_HEAD").exists() {
        (OpKind::Revert, None, None)
    } else {
        return None;
    };
    let conflicted = read::status(runner)
        .map(|entries| entries.iter().filter(|e| e.conflict.is_some()).count() as u32)
        .unwrap_or(0);
    Some(InProgressOp {
        kind,
        step,
        onto,
        conflicted,
    })
}

/// Finishes the operation once every conflict is resolved.
pub fn continue_op(runner: &GitRunner, git_dir: &Path, kind: OpKind) -> Result<(), GitError> {
    let args: &[&str] = match kind {
        OpKind::Merge => &["commit", "-q", "--no-edit"],
        OpKind::Rebase => &["rebase", "--continue"],
        OpKind::CherryPick => &["cherry-pick", "--continue"],
        OpKind::Revert => &["revert", "--continue"],
    };
    or_conflict(runner, git_dir, runner.write(args))
}

/// Gives the operation up and puts the repository back as it was.
pub fn abort_op(runner: &GitRunner, kind: OpKind) -> Result<(), GitError> {
    let verb = match kind {
        OpKind::Merge => "merge",
        OpKind::Rebase => "rebase",
        OpKind::CherryPick => "cherry-pick",
        OpKind::Revert => "revert",
    };
    runner.write([verb, "--abort"]).map(drop)
}

fn add(runner: &GitRunner, path: &RepoPath) -> Result<(), GitError> {
    runner
        .write(["add".as_ref(), "--".as_ref(), path.as_os_str()])
        .map(drop)
}

/// Writes the resolved content of a conflicted file and stages it.
pub fn resolve_file(runner: &GitRunner, path: &RepoPath, content: &[u8]) -> Result<(), GitError> {
    if !runner.trusted() {
        return Err(GitError::untrusted());
    }
    std::fs::write(runner.dir().join(path.as_os_str()), content).map_err(|error| GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: error.to_string(),
    })?;
    add(runner, path)
}

/// Takes one whole side of a conflicted file and stages it. For binary
/// files and for files one side deleted.
pub fn resolve_side(runner: &GitRunner, path: &RepoPath, side: Side) -> Result<(), GitError> {
    let flag = match side {
        Side::Ours => "--ours",
        Side::Theirs => "--theirs",
    };
    let taken = runner.write([
        "checkout".as_ref(),
        flag.as_ref(),
        "--".as_ref(),
        path.as_os_str(),
    ]);
    match taken {
        Ok(_) => add(runner, path),
        // That side deleted the file, so resolving to it removes the file.
        Err(_) => runner
            .write([
                "rm".as_ref(),
                "-q".as_ref(),
                "--".as_ref(),
                path.as_os_str(),
            ])
            .map(drop),
    }
}
