//! macOS-specific code. Nothing outside this module may assume macOS.

use std::path::PathBuf;

/// Where the log file goes: `~/Library/Logs/Safi Git Tree`.
pub fn log_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Logs").join(crate::APP_NAME))
}
