//! Agent sessions: each a coding agent at work in its own worktree
//! (FR-AGENT-04). Shown in place of the graph.

use gittree_core::agent::{Session, SessionState};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, div, px,
};
use rust_i18n::t;

use super::dialogs;
use super::graph::mono;
use super::ui::{empty_state, ic};
use crate::app_state::app_state;
use crate::palette::{Palette, palette};
use crate::repo_state::{Op, RepoState};

pub struct AgentsView {
    repo: Entity<RepoState>,
}

impl AgentsView {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self { repo }
    }

    fn card(
        &self,
        ix: usize,
        session: &Session,
        on_base: bool,
        p: Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (label, colour, subtle) = match session.state {
            SessionState::Working => (t!("agents.state_working"), p.info, p.accent_subtle),
            SessionState::Done => (t!("agents.state_done"), p.success, p.success_subtle),
            SessionState::Failed => (t!("agents.state_failed"), p.danger, p.danger_subtle),
            SessionState::Stopped => (t!("agents.state_stopped"), p.warning, p.warning_subtle),
        };
        let state = h_flex()
            .flex_none()
            .h(px(20.))
            .px_2()
            .gap(px(6.))
            .rounded(px(10.))
            .bg(subtle)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colour)
            .child(div().size(px(6.)).rounded_full().bg(colour))
            .child(label.into_owned());
        let initial: String = session
            .agent
            .chars()
            .take(1)
            .collect::<String>()
            .to_uppercase();
        // What the agent last said, or the task while it has said nothing.
        let text = if session.note.is_empty() {
            session.task.clone()
        } else {
            session.note.clone()
        };
        let working = session.state == SessionState::Working;
        let branch = session.branch.clone();
        let button = |id: &'static str, label: String| Button::new((id, ix)).small().label(label);
        let actions = h_flex()
            .gap_2()
            .when(working, |row| {
                let (repo, branch) = (self.repo.clone(), branch.clone());
                row.child(button("stop", t!("agents.stop").into_owned()).on_click(
                    move |_, _, cx| repo.update(cx, |repo, _| repo.stop_session(&branch)),
                ))
            })
            .when(!working, |row| {
                let (repo, branch) = (self.repo.clone(), branch.clone());
                row.child(button("remove", t!("agents.remove").into_owned()).on_click(
                    move |_, _, cx| repo.update(cx, |repo, cx| repo.remove_session(&branch, cx)),
                ))
            })
            .child({
                // The worktree opens as a repository of its own, where the
                // agent's changes can be read, staged and committed.
                let path = session.path.clone();
                button("review", t!("agents.review").into_owned()).on_click(move |_, _, cx| {
                    let path = path.clone();
                    app_state(cx).update(cx, |app, cx| app.open(path, cx))
                })
            })
            .when(session.state == SessionState::Done && on_base, |row| {
                let (repo, branch) = (self.repo.clone(), branch.clone());
                row.child(
                    button(
                        "merge",
                        t!("agents.merge", base = &session.base).into_owned(),
                    )
                    .primary()
                    .on_click(move |_, _, cx| {
                        let op = Op::Merge {
                            branch: branch.clone(),
                            no_ff: false,
                        };
                        repo.update(cx, |repo, cx| repo.run_op(op, cx))
                    }),
                )
            });
        v_flex()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(10.))
            .bg(p.bg_panel)
            .border_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .gap(px(10.))
                    .child(
                        h_flex()
                            .size(px(28.))
                            .flex_none()
                            .justify_center()
                            .rounded(px(6.))
                            .bg(p.bg_elevated)
                            .border_1()
                            .border_color(p.border)
                            .font_weight(FontWeight::MEDIUM)
                            .child(initial),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(session.agent.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap(px(4.))
                                    .text_size(px(11.))
                                    .text_color(p.text_secondary)
                                    .font_family(mono(cx))
                                    .child(ic("branch", 12., p.text_tertiary))
                                    .child(div().truncate().child(session.branch.clone())),
                            ),
                    )
                    .child(state),
            )
            .child(div().text_color(p.text_secondary).child(text))
            .child(h_flex().child(div().flex_1()).child(actions))
            .into_any_element()
    }
}

impl Render for AgentsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let repo = self.repo.read(cx);
        let trusted = repo.repository.trusted;
        let head = match &repo.repository.head {
            gittree_core::model::Head::Branch(name) => Some(name.as_str()),
            _ => None,
        };
        let handle = self.repo.clone();
        let header = h_flex()
            .flex_none()
            .h(px(44.))
            .px(px(14.))
            .gap_3()
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.title").into_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(p.text_tertiary)
                    .child(t!("agents.hint").into_owned()),
            )
            .when(trusted, |header| {
                header.child(
                    Button::new("new-session")
                        .small()
                        .primary()
                        .label(t!("agents.new").into_owned())
                        .on_click(move |_, window, cx| {
                            dialogs::start_session(handle.clone(), window, cx)
                        }),
                )
            });
        let body = if repo.sessions.is_empty() {
            empty_state(
                "branch",
                t!("agents.empty_title").into_owned(),
                t!("agents.empty").into_owned(),
                p,
            )
            .into_any_element()
        } else {
            v_flex()
                .id("sessions")
                .flex_1()
                .p(px(14.))
                .gap(px(12.))
                // The newest first.
                .children(repo.sessions.iter().enumerate().rev().map(|(ix, session)| {
                    let on_base = head == Some(session.base.as_str());
                    self.card(ix, session, on_base, p, cx)
                }))
                .overflow_y_scrollbar()
                .into_any_element()
        };
        v_flex().size_full().bg(p.bg_app).child(header).child(body)
    }
}
