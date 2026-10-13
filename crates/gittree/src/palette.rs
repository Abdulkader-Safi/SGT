//! The design's colour tokens, in a dark and a light set. Views take their
//! colours from here. `theme::apply` copies the matching ones into the
//! kit's theme, so its buttons, inputs and dialogs agree.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg_app: Hsla,
    pub bg_panel: Hsla,
    pub bg_elevated: Hsla,
    pub bg_sunken: Hsla,
    pub bg_hover: Hsla,
    pub bg_selected: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub on_accent: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_subtle: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    pub success_subtle: Hsla,
    pub warning_subtle: Hsla,
    pub danger_subtle: Hsla,
    pub diff_added: Hsla,
    pub diff_added_gutter: Hsla,
    pub diff_removed: Hsla,
    pub diff_removed_gutter: Hsla,
    pub diff_hunk: Hsla,
    pub diff_picked: Hsla,
    pub scrim: Hsla,
    /// Graph lanes cycle through these in order.
    pub lanes: [Hsla; 8],
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        let pick = |d: u32, l: u32| -> Hsla { rgb(if dark { d } else { l }).into() };
        Self {
            bg_app: pick(0x0E1014, 0xF6F7F9),
            bg_panel: pick(0x14171C, 0xFFFFFF),
            bg_elevated: pick(0x1B1F26, 0xFFFFFF),
            bg_sunken: pick(0x0A0C0F, 0xF0F2F5),
            bg_hover: pick(0x1F242C, 0xEEF0F3),
            bg_selected: pick(0x1C2A44, 0xDCE8FF),
            border: pick(0x252A33, 0xE1E4EA),
            border_strong: pick(0x363D49, 0xC9CED8),
            text: pick(0xE8EAEE, 0x14171C),
            text_secondary: pick(0xA0A7B4, 0x535B69),
            text_tertiary: pick(0x6B7380, 0x8A919E),
            on_accent: pick(0xFFFFFF, 0xFFFFFF),
            accent: pick(0x4C8DFF, 0x2563EB),
            accent_hover: pick(0x6BA1FF, 0x1D4FD8),
            accent_subtle: pick(0x16233D, 0xE6EEFF),
            success: pick(0x3FB950, 0x1A7F37),
            warning: pick(0xE3B341, 0x9A6700),
            danger: pick(0xF85149, 0xCF222E),
            info: pick(0x58A6FF, 0x0969DA),
            success_subtle: pick(0x12261A, 0xE6F6EA),
            warning_subtle: pick(0x2B2411, 0xFFF4D6),
            danger_subtle: pick(0x2D1416, 0xFFEBEC),
            diff_added: pick(0x12261A, 0xE6F6EA),
            diff_added_gutter: pick(0x1B3A26, 0xCDEFD6),
            diff_removed: pick(0x2D1416, 0xFFEBEC),
            diff_removed_gutter: pick(0x45191D, 0xFFD4D7),
            diff_hunk: pick(0x151C2B, 0xEEF3FC),
            diff_picked: pick(0x1E3358, 0xCFE0FF),
            scrim: rgba(if dark { 0x0000008C } else { 0x0F141E59 }).into(),
            lanes: [
                pick(0x4C8DFF, 0x2563EB),
                pick(0xF2A33A, 0xC2740A),
                pick(0x3FB950, 0x1A7F37),
                pick(0xD16DE0, 0xA63DB8),
                pick(0xF0606B, 0xCF222E),
                pick(0x2EC4C4, 0x0E8A8A),
                pick(0xE3D04A, 0x8F7A00),
                pick(0x9C8CFF, 0x6B57E0),
            ],
        }
    }
}

/// The palette for the current theme.
pub fn palette(cx: &App) -> Palette {
    Palette::new(cx.theme().is_dark())
}
