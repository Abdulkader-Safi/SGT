use gittree_core::model::RepoPath;
use gittree_core::ops::merge::Side;
use gittree_core::parse::conflict::{Choice, parse_conflicts};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::graph::mono;
use super::ui::{big_icon, caption, empty_state, ic, status_mark};
use crate::palette::palette;
use crate::repo_state::{Op, RepoState};

/// Resolves conflicted files: each region by side or both, the result by
/// hand, and whole files for binary and delete conflicts (FR-MERGE-05).
pub struct ConflictView {
    repo: Entity<RepoState>,
    /// The result, which the user may also edit.
    result: Entity<TextareaState>,
    /// The choices the result text was last built from.
    built: Option<(RepoPath, Vec<Choice>)>,
}

impl ConflictView {
    pub fn new(repo: Entity<RepoState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let result = cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 12));
        cx.subscribe(&result, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.observe_in(&repo, window, |this, _, window, cx| {
            this.sync(window, cx);
            cx.notify();
        })
        .detach();
        Self {
            repo,
            result,
            built: None,
        }
    }

    /// Opens the first conflicted file when none is open, and rebuilds the
    /// result text when a choice changed.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.read(cx);
        if !repo.resolving {
            self.built = None;
            return;
        }
        let Some(file) = &repo.conflict else {
            // Not one that was just resolved and is waiting for the next
            // status read to say so.
            let next = repo
                .status
                .iter()
                .find(|entry| entry.conflict.is_some() && !repo.resolved.contains(&entry.path))
                .map(|entry| entry.path.clone());
            self.built = None;
            if let Some(path) = next.filter(|_| !repo.busy) {
                self.repo
                    .update(cx, |repo, cx| repo.open_conflict(path, cx));
            }
            return;
        };
        let key = (file.path.clone(), file.choices.clone());
        if self.built.as_ref() != Some(&key) {
            let text = String::from_utf8_lossy(&file.result()).into_owned();
            self.built = Some(key);
            self.result
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

impl Render for ConflictView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mono = mono(cx);
        let repo = self.repo.read(cx);
        let busy = repo.busy;
        let open: Vec<&RepoPath> = repo
            .status
            .iter()
            .filter(|entry| entry.conflict.is_some() && !repo.resolved.contains(&entry.path))
            .map(|entry| &entry.path)
            .collect();
        let total = open.len() + repo.resolved.len();
        let panel = v_flex()
            .size_full()
            .bg(p.bg_panel)
            .border_l_1()
            .border_color(p.border);

        let header = h_flex()
            .flex_none()
            .h(px(44.))
            .px(px(14.))
            .gap_2()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("conflict.title").into_owned()),
            )
            .child(
                div().text_size(px(12.)).text_color(p.text_secondary).child(
                    t!(
                        "conflict.progress",
                        done = repo.resolved.len(),
                        total = total
                    )
                    .into_owned(),
                ),
            );
        let current = repo.conflict.as_ref().map(|file| file.path.clone());
        let file_row = |ix: usize, path: &RepoPath, done: bool| {
            let selected = current.as_ref() == Some(path);
            let (handle, target) = (self.repo.clone(), path.clone());
            h_flex()
                .id(("conflict-file", ix))
                .h(px(28.))
                .px_3()
                .gap_2()
                .when(selected, |row| row.bg(p.bg_selected))
                .when(!done, |row| {
                    row.cursor_pointer().on_click(move |_, _, cx| {
                        handle.update(cx, |repo, cx| repo.open_conflict(target.clone(), cx))
                    })
                })
                .child(if done {
                    ic("check", 14., p.success)
                } else {
                    status_mark(gittree_core::model::Change::Modified, true, p)
                        .font_family(mono.clone())
                        .into_any_element()
                })
                .child(div().flex_1().truncate().child(path.to_string()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if done { p.success } else { p.danger })
                        .child(if done {
                            t!("conflict.resolved").into_owned()
                        } else {
                            t!("conflict.unresolved").into_owned()
                        }),
                )
        };
        let files = v_flex()
            .flex_none()
            .border_b_1()
            .border_color(p.border)
            .children(
                open.iter()
                    .enumerate()
                    .map(|(ix, path)| file_row(ix, path, false)),
            )
            .children(
                repo.resolved
                    .iter()
                    .enumerate()
                    .map(|(ix, path)| file_row(open.len() + ix, path, true)),
            );

        // Everything is resolved: the operation can go on.
        if open.is_empty() {
            let handle = self.repo.clone();
            return panel
                .child(header)
                .child(files)
                .child(
                    empty_state(
                        "check",
                        t!("conflict.all_resolved").into_owned(),
                        t!("conflict.all_resolved_text").into_owned(),
                        p,
                    )
                    .child(
                        Button::new("continue-op")
                            .primary()
                            .disabled(busy)
                            .label(t!("banner.continue").into_owned())
                            .on_click(move |_, _, cx| {
                                handle.update(cx, |repo, cx| repo.run_op(Op::ContinueOp, cx))
                            }),
                    ),
                )
                .into_any_element();
        }
        let Some(file) = &repo.conflict else {
            return panel.child(header).child(files).into_any_element();
        };
        let path = file.path.clone();

        // Binary files and files one side deleted have no regions.
        if file.regions.is_empty() {
            let side = |id: &'static str, label: String, side: Side| {
                let (handle, path) = (self.repo.clone(), path.clone());
                Button::new(id)
                    .label(label)
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let op = Op::ResolveSide {
                            path: path.clone(),
                            side,
                        };
                        handle.update(cx, |repo, cx| repo.run_op(op, cx))
                    })
            };
            return panel
                .child(header)
                .child(files)
                .child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .p_6()
                        .child(big_icon("file", p.text_secondary, p.bg_elevated, p))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("conflict.whole_file").into_owned()),
                        )
                        .child(
                            div()
                                .max_w(px(360.))
                                .text_center()
                                .text_color(p.text_secondary)
                                .child(
                                    t!("conflict.whole_file_text", path = path.to_string())
                                        .into_owned(),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(side(
                                    "take-ours",
                                    t!("conflict.keep_current").into_owned(),
                                    Side::Ours,
                                ))
                                .child(side(
                                    "take-theirs",
                                    t!("conflict.take_incoming_file").into_owned(),
                                    Side::Theirs,
                                )),
                        ),
                )
                .into_any_element();
        }

        let block = |label: String, lane: usize, text: String, dim: bool, taken: bool| {
            v_flex()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .overflow_hidden()
                .when(dim, |block| block.opacity(0.4))
                .child(
                    h_flex()
                        .h(px(28.))
                        .px(px(10.))
                        .gap_2()
                        .bg(p.bg_elevated)
                        .border_b_1()
                        .border_color(p.border)
                        .child(div().size(px(8.)).rounded_full().bg(p.lanes[lane]))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .when(taken, |head| head.child(ic("check", 12., p.success))),
                )
                .child(
                    div()
                        .p_2()
                        .bg(p.bg_app)
                        .font_family(mono.clone())
                        .text_size(px(12.))
                        .child(SharedString::from(text)),
                )
        };
        let count = file.regions.len();
        let regions =
            file.regions.iter().enumerate().map(|(ix, region)| {
                let choice = file.choices[ix].clone();
                let text = |range: &std::ops::Range<usize>| {
                    String::from_utf8_lossy(&file.content[range.clone()])
                        .trim_end_matches('\n')
                        .replace('\t', "    ")
                };
                let pick = |id: &'static str, label: String, choice: Choice| {
                    let handle = self.repo.clone();
                    Button::new((id, ix))
                        .xsmall()
                        .label(label)
                        .on_click(move |_, _, cx| {
                            handle.update(cx, |repo, cx| repo.choose(ix, choice.clone(), cx))
                        })
                };
                let open = choice == Choice::Unresolved;
                v_flex()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(
                                t!("conflict.region", n = ix + 1, count = count).into_owned(),
                            ))
                            .when(open, |row| {
                                row.child(pick(
                                    "take-current",
                                    t!("conflict.take_current").into_owned(),
                                    Choice::Ours,
                                ))
                                .child(pick(
                                    "take-incoming",
                                    t!("conflict.take_incoming").into_owned(),
                                    Choice::Theirs,
                                ))
                                .child(pick(
                                    "take-both",
                                    t!("conflict.take_both").into_owned(),
                                    Choice::Both,
                                ))
                            })
                            .when(!open, |row| {
                                row.child(
                                    pick(
                                        "change",
                                        t!("conflict.change").into_owned(),
                                        Choice::Unresolved,
                                    )
                                    .ghost(),
                                )
                            }),
                    )
                    .child(block(
                        t!("conflict.current").into_owned(),
                        0,
                        text(&region.ours),
                        matches!(choice, Choice::Theirs),
                        matches!(choice, Choice::Ours | Choice::Both),
                    ))
                    .child(block(
                        t!("conflict.incoming").into_owned(),
                        1,
                        text(&region.theirs),
                        matches!(choice, Choice::Ours),
                        matches!(choice, Choice::Theirs | Choice::Both),
                    ))
            });

        let typed = self.result.read(cx).value().to_string();
        let left = parse_conflicts(typed.as_bytes()).len();
        let (handle, result) = (self.repo.clone(), self.result.clone());
        let resolve_path = path.clone();
        panel
            .child(header)
            .child(files)
            .child(
                v_flex()
                    .id("conflict-regions")
                    .flex_1()
                    .children(regions)
                    .child(
                        v_flex()
                            .p_3()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(caption(t!("conflict.result").into_owned(), p))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.))
                                            .text_color(p.text_tertiary)
                                            .child(t!("conflict.edit_hint").into_owned()),
                                    )
                                    .when(left > 0, |row| {
                                        row.child(
                                            div().text_size(px(12.)).text_color(p.danger).child(
                                                t!("conflict.left", count = left).into_owned(),
                                            ),
                                        )
                                    }),
                            )
                            .child(Textarea::new(&self.result).small())
                            .child(
                                h_flex().child(div().flex_1()).child(
                                    Button::new("mark-resolved")
                                        .primary()
                                        .disabled(left > 0 || busy)
                                        .label(t!("conflict.mark_resolved").into_owned())
                                        .on_click(move |_, _, cx| {
                                            let content =
                                                result.read(cx).value().to_string().into_bytes();
                                            let op = Op::ResolveFile {
                                                path: resolve_path.clone(),
                                                content,
                                            };
                                            handle.update(cx, |repo, cx| repo.run_op(op, cx))
                                        }),
                                ),
                            ),
                    )
                    .overflow_y_scrollbar(),
            )
            .into_any_element()
    }
}
