use std::io::{self, BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::GitError;
use crate::model::{
    Change, Commit, CommitDetails, DiffKind, FileChange, FileDiff, Head, Oid, Ref, RepoPath,
};
use crate::parse::diff::parse_file_diff;
use crate::parse::log::{LOG_FORMAT, LogParser};
use crate::parse::refs::{REF_FORMAT, parse_refs};
use crate::runner::{Access, GitRunner};

pub const LOG_BATCH: usize = 2_000;

const LOG_ARGS: [&str; 6] = [
    "log",
    // The app's snapshot refs must not show up as history.
    "--exclude=refs/gittree/*",
    "--all",
    "--date-order",
    "-z",
    LOG_FORMAT,
];

/// Streams every commit reachable from any ref, newest first, in batches of
/// `LOG_BATCH`. Setting `cancel` stops it with `Cancelled`.
pub fn stream_log(
    runner: &GitRunner,
    mut on_batch: impl FnMut(Vec<Commit>),
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    runner.run(Access::Read, LOG_ARGS, None, Some(cancel), |stdout| {
        let mut stdout = BufReader::with_capacity(1 << 16, stdout);
        let mut parser = LogParser::default();
        let mut record = Vec::new();
        let mut batch = Vec::with_capacity(LOG_BATCH);
        loop {
            // Checked here as well as by the runner, so no batch is
            // delivered after the caller cancels.
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("cancelled"));
            }
            record.clear();
            if stdout.read_until(0, &mut record)? == 0 {
                break;
            }
            if record.last() == Some(&0) {
                record.pop();
            }
            if let Some(commit) = parser.parse(&record) {
                batch.push(commit);
                if batch.len() == LOG_BATCH {
                    on_batch(std::mem::replace(&mut batch, Vec::with_capacity(LOG_BATCH)));
                }
            }
        }
        if !batch.is_empty() {
            on_batch(batch);
        }
        Ok(())
    })
}

/// Where HEAD points: a branch, a branch with no commit yet, or a commit.
pub fn head(runner: &GitRunner) -> Result<Head, GitError> {
    match runner.read(["symbolic-ref", "-q", "HEAD"]) {
        Ok(name) => {
            let name = String::from_utf8_lossy(&name);
            let name = name.trim_end();
            let name = name.strip_prefix("refs/heads/").unwrap_or(name).to_owned();
            let born = runner.read(["rev-parse", "-q", "--verify", "HEAD"]).is_ok();
            Ok(if born {
                Head::Branch(name)
            } else {
                Head::Unborn(name)
            })
        }
        // Exit code 1 with -q means HEAD is not a symbolic ref.
        Err(error) if error.exit_code == Some(1) => {
            let oid = runner.read(["rev-parse", "--verify", "HEAD"])?;
            Ok(Head::Detached(
                String::from_utf8_lossy(&oid).trim_end().into(),
            ))
        }
        Err(error) => Err(error),
    }
}

/// Branches, remote branches, tags and the stash, in Git's order.
pub fn list_refs(runner: &GitRunner) -> Result<Vec<Ref>, GitError> {
    Ok(parse_refs(&runner.read(["for-each-ref", REF_FORMAT])?))
}

/// The full message, committer and dates of one commit.
pub fn commit_details(runner: &GitRunner, oid: &Oid) -> Result<CommitDetails, GitError> {
    let out = runner.read([
        "show",
        "-s",
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%B",
        oid,
    ])?;
    let text = String::from_utf8_lossy(&out);
    let mut fields = text.splitn(9, '\x1f');
    let mut next = || fields.next().unwrap_or_default();
    Ok(CommitDetails {
        oid: next().into(),
        parents: next().split_ascii_whitespace().map(Oid::from).collect(),
        author_name: next().to_owned(),
        author_email: next().to_owned(),
        author_time: next().parse().unwrap_or(0),
        committer_name: next().to_owned(),
        committer_email: next().to_owned(),
        commit_time: next().parse().unwrap_or(0),
        message: next().trim_end().to_owned(),
    })
}

/// The files a commit changed. A merge is compared with its first parent.
pub fn commit_files(runner: &GitRunner, oid: &Oid) -> Result<Vec<FileChange>, GitError> {
    let out = runner.read([
        "diff-tree",
        "-r",
        "-z",
        "--name-status",
        "-M",
        "--root",
        "--no-commit-id",
        "--diff-merges=first-parent",
        oid,
    ])?;
    let mut fields = out.split(|&b| b == 0);
    let mut files = Vec::new();
    while let Some(status) = fields.next().filter(|status| !status.is_empty()) {
        let change = match status[0] {
            b'A' => Change::Added,
            b'D' => Change::Deleted,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'T' => Change::TypeChange,
            _ => Change::Modified,
        };
        let mut path = || fields.next().map(|path| RepoPath(path.to_vec()));
        // A rename or copy lists the old path, then the new one.
        let orig_path = matches!(change, Change::Renamed | Change::Copied)
            .then(&mut path)
            .flatten();
        let Some(path) = path() else { break };
        files.push(FileChange {
            change,
            path,
            orig_path,
        });
    }
    Ok(files)
}

/// A diff longer than this is cut to `LARGE_DIFF_SHOWN` lines until the user
/// asks for all of it.
pub const LARGE_DIFF_LINES: usize = 20_000;
pub const LARGE_DIFF_SHOWN: usize = 2_000;

/// The diff of one file in a commit, against the first parent.
pub fn diff_commit_file(
    runner: &GitRunner,
    oid: &Oid,
    file: &FileChange,
    load_all: bool,
) -> Result<FileDiff, GitError> {
    let mut args: Vec<&std::ffi::OsStr> = [
        "show",
        "--format=",
        "--patch",
        "-M",
        "--diff-merges=first-parent",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
    ]
    .iter()
    .map(|arg| arg.as_ref())
    .collect();
    args.push(oid.as_ref().as_ref());
    args.push("--".as_ref());
    // Both names, or Git cannot see that the file was renamed.
    args.extend(file.orig_path.as_ref().map(RepoPath::as_os_str));
    args.push(file.path.as_os_str());

    let mut diff = parse_file_diff(&runner.read(args)?);
    diff.old_path = match file.change {
        Change::Added => None,
        _ => Some(file.orig_path.clone().unwrap_or_else(|| file.path.clone())),
    };
    diff.new_path = (file.change != Change::Deleted).then(|| file.path.clone());

    if matches!(diff.kind, DiffKind::Binary { .. }) {
        let size = |rev: String, path: &Option<RepoPath>| -> u64 {
            let Some(path) = path else { return 0 };
            let mut spec = rev.into_bytes();
            spec.push(b':');
            spec.extend_from_slice(&path.0);
            let spec: &std::ffi::OsStr = std::os::unix::ffi::OsStrExt::from_bytes(&spec);
            runner
                .read(["cat-file".as_ref(), "-s".as_ref(), spec])
                .ok()
                .and_then(|out| String::from_utf8_lossy(&out).trim().parse().ok())
                .unwrap_or(0)
        };
        diff.kind = DiffKind::Binary {
            old_size: size(format!("{oid}^"), &diff.old_path),
            new_size: size(oid.to_string(), &diff.new_path),
        };
    }
    let lines: usize = diff.hunks.iter().map(|hunk| hunk.lines.len()).sum();
    if lines > LARGE_DIFF_LINES && !load_all {
        let mut room = LARGE_DIFF_SHOWN;
        diff.hunks.retain_mut(|hunk| {
            hunk.lines.truncate(room);
            room -= hunk.lines.len();
            !hunk.lines.is_empty()
        });
        diff.kind = DiffKind::TooLarge { lines };
    }
    Ok(diff)
}
