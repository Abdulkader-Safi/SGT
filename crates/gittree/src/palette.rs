//! The design's colour tokens, in a dark and a light set. Views take their
//! colours from here. `theme::apply` copies the matching ones into the
//! kit's theme, so its buttons, inputs and dialogs agree.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Global, Hsla, rgb, rgba};

#[derive(Clone, Copy, PartialEq)]
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

impl Palette {
    /// White text on black, for the high-contrast theme (FR-SET-03).
    pub fn high_contrast() -> Self {
        let c = |hex: u32| -> Hsla { rgb(hex).into() };
        Self {
            bg_app: c(0x000000),
            bg_panel: c(0x0B0B0B),
            bg_elevated: c(0x161616),
            bg_sunken: c(0x000000),
            bg_hover: c(0x242424),
            bg_selected: c(0x0A2E66),
            border: c(0x7A7A7A),
            border_strong: c(0xB0B0B0),
            text: c(0xFFFFFF),
            text_secondary: c(0xE6E6E6),
            text_tertiary: c(0xC4C4C4),
            accent: c(0x7AB8FF),
            accent_hover: c(0x9CCBFF),
            accent_fill: c(0x0050C8),
            accent_fill_hover: c(0x0044AA),
            accent_subtle: c(0x0D2547),
            success: c(0x5FE07A),
            warning: c(0xFFD24D),
            danger: c(0xFF7B72),
            info: c(0x7AB8FF),
            success_subtle: c(0x0B2614),
            warning_subtle: c(0x2B2208),
            danger_subtle: c(0x33100F),
            diff_added: c(0x0B2614),
            diff_added_gutter: c(0x15452A),
            diff_removed: c(0x33100F),
            diff_removed_gutter: c(0x5A1A18),
            diff_hunk: c(0x0F1A2E),
            diff_picked: c(0x1A3D7A),
            ..Self::new(true)
        }
    }

    /// The colour a user theme names, by token: `text`, `accent`,
    /// `lane1` to `lane8`, and so on.
    fn slot(&mut self, name: &str) -> Option<&mut Hsla> {
        Some(match name {
            "bg_app" => &mut self.bg_app,
            "bg_panel" => &mut self.bg_panel,
            "bg_elevated" => &mut self.bg_elevated,
            "bg_sunken" => &mut self.bg_sunken,
            "bg_hover" => &mut self.bg_hover,
            "bg_selected" => &mut self.bg_selected,
            "border" => &mut self.border,
            "border_strong" => &mut self.border_strong,
            "text" => &mut self.text,
            "text_secondary" => &mut self.text_secondary,
            "text_tertiary" => &mut self.text_tertiary,
            "on_accent" => &mut self.on_accent,
            "on_lane" => &mut self.on_lane,
            "accent" => &mut self.accent,
            "accent_hover" => &mut self.accent_hover,
            "accent_fill" => &mut self.accent_fill,
            "accent_fill_hover" => &mut self.accent_fill_hover,
            "accent_subtle" => &mut self.accent_subtle,
            "success" => &mut self.success,
            "warning" => &mut self.warning,
            "danger" => &mut self.danger,
            "info" => &mut self.info,
            "success_subtle" => &mut self.success_subtle,
            "warning_subtle" => &mut self.warning_subtle,
            "danger_subtle" => &mut self.danger_subtle,
            "diff_added" => &mut self.diff_added,
            "diff_added_gutter" => &mut self.diff_added_gutter,
            "diff_removed" => &mut self.diff_removed,
            "diff_removed_gutter" => &mut self.diff_removed_gutter,
            "diff_hunk" => &mut self.diff_hunk,
            "diff_picked" => &mut self.diff_picked,
            "scrim" => &mut self.scrim,
            lane => {
                let n: usize = lane.strip_prefix("lane")?.parse().ok()?;
                self.lanes.get_mut(n.checked_sub(1)?)?
            }
        })
    }

    /// This palette with a user theme's colours laid over it: an object
    /// from token to `#rrggbb` or `#rrggbbaa`. Returns the entries it
    /// could not use.
    pub fn with_colors(
        mut self,
        colors: &serde_json::Map<String, serde_json::Value>,
    ) -> (Self, Vec<String>) {
        let mut skipped = Vec::new();
        for (name, value) in colors {
            let hex = value.as_str().and_then(|text| text.strip_prefix('#'));
            let colour = hex.and_then(|hex| match hex.len() {
                6 => u32::from_str_radix(hex, 16).ok().map(|v| rgb(v).into()),
                8 => u32::from_str_radix(hex, 16).ok().map(|v| rgba(v).into()),
                _ => None,
            });
            match (self.slot(name), colour) {
                (Some(slot), Some(colour)) => *slot = colour,
                _ => skipped.push(name.clone()),
            }
        }
        (self, skipped)
    }
}

/// The palette `theme::apply` chose, for every view to read.
pub struct CurrentPalette(pub Palette);

impl Global for CurrentPalette {}

/// The palette for the current theme.
pub fn palette(cx: &App) -> Palette {
    cx.try_global::<CurrentPalette>()
        .map_or_else(|| Palette::new(cx.theme().is_dark()), |current| current.0)
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
    fn a_user_theme_sets_named_colours_and_skips_the_rest() {
        let colors = serde_json::json!({
            "accent": "#FF8800", "lane3": "#00000080", "nope": "#FFFFFF", "text": "red"
        });
        let (p, skipped) = Palette::new(true).with_colors(colors.as_object().unwrap());
        assert_eq!(p.accent, rgb(0xFF8800).into());
        assert_eq!(p.lanes[2], rgba(0x00000080).into());
        assert_eq!(skipped, ["nope", "text"]);
        assert_eq!(p.text, Palette::new(true).text);
    }

    #[test]
    fn text_meets_the_contrast_rule_in_every_theme() {
        let themes = [
            ("dark", Palette::new(true)),
            ("light", Palette::new(false)),
            ("high contrast", Palette::high_contrast()),
        ];
        for (dark, p) in themes {
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
