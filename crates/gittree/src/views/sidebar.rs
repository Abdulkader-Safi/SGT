use gittree_core::model::{Ref, RefKind};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::ui::{caption, ic};
use crate::palette::palette;
use crate::repo_state::RepoState;

const GROUPS: [(RefKind, &str); 4] = [
    (RefKind::Local, "sidebar.local"),
    (RefKind::Remote, "sidebar.remotes"),
    (RefKind::Tag, "sidebar.tags"),
    (RefKind::Stash, "sidebar.stashes"),
];

/// Branches, remote branches, tags and stashes, with a name filter
/// (FR-BRANCH-01).
pub struct Sidebar {
    repo: Entity<RepoState>,
    filter: Entity<InputState>,
    collapsed: [bool; GROUPS.len()],
}

impl Sidebar {
    pub fn new(repo: Entity<RepoState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("sidebar.filter").into_owned()));
        cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            repo,
            filter,
            collapsed: [false; GROUPS.len()],
        }
    }

    fn row(
        &self,
        ix: usize,
        r: &Ref,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let repo = self.repo.clone();
        let target = r.target.clone();
        let name: SharedString = r.short_name().to_owned().into();
        let icon = match r.kind {
            RefKind::Local => "branch",
            RefKind::Remote => "remote",
            RefKind::Tag => "tag",
            RefKind::Stash => "stash",
        };
        let track = [("↑", r.ahead), ("↓", r.behind)]
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(arrow, count)| format!("{arrow}{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        h_flex()
            .id(("ref", ix))
            .w_full()
            .h(px(28.))
            .pl(px(30.))
            .pr_3()
            .gap_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(p.bg_selected))
            .when(!selected, |row| row.hover(|style| style.bg(p.bg_hover)))
            .when(r.is_current, |row| row.font_weight(FontWeight::MEDIUM))
            .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&target, cx)))
            .child(ic(
                icon,
                14.,
                if r.is_current {
                    p.accent
                } else {
                    p.text_tertiary
                },
            ))
            .child(div().flex_1().truncate().child(name))
            .when(!track.is_empty(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(p.text_secondary)
                        .child(track),
                )
            })
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let repo = self.repo.read(cx);
        let selected = repo.selection.oid.clone();

        let groups = GROUPS.iter().enumerate().map(|(group, (kind, title))| {
            let rows: Vec<_> = repo
                .refs
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == *kind)
                .filter(|(_, r)| {
                    needle.is_empty() || r.short_name().to_lowercase().contains(&needle)
                })
                .map(|(ix, r)| {
                    // The branch or tag that names the selected commit.
                    let is_selected = selected.as_ref() == Some(&r.target);
                    self.row(ix, r, is_selected, cx).into_any_element()
                })
                .collect();
            let collapsed = self.collapsed[group];
            let count = rows.len();
            let empty_stashes =
                (*kind == RefKind::Stash && count == 0 && needle.is_empty()).then(|| {
                    div()
                        .pl(px(30.))
                        .pr_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .child(t!("sidebar.no_stashes").into_owned())
                });
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .id(("group", group))
                        .h(px(26.))
                        .px_3()
                        .gap(px(6.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.collapsed[group] = !this.collapsed[group];
                            cx.notify();
                        }))
                        .child(ic(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                            p.text_tertiary,
                        ))
                        .child(div().flex_1().child(caption(t!(*title).to_uppercase(), p)))
                        .child(caption(count.to_string(), p)),
                )
                .when(!collapsed, |group| {
                    group.children(rows).children(empty_stashes)
                })
        });

        v_flex()
            .size_full()
            .bg(p.bg_panel)
            .border_r_1()
            .border_color(p.border)
            .child(div().p(px(10.)).child(
                Input::new(&self.filter).small().cleanable(true).prefix(ic(
                    "search",
                    14.,
                    p.text_tertiary,
                )),
            ))
            .child(
                v_flex()
                    .id("refs")
                    .flex_1()
                    .children(groups)
                    .overflow_y_scrollbar(),
            )
    }
}
