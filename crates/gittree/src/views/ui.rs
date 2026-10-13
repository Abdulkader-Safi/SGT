//! Small pieces the views share, drawn to the design: icons, labels, marks,
//! badges, key hints, captions, banners and empty states.

use gittree_core::model::Change;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, Div, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px,
};

use crate::icons::icon;
use crate::palette::Palette;

/// An icon at a size and colour.
pub fn ic(name: &str, size: f32, color: Hsla) -> AnyElement {
    icon(name)
        .size(px(size))
        .flex_none()
        .text_color(color)
        .into_any_element()
}

/// Small capitals for group and column titles.
pub fn caption(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.text_tertiary)
        .child(text.into())
}

/// A count in a pill.
pub fn badge(text: impl Into<SharedString>, p: Palette) -> Div {
    h_flex()
        .flex_none()
        .h(px(16.))
        .px(px(6.))
        .rounded(px(8.))
        .bg(p.bg_hover)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.text_secondary)
        .child(text.into())
}

/// A shortcut hint.
pub fn kbd(text: impl Into<SharedString>, p: Palette) -> Div {
    h_flex()
        .flex_none()
        .h(px(18.))
        .px(px(5.))
        .rounded(px(4.))
        .bg(p.bg_elevated)
        .border_1()
        .border_color(p.border_strong)
        .text_size(px(11.))
        .text_color(p.text_secondary)
        .child(text.into())
}

/// What a graph label names. Each kind has its own colours and icon.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RefLabel {
    Current,
    Local,
    Remote,
    Tag,
    Stash,
    Head,
}

pub fn ref_label(kind: RefLabel, text: impl Into<SharedString>, p: Palette) -> Div {
    let (background, border, foreground, name) = match kind {
        RefLabel::Current => (Some(p.accent), None, p.on_accent, "branch"),
        RefLabel::Local => (Some(p.success_subtle), None, p.success, "branch"),
        RefLabel::Remote => (
            Some(p.bg_elevated),
            Some(p.border_strong),
            p.text_secondary,
            "remote",
        ),
        RefLabel::Tag => (Some(p.warning_subtle), None, p.warning, "tag"),
        RefLabel::Stash => (Some(p.accent_subtle), None, p.info, "stash"),
        RefLabel::Head => (None, Some(p.text), p.text, "commit"),
    };
    let mut label = h_flex()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .gap(px(4.))
        .rounded(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(foreground)
        .child(ic(name, 10., foreground))
        .child(text.into());
    if let Some(background) = background {
        label = label.bg(background);
    }
    if let Some(border) = border {
        label = label.border_1().border_color(border);
    }
    label
}

/// One letter for a file's status, always next to its path.
pub fn status_mark(change: Change, conflicted: bool, p: Palette) -> Div {
    let (letter, color) = match change {
        _ if conflicted => ("!", p.danger),
        Change::Added => ("A", p.success),
        Change::Untracked => ("?", p.text_tertiary),
        Change::Deleted => ("D", p.danger),
        Change::Renamed => ("R", p.info),
        Change::Copied => ("C", p.info),
        Change::TypeChange => ("T", p.warning),
        Change::Modified | Change::Unmodified => ("M", p.warning),
    };
    let mark = h_flex()
        .flex_none()
        .size(px(16.))
        .justify_center()
        .rounded(px(4.))
        .text_size(px(11.))
        .text_color(color)
        .child(letter);
    if conflicted {
        mark.bg(p.danger_subtle)
    } else {
        mark
    }
}

#[derive(Clone, Copy)]
pub enum BannerKind {
    Warning,
    Info,
    Danger,
}

/// The strip under the toolbar: an icon and one line of text. The caller
/// adds its buttons.
pub fn banner(kind: BannerKind, icon_name: &str, text: impl Into<SharedString>, p: Palette) -> Div {
    let (background, color) = match kind {
        BannerKind::Warning => (p.warning_subtle, p.warning),
        BannerKind::Info => (p.accent_subtle, p.info),
        BannerKind::Danger => (p.danger_subtle, p.danger),
    };
    h_flex()
        .flex_none()
        .min_h(px(40.))
        .px_4()
        .py_2()
        .gap(px(10.))
        .bg(background)
        .border_b_1()
        .border_color(p.border)
        .child(ic(icon_name, 16., color))
        .child(div().flex_1().child(text.into()))
}

/// An icon in a circle, for empty and error states.
pub fn big_icon(name: &str, color: Hsla, background: Hsla, p: Palette) -> Div {
    h_flex()
        .flex_none()
        .size(px(48.))
        .justify_center()
        .rounded_full()
        .bg(background)
        .border_1()
        .border_color(p.border)
        .child(ic(name, 22., color))
}

/// What is empty, then the next step.
pub fn empty_state(
    icon_name: &str,
    title: impl Into<SharedString>,
    text: impl Into<SharedString>,
    p: Palette,
) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(big_icon(icon_name, p.text_secondary, p.bg_elevated, p))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(340.))
                .text_center()
                .text_color(p.text_secondary)
                .child(text.into()),
        )
}
