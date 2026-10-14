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
    /// Text on `accent_fill`.
    pub on_accent: Hsla,
    /// Text on a lane colour.
    pub on_lane: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    /// The accent as a background under `on_accent`. Darker than `accent`
    /// in the dark theme, where `accent` is tuned to be read as text.
    pub accent_fill: Hsla,
    pub accent_fill_hover: Hsla,
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
            text_tertiary: pick(0x8A92A0, 0x5E6572),
            on_accent: pick(0xFFFFFF, 0xFFFFFF),
            on_lane: pick(0x0A0C0F, 0xFFFFFF),
            accent: pick(0x4C8DFF, 0x2160E6),
            accent_hover: pick(0x6BA1FF, 0x1D4FD8),
            accent_fill: pick(0x2F6FE4, 0x2160E6),
            accent_fill_hover: pick(0x2866DB, 0x1D4FD8),
            accent_subtle: pick(0x16233D, 0xE6EEFF),
            success: pick(0x3FB950, 0x1A7F37),
            warning: pick(0xE3B341, 0x946300),
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
                pick(0x4C8DFF, 0x2160E6),
                pick(0xF2A33A, 0xA66208),
                pick(0x3FB950, 0x1A7F37),
                pick(0xD16DE0, 0xA63DB8),
                pick(0xF0606B, 0xCF222E),
                pick(0x2EC4C4, 0x0C8080),
                pick(0xE3D04A, 0x857100),
                pick(0x9C8CFF, 0x6B57E0),
            ],
        }
    }
}

/// The palette for the current theme.
pub fn palette(cx: &App) -> Palette {
    Palette::new(cx.theme().is_dark())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Rgba;

    /// The WCAG contrast ratio of two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |colour: Hsla| {
            let Rgba { r, g, b, .. } = colour.into();
            let linear = |c: f32| {
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// NFR-06: text is at least 4.5:1 against what it sits on.
    #[test]
    fn text_meets_the_contrast_rule_in_both_themes() {
        for dark in [true, false] {
            let p = Palette::new(dark);
            let surfaces = [
                p.bg_app,
                p.bg_panel,
                p.bg_elevated,
                p.bg_sunken,
                p.bg_hover,
                p.bg_selected,
            ];
            let mut pairs = Vec::new();
            for text in [p.text, p.text_secondary, p.text_tertiary] {
                pairs.extend(surfaces.map(|surface| (text, surface)));
            }
            pairs.extend(p.lanes.map(|lane| (p.on_lane, lane)));
            pairs.extend([
                (p.on_accent, p.accent_fill),
                (p.on_accent, p.accent_fill_hover),
                (p.accent, p.bg_panel),
                (p.accent, p.accent_subtle),
                (p.success, p.success_subtle),
                (p.warning, p.warning_subtle),
                (p.danger, p.danger_subtle),
                (p.text, p.diff_added),
                (p.text, p.diff_removed),
                (p.text, p.diff_picked),
                // Changed words inside a changed line.
                (p.text, p.diff_added_gutter),
                (p.text, p.diff_removed_gutter),
            ]);
            for (index, (text, surface)) in pairs.into_iter().enumerate() {
                let ratio = contrast(text, surface);
                assert!(ratio >= 4.5, "dark={dark} pair {index}: {ratio:.2}");
            }
        }
    }
}
