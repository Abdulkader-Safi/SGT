use std::ops::Range;

use gittree_core::model::{Head, RefKind};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    TestSupportExt, UniformListScrollHandle, Window, div, px, uniform_list,
};
use rust_i18n::t;

use super::graph_element::{LANE_WIDTH, ROW_HEIGHT, graph_cell};
use gittree_core::ops::branch::Target;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};

use super::dialogs;
use super::ui::{RefLabel, badge, big_icon, caption, empty_state, ref_label};
use crate::actions::FocusChanges;
use crate::local_time;
use crate::palette::{Palette, palette};
use crate::repo_state::Op;
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

fn ref_labels(repo: &RepoState, oid: &str, p: Palette) -> Vec<gpui_kit::AnyElement> {
    let mut labels = Vec::new();
    if matches!(&repo.repository.head, Head::Detached(head) if &**head == oid) {
        let text = t!("graph.head").into_owned();
        labels.push(ref_label(RefLabel::Head, text, p).into_any_element());
    }
    for &ix in repo.refs_at.get(oid).into_iter().flatten() {
        let r = &repo.refs[ix];
        let kind = match r.kind {
            RefKind::Local if r.is_current => RefLabel::Current,
            RefKind::Local => RefLabel::Local,
            RefKind::Remote => RefLabel::Remote,
            RefKind::Tag => RefLabel::Tag,
            RefKind::Stash => RefLabel::Stash,
        };
        let name: SharedString = r.short_name().to_owned().into();
        labels.push(ref_label(kind, name, p).into_any_element());
    }
    labels
}

/// The widths of the author, date and hash columns.
const COLUMNS: [f32; 3] = [96., 112., 60.];

impl Render for GraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let repo = self.repo.read(cx);

        if let Some(error) = &repo.log_error {
            let message = crate::error_message(error.kind);
            let repo = self.repo.clone();
            return v_flex()
                .size_full()
                .bg(p.bg_app)
                .items_center()
                .justify_center()
                .gap_3()
                .child(big_icon("warning", p.danger, p.danger_subtle, p))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("graph.load_failed").into_owned()),
                )
                .child(div().text_color(p.text_secondary).child(message))
                .child(
                    Button::new("retry-log")
                        .primary()
                        .label(t!("graph.retry").into_owned())
                        .on_click(move |_, _, cx| {
                            repo.update(cx, |repo, cx| repo.refresh(Scope::All, cx))
                        }),
                )
                .into_any_element();
        }

        // At least three lanes wide, so the column title fits.
        let lanes = repo.history.lanes.max(3);
        let lane_width = LANE_WIDTH * lanes as f32;
        let header = h_flex()
            .flex_none()
            .h(px(26.))
            .pl_2()
            .pr_3()
            .gap_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .w(lane_width)
                    .flex_none()
                    .overflow_hidden()
                    .child(caption(t!("graph.col_graph").into_owned(), p)),
            )
            .child(
                div()
                    .flex_1()
                    .child(caption(t!("graph.col_description").into_owned(), p)),
            )
            .children(
                ["graph.col_author", "graph.col_date", "graph.col_commit"]
                    .into_iter()
                    .zip(COLUMNS)
                    .map(|(key, width)| {
                        div()
                            .w(px(width))
                            .flex_none()
                            .child(caption(t!(key).into_owned(), p))
                    }),
            );

        if repo.loading && repo.history.commits.is_empty() {
            // Skeleton rows until the first batch arrives.
            return v_flex()
                .size_full()
                .bg(p.bg_app)
                .child(header)
                .children((0..14).map(|n| {
                    h_flex()
                        .h(ROW_HEIGHT)
                        .px_3()
                        .gap_3()
                        .opacity(1. - n as f32 * 0.06)
                        .child(Skeleton::new().size(px(8.)).rounded_full())
                        .child(
                            Skeleton::new()
                                .h(px(10.))
                                .w(px(220. + 37. * (n % 5) as f32)),
                        )
                        .child(div().flex_1())
                        .child(Skeleton::new().h(px(10.)).w(px(80.)))
                        .child(Skeleton::new().h(px(10.)).w(px(60.)))
                }))
                .into_any_element();
        }

        let handle = self.repo.clone();
        let focus = self.focus.clone();
        let list = uniform_list(
            "graph",
            repo.history.commits.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let repo = this.repo.read(cx);
                let selected = repo.selected_row();
                let searching = !repo.search.query.trim().is_empty() && !repo.search.running;
                range
                    .map(|ix| {
                        let commit = &repo.history.commits[ix];
                        let is_match = repo.search.matches.binary_search(&ix).is_ok();
                        let is_selected = selected == Some(ix);
                        let (handle, focus) = (handle.clone(), focus.clone());
                        let cell = |width: f32, color, text: SharedString| {
                            div()
                                .w(px(width))
                                .flex_none()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(color)
                                .child(text)
                        };
                        h_flex()
                            .id(ix)
                            .h(ROW_HEIGHT)
                            .w_full()
                            .pl_2()
                            .pr_3()
                            .gap_2()
                            .when(!is_selected, |row| row.hover(|style| style.bg(p.bg_hover)))
                            .when_some(
                                is_selected
                                    .then_some(p.bg_selected)
                                    .or(is_match.then_some(p.warning_subtle)),
                                |row, color| row.bg(color),
                            )
                            // While a search is on, rows that do not match step back.
                            .when(searching && !is_match && !is_selected, |row| {
                                row.opacity(0.45)
                            })
                            .on_click(move |_, window, cx| {
                                focus.focus(window, cx);
                                handle.update(cx, |repo, cx| repo.select_row(ix, cx));
                            })
                            .child(graph_cell(
                                repo.history.rows[ix].clone(),
                                lanes,
                                p.lanes,
                                p.bg_app,
                            ))
                            .children(ref_labels(repo, &commit.oid, p))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .child(SharedString::from(commit.summary.clone())),
                            )
                            .child(cell(
                                COLUMNS[0],
                                p.text_secondary,
                                SharedString::from(commit.author_name.clone()),
                            ))
                            .child(cell(
                                COLUMNS[1],
                                p.text_secondary,
                                local_time(commit.author_time).into(),
                            ))
                            .child(
                                cell(
                                    COLUMNS[2],
                                    p.text_tertiary,
                                    commit.oid[..7.min(commit.oid.len())].to_string().into(),
                                )
                                .font_family(mono(cx)),
                            )
                            .context_menu({
                                let repo = this.repo.clone();
                                let oid = commit.oid.clone();
                                let label = format!(
                                    "{}  {}",
                                    &commit.oid[..7.min(commit.oid.len())],
                                    commit.summary
                                );
                                move |menu, _, cx| commit_menu(menu, &repo, &oid, &label, cx)
                            })
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();

        // The work in progress sits above the newest commit (FR-GRAPH-04).
        let wip = repo.wip_visible().then(|| {
            let (handle, focus) = (self.repo.clone(), self.focus.clone());
            let clean = repo.status.is_empty();
            let selected = repo.selection.wip;
            h_flex()
                .id("wip")
                .h(ROW_HEIGHT)
                .flex_none()
                .w_full()
                .pl_2()
                .pr_3()
                .gap_2()
                .cursor_pointer()
                .when(selected, |row| row.bg(p.bg_selected))
                .when(!selected, |row| row.hover(|style| style.bg(p.bg_hover)))
                .on_click(move |_, window, cx| {
                    focus.focus(window, cx);
                    handle.update(cx, |repo, cx| repo.select_wip(cx));
                })
                .child(
                    h_flex().w(lane_width).flex_none().child(
                        div()
                            .ml(px(3.))
                            .size(px(10.))
                            .rounded_full()
                            .border_2()
                            .border_dashed()
                            .border_color(p.text_secondary),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(if clean {
                            t!("graph.wip_none").into_owned()
                        } else {
                            t!("graph.wip_title").into_owned()
                        }),
                )
                .when(!clean, |row| {
                    row.child(badge(
                        t!("graph.wip_files", count = repo.status.len()).into_owned(),
                        p,
                    ))
                })
                // Lets the headless tests find and click the row.
                .test_support()
        });
        let body = if repo.history.commits.is_empty() {
            empty_state(
                "commit",
                t!("workspace.no_commits_title").into_owned(),
                t!("workspace.no_commits_text").into_owned(),
                p,
            )
            .child(
                Button::new("show-changes")
                    .label(t!("graph.show_changes").into_owned())
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(FocusChanges), cx)),
            )
            .into_any_element()
        } else {
            list.into_any_element()
        };

        v_flex()
            .id("graph-panel")
            .size_full()
            .bg(p.bg_app)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .when(!repo.history.commits.is_empty(), |panel| {
                panel.child(header)
            })
            .children(wip)
            .child(div().flex_1().overflow_hidden().child(body))
            .into_any_element()
    }
}

/// The right-click menu of a commit (FR-GRAPH-08). Cherry-pick, revert and
/// reset join it with the merge milestone.
fn commit_menu(
    menu: PopupMenu,
    repo: &Entity<RepoState>,
    oid: &gittree_core::model::Oid,
    label: &str,
    cx: &App,
) -> PopupMenu {
    let state = repo.read(cx);
    let trusted = state.repository.trusted;
    let head_name = match &state.repository.head {
        Head::Branch(name) | Head::Unborn(name) => name.clone(),
        Head::Detached(_) => "HEAD".to_owned(),
    };
    let row = state.history.row_of(oid);
    // Cherry-picking or resetting to the commit HEAD is on does nothing.
    let is_head = state.refs.iter().any(|r| r.is_current && &r.target == oid);
    let parents = row
        .map(|row| state.history.commits[row].parents.clone())
        .unwrap_or_default();
    let item = |icon_name: &str, key: &str| {
        PopupMenuItem::new(t!(key).into_owned()).icon(crate::icons::icon(icon_name))
    };
    let (checkout_repo, checkout_oid) = (repo.clone(), oid.clone());
    let (branch_repo, branch_at, branch_label) = (repo.clone(), oid.to_string(), label.to_owned());
    let (tag_repo, tag_at, tag_label) = (repo.clone(), oid.to_string(), label.to_owned());
    let copied = oid.to_string();
    menu.item(
        item("check", "menu_item.checkout_commit")
            .disabled(!trusted)
            .on_click(move |_, _, cx| {
                let op = Op::Checkout(Target::Commit(checkout_oid.clone()));
                checkout_repo.update(cx, |repo, cx| repo.run_op(op, cx))
            }),
    )
    .item(
        item("branch", "menu_item.create_branch")
            .disabled(!trusted)
            .on_click(move |_, window, cx| {
                dialogs::create_branch(
                    branch_repo.clone(),
                    branch_at.clone(),
                    branch_label.clone(),
                    window,
                    cx,
                )
            }),
    )
    .item(
        item("tag", "menu_item.create_tag")
            .disabled(!trusted)
            .on_click(move |_, window, cx| {
                dialogs::create_tag(
                    tag_repo.clone(),
                    tag_at.clone(),
                    tag_label.clone(),
                    window,
                    cx,
                )
            }),
    )
    .separator()
    .item({
        let (repo, oid, parents) = (repo.clone(), oid.clone(), parents.clone());
        item("commit", "menu_item.cherry_pick")
            .disabled(!trusted || is_head)
            .on_click(move |_, window, cx| {
                dialogs::pick(
                    repo.clone(),
                    oid.clone(),
                    parents.clone(),
                    false,
                    window,
                    cx,
                )
            })
    })
    .item({
        let (repo, oid, parents) = (repo.clone(), oid.clone(), parents.clone());
        item("undo", "menu_item.revert")
            .disabled(!trusted)
            .on_click(move |_, window, cx| {
                dialogs::pick(repo.clone(), oid.clone(), parents.clone(), true, window, cx)
            })
    })
    .item({
        let (repo, target, branch) = (repo.clone(), oid.to_string(), head_name.clone());
        PopupMenuItem::new(t!("menu_item.reset_here", branch = head_name).into_owned())
            .icon(crate::icons::icon("fetch"))
            .disabled(!trusted || is_head)
            .on_click(move |_, window, cx| {
                dialogs::reset(repo.clone(), branch.clone(), target.clone(), window, cx)
            })
    })
    .separator()
    .item(
        item("copy", "menu_item.copy_hash").on_click(move |_, _, cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copied.clone()))
        }),
    )
}

/// The font for hashes, line numbers and code.
pub fn mono(cx: &App) -> SharedString {
    crate::app_state::app_state(cx)
        .read(cx)
        .settings
        .diff_font_family
        .clone()
        .into()
}
