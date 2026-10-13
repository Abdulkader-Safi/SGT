use std::io::{self, BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::GitError;
use crate::model::{
    Change, Commit, CommitDetails, DiffKind, DiffLine, FileChange, FileDiff, Head, Hunk, LineKind,
    Oid, Ref, RepoPath, StatusEntry,
};
use crate::parse::diff::parse_file_diff;
use crate::parse::log::{LOG_FORMAT, LogParser};
use crate::parse::refs::{REF_FORMAT, parse_refs};
use crate::parse::status::parse_status;
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
        diff.kind = DiffKind::Binary {
            old_size: blob_size(runner, &format!("{oid}^"), &diff.old_path),
            new_size: blob_size(runner, oid, &diff.new_path),
        };
    }
    Ok(cut_large(diff, load_all))
}

/// The size of `rev:path`, or 0 when there is no such file. An empty `rev`
/// reads the index.
fn blob_size(runner: &GitRunner, rev: &str, path: &Option<RepoPath>) -> u64 {
    let Some(path) = path else { return 0 };
    let mut spec = format!("{rev}:").into_bytes();
    spec.extend_from_slice(&path.0);
    let spec: &std::ffi::OsStr = std::os::unix::ffi::OsStrExt::from_bytes(&spec);
    runner
        .read(["cat-file".as_ref(), "-s".as_ref(), spec])
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out).trim().parse().ok())
        .unwrap_or(0)
}

/// Cuts a very large diff to its first `LARGE_DIFF_SHOWN` lines.
fn cut_large(mut diff: FileDiff, load_all: bool) -> FileDiff {
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
    diff
}

/// Every changed, staged or untracked file.
pub fn status(runner: &GitRunner) -> Result<Vec<StatusEntry>, GitError> {
    let out = runner.read(["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    Ok(parse_status(&out))
}

/// The diff of one changed file: HEAD against the index when `staged`, else
/// the index against the working tree. An untracked file is read from disk
/// and shown as all added.
pub fn diff_worktree(
    runner: &GitRunner,
    entry: &StatusEntry,
    staged: bool,
    load_all: bool,
) -> Result<FileDiff, GitError> {
    if !staged && entry.worktree == Change::Untracked {
        return Ok(cut_large(untracked_diff(runner, &entry.path), load_all));
    }
    let mut args: Vec<&std::ffi::OsStr> =
        ["diff", "-M", "--no-color", "--no-ext-diff", "--no-textconv"]
            .iter()
            .map(|arg| arg.as_ref())
            .collect();
    if staged {
        args.push("--cached".as_ref());
    }
    args.push("--".as_ref());
    // A rename only shows in the index. Both names, or Git cannot see it.
    let orig_path = entry.orig_path.as_ref().filter(|_| staged);
    args.extend(orig_path.map(RepoPath::as_os_str));
    args.push(entry.path.as_os_str());

    let mut diff = parse_file_diff(&runner.read(args)?);
    let change = if staged { entry.index } else { entry.worktree };
    diff.old_path = (change != Change::Added).then(|| orig_path.unwrap_or(&entry.path).clone());
    diff.new_path = (change != Change::Deleted).then(|| entry.path.clone());

    if matches!(diff.kind, DiffKind::Binary { .. }) {
        let on_disk = || {
            std::fs::symlink_metadata(runner.dir().join(entry.path.as_os_str()))
                .map_or(0, |meta| meta.len())
        };
        diff.kind = if staged {
            DiffKind::Binary {
                old_size: blob_size(runner, "HEAD", &diff.old_path),
                new_size: blob_size(runner, "", &diff.new_path),
            }
        } else {
            DiffKind::Binary {
                old_size: blob_size(runner, "", &diff.old_path),
                new_size: on_disk(),
            }
        };
    }
    Ok(cut_large(diff, load_all))
}

/// Builds the diff Git would print for a new file, from the file on disk.
/// A file that cannot be read gives an empty diff.
fn untracked_diff(runner: &GitRunner, path: &RepoPath) -> FileDiff {
    use std::os::unix::fs::PermissionsExt;

    let full = runner.dir().join(path.as_os_str());
    let meta = std::fs::symlink_metadata(&full).ok();
    let is_link = meta.as_ref().is_some_and(|meta| meta.is_symlink());
    let content = if is_link {
        std::fs::read_link(&full)
            .map(|target| std::os::unix::ffi::OsStringExt::into_vec(target.into_os_string()))
            .unwrap_or_default()
    } else {
        std::fs::read(&full).unwrap_or_default()
    };
    let mode = match &meta {
        _ if is_link => "120000",
        Some(meta) if meta.permissions().mode() & 0o111 != 0 => "100755",
        _ => "100644",
    };
    let name = quote_path(b"a/", &path.0);
    let mut header = b"diff --git ".to_vec();
    header.extend_from_slice(&name);
    header.push(b' ');
    header.extend_from_slice(&quote_path(b"b/", &path.0));
    header.extend_from_slice(format!("\nnew file mode {mode}\n--- /dev/null\n+++ ").as_bytes());
    header.extend_from_slice(&quote_path(b"b/", &path.0));
    header.push(b'\n');

    let mut diff = FileDiff {
        old_path: None,
        new_path: Some(path.clone()),
        kind: DiffKind::Text,
        header,
        hunks: Vec::new(),
        no_newline_old: false,
        no_newline_new: false,
    };
    // Git calls a file binary when a NUL shows in its first 8,000 bytes.
    if content.iter().take(8000).any(|&b| b == 0) {
        diff.kind = DiffKind::Binary {
            old_size: 0,
            new_size: content.len() as u64,
        };
        return diff;
    }
    let lines: Vec<DiffLine> = content
        .split_inclusive(|&b| b == b'\n')
        .zip(1..)
        .map(|(text, number)| DiffLine {
            kind: LineKind::Add,
            old_no: None,
            new_no: Some(number),
            text: text.to_vec(),
        })
        .collect();
    if !lines.is_empty() {
        diff.no_newline_new = !content.ends_with(b"\n");
        diff.hunks.push(Hunk {
            old_start: 0,
            old_len: 0,
            new_start: 1,
            new_len: lines.len() as u32,
            section: String::new(),
            lines,
        });
    }
    diff
}

/// A path as Git writes it in a patch header: in double quotes with C-style
/// escapes when it holds anything but plain printable ASCII.
pub fn quote_path(prefix: &[u8], path: &[u8]) -> Vec<u8> {
    let plain = |b: u8| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\';
    let mut out = Vec::with_capacity(prefix.len() + path.len() + 2);
    if path.iter().all(|&b| plain(b)) {
        out.extend_from_slice(prefix);
        out.extend_from_slice(path);
        return out;
    }
    out.push(b'"');
    out.extend_from_slice(prefix);
    for &b in path {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            _ if plain(b) => out.push(b),
            _ => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
        }
    }
    out.push(b'"');
    out
}
