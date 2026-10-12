use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{GitError, GitErrorKind};
use crate::model::Commit;
use crate::parse::log::{LOG_FORMAT, LogParser};
use crate::runner::GitRunner;

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
/// `LOG_BATCH`. Checks `cancel` once per commit and kills Git when it is set.
pub fn stream_log(
    runner: &GitRunner,
    mut on_batch: impl FnMut(Vec<Commit>),
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    let error = |kind, exit_code, stderr| GitError {
        kind,
        args: LOG_ARGS.iter().map(|arg| arg.to_string()).collect(),
        exit_code,
        stderr,
    };

    let mut child = runner
        .read_command(LOG_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // ponytail: stderr is read after stdout ends. Fine for `git log`,
        // whose errors fit in the pipe buffer. Drain it on a thread if a
        // call can ever write more than 64 KB of errors.
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| error(GitErrorKind::GitNotFound, None, e.to_string()))?;

    let mut stdout = BufReader::with_capacity(1 << 16, child.stdout.take().expect("piped"));
    let mut parser = LogParser::default();
    let mut record = Vec::new();
    let mut batch = Vec::with_capacity(LOG_BATCH);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error(GitErrorKind::Cancelled, None, String::new()));
        }
        record.clear();
        let read = stdout
            .read_until(0, &mut record)
            .map_err(|e| error(GitErrorKind::Other, None, e.to_string()))?;
        if read == 0 {
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

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let status = child
        .wait()
        .map_err(|e| error(GitErrorKind::Other, None, e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(error(GitErrorKind::Other, status.code(), stderr))
    }
}
