use std::borrow::Cow;
use std::ffi::OsStr;

use crate::error::GitError;
use crate::model::{FileDiff, Oid, RepoPath};
use crate::patch::{self, Lines};
use crate::runner::{Access, GitRunner};

/// Where a partial patch goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Into the index.
    Stage,
    /// Out of the index.
    Unstage,
    /// Out of the working tree. Snapshot first.
    Discard,
}

fn with_paths<'a>(args: &[&'a str], paths: &'a [RepoPath]) -> Vec<Cow<'a, OsStr>> {
    let mut all: Vec<_> = args.iter().map(|&arg| OsStr::new(arg).into()).collect();
    all.push(OsStr::new("--").into());
    all.extend(paths.iter().map(RepoPath::as_os_str));
    all
}

/// Stages whole files, including deletions and untracked files.
pub fn stage(runner: &GitRunner, paths: &[RepoPath]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    runner.write(with_paths(&["add", "-A"], paths)).map(drop)
}

/// Unstages whole files. For a renamed file pass both names. Before the
/// first commit there is no HEAD to restore from, so the files leave the
/// index instead.
pub fn unstage(runner: &GitRunner, paths: &[RepoPath], has_commits: bool) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let args: &[&str] = if has_commits {
        &["restore", "--staged"]
    } else {
        &["rm", "--cached", "-r", "-q"]
    };
    runner.write(with_paths(args, paths)).map(drop)
}

/// Applies a patch from `patch::build`, after Git has checked that it fits.
pub fn apply_patch(runner: &GitRunner, patch: &[u8], target: Target) -> Result<(), GitError> {
    let flags: &[&str] = match target {
        Target::Stage => &["--cached"],
        Target::Unstage => &["--cached", "--reverse"],
        Target::Discard => &["--reverse"],
    };
    for check in [true, false] {
        let mut args = vec!["apply", "--whitespace=nowarn"];
        args.extend(flags);
        args.extend(check.then_some("--check"));
        // "-" reads the patch from stdin.
        args.push("-");
        runner.run(Access::Write, args, Some(patch), None, |_| Ok(()))?;
    }
    Ok(())
}

/// Stages or unstages the selected lines of a diff. Discarding goes through
/// `patch::build` and `apply_patch`, with a journal snapshot in between.
pub fn apply_selection(
    runner: &GitRunner,
    diff: &FileDiff,
    selected: &Lines,
    target: Target,
) -> Result<(), GitError> {
    match patch::build(diff, selected, target != Target::Stage) {
        Some(patch) => apply_patch(runner, &patch, target),
        None => Ok(()),
    }
}

/// Puts tracked files back to their HEAD content, in the index and on disk,
/// and deletes untracked ones. The caller snapshots first.
pub fn discard_files(
    runner: &GitRunner,
    tracked: &[RepoPath],
    untracked: &[RepoPath],
    has_commits: bool,
) -> Result<(), GitError> {
    // Checked before anything is deleted.
    if !runner.trusted() {
        return Err(GitError::untrusted());
    }
    if !tracked.is_empty() {
        let args: &[&str] = if has_commits {
            &["restore", "--source=HEAD", "--staged", "--worktree"]
        } else {
            // Nothing is committed yet, so every staged file is a new one.
            &["rm", "--cached", "-r", "-f", "-q"]
        };
        runner.write(with_paths(args, tracked))?;
    }
    for path in untracked
        .iter()
        .chain(tracked.iter().filter(|_| !has_commits))
    {
        let _ = std::fs::remove_file(runner.dir().join(path.as_os_str()));
    }
    Ok(())
}

/// Commits what is staged. The message goes in on stdin, so nothing in it
/// can be read as an option. Returns the new HEAD.
pub fn commit(runner: &GitRunner, message: &str, amend: bool) -> Result<Oid, GitError> {
    let mut args = vec!["commit", "-q", "-F", "-"];
    args.extend(amend.then_some("--amend"));
    runner.run(Access::Write, args, Some(message.as_bytes()), None, |_| {
        Ok(())
    })?;
    let oid = runner.read(["rev-parse", "HEAD"])?;
    Ok(String::from_utf8_lossy(&oid).trim_end().into())
}

/// A `.gitignore` line that matches exactly this path from the top of the
/// working tree. Characters Git reads as wildcards are escaped.
pub fn ignore_pattern(path: &RepoPath) -> String {
    let mut pattern = String::from("/");
    for c in path.to_string().chars() {
        if matches!(c, '*' | '?' | '[' | '\\' | '!' | '#') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    // A trailing space is dropped by Git unless it is escaped.
    if pattern.ends_with(' ') {
        pattern.pop();
        pattern.push_str("\\ ");
    }
    pattern
}

/// Adds a line to the `.gitignore` at the top of the working tree, making
/// the file if there is none (FR-STAGE-09). Refused when not trusted.
pub fn add_ignore(runner: &GitRunner, pattern: &str) -> Result<(), GitError> {
    use std::io::Write;

    let fail = |error: std::io::Error| GitError {
        kind: crate::error::GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: format!("Could not change .gitignore: {error}"),
    };
    if !runner.trusted() {
        return Err(GitError::untrusted());
    }
    let file = runner.dir().join(".gitignore");
    let before = std::fs::read(&file).unwrap_or_default();
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(fail)?;
    // The new line must not join the last one.
    let gap = if before.is_empty() || before.ends_with(b"\n") {
        ""
    } else {
        "\n"
    };
    writeln!(out, "{gap}{pattern}").map_err(fail)
}
