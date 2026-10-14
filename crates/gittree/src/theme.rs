use gittree_core::store::ThemeChoice;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, px};

use crate::app_state::app_state;
use crate::palette::{CurrentPalette, Palette};

/// Makes the kit's theme match the settings and the design's colours.
/// "System" follows macOS. Safe to call often: it only touches the theme
/// when something differs.
pub fn apply(cx: &mut App) {
    let app = app_state(cx).read(cx);
    let settings = &app.settings;
    let mode = match settings.theme {
        ThemeChoice::System => ThemeMode::from(cx.window_appearance()),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark | ThemeChoice::HighContrast => ThemeMode::Dark,
    };
    let font_size = px(settings.ui_font_size * crate::zoom::clamp(settings.zoom));
    let mut p = match settings.theme {
        ThemeChoice::HighContrast => Palette::high_contrast(),
        _ => Palette::new(mode.is_dark()),
    };
    // A user theme's colours go over the chosen one (FR-SET-03).
    let custom = settings.custom_theme.trim();
    if let Some((_, colors)) = app.themes.iter().find(|(name, _)| name == custom) {
        p = p.with_colors(colors).0;
    }
    if cx.theme().mode != mode {
        // Loading a mode replaces the colours, so they go in afterwards.
        Theme::change(mode, None, cx);
    }
    let changed = cx
        .try_global::<CurrentPalette>()
        .is_none_or(|current| current.0 != p);
    if changed {
        cx.set_global(CurrentPalette(p));
    }
    if changed || cx.theme().font_size != font_size || cx.theme().background != p.bg_app {
        Theme::update(cx, |theme| {
            theme.font_size = font_size;
            theme.radius = px(6.);
            theme.background = p.bg_app;
            theme.foreground = p.text;
            theme.border = p.border;
            theme.input = p.border_strong;
            theme.ring = p.accent;
            theme.muted = p.bg_hover;
            theme.muted_foreground = p.text_secondary;
            theme.popover = p.bg_elevated;
            theme.popover_foreground = p.text;
            theme.primary = p.accent_fill;
            theme.primary_hover = p.accent_fill_hover;
            theme.primary_active = p.accent_fill_hover;
            theme.primary_foreground = p.on_accent;
            theme.secondary = p.bg_elevated;
            theme.secondary_hover = p.bg_hover;
            theme.secondary_active = p.bg_hover;
            theme.secondary_foreground = p.text;
            theme.accent = p.bg_hover;
            theme.accent_foreground = p.text;
            theme.danger = p.danger;
            theme.success = p.success;
            theme.warning = p.warning;
            theme.info = p.info;
            theme.link = p.accent;
            theme.list_hover = p.bg_hover;
            theme.list_active = p.bg_selected;
            theme.sidebar = p.bg_panel;
            theme.sidebar_border = p.border;
            theme.title_bar = p.bg_panel;
            theme.title_bar_border = p.border;
            theme.skeleton = p.bg_hover;
            theme.drop_target = p.accent_subtle;
            theme.overlay = p.scrim;
            theme.selection = p.diff_picked;
        });
    }
}
