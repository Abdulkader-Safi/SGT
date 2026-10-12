use gittree_core::store::ThemeChoice;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, px};

use crate::app_state::app_state;

/// Makes the kit's theme match the settings. "System" follows macOS. Safe to
/// call often: it only touches the theme when something differs.
pub fn apply(cx: &mut App) {
    let settings = &app_state(cx).read(cx).settings;
    let mode = match settings.theme {
        ThemeChoice::System => ThemeMode::from(cx.window_appearance()),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    };
    let font_size = px(settings.ui_font_size);
    if cx.theme().mode != mode {
        Theme::change(mode, None, cx);
    }
    if cx.theme().font_size != font_size {
        Theme::update(cx, |theme| theme.font_size = font_size);
    }
}
