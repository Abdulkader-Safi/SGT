//! The Safi Git Tree app. The binaries and the UI tests share these modules.

pub mod actions;
pub mod app_state;
pub mod icons;
pub mod logging;
pub mod menus;
pub mod palette;
pub mod platform;
pub mod repo_state;
pub mod theme;
pub mod views;
pub mod zoom;

use gittree_core::GitErrorKind;
use gittree_core::store::WindowState;
use gpui_kit::{App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};
use rust_i18n::t;

rust_i18n::i18n!("locales", fallback = "en");

/// The working name. Change it here and in the bundle metadata.
pub const APP_NAME: &str = "Safi Git Tree";

/// Options for the main window. `saved` is where the window was last time.
pub fn window_options(saved: Option<WindowState>, cx: &App) -> WindowOptions {
    let bounds = match saved {
        Some(saved) => {
            let frame = Bounds::new(
                point(px(saved.x), px(saved.y)),
                size(px(saved.width), px(saved.height)),
            );
            if saved.maximized {
                WindowBounds::Maximized(frame)
            } else {
                WindowBounds::Windowed(frame)
            }
        }
        None => WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(800.)), cx)),
    };
    WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            // On macOS the toolbar draws under the title bar, next to the
            // traffic lights, as one strip.
            appears_transparent: platform::MAC,
            traffic_light_position: platform::MAC.then(|| point(px(16.), px(17.))),
        }),
        ..Default::default()
    }
}

/// The plain-language message for a kind of Git failure.
pub fn error_message(kind: GitErrorKind) -> String {
    let key = match kind {
        GitErrorKind::GitNotFound => "errors.git_not_found",
        GitErrorKind::GitTooOld => "errors.git_too_old",
        GitErrorKind::NotARepository => "errors.not_a_repository",
        GitErrorKind::Untrusted => "errors.untrusted",
        GitErrorKind::IndexLocked => "errors.index_locked",
        GitErrorKind::Conflict => "errors.conflict",
        GitErrorKind::DirtyWorkTree => "errors.dirty_work_tree",
        GitErrorKind::NonFastForward => "errors.non_fast_forward",
        GitErrorKind::AuthFailed => "errors.auth_failed",
        GitErrorKind::Network => "errors.network",
        GitErrorKind::RefExists => "errors.ref_exists",
        GitErrorKind::RefNotFound => "errors.ref_not_found",
        GitErrorKind::PatchDoesNotApply => "errors.patch_does_not_apply",
        GitErrorKind::SigningFailed => "errors.signing_failed",
        GitErrorKind::Cancelled => "errors.cancelled",
        GitErrorKind::Other => "errors.other",
    };
    t!(key).into_owned()
}

/// A time as the user's clock shows it, for example "2026-10-12 17:16".
pub fn local_time(unix_seconds: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix_seconds, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_kind_has_its_own_message() {
        let messages: Vec<String> = GitErrorKind::ALL.into_iter().map(error_message).collect();
        for message in &messages {
            // A missing key comes back as the key itself.
            assert!(!message.starts_with("errors."), "{message}");
            assert!(message.ends_with('.'), "{message}");
        }
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len());
    }
}
