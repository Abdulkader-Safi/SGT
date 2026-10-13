//! Worktrees: more working folders on one repository, each on its own
//! branch (FR-AGENT-04).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::branch::{check_name, check_rev, refused};
use crate::error::GitError;
use crate::model::{Oid, os_str};
use crate::runner::GitRunner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// `None` in a bare repository's entry.
    pub head: Option<Oid>,
    /// The short branch name, or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The repository's first working tree, which cannot be removed.
    pub main: bool,
    pub locked: bool,
}

/// Every worktree, the main one first.
pub fn list(runner: &GitRunner) -> Result<Vec<Worktree>, GitError> {
    let out = runner.read(["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse(&out))
}

/// Reads `git worktree list --porcelain -z`: NUL-ended "key value" lines,
/// with an empty one after each worktree.
fn parse(out: &[u8]) -> Vec<Worktree> {
    let mut found: Vec<Worktree> = Vec::new();
    let mut open = false;
    for line in out.split(|&b| b == 0) {
        if line.is_empty() {
            open = false;
            continue;
        }
        let space = line.iter().position(|&b| b == b' ').unwrap_or(line.len());
        let (key, value) = (&line[..space], line.get(space + 1..).unwrap_or_default());
        if key == b"worktree" {
            found.push(Worktree {
                path: PathBuf::from(&*os_str(value)),
                head: None,
                branch: None,
                main: found.is_empty(),
                locked: false,
            });
            open = true;
            continue;
        }
        let Some(tree) = found.last_mut().filter(|_| open) else {
            continue;
        };
        match key {
            b"HEAD" => tree.head = Some(String::from_utf8_lossy(value).as_ref().into()),
            b"branch" => {
                let name = String::from_utf8_lossy(value);
                tree.branch = Some(name.strip_prefix("refs/heads/").unwrap_or(&name).to_owned());
            }
            b"locked" => tree.locked = true,
            _ => {}
        }
    }
    found
}

/// A worktree folder is named in full, so it can never read as an option.
fn check_path(path: &Path) -> Result<(), GitError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(refused("A worktree folder needs its full path."))
    }
}

/// Makes a worktree at `path` on a new branch that starts at `start`.
pub fn add(runner: &GitRunner, path: &Path, branch: &str, start: &str) -> Result<(), GitError> {
    check_path(path)?;
    check_name(runner, branch)?;
    check_rev(start)?;
    runner
        .write([
            OsStr::new("worktree"),
            "add".as_ref(),
            "-b".as_ref(),
            branch.as_ref(),
            path.as_os_str(),
            start.as_ref(),
        ])
        .map(drop)
}

/// Removes a worktree. Git refuses while it has changed or untracked
/// files, and this never forces it, so no work is lost. Its branch stays.
pub fn remove(runner: &GitRunner, path: &Path) -> Result<(), GitError> {
    check_path(path)?;
    runner
        .write([OsStr::new("worktree"), "remove".as_ref(), path.as_os_str()])
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_branches_detached_heads_and_locks() {
        let out = b"worktree /repo\0HEAD aaa\0branch refs/heads/main\0\0\
worktree /repo wt/one\0HEAD bbb\0detached\0locked\0\0\
worktree /wt/two\0HEAD ccc\0branch refs/heads/agent/x\0locked by hand\0\0";
        let found = parse(out);
        let brief: Vec<_> = found
            .iter()
            .map(|w| {
                (
                    w.path.to_str().unwrap(),
                    w.branch.as_deref(),
                    w.main,
                    w.locked,
                )
            })
            .collect();
        assert_eq!(
            brief,
            [
                ("/repo", Some("main"), true, false),
                ("/repo wt/one", None, false, true),
                ("/wt/two", Some("agent/x"), false, true),
            ]
        );
        assert_eq!(found[0].head.as_deref(), Some("aaa"));
    }
}
