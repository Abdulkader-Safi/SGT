use gittree_core::model::Change;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::diff::DiffPane;
use crate::local_time;
use crate::repo_state::RepoState;

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

        v_flex()
            .size_full()
            .text_sm()
            .child(header)
            .child(files)
            .child(div().flex_1().overflow_hidden().child(self.diff.clone()))
            .into_any_element()
    }
}
