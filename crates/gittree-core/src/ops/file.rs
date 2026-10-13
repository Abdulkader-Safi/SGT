//! One file over time: who last changed each line, and the commits that
//! touched it (FR-AGENT-03).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::Arc;

use crate::error::GitError;
use crate::model::{Change, Commit, FileChange, Oid, RepoPath};
use crate::parse::log::LogParser;
use crate::runner::GitRunner;

/// One line of a file with the commit that last changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub oid: Oid,
    pub author: Arc<str>,
    /// Unix seconds, UTC.
    pub time: i64,
    pub summary: Arc<str>,
    /// Without the line ending.
    pub text: Vec<u8>,
}

/// Blames `path` as it is in `rev`.
pub fn blame(runner: &GitRunner, rev: &str, path: &RepoPath) -> Result<Vec<BlameLine>, GitError> {
    let out = runner.read([
        OsStr::new("blame"),
        "--porcelain".as_ref(),
        rev.as_ref(),
        "--".as_ref(),
        &path.as_os_str(),
    ])?;
    Ok(parse_blame(&out))
}

/// Reads `git blame --porcelain`. Each line has a header starting with the
/// commit ID, and the line itself after a tab. A commit's details are only
/// printed the first time it shows up.
fn parse_blame(out: &[u8]) -> Vec<BlameLine> {
    struct Seen {
        author: Arc<str>,
        time: i64,
        summary: Arc<str>,
    }
    let mut seen: HashMap<Oid, Seen> = HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<Oid> = None;
    for raw in out.split(|&b| b == b'\n') {
        if let Some(text) = raw.strip_prefix(b"\t") {
            let Some(oid) = current.take() else { continue };
            let commit = &seen[&oid];
            lines.push(BlameLine {
                author: commit.author.clone(),
                time: commit.time,
                summary: commit.summary.clone(),
                text: text.strip_suffix(b"\r").unwrap_or(text).to_vec(),
                oid,
            });
            continue;
        }
        let text = String::from_utf8_lossy(raw);
        let (key, value) = text.split_once(' ').unwrap_or((&text, ""));
        match (&current, key) {
            (None, oid) if !oid.is_empty() => {
                let oid: Oid = oid.into();
                seen.entry(oid.clone()).or_insert_with(|| Seen {
                    author: "".into(),
                    time: 0,
                    summary: "".into(),
                });
                current = Some(oid);
            }
            (Some(oid), "author") => seen.get_mut(oid).unwrap().author = value.into(),
            (Some(oid), "author-time") => {
                seen.get_mut(oid).unwrap().time = value.parse().unwrap_or(0)
            }
            (Some(oid), "summary") => seen.get_mut(oid).unwrap().summary = value.into(),
            _ => {}
        }
    }
    lines
}

/// A commit that touched the file, and what it did to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommit {
    pub commit: Commit,
    /// The file as that commit names it, with the old name of a rename.
    pub file: FileChange,
}

/// The commits reachable from `rev` that changed `path`, newest first,
/// following the file across renames. At most `limit` of them.
pub fn history(
    runner: &GitRunner,
    rev: &str,
    path: &RepoPath,
    limit: usize,
) -> Result<Vec<FileCommit>, GitError> {
    let out = runner.read([
        OsStr::new("log"),
        "--follow".as_ref(),
        "-z".as_ref(),
        "--name-status".as_ref(),
        "-M".as_ref(),
        format!("--max-count={limit}").as_ref(),
        "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f".as_ref(),
        rev.as_ref(),
        "--".as_ref(),
        &path.as_os_str(),
    ])?;
    Ok(parse_history(&out))
}

/// Reads the log above: a record starts with the record separator, its six
/// fields each end with the unit separator, and the name and status of the
/// file follow as NUL-ended words.
fn parse_history(out: &[u8]) -> Vec<FileCommit> {
    let mut parser = LogParser::default();
    out.split(|&b| b == 0x1e)
        .filter_map(|record| {
            let end = record.iter().rposition(|&b| b == 0x1f)?;
            let commit = parser.parse(&record[..end])?;
            let mut words = record[end + 1..]
                .split(|&b| b == 0)
                .map(|word| word.strip_prefix(b"\n").unwrap_or(word))
                .filter(|word| !word.is_empty());
            let status = words.next()?;
            let first = RepoPath(words.next()?.to_vec());
            let (change, path, orig_path) = match status.first()? {
                b'R' => (
                    Change::Renamed,
                    RepoPath(words.next()?.to_vec()),
                    Some(first),
                ),
                b'C' => (
                    Change::Copied,
                    RepoPath(words.next()?.to_vec()),
                    Some(first),
                ),
                b'A' => (Change::Added, first, None),
                b'D' => (Change::Deleted, first, None),
                b'T' => (Change::TypeChange, first, None),
                _ => (Change::Modified, first, None),
            };
            Some(FileCommit {
                commit,
                file: FileChange {
                    change,
                    path,
                    orig_path,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_reuses_a_commit_it_has_already_described() {
        let out = b"aaaa 1 1 2\nauthor Ada\nauthor-time 100\nsummary First\nfilename f\n\tone\n\
bbbb 2 2 1\nauthor Grace\nauthor-time 200\nsummary Second one\nprevious aaaa f\nfilename f\n\ttwo\r\n\
aaaa 3 3\n\t\tthree\n";
        let lines = parse_blame(out);
        let brief: Vec<_> = lines
            .iter()
            .map(|l| (&*l.oid, &*l.author, l.time, &*l.summary, &l.text[..]))
            .collect();
        assert_eq!(
            brief,
            [
                ("aaaa", "Ada", 100, "First", &b"one"[..]),
                ("bbbb", "Grace", 200, "Second one", b"two"),
                // A line that starts with a tab keeps it.
                ("aaaa", "Ada", 100, "First", b"\tthree"),
            ]
        );
    }

    #[test]
    fn history_reads_renames_and_plain_changes() {
        let out = b"\x1eb2\x1fa1\x1fAda\x1fa@x\x1f20\x1fMove it\x1f\0\nR100\0old.rs\0new.rs\0\
\x1ea1\x1f\x1fAda\x1fa@x\x1f10\x1fStart\x1f\0\nA\0old.rs\0";
        let found = parse_history(out);
        assert_eq!(found.len(), 2);
        assert_eq!(&*found[0].commit.summary, "Move it");
        assert_eq!(
            found[0].file,
            FileChange {
                change: Change::Renamed,
                path: "new.rs".into(),
                orig_path: Some("old.rs".into()),
            }
        );
        assert_eq!(
            (found[1].file.change, found[1].file.path.to_string()),
            (Change::Added, "old.rs".to_owned())
        );
    }
}
