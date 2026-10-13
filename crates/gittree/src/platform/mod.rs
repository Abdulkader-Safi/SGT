//! Code that differs between macOS, Windows and Linux. Nothing outside
//! this module may assume one of them.

use std::path::PathBuf;

/// macOS draws the toolbar under its title bar. Windows and Linux keep the
/// system title bar, with its own buttons.
pub const MAC: bool = cfg!(target_os = "macos");

/// The command the Git check suggests when Git is missing.
// ponytail: one Linux command. Other distributions have their own.
pub const GIT_INSTALL: &str = if MAC {
    "xcode-select --install"
} else if cfg!(windows) {
    "winget install Git.Git"
} else {
    "sudo apt install git"
};

/// Where the log file goes: `~/Library/Logs/Safi Git Tree` on macOS, and a
/// `logs` folder in the app's local data folder elsewhere.
pub fn log_dir() -> Option<PathBuf> {
    if MAC {
        Some(dirs::home_dir()?.join("Library/Logs").join(crate::APP_NAME))
    } else {
        Some(dirs::data_local_dir()?.join(crate::APP_NAME).join("logs"))
    }
}

/// A shortcut hint as this system writes it. Hints are written the macOS
/// way ("⇧⌘O") and become "Ctrl+Shift+O" elsewhere, where the bindings use
/// Ctrl. Text without those symbols comes back unchanged.
pub fn shortcut(hint: &str) -> String {
    if MAC {
        return hint.to_owned();
    }
    spelled_out(hint)
}

fn spelled_out(hint: &str) -> String {
    let mut out = String::new();
    for (symbol, name) in [('⌘', "Ctrl+"), ('⌥', "Alt+"), ('⇧', "Shift+")] {
        if hint.contains(symbol) {
            out.push_str(name);
        }
    }
    out.extend(hint.chars().filter(|c| !"⌘⌥⇧".contains(*c)));
    out.replace('↵', "Enter")
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcuts_are_spelled_out_with_ctrl_first() {
        assert_eq!(super::spelled_out("⇧⌘O"), "Ctrl+Shift+O");
        assert_eq!(super::spelled_out("⌘↵"), "Ctrl+Enter");
        assert_eq!(super::spelled_out("Space"), "Space");
    }
}
