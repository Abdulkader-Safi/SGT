//! Branches, tags, checkout and stash.

use crate::error::{GitError, GitErrorKind};
use crate::model::Oid;
use crate::runner::{Access, GitRunner};

pub(crate) fn refused(message: &str) -> GitError {
    GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: message.to_owned(),
    }
}

/// Refuses a branch or tag name Git would not take, or would read as an
/// option.
pub fn check_name(runner: &GitRunner, name: &str) -> Result<(), GitError> {
    if name.is_empty() || name.starts_with('-') {
        return Err(refused("A name cannot be empty or start with a dash."));
    }
    runner
        .read(["check-ref-format", "--branch", name])
        .map(drop)
        .map_err(|_| refused("Git does not allow this name."))
}

/// A commit, branch or tag to start from. It must not look like an option.
pub(crate) fn check_rev(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(refused("A revision cannot be empty or start with a dash."));
    }
    Ok(())
}

pub fn create_branch(
    runner: &GitRunner,
    name: &str,
    at: &str,
    checkout: bool,
) -> Result<(), GitError> {
    check_name(runner, name)?;
    check_rev(at)?;
    if checkout {
        runner.write(["switch", "-q", "-c", name, at]).map(drop)
    } else {
        runner.write(["branch", name, at]).map(drop)
    }
}

pub fn rename_branch(runner: &GitRunner, old: &str, new: &str) -> Result<(), GitError> {
    check_name(runner, old)?;
    check_name(runner, new)?;
    runner.write(["branch", "-m", old, new]).map(drop)
}

/// Deletes a local branch. Without `force` Git refuses one that is not
/// merged.
pub fn delete_branch(runner: &GitRunner, name: &str, force: bool) -> Result<(), GitError> {
    check_name(runner, name)?;
    let flag = if force { "-D" } else { "-d" };
    runner.write(["branch", flag, name]).map(drop)
}

/// The commits only this branch has: gone from every branch once it is
/// deleted. Newest first, as (ID, summary).
pub fn unmerged_commits(runner: &GitRunner, branch: &str) -> Result<Vec<(Oid, String)>, GitError> {
    check_name(runner, branch)?;
    let full = format!("refs/heads/{branch}");
    // For --branches the pattern is the name without refs/heads/.
    let exclude = format!("--exclude={branch}");
    let out = runner.read([
        "log",
        "--format=%H%x1f%s",
        &full,
        "--not",
        &exclude,
        "--branches",
        "--remotes",
    ])?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.split_once('\x1f'))
        .map(|(oid, summary)| (oid.into(), summary.to_owned()))
        .collect())
}

/// What to check out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A local branch, by short name.
    Branch(String),
    /// A remote branch such as `origin/main`. Makes a local branch that
    /// tracks it.
    Remote(String),
    /// A commit, leaving HEAD detached.
    Commit(Oid),
}

pub fn checkout(runner: &GitRunner, target: &Target) -> Result<(), GitError> {
    let args: Vec<&str> = match target {
        Target::Branch(name) => {
            check_name(runner, name)?;
            vec!["switch", "-q", name]
        }
        Target::Remote(name) => {
            check_rev(name)?;
            vec!["switch", "-q", "--track", name]
        }
        Target::Commit(oid) => {
            check_rev(oid)?;
            vec!["switch", "-q", "--detach", oid]
        }
    };
    runner.write(args).map(drop)
}

/// A lightweight tag, or an annotated one when there is a message.
pub fn create_tag(
    runner: &GitRunner,
    name: &str,
    at: &str,
    message: Option<&str>,
) -> Result<(), GitError> {
    check_name(runner, name)?;
    check_rev(at)?;
    match message.filter(|message| !message.trim().is_empty()) {
        Some(message) => runner.run(
            Access::Write,
            ["tag", "-a", "-F", "-", name, at],
            Some(message.as_bytes()),
            None,
            |_| Ok(()),
        ),
        None => runner.write(["tag", name, at]).map(drop),
    }
}

pub fn delete_tag(runner: &GitRunner, name: &str) -> Result<(), GitError> {
    check_name(runner, name)?;
    runner.write(["tag", "-d", name]).map(drop)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// Position: `stash@{index}`.
    pub index: u32,
    pub oid: Oid,
    pub message: String,
    pub time: i64,
    /// The commit it was made on.
    pub base: Oid,
}

/// Every stash, newest first. No stash is not an error.
pub fn list_stashes(runner: &GitRunner) -> Result<Vec<Stash>, GitError> {
    if runner
        .read(["rev-parse", "-q", "--verify", "refs/stash"])
        .is_err()
    {
        return Ok(Vec::new());
    }
    let out = runner.read(["log", "-g", "--format=%H%x1f%gs%x1f%ct%x1f%P", "refs/stash"])?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let mut fields = line.split('\x1f');
            Some(Stash {
                index: index as u32,
                oid: fields.next()?.into(),
                message: fields.next()?.to_owned(),
                time: fields.next()?.parse().unwrap_or(0),
                base: fields.next()?.split(' ').next()?.into(),
            })
        })
        .collect())
}

pub fn stash_push(
    runner: &GitRunner,
    message: &str,
    include_untracked: bool,
) -> Result<(), GitError> {
    let mut args = vec!["stash", "push", "-q"];
    args.extend(include_untracked.then_some("--include-untracked"));
    if !message.trim().is_empty() {
        args.extend(["-m", message]);
    }
    runner.write(args).map(drop)
}

fn stash(runner: &GitRunner, verb: &str, index: u32) -> Result<(), GitError> {
    let name = format!("stash@{{{index}}}");
    runner.write(["stash", verb, "-q", &name]).map(drop)
}

/// Puts a stash back and keeps it.
pub fn stash_apply(runner: &GitRunner, index: u32) -> Result<(), GitError> {
    stash(runner, "apply", index)
}

/// Puts a stash back and removes it. On a conflict Git keeps the stash.
pub fn stash_pop(runner: &GitRunner, index: u32) -> Result<(), GitError> {
    stash(runner, "pop", index)
}

/// Makes a branch named `name` at the commit the stash was made on, checks
/// it out and applies the stash there. Git drops the stash only once it
/// applied cleanly (FR-STASH-02).
pub fn stash_branch(runner: &GitRunner, name: &str, index: u32) -> Result<(), GitError> {
    check_name(runner, name)?;
    let stash = format!("stash@{{{index}}}");
    runner.write(["stash", "branch", name, &stash]).map(drop)
}

/// Sets the branch `upstream` tracks, such as `origin/main`, or stops it
/// tracking one when `upstream` is `None` (FR-BRANCH-05).
pub fn set_upstream(
    runner: &GitRunner,
    branch: &str,
    upstream: Option<&str>,
) -> Result<(), GitError> {
    check_name(runner, branch)?;
    match upstream {
        Some(upstream) => {
            check_rev(upstream)?;
            let flag = format!("--set-upstream-to={upstream}");
            runner.write(["branch", "-q", &flag, branch]).map(drop)
        }
        None => runner
            .write(["branch", "--unset-upstream", branch])
            .map(drop),
    }
}

pub fn stash_drop(runner: &GitRunner, index: u32) -> Result<(), GitError> {
    stash(runner, "drop", index)
}
