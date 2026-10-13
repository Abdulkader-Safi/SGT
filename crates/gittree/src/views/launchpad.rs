//! The Launchpad: open pull requests and issues that concern the user,
//! read through `gh` (FR-AGENT-06). Shown in place of the graph.

use gittree_core::hosting::{Checks, Issue, PullRequest};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::dialogs;
use super::ui::{badge, caption, empty_state, ic};
use crate::palette::{Palette, palette};
use crate::repo_state::RepoState;

pub struct LaunchpadView {
    repo: Entity<RepoState>,
}

impl LaunchpadView {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self { repo }
    }
}

/// A row that opens its address in the browser.
fn row(id: (&'static str, u64), url: &str, p: Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    let url = url.to_owned();
    h_flex()
        .id(id)
        .h(px(42.))
        .flex_none()
        .px(px(14.))
        .gap_3()
        .cursor_pointer()
        .border_b_1()
        .border_color(p.border)
        .hover(|style| style.bg(p.bg_hover))
        .on_click(move |_, _, cx| cx.open_url(&url))
}

fn title_and_note(title: &str, note: String, p: Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .overflow_hidden()
        .child(div().truncate().child(title.to_owned()))
        .child(
            div()
                .truncate()
                .text_size(px(12.))
                .text_color(p.text_tertiary)
                .child(note),
        )
        .into_any_element()
}

fn pr_row(pr: &PullRequest, p: Palette) -> AnyElement {
    let (label, colour, subtle) = match pr.checks {
        Checks::None => (String::new(), p.text_tertiary, p.bg_hover),
        Checks::Pass => (
            t!("launchpad.checks_pass").into_owned(),
            p.success,
            p.success_subtle,
        ),
        Checks::Running => (
            t!("launchpad.checks_running").into_owned(),
            p.info,
            p.accent_subtle,
        ),
        Checks::Failing(count) => (
            t!("launchpad.checks_failing", count = count).into_owned(),
            p.danger,
            p.danger_subtle,
        ),
    };
    let review = match pr.review.as_str() {
        _ if pr.draft => t!("launchpad.draft").into_owned(),
        "APPROVED" => t!("launchpad.approved").into_owned(),
        "CHANGES_REQUESTED" => t!("launchpad.changes_requested").into_owned(),
        _ => String::new(),
    };
    let date = pr.updated.get(..10).unwrap_or_default();
    row(("pr", pr.number), &pr.url, p)
        .child(ic(
            "merge",
            14.,
            if pr.needs_work() { p.danger } else { p.success },
        ))
        .child(title_and_note(
            &pr.title,
            format!("#{} · {} · {date}", pr.number, pr.author),
            p,
        ))
        .when(!review.is_empty(), |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(p.text_secondary)
                    .child(review),
            )
        })
        .child(
            h_flex()
                .flex_none()
                .gap(px(6.))
                .text_size(px(12.))
                .child(
                    div()
                        .text_color(p.success)
                        .child(format!("+{}", pr.additions)),
                )
                .child(
                    div()
                        .text_color(p.danger)
                        .child(format!("-{}", pr.deletions)),
                ),
        )
        .when(!label.is_empty(), |row| {
            row.child(
                h_flex()
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
                    .child(label),
            )
        })
        .into_any_element()
}

fn issue_row(issue: &Issue, p: Palette) -> AnyElement {
    let date = issue.updated.get(..10).unwrap_or_default();
    row(("issue", issue.number), &issue.url, p)
        .child(ic("info", 14., p.text_secondary))
        .child(title_and_note(
            &issue.title,
            format!("#{} · {} · {date}", issue.number, issue.author),
            p,
        ))
        .into_any_element()
}

fn group(title: String, rows: Vec<AnyElement>, p: Palette) -> Option<AnyElement> {
    (!rows.is_empty()).then(|| {
        v_flex()
            .child(
                h_flex()
                    .h(px(30.))
                    .px(px(14.))
                    .gap_2()
                    .bg(p.bg_panel)
                    .border_b_1()
                    .border_color(p.border)
                    .child(caption(title.to_uppercase(), p))
                    .child(badge(rows.len().to_string(), p)),
            )
            .children(rows)
            .into_any_element()
    })
}

impl Render for LaunchpadView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let repo = self.repo.read(cx);
        let state = &repo.launchpad;
        let issues_tab = state.issues_tab;
        let can_create = repo.repository.trusted && repo.pull_request_branches().is_some();

        let tab = |id: &'static str, label: String, issues: bool| {
            let on = issues == issues_tab;
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
                .when(on, |tab| {
                    tab.bg(p.bg_elevated)
                        .border_1()
                        .border_color(p.border_strong)
                })
                .when(!on, |tab| {
                    tab.cursor_pointer().on_click(move |_, _, cx| {
                        repo.update(cx, |repo, cx| {
                            repo.launchpad.issues_tab = issues;
                            cx.notify();
                        })
                    })
                })
                .child(label)
        };
        let (refresh, create) = (self.repo.clone(), self.repo.clone());
        let header =
            h_flex()
                .flex_none()
                .h(px(44.))
                .px(px(14.))
                .gap_3()
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("launchpad.title").into_owned()),
                )
                .child(
                    h_flex()
                        .flex_none()
                        .p(px(2.))
                        .rounded(px(6.))
                        .bg(p.bg_sunken)
                        .border_1()
                        .border_color(p.border)
                        .child(tab(
                            "tab-prs",
                            t!("launchpad.pull_requests").into_owned(),
                            false,
                        ))
                        .child(tab("tab-issues", t!("launchpad.issues").into_owned(), true)),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_right()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .children(state.data.as_ref().map(|data| {
                            t!("launchpad.signed_in", login = &data.login).into_owned()
                        })),
                )
                .child(
                    Button::new("launchpad-refresh")
                        .small()
                        .loading(state.loading)
                        .label(t!("launchpad.refresh").into_owned())
                        .on_click(move |_, _, cx| {
                            refresh.update(cx, |repo, cx| repo.load_launchpad(cx))
                        }),
                )
                .when(can_create, |header| {
                    header.child(
                        Button::new("create-pr")
                            .small()
                            .primary()
                            .label(t!("launchpad.create").into_owned())
                            .on_click(move |_, window, cx| {
                                dialogs::create_pull_request(create.clone(), window, cx)
                            }),
                    )
                });

        let note = |icon: &str, title: String, text: String| {
            empty_state(icon, title, text, p).into_any_element()
        };
        let body = match (&state.data, &state.error) {
            (_, Some(problem)) => note(
                "warning",
                t!("launchpad.error_title").into_owned(),
                problem.clone(),
            ),
            (None, None) => note("fetch", String::new(), t!("launchpad.loading").into_owned()),
            (Some(data), None) if issues_tab => {
                let rows: Vec<AnyElement> = data.issues.iter().map(|i| issue_row(i, p)).collect();
                match group(t!("launchpad.assigned").into_owned(), rows, p) {
                    Some(list) => v_flex()
                        .id("issues")
                        .flex_1()
                        .child(list)
                        .overflow_y_scrollbar()
                        .into_any_element(),
                    None => note(
                        "check",
                        String::new(),
                        t!("launchpad.no_issues").into_owned(),
                    ),
                }
            }
            (Some(data), None) => {
                let rows = |keep: &dyn Fn(&PullRequest) -> bool, from: &[PullRequest]| {
                    from.iter()
                        .filter(|pr| keep(pr))
                        .map(|pr| pr_row(pr, p))
                        .collect::<Vec<_>>()
                };
                let groups: Vec<AnyElement> = [
                    group(
                        t!("launchpad.to_review").into_owned(),
                        rows(&|_| true, &data.to_review),
                        p,
                    ),
                    group(
                        t!("launchpad.waiting").into_owned(),
                        rows(&|pr| !pr.draft && !pr.needs_work(), &data.mine),
                        p,
                    ),
                    group(
                        t!("launchpad.needs_work").into_owned(),
                        rows(&|pr| !pr.draft && pr.needs_work(), &data.mine),
                        p,
                    ),
                    group(
                        t!("launchpad.drafts").into_owned(),
                        rows(&|pr| pr.draft, &data.mine),
                        p,
                    ),
                ]
                .into_iter()
                .flatten()
                .collect();
                if groups.is_empty() {
                    note(
                        "check",
                        t!("launchpad.none_title").into_owned(),
                        t!("launchpad.none").into_owned(),
                    )
                } else {
                    v_flex()
                        .id("pull-requests")
                        .flex_1()
                        .children(groups)
                        .overflow_y_scrollbar()
                        .into_any_element()
                }
            }
        };
        v_flex().size_full().bg(p.bg_app).child(header).child(body)
    }
}
