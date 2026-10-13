use gittree_core::model::{Ref, RefKind};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use rust_i18n::t;

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

    fn row(&self, ix: usize, r: &Ref, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.list_hover);
        let repo = self.repo.clone();
        let target = r.target.clone();
        let name: SharedString = r.short_name().to_owned().into();
        let track = [("↑", r.ahead), ("↓", r.behind)]
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(arrow, count)| format!("{arrow}{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        h_flex()
            .id(("ref", ix))
            .w_full()
            .pl_5()
            .pr_2()
            .py_0p5()
            .gap_2()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .when(r.is_current, |row| {
                row.font_weight(gpui_kit::FontWeight::BOLD)
            })
            .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&target, cx)))
            .child(div().flex_1().truncate().child(name))
            .when(!track.is_empty(), |row| {
                row.child(div().flex_none().text_xs().text_color(muted).child(track))
            })
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, background) = (theme.muted_foreground, theme.sidebar);
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let repo = self.repo.read(cx);

        let groups = GROUPS.iter().enumerate().map(|(group, (kind, title))| {
            let rows: Vec<_> = repo
                .refs
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == *kind)
                .filter(|(_, r)| {
                    needle.is_empty() || r.short_name().to_lowercase().contains(&needle)
                })
                .map(|(ix, r)| self.row(ix, r, cx).into_any_element())
                .collect();
            let collapsed = self.collapsed[group];
            let count = rows.len();
            let empty_stashes =
                (*kind == RefKind::Stash && count == 0 && needle.is_empty()).then(|| {
                    div()
                        .pl_5()
                        .pr_2()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("sidebar.no_stashes").into_owned())
                });
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .id(("group", group))
                        .px_2()
                        .py_1()
                        .gap_1()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(muted)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.collapsed[group] = !this.collapsed[group];
                            cx.notify();
                        }))
                        .child(if collapsed { "▸" } else { "▾" })
                        .child(div().flex_1().child(t!(*title).into_owned()))
                        .child(count.to_string()),
                )
                .when(!collapsed, |group| {
                    group.children(rows).children(empty_stashes)
                })
        });

        v_flex()
            .size_full()
            .bg(background)
            .text_sm()
            .child(
                div()
                    .p_2()
                    .child(Input::new(&self.filter).small().cleanable(true)),
            )
            .child(
                v_flex()
                    .id("refs")
                    .flex_1()
                    .gap_1()
                    .children(groups)
                    .overflow_y_scrollbar(),
            )
    }
}
