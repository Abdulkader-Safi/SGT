use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div,
};

use crate::zoom::px;
use rust_i18n::t;

use super::diff::DiffPane;
use super::graph::mono;
use super::ui::{caption, empty_state, ic, status_mark};
use crate::local_time;
use crate::palette::palette;
use crate::repo_state::{FileMode, RepoState};

/// The selected commit: its header, the files it changed and the diff of
/// one of them (FR-GRAPH-07).
pub struct CommitDetail {
    repo: Entity<RepoState>,
    diff: Entity<DiffPane>,
}

impl CommitDetail {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            diff: cx.new(|cx| DiffPane::new(repo.clone(), cx)),
            repo,
        }
    }
}

impl Render for CommitDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mono = mono(cx);
        let repo = self.repo.read(cx);
        let selection = &repo.selection;
        let panel = || {
            v_flex()
                .size_full()
                .bg(p.bg_panel)
                .border_l_1()
                .border_color(p.border)
        };

        let Some(details) = &selection.details else {
            let text = if selection.oid.is_some() {
                t!("detail.loading")
            } else {
                t!("detail.none")
            };
            return panel()
                .child(empty_state("commit", "", text.into_owned(), p))
                .into_any_element();
        };

        let (summary, body) = details
            .message
            .split_once('\n')
            .unwrap_or((&details.message, ""));
        let field = |label: String, value: gpui_kit::AnyElement| {
            h_flex()
                .gap(px(10.))
                .items_start()
                .text_size(px(12.))
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .text_color(p.text_tertiary)
                        .child(label),
                )
                .child(div().flex_1().overflow_hidden().child(value))
        };
        let person = |name: &str, email: &str, time: i64, lane: usize| {
            let initial: String = name.chars().take(1).collect::<String>().to_uppercase();
            h_flex()
                .gap(px(6.))
                .child(
                    h_flex()
                        .flex_none()
                        .size(px(18.))
                        .justify_center()
                        .rounded_full()
                        .bg(p.lanes[lane])
                        .text_size(px(11.))
                        .text_color(p.on_lane)
                        .child(initial),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .truncate()
                        .text_color(p.text_secondary)
                        .child(format!("{email} · {}", local_time(time))),
                )
                .into_any_element()
        };
        // The same person always gets the same colour.
        let lane = |name: &str| name.bytes().map(usize::from).sum::<usize>() % 8;
        let parents = h_flex().gap_2().font_family(mono.clone()).children(
            details.parents.iter().enumerate().map(|(ix, parent)| {
                let (repo, oid) = (self.repo.clone(), parent.clone());
                div()
                    .id(("parent", ix))
                    .cursor_pointer()
                    .text_color(p.accent)
                    .child(parent[..7.min(parent.len())].to_string())
                    .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&oid, cx)))
            }),
        );
        // "Explain this commit", then what the agent said about it.
        let explained = repo
            .ai
            .explained
            .as_ref()
            .filter(|(oid, _)| *oid == details.oid);
        let explanation = match explained {
            Some((_, text)) => {
                let copied = text.clone();
                v_flex()
                    .p(px(12.))
                    .gap_2()
                    .rounded(px(8.))
                    .bg(p.bg_elevated)
                    .border_1()
                    .border_color(p.border)
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(ic("commit", 14., p.accent))
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(t!("ai.explained").into_owned()),
                            )
                            .child(
                                div()
                                    .id("copy-explanation")
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .text_color(p.text_secondary)
                                    .child(t!("ai.copy").into_owned())
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copied.clone(),
                                        ))
                                    }),
                            ),
                    )
                    .child(div().text_color(p.text_secondary).child(text.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_tertiary)
                            .child(t!("ai.caveat").into_owned()),
                    )
                    .into_any_element()
            }
            None if repo.ai.explaining.as_ref() == Some(&details.oid) => div()
                .text_size(px(12.))
                .text_color(p.text_tertiary)
                .child(t!("ai.explaining").into_owned())
                .into_any_element(),
            None => {
                let repo = self.repo.clone();
                div()
                    .id("explain")
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .child(t!("ai.explain").into_owned())
                    .on_click(move |_, _, cx| {
                        let settings = &crate::app_state::app_state(cx).read(cx).settings;
                        let (kind, command) = (settings.ai_agent, settings.ai_command.clone());
                        repo.update(cx, |repo, cx| repo.explain_commit(kind, &command, cx))
                    })
                    .into_any_element()
            }
        };
        let oid = details.oid.to_string();
        let same_committer = details.author_name == details.committer_name
            && details.author_email == details.committer_email;
        let header = v_flex()
            .p(px(14.))
            .gap(px(10.))
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(summary.to_owned()),
            )
            .when(!body.trim().is_empty(), |header| {
                header.child(
                    div()
                        .text_color(p.text_secondary)
                        .child(body.trim().to_owned()),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(field(
                        t!("detail.author").into_owned(),
                        person(
                            &details.author_name,
                            &details.author_email,
                            details.author_time,
                            lane(&details.author_name),
                        ),
                    ))
                    .when(!same_committer, |fields| {
                        fields.child(field(
                            t!("detail.committer").into_owned(),
                            person(
                                &details.committer_name,
                                &details.committer_email,
                                details.commit_time,
                                lane(&details.committer_name),
                            ),
                        ))
                    })
                    .child(field(
                        t!("detail.commit").into_owned(),
                        h_flex()
                            .id("copy-hash")
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_color(p.text_secondary)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(oid.clone()))
                            })
                            .child(
                                div()
                                    .truncate()
                                    .font_family(mono.clone())
                                    .text_size(px(11.))
                                    .child(details.oid.to_string()),
                            )
                            .child(ic("copy", 12., p.text_tertiary))
                            .into_any_element(),
                    ))
                    .when(!details.parents.is_empty(), |fields| {
                        fields.child(field(
                            t!("detail.parents").into_owned(),
                            parents.into_any_element(),
                        ))
                    }),
            )
            .child(explanation);

        let file_link = |id: &'static str, mode: FileMode| {
            let repo = self.repo.clone();
            let label = match mode {
                FileMode::Blame => t!("file.blame"),
                FileMode::History => t!("file.history"),
            };
            div()
                .id(id)
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(p.accent)
                .child(label.into_owned())
                .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.show_file(mode, cx)))
        };
        let files = v_flex()
            .id("files")
            .max_h(px(180.))
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .h(px(28.))
                    .px_3()
                    .gap_3()
                    .child(caption(
                        if selection.files.is_empty() {
                            t!("detail.no_files").into_owned()
                        } else {
                            t!("detail.files", count = selection.files.len()).to_uppercase()
                        },
                        p,
                    ))
                    .child(div().flex_1())
                    // Blame and history are of the selected file.
                    .when(selection.file.is_some(), |row| {
                        row.child(file_link("blame", FileMode::Blame))
                            .child(file_link("history", FileMode::History))
                    }),
            )
            .children(selection.files.iter().enumerate().map(|(ix, file)| {
                let repo = self.repo.clone();
                let path = match &file.orig_path {
                    Some(from) => format!("{from} → {}", file.path),
                    None => file.path.to_string(),
                };
                let selected = selection.file == Some(ix);
                h_flex()
                    .id(("file", ix))
                    .h(px(26.))
                    .px_3()
                    .gap_2()
                    .cursor_pointer()
                    .when(selected, |row| row.bg(p.bg_selected))
                    .when(!selected, |row| row.hover(|style| style.bg(p.bg_hover)))
                    .on_click(move |_, _, cx| {
                        repo.update(cx, |repo, cx| repo.select_file(ix, false, cx))
                    })
                    .child(status_mark(file.change, false, p).font_family(mono.clone()))
                    .child(div().flex_1().truncate().child(path))
            }))
            .overflow_y_scrollbar();

        panel()
            .child(header)
            .child(files)
            .child(div().flex_1().overflow_hidden().child(self.diff.clone()))
            .into_any_element()
    }
}
