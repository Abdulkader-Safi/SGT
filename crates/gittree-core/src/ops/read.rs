use std::io::{self, BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::GitError;
use crate::model::{Commit, Head};
use crate::parse::log::{LOG_FORMAT, LogParser};
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
