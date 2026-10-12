use std::fs::OpenOptions;
use std::sync::Mutex;

use tracing::Level;

const LOG_FILE: &str = "gittree.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Sends log records to the log file. The level comes from `GITTREE_LOG`
/// (`error`, `warn`, `info` or `debug`) and is `info` when unset. Without a
/// writable log folder the app runs with no log.
pub fn init() {
    let Some(dir) = crate::platform::log_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(LOG_FILE);
    // ponytail: one old file is kept. Use a rolling appender if anyone ever
    // needs more history than 5 MB.
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let level = std::env::var("GITTREE_LOG")
        .ok()
        .and_then(|level| level.parse::<Level>().ok())
        .unwrap_or(Level::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "{} started",
        crate::APP_NAME
    );
}
