use std::ops::Range;

use gittree_core::model::{Head, RefKind};
use gpui_kit::component::button::Button;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    TestSupportExt, UniformListScrollHandle, Window, div, px, uniform_list,
};
use rust_i18n::t;

use super::graph_element::{ROW_HEIGHT, graph_cell};
use crate::local_time;
use crate::repo_state::{RepoEvent, RepoState, Scope};

/// Rows a Page Up or Page Down moves.
const PAGE: isize = 20;

/// The commit graph: lanes, ref labels and one line per commit. It reads
/// everything from `RepoState`.
pub struct GraphView {
    repo: Entity<RepoState>,
    pub scroll: UniformListScrollHandle,
    pub focus: FocusHandle,
}

impl GraphView {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&repo, |this, _, event: &RepoEvent, _| {
            if let RepoEvent::Reveal(row) = event {
                this.scroll.scroll_to_item(*row, ScrollStrategy::Nearest);
            }
        })
        .detach();
        Self {
            repo,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.keystroke.key.as_str() {
            "up" => -1,
            "down" => 1,
            "pageup" => -PAGE,
            "pagedown" => PAGE,
            "home" => isize::MIN / 2,
            "end" => isize::MAX / 2,
            _ => return,
        };
        self.repo
            .update(cx, |repo, cx| repo.move_selection(delta, cx));
        cx.stop_propagation();
    }
}

/// A small label for a branch, tag or HEAD on a graph row.
fn ref_label(text: SharedString, background: Hsla, foreground: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .px_1p5()
        .rounded_sm()
        .text_xs()
        .bg(background)
        .text_color(foreground)
        .child(text)
}

fn ref_labels(repo: &RepoState, oid: &str, cx: &App) -> Vec<gpui_kit::AnyElement> {
    let theme = cx.theme();
    let mut labels = Vec::new();
    if matches!(&repo.repository.head, Head::Detached(head) if &**head == oid) {
        labels.push(
            ref_label(
                t!("graph.head").into_owned().into(),
                theme.primary,
                theme.primary_foreground,
            )
            .into_any_element(),
        );
    }
    for &ix in repo.refs_at.get(oid).into_iter().flatten() {
        let r = &repo.refs[ix];
        // Each kind looks different, and the current branch stands out.
        let (background, foreground) = match r.kind {
            RefKind::Local if r.is_current => (theme.primary, theme.primary_foreground),
            RefKind::Local => (theme.success.opacity(0.25), theme.foreground),
            RefKind::Remote => (theme.muted, theme.muted_foreground),
            RefKind::Tag => (theme.warning.opacity(0.3), theme.foreground),
            RefKind::Stash => (theme.info.opacity(0.3), theme.foreground),
        };
        let name: SharedString = r.short_name().to_owned().into();
        labels.push(ref_label(name, background, foreground).into_any_element());
    }
    labels
}

impl Render for GraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, muted, selected_bg, match_bg) = (
            theme.background,
            theme.muted_foreground,
            theme.list_active,
            theme.warning.opacity(0.12),
        );
        let repo = self.repo.read(cx);

        if let Some(error) = &repo.log_error {
            let message = crate::error_message(error.kind);
            let repo = self.repo.clone();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(t!("graph.load_failed").into_owned())
                .child(div().text_color(muted).child(message))
                .child(
                    Button::new("retry-log")
                        .label(t!("graph.retry").into_owned())
                        .on_click(move |_, _, cx| {
                            repo.update(cx, |repo, cx| repo.refresh(Scope::All, cx))
                        }),
                )
                .into_any_element();
        }
        if repo.loading && repo.history.commits.is_empty() {
            // Skeleton rows until the first batch arrives.
            return v_flex()
                .size_full()
                .p_2()
                .gap_2()
                .children((0..12).map(|n| {
                    Skeleton::new()
                        .h(px(16.))
                        .w(px(260. + 37. * (n % 5) as f32))
                }))
                .into_any_element();
        }

        let lanes = repo.history.lanes;
        let handle = self.repo.clone();
        let focus = self.focus.clone();
        let list = uniform_list(
            "graph",
            repo.history.commits.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let repo = this.repo.read(cx);
                let selected = repo.selected_row();
                range
                    .map(|ix| {
                        let commit = &repo.history.commits[ix];
                        let is_match = repo.search.matches.binary_search(&ix).is_ok();
                        let (handle, focus) = (handle.clone(), focus.clone());
                        h_flex()
                            .id(ix)
                            .h(ROW_HEIGHT)
                            .w_full()
                            .px_2()
                            .gap_2()
                            .when_some(
                                (selected == Some(ix))
                                    .then_some(selected_bg)
                                    .or(is_match.then_some(match_bg)),
                                |row, color| row.bg(color),
                            )
                            .on_click(move |_, window, cx| {
                                focus.focus(window, cx);
                                handle.update(cx, |repo, cx| repo.select_row(ix, cx));
                            })
                            .child(graph_cell(repo.history.rows[ix].clone(), lanes, background))
                            .children(ref_labels(repo, &commit.oid, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .child(SharedString::from(commit.summary.clone())),
                            )
                            .child(
                                div()
                                    .w(px(130.))
                                    .flex_none()
                                    .truncate()
                                    .text_color(muted)
                                    .child(SharedString::from(commit.author_name.clone())),
                            )
                            .child(
                                div()
                                    .w(px(118.))
                                    .flex_none()
                                    .text_color(muted)
                                    .child(local_time(commit.author_time)),
                            )
                            .child(
                                div()
                                    .w(px(60.))
                                    .flex_none()
                                    .text_color(muted)
                                    .child(commit.oid[..7.min(commit.oid.len())].to_string()),
                            )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();

        // The work in progress sits above the newest commit (FR-GRAPH-04).
        let wip = repo.wip_visible().then(|| {
            let (handle, focus) = (self.repo.clone(), self.focus.clone());
            let text = if repo.status.is_empty() {
                t!("graph.wip_none").into_owned()
            } else {
                t!("graph.wip", count = repo.status.len()).into_owned()
            };
            h_flex()
                .id("wip")
                .h(ROW_HEIGHT)
                .flex_none()
                .w_full()
                .px_2()
                .gap_2()
                .cursor_pointer()
                .when(repo.selection.wip, |row| row.bg(selected_bg))
                .on_click(move |_, window, cx| {
                    focus.focus(window, cx);
                    handle.update(cx, |repo, cx| repo.select_wip(cx));
                })
                .child(
                    div()
                        .size(px(10.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(muted),
                )
                .child(div().flex_1().truncate().child(text))
                // Lets the headless tests find and click the row.
                .test_support()
        });
        let body = if repo.history.commits.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .child(t!("workspace.no_commits_title").into_owned()),
                )
                .child(
                    div()
                        .text_color(muted)
                        .child(t!("workspace.no_commits_text").into_owned()),
                )
                .into_any_element()
        } else {
            list.into_any_element()
        };

        v_flex()
            .id("graph-panel")
            .size_full()
            .bg(background)
            .text_sm()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .children(wip)
            .child(div().flex_1().overflow_hidden().child(body))
            .into_any_element()
    }
}
