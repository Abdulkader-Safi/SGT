use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::error::{GitError, GitErrorKind};
use crate::model::{GitVersion, ObjectFormat, Repository};
use crate::ops::read::head;
use crate::runner::GitRunner;
use crate::store::TrustList;

/// The oldest Git the app works with.
pub const MIN_GIT: (u32, u32) = (2, 39);

/// Finds Git and checks its version. `GITTREE_GIT` wins, then `custom` (the
/// path from settings), then the `git` on `PATH`.
pub fn find_git(custom: Option<&Path>) -> Result<GitVersion, GitError> {
    let path = std::env::var_os("GITTREE_GIT")
        .map(PathBuf::from)
        .or_else(|| custom.map(Path::to_owned))
        .unwrap_or_else(|| "git".into());
    let runner = GitRunner::new(&path, "/", false);
    let failed = |kind, text: String| GitError {
        kind,
        args: vec!["--version".into()],
        exit_code: None,
        stderr: text,
    };

    // Whatever goes wrong here, the answer for the user is the same: this
    // is not a Git the app can use.
    let out = runner
        .read(["--version"])
        .map_err(|e| failed(GitErrorKind::GitNotFound, e.stderr))?;
    let text = String::from_utf8_lossy(&out).trim().to_owned();
    let Some((major, minor)) = parse_version(&text) else {
        return Err(failed(GitErrorKind::GitNotFound, text));
    };
    if (major, minor) < MIN_GIT {
        return Err(failed(GitErrorKind::GitTooOld, text));
    }
    Ok(GitVersion {
        path,
        major,
        minor,
        text,
    })
}

/// Reads "git version 2.39.5 (Apple Git-154)" as (2, 39).
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text.strip_prefix("git version ")?.split(['.', ' ']);
    Some((numbers.next()?.parse().ok()?, numbers.next()?.parse().ok()?))
}

/// Opens the repository that contains `path`. It is trusted only if its
/// working tree root is on the trust list.
pub fn open(git: &Path, path: &Path, trust: &TrustList) -> Result<Repository, GitError> {
    // Read as untrusted: nothing is known about this folder yet.
    let probe = GitRunner::new(git, path, false);
    let out = probe.read([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-dir",
        "--git-common-dir",
        "--show-object-format",
    ])?;
    let mut lines = out.split(|&b| b == b'\n');
    let mut next_path = || PathBuf::from(OsStr::from_bytes(lines.next().unwrap_or_default()));
    let path = next_path();
    let path = path.canonicalize().unwrap_or(path);
    let git_dir = next_path();
    let common_dir = next_path();
    let object_format = match lines.next() {
        Some(b"sha256") => ObjectFormat::Sha256,
        _ => ObjectFormat::Sha1,
    };
    let trusted = trust.contains(&path);
    let head = head(&GitRunner::new(git, &path, trusted))?;
    Ok(Repository {
        path,
        git_dir,
        common_dir,
        trusted,
        head,
        object_format,
    })
}

/// Makes `path` a repository and trusts it, because the app created it. A
/// folder that already is a repository is opened as it is, with no change to
/// its trust. The caller saves the trust list.
pub fn init(git: &Path, path: &Path, trust: &mut TrustList) -> Result<Repository, GitError> {
    if !path.join(".git").exists() {
        GitRunner::new(git, path, true).write(["init"])?;
        trust.insert(&path.canonicalize().unwrap_or_else(|_| path.to_owned()));
    }
    open(git, path, trust)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_versions() {
        assert_eq!(parse_version("git version 2.55.0"), Some((2, 55)));
        assert_eq!(
            parse_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39))
        );
        assert_eq!(parse_version("git version 2.39"), Some((2, 39)));
        assert_eq!(parse_version("not git"), None);
        assert!((2, 38) < MIN_GIT && (2, 39) >= MIN_GIT && (3, 0) >= MIN_GIT);
    }
}
