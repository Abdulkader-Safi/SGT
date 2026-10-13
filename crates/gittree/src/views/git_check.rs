use gittree_core::ops::setup::MIN_GIT;
use gittree_core::{GitError, GitErrorKind};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, IntoElement, ParentElement, Render, Styled, Window,
    div, px,
};
use rust_i18n::t;

use super::graph::mono;
use super::ui::{big_icon, kbd};
use crate::app_state::{AppState, GitCheck};
use crate::error_message;
use crate::palette::palette;

/// Blocks the app until a usable Git is found (FR-REPO-07).
pub struct GitCheckView {
    app: Entity<AppState>,
    path: Entity<InputState>,
}

impl GitCheckView {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = app.read(cx).settings.git_path.clone();
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/usr/bin/git")
                .default_value(current.map(|p| p.display().to_string()).unwrap_or_default())
        });
        cx.subscribe(&path, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.retry(cx);
            }
        })
        .detach();
        Self { app, path }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let typed = self.path.read(cx).value().trim().to_owned();
        let path = (!typed.is_empty()).then(|| typed.into());
        self.app.update(cx, |app, cx| {
            if app.settings.git_path == path {
                app.check_git(cx);
            } else {
                // Changing the path checks again by itself.
                app.update_settings(|settings| settings.git_path = path, cx);
            }
        });
    }
}

impl Render for GitCheckView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mono = mono(cx);
        let error: Option<GitError> = match &self.app.read(cx).git {
            GitCheck::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let required = format!("{}.{}", MIN_GIT.0, MIN_GIT.1);
        let too_old = error
            .as_ref()
            .is_some_and(|error| error.kind == GitErrorKind::GitTooOld);
        let found = match &error {
            Some(error) if too_old => error.stderr.trim().to_owned(),
            _ => t!("git_check.found_none").into_owned(),
        };
        let row = |label: String, value: String, color| {
            h_flex()
                .h(px(32.))
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_color(p.text_secondary)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_family(mono.clone())
                        .text_size(px(12.))
                        .text_color(color)
                        .child(value),
                )
        };

        v_flex()
            .size_full()
            .bg(p.bg_app)
            .text_size(px(13.))
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(480.))
                    .gap_5()
                    .items_center()
                    .child(big_icon("warning", p.warning, p.warning_subtle, p))
                    .child(
                        v_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("git_check.title").into_owned()),
                            )
                            .children(error.as_ref().map(|error| {
                                div()
                                    .text_center()
                                    .text_color(p.text_secondary)
                                    .child(error_message(error.kind))
                            })),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .rounded(px(8.))
                            .bg(p.bg_panel)
                            .border_1()
                            .border_color(p.border)
                            .overflow_hidden()
                            .child(row(
                                t!("git_check.found_label").into_owned(),
                                found,
                                if too_old { p.danger } else { p.text_tertiary },
                            ))
                            .child(row(
                                t!("git_check.required_label").into_owned(),
                                t!("git_check.required", version = required).into_owned(),
                                p.text,
                            )),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text_secondary)
                                    .child(t!("git_check.path_label").into_owned()),
                            )
                            .child(Input::new(&self.path).id("git-path"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.text_tertiary)
                                    .child(t!("git_check.path_help").into_owned()),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.text_tertiary)
                                    .child(t!("git_check.install_with").into_owned()),
                            )
                            .child(kbd("xcode-select --install", p).font_family(mono.clone()))
                            .child(div().flex_1())
                            .child(
                                Button::new("retry")
                                    .primary()
                                    .label(t!("git_check.retry").into_owned())
                                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                            ),
                    ),
            )
    }
}
