//! Remotes, and the four commands that use the network: clone, fetch, pull
//! and push.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::error::{GitError, GitErrorKind};
use crate::runner::GitRunner;
use crate::store::PullMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    /// Equals `fetch_url` unless set on its own.
    pub push_url: String,
}

/// What Git says it is doing, from a line such as
/// "Receiving objects:  62% (12480/20112), 18.4 MiB | 3.1 MiB/s".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub phase: String,
    pub percent: Option<u8>,
}

pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.strip_prefix("remote: ").unwrap_or(line);
    let (phase, rest) = line.split_once(':')?;
    let percent = rest
        .split('%')
        .next()
        .filter(|_| rest.contains('%'))
        .and_then(|number| number.trim().parse().ok());
    // Only Git's counting lines, not error text with a colon in it.
    let counting = percent.is_some() || rest.trim_start().starts_with(|c: char| c.is_ascii_digit());
    (counting && !phase.contains(' ') || percent.is_some()).then(|| Progress {
        phase: phase.trim().to_owned(),
        percent,
    })
}

/// Extra environment for a network call: the askpass helper.
pub type NetEnv = Vec<(String, String)>;

fn refused(message: &str) -> GitError {
    GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: message.to_owned(),
    }
}

fn check_name(name: &str) -> Result<(), GitError> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
    ok.then_some(())
        .ok_or_else(|| refused("This name is not allowed."))
}

/// A URL Git may be given: HTTPS, SSH, the `user@host:path` form, or a
/// local path. The `ext::` and `fd::` transports run programs and are
/// refused, as is anything Git would read as an option.
pub fn check_url(url: &str) -> Result<(), GitError> {
    let bad = url.is_empty()
        || url.starts_with('-')
        || url.contains("::")
        || url.chars().any(char::is_control);
    let scheme = url.split_once("://").map(|(scheme, _)| scheme);
    let known = match scheme {
        Some(scheme) => matches!(scheme, "https" | "ssh" | "file" | "http" | "git"),
        None => true,
    };
    (!bad && known)
        .then_some(())
        .ok_or_else(|| refused("This URL is not allowed."))
}

pub fn list_remotes(runner: &GitRunner) -> Result<Vec<Remote>, GitError> {
    let names = runner.read(["remote"])?;
    let mut remotes = Vec::new();
    for name in String::from_utf8_lossy(&names).lines() {
        let url = |push: bool| {
            let mut args = vec!["remote", "get-url"];
            args.extend(push.then_some("--push"));
            args.push(name);
            runner
                .read(args)
                .map(|out| String::from_utf8_lossy(&out).trim().to_owned())
                .unwrap_or_default()
        };
        remotes.push(Remote {
            name: name.to_owned(),
            fetch_url: url(false),
            push_url: url(true),
        });
    }
    Ok(remotes)
}

pub fn add_remote(runner: &GitRunner, name: &str, url: &str) -> Result<(), GitError> {
    check_name(name)?;
    check_url(url)?;
    runner.write(["remote", "add", name, url]).map(drop)
}

pub fn rename_remote(runner: &GitRunner, old: &str, new: &str) -> Result<(), GitError> {
    check_name(old)?;
    check_name(new)?;
    runner.write(["remote", "rename", old, new]).map(drop)
}

pub fn set_remote_url(runner: &GitRunner, name: &str, url: &str) -> Result<(), GitError> {
    check_name(name)?;
    check_url(url)?;
    runner.write(["remote", "set-url", name, url]).map(drop)
}

pub fn remove_remote(runner: &GitRunner, name: &str) -> Result<(), GitError> {
    check_name(name)?;
    runner.write(["remote", "remove", name]).map(drop)
}

fn network(
    runner: &GitRunner,
    args: Vec<&str>,
    env: &NetEnv,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    runner.run_network(args, env, cancel, |line| {
        if let Some(progress) = parse_progress(line) {
            on_progress(progress);
        }
    })
}

/// Fetches every remote and drops remote branches that are gone.
pub fn fetch(
    runner: &GitRunner,
    env: &NetEnv,
    cancel: &AtomicBool,
    on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    network(
        runner,
        vec!["fetch", "--all", "--prune", "--progress"],
        env,
        cancel,
        on_progress,
    )
}

pub fn pull(
    runner: &GitRunner,
    mode: PullMode,
    env: &NetEnv,
    cancel: &AtomicBool,
    on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    let how = match mode {
        PullMode::FfOnly => "--ff-only",
        PullMode::Merge => "--no-rebase",
        PullMode::Rebase => "--rebase",
    };
    network(
        runner,
        vec!["pull", "--progress", how],
        env,
        cancel,
        on_progress,
    )
}

/// Pushes a branch. `set_upstream` publishes a branch that has none.
/// `lease` force-pushes, but only if the remote branch is still at that
/// commit: Git refuses when someone else pushed meanwhile.
pub fn push(
    runner: &GitRunner,
    remote: &str,
    branch: &str,
    set_upstream: bool,
    lease: Option<&str>,
    env: &NetEnv,
    cancel: &AtomicBool,
    on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    check_name(remote)?;
    check_name(branch)?;
    let lease = lease.map(|oid| format!("--force-with-lease={branch}:{oid}"));
    let mut args = vec!["push", "--progress"];
    args.extend(set_upstream.then_some("-u"));
    args.extend(lease.as_deref());
    args.extend([remote, branch]);
    network(runner, args, env, cancel, on_progress)
}

/// Pushes one tag, or with `delete` removes it on the remote.
pub fn push_tag(
    runner: &GitRunner,
    remote: &str,
    tag: &str,
    delete: bool,
    env: &NetEnv,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    check_name(remote)?;
    check_name(tag)?;
    let spec = if delete {
        format!(":refs/tags/{tag}")
    } else {
        format!("refs/tags/{tag}")
    };
    network(runner, vec!["push", remote, &spec], env, cancel, |_| {})
}

/// Clones into `dest`. `runner` must be a trusted runner in the folder that
/// will hold it. On failure or cancel the partial folder is removed.
pub fn clone(
    runner: &GitRunner,
    url: &str,
    dest: &Path,
    env: &NetEnv,
    cancel: &AtomicBool,
    on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    check_url(url)?;
    if dest.exists() && std::fs::read_dir(dest).is_ok_and(|mut dir| dir.next().is_some()) {
        return Err(refused("The destination folder is not empty."));
    }
    let made_here = !dest.exists();
    let target = dest.to_string_lossy();
    let result = network(
        runner,
        vec!["clone", "--progress", "--", url, &target],
        env,
        cancel,
        on_progress,
    );
    if result.is_err() && made_here {
        let _ = std::fs::remove_dir_all(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_progress_lines() {
        let read = |line: &str| parse_progress(line).map(|p| (p.phase, p.percent));
        assert_eq!(
            read("Receiving objects:  62% (12480/20112), 18.4 MiB | 3.1 MiB/s"),
            Some(("Receiving objects".into(), Some(62)))
        );
        assert_eq!(
            read("remote: Counting objects: 100% (5/5), done."),
            Some(("Counting objects".into(), Some(100)))
        );
        assert_eq!(read("fatal: Could not read from remote repository."), None);
        assert_eq!(read("To github.com:a/b.git"), None);
    }

    #[test]
    fn refuses_urls_that_run_programs_or_look_like_options() {
        for url in [
            "https://example.com/a.git",
            "ssh://git@example.com/a.git",
            "git@example.com:a/b.git",
            "/Users/me/repo",
            "file:///Users/me/repo",
        ] {
            assert!(check_url(url).is_ok(), "{url}");
        }
        for url in [
            "",
            "--upload-pack=x",
            "ext::sh -c x",
            "fd::3",
            "ftp://x/y",
            "a\nb",
        ] {
            assert!(check_url(url).is_err(), "{url:?}");
        }
    }
}
