use std::ops::Range;

use gittree_core::model::{Change, DiffKind, FileDiff, LineKind};
use gittree_core::ops::read::LARGE_DIFF_SHOWN;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div, px, uniform_list,
};
use rust_i18n::t;

use crate::app_state::app_state;
use crate::local_time;
use crate::repo_state::RepoState;

const LINE_HEIGHT: f32 = 20.;

/// The selected commit: its header, the files it changed and the diff of
/// one of them (FR-GRAPH-07, FR-DIFF-01, FR-DIFF-09).
pub struct CommitDetail {
    repo: Entity<RepoState>,
    diff_scroll: UniformListScrollHandle,
}

impl CommitDetail {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            repo,
            diff_scroll: UniformListScrollHandle::new(),
        }
    }
}

fn change_mark(change: Change) -> &'static str {
    match change {
        Change::Added | Change::Untracked => "A",
        Change::Deleted => "D",
        Change::Renamed => "R",
        Change::Copied => "C",
        Change::TypeChange => "T",
        Change::Modified | Change::Unmodified => "M",
    }
}

/// A diff row: a hunk header, or line `1` of hunk `0`.
#[derive(Clone, Copy)]
enum DiffRow {
    Hunk(usize),
    Line(usize, usize),
}

fn diff_rows(diff: &FileDiff) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for (h, hunk) in diff.hunks.iter().enumerate() {
        rows.push(DiffRow::Hunk(h));
        rows.extend((0..hunk.lines.len()).map(|l| DiffRow::Line(h, l)));
    }
    rows
}

impl Render for CommitDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, hover, selected_bg, link) = (
            theme.muted_foreground,
            theme.border,
            theme.list_hover,
            theme.list_active,
            theme.link,
        );
        let (added_bg, removed_bg, hunk_bg) = (
            theme.success.opacity(0.15),
            theme.danger.opacity(0.15),
            theme.muted,
        );
        let settings = &app_state(cx).read(cx).settings;
        let (mono, mono_size) = (
            SharedString::from(settings.diff_font_family.clone()),
            px(settings.diff_font_size),
        );
        let repo = self.repo.read(cx);
        let selection = &repo.selection;

        let Some(details) = &selection.details else {
            let text = if selection.oid.is_some() {
                t!("detail.loading")
            } else {
                t!("detail.none")
            };
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(muted)
                .child(text.into_owned())
                .into_any_element();
        };

        let field = |label: String, value: gpui_kit::AnyElement| {
            h_flex()
                .gap_2()
                .items_start()
                .child(div().w(px(76.)).flex_none().text_color(muted).child(label))
                .child(div().flex_1().overflow_hidden().child(value))
        };
        let person = |name: &str, email: &str, time: i64| {
            format!("{name} <{email}>, {}", local_time(time)).into_any_element()
        };
        let parents = h_flex()
            .gap_2()
            .children(details.parents.iter().enumerate().map(|(ix, parent)| {
                let (repo, oid) = (self.repo.clone(), parent.clone());
                div()
                    .id(("parent", ix))
                    .cursor_pointer()
                    .text_color(link)
                    .child(parent[..7.min(parent.len())].to_string())
                    .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&oid, cx)))
            }));
        let header = v_flex()
            .p_3()
            .gap_1()
            .border_b_1()
            .border_color(border)
            .child(div().pb_1().child(details.message.clone()))
            .child(field(
                t!("detail.author").into_owned(),
                person(
                    &details.author_name,
                    &details.author_email,
                    details.author_time,
                ),
            ))
            .child(field(
                t!("detail.committer").into_owned(),
                person(
                    &details.committer_name,
                    &details.committer_email,
                    details.commit_time,
                ),
            ))
            .child(field(
                t!("detail.commit").into_owned(),
                details.oid.to_string().into_any_element(),
            ))
            .when(!details.parents.is_empty(), |header| {
                header.child(field(
                    t!("detail.parents").into_owned(),
                    parents.into_any_element(),
                ))
            });

        let files = v_flex()
            .id("files")
            .max_h(px(180.))
            .border_b_1()
            .border_color(border)
            .child(div().px_3().py_1().text_xs().text_color(muted).child(
                if selection.files.is_empty() {
                    t!("detail.no_files").into_owned()
                } else {
                    t!("detail.files", count = selection.files.len()).into_owned()
                },
            ))
            .children(selection.files.iter().enumerate().map(|(ix, file)| {
                let repo = self.repo.clone();
                let path = match &file.orig_path {
                    Some(from) => format!("{from} → {}", file.path),
                    None => file.path.to_string(),
                };
                h_flex()
                    .id(("file", ix))
                    .px_3()
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .when(selection.file == Some(ix), |row| row.bg(selected_bg))
                    .on_click(move |_, _, cx| {
                        repo.update(cx, |repo, cx| repo.select_file(ix, false, cx))
                    })
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_color(muted)
                            .child(change_mark(file.change)),
                    )
                    .child(div().flex_1().truncate().child(path))
            }))
            .overflow_y_scrollbar();

        let note = |text: String| div().p_3().text_color(muted).child(text).into_any_element();
        let diff = match &selection.diff {
            None if selection.diff_loading => note(t!("detail.loading").into_owned()),
            None => div().into_any_element(),
            Some(diff) => match diff.kind {
                DiffKind::Binary { old_size, new_size } => {
                    note(t!("detail.binary", old = old_size, new = new_size).into_owned())
                }
                DiffKind::ModeOnly => note(t!("detail.mode_only").into_owned()),
                _ if diff.hunks.is_empty() => note(t!("detail.unchanged").into_owned()),
                _ => {
                    let rows = diff_rows(diff);
                    let list = uniform_list(
                        "diff",
                        rows.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let repo = this.repo.read(cx);
                            let Some(diff) = &repo.selection.diff else {
                                return Vec::new();
                            };
                            let number = |n: Option<u32>| {
                                div()
                                    .w(px(44.))
                                    .flex_none()
                                    .px_1()
                                    .text_right()
                                    .text_color(muted)
                                    .child(n.map(|n| n.to_string()).unwrap_or_default())
                            };
                            range
                                .filter_map(|ix| rows.get(ix).copied())
                                .map(|row| match row {
                                    DiffRow::Hunk(h) => {
                                        let hunk = &diff.hunks[h];
                                        h_flex()
                                            .h(px(LINE_HEIGHT))
                                            .w_full()
                                            .px_2()
                                            .bg(hunk_bg)
                                            .text_color(muted)
                                            .whitespace_nowrap()
                                            .child(format!(
                                                "@@ -{},{} +{},{} @@ {}",
                                                hunk.old_start,
                                                hunk.old_len,
                                                hunk.new_start,
                                                hunk.new_len,
                                                hunk.section
                                            ))
                                    }
                                    DiffRow::Line(h, l) => {
                                        let line = &diff.hunks[h].lines[l];
                                        let (sign, color) = match line.kind {
                                            LineKind::Add => ("+", Some(added_bg)),
                                            LineKind::Remove => ("-", Some(removed_bg)),
                                            LineKind::Context => (" ", None),
                                        };
                                        let text = String::from_utf8_lossy(&line.text);
                                        let text = text
                                            .trim_end_matches(['\n', '\r'])
                                            .replace('\t', "    ");
                                        h_flex()
                                            .h(px(LINE_HEIGHT))
                                            .w_full()
                                            .when_some(color, |row, color| row.bg(color))
                                            .child(number(line.old_no))
                                            .child(number(line.new_no))
                                            .child(div().w(px(14.)).flex_none().child(sign))
                                            .child(div().flex_1().whitespace_nowrap().child(text))
                                    }
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&self.diff_scroll)
                    .flex_1()
                    .w_full()
                    .font_family(mono)
                    .text_size(mono_size);

                    let cut = match diff.kind {
                        DiffKind::TooLarge { lines } => {
                            let repo = self.repo.clone();
                            let file = selection.file;
                            Some(
                                h_flex()
                                    .px_3()
                                    .py_1()
                                    .gap_3()
                                    .border_b_1()
                                    .border_color(border)
                                    .child(
                                        div().flex_1().text_color(muted).child(
                                            t!(
                                                "detail.too_large",
                                                lines = lines,
                                                shown = LARGE_DIFF_SHOWN
                                            )
                                            .into_owned(),
                                        ),
                                    )
                                    .child(
                                        Button::new("load-all")
                                            .small()
                                            .label(t!("detail.load_all").into_owned())
                                            .on_click(move |_, _, cx| {
                                                if let Some(file) = file {
                                                    repo.update(cx, |repo, cx| {
                                                        repo.select_file(file, true, cx)
                                                    });
                                                }
                                            }),
                                    ),
                            )
                        }
                        _ => None,
                    };
                    v_flex()
                        .size_full()
                        .children(cut)
                        .child(list)
                        .into_any_element()
                }
            },
        };

        v_flex()
            .size_full()
            .text_sm()
            .child(header)
            .child(files)
            .child(div().flex_1().overflow_hidden().child(diff))
            .into_any_element()
    }
}
