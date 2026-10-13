//! One file over time: its blame or its history, shown in place of the
//! commit details (FR-AGENT-03).

use std::ops::Range;

use gittree_core::model::Oid;
use gittree_core::ops::file::{BlameLine, FileCommit};
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window,
    div, px, uniform_list,
};
use rust_i18n::t;

use super::diff::DiffPane;
use super::graph::mono;
use super::ui::{empty_state, ic};
use crate::local_time;
use crate::palette::{Palette, palette};
use crate::repo_state::{FileMode, RepoState};

const LINE_HEIGHT: f32 = 22.;

pub struct FileViewPanel {
    repo: Entity<RepoState>,
    /// What the picked history row did to the file.
    diff: Entity<DiffPane>,
    scroll: UniformListScrollHandle,
}

impl FileViewPanel {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            diff: cx.new(|cx| DiffPane::for_file_view(repo.clone(), cx)),
            scroll: UniformListScrollHandle::new(),
            repo,
        }
    }
}

/// The same commit always gets the same lane colour.
fn lane_of(oid: &str) -> usize {
    oid.bytes().map(usize::from).sum::<usize>() % 8
}

fn short(oid: &str) -> String {
    oid[..7.min(oid.len())].to_owned()
}

/// One blame line. The commit is named on the first line of each run.
fn blame_row(
    ix: usize,
    lines: &[BlameLine],
    picked: Option<&Oid>,
    mono: gpui_kit::SharedString,
    p: Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let line = &lines[ix];
    let first = ix == 0 || lines[ix - 1].oid != line.oid;
    let text = String::from_utf8_lossy(&line.text).replace('\t', "    ");
    h_flex()
        .id(("blame", ix))
        .h(px(LINE_HEIGHT))
        .cursor_pointer()
        .when(picked == Some(&line.oid), |row| row.bg(p.bg_selected))
        .child(
            div()
                .w(px(3.))
                .h_full()
                .flex_none()
                .bg(p.lanes[lane_of(&line.oid)]),
        )
        .child(
            h_flex()
                .w(px(190.))
                .h_full()
                .flex_none()
                .px_2()
                .gap_2()
                .text_size(px(12.))
                .border_r_1()
                .border_color(p.border)
                .when(first, |meta| {
                    meta.child(
                        div()
                            .flex_none()
                            .font_family(mono.clone())
                            .text_size(px(11.))
                            .text_color(p.accent)
                            .child(short(&line.oid)),
                    )
                    .child(div().flex_1().truncate().child(line.author.to_string()))
                    .child(
                        div()
                            .flex_none()
                            .text_color(p.text_tertiary)
                            .child(local_time(line.time)[..10].to_owned()),
                    )
                }),
        )
        .child(
            div()
                .w(px(44.))
                .flex_none()
                .pr_2()
                .text_right()
                .font_family(mono.clone())
                .text_size(px(11.))
                .text_color(p.text_tertiary)
                .child((ix + 1).to_string()),
        )
        .child(
            div()
                .flex_1()
                .whitespace_nowrap()
                .overflow_hidden()
                .font_family(mono)
                .text_size(px(12.))
                .child(text),
        )
}

fn history_row(ix: usize, entry: &FileCommit, picked: bool, p: Palette) -> AnyElement {
    let commit = &entry.commit;
    let mut note = format!(
        "{} · {}",
        commit.author_name,
        local_time(commit.author_time)
    );
    if let Some(from) = &entry.file.orig_path {
        note = format!(
            "{note} · {}",
            t!("file.renamed_from", path = from.to_string())
        );
    }
    h_flex()
        .id(("file-commit", ix))
        .h(px(40.))
        .flex_none()
        .px_3()
        .gap_3()
        .cursor_pointer()
        .border_b_1()
        .border_color(p.border)
        .when(picked, |row| row.bg(p.bg_selected))
        .when(!picked, |row| row.hover(|style| style.bg(p.bg_hover)))
        .child(div().size(px(8.)).flex_none().rounded_full().bg(p.accent))
        .child(
            v_flex()
                .flex_1()
                .overflow_hidden()
                .child(div().truncate().child(commit.summary.to_string()))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .child(note),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(p.text_tertiary)
                .child(short(&commit.oid)),
        )
        .into_any_element()
}

impl Render for FileViewPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mono = mono(cx);
        let repo = self.repo.read(cx);
        let panel = v_flex()
            .size_full()
            .bg(p.bg_panel)
            .border_l_1()
            .border_color(p.border);
        let Some(view) = &repo.file_view else {
            return panel.into_any_element();
        };
        let mode = view.mode;

        let segment = |id: &'static str, label: String, target: Option<FileMode>| {
            let on = target == Some(mode);
            let repo = self.repo.clone();
            div()
                .id(id)
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { p.text } else { p.text_tertiary })
                .when(on, |segment| {
                    segment
                        .bg(p.bg_elevated)
                        .border_1()
                        .border_color(p.border_strong)
                })
                .when(!on, |segment| {
                    segment.cursor_pointer().on_click(move |_, _, cx| {
                        repo.update(cx, |repo, cx| match target {
                            Some(mode) => repo.show_file(mode, cx),
                            None => repo.close_file_view(cx),
                        })
                    })
                })
                .child(label)
        };
        let header = h_flex()
            .flex_none()
            .h(px(44.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(p.border)
            .child(ic("file", 16., p.text_secondary))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(view.path.to_string()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .p(px(2.))
                    .rounded(px(6.))
                    .bg(p.bg_sunken)
                    .border_1()
                    .border_color(p.border)
                    .child(segment("mode-diff", t!("file.diff").into_owned(), None))
                    .child(segment(
                        "mode-blame",
                        t!("file.blame").into_owned(),
                        Some(FileMode::Blame),
                    ))
                    .child(segment(
                        "mode-history",
                        t!("file.history").into_owned(),
                        Some(FileMode::History),
                    )),
            );

        let note = |text: String| empty_state("file", "", text, p).into_any_element();
        let body = match mode {
            FileMode::Blame => match view.blame.clone() {
                None => note(t!("detail.loading").into_owned()),
                Some(lines) if lines.is_empty() => note(t!("file.no_blame").into_owned()),
                Some(lines) => {
                    let picked = view.picked.clone();
                    let handle = self.repo.clone();
                    uniform_list(
                        "blame",
                        lines.len(),
                        cx.processor(move |_, range: Range<usize>, _, _| {
                            range
                                .map(|ix| {
                                    let (repo, oid) = (handle.clone(), lines[ix].oid.clone());
                                    blame_row(ix, &lines, picked.as_ref(), mono.clone(), p)
                                        .on_click(move |_, _, cx| {
                                            repo.update(cx, |repo, cx| {
                                                repo.pick_blame_commit(oid.clone(), cx)
                                            })
                                        })
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .into_any_element()
                }
            },
            FileMode::History => match view.history.clone() {
                None => note(t!("detail.loading").into_owned()),
                Some(list) if list.is_empty() => note(t!("file.no_history").into_owned()),
                Some(list) => {
                    let rows = list.iter().enumerate().map(|(ix, entry)| {
                        let repo = self.repo.clone();
                        let picked = view.picked.as_ref() == Some(&entry.commit.oid);
                        div()
                            .id(("file-commit-row", ix))
                            .on_click(move |_, _, cx| {
                                repo.update(cx, |repo, cx| repo.pick_file_commit(ix, cx))
                            })
                            .child(history_row(ix, entry, picked, p))
                    });
                    let below = if view.picked.is_some() {
                        self.diff.clone().into_any_element()
                    } else {
                        note(t!("file.history_hint").into_owned())
                    };
                    v_flex()
                        .flex_1()
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .id("file-history")
                                .flex_none()
                                .max_h(px(240.))
                                .border_b_1()
                                .border_color(p.border)
                                .children(rows)
                                .overflow_y_scrollbar(),
                        )
                        .child(div().flex_1().overflow_hidden().child(below))
                        .into_any_element()
                }
            },
        };

        // The picked commit, with the way to it in the graph.
        let picked = view.picked.as_ref().and_then(|oid| {
            let from_blame = view.blame.as_ref().and_then(|lines| {
                let line = lines.iter().find(|line| &line.oid == oid)?;
                Some((line.summary.to_string(), line.author.to_string(), line.time))
            });
            let from_history = || {
                let list = view.history.as_ref()?;
                let entry = list.iter().find(|entry| &entry.commit.oid == oid)?;
                let commit = &entry.commit;
                Some((
                    commit.summary.to_string(),
                    commit.author_name.to_string(),
                    commit.author_time,
                ))
            };
            let found = match mode {
                FileMode::Blame => from_blame,
                FileMode::History => from_history(),
            };
            found.map(|(summary, author, time)| (oid.clone(), summary, author, time))
        });
        let footer = picked.map(|(oid, summary, author, time)| {
            let repo = self.repo.clone();
            let target = oid.clone();
            h_flex()
                .flex_none()
                .h(px(40.))
                .px_3()
                .gap_2()
                .border_t_1()
                .border_color(p.border)
                .text_size(px(12.))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(p.accent)
                        .child(short(&oid)),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_color(p.text_secondary)
                        .child(format!("{summary} · {author} · {}", local_time(time))),
                )
                .child(
                    Button::new("show-commit")
                        .small()
                        .label(t!("file.show_commit").into_owned())
                        .on_click(move |_, _, cx| {
                            repo.update(cx, |repo, cx| repo.select_oid(&target, cx))
                        }),
                )
        });

        panel
            .child(header)
            .child(body)
            .children(footer)
            .into_any_element()
    }
}
