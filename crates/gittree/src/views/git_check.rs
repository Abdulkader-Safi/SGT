use gittree_core::ops::setup::MIN_GIT;
use gittree_core::{GitError, GitErrorKind};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use rust_i18n::t;

use crate::app_state::{AppState, GitCheck};
use crate::error_message;

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
        let error: Option<GitError> = match &self.app.read(cx).git {
            GitCheck::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let muted = cx.theme().muted_foreground;
        let required = format!("{}.{}", MIN_GIT.0, MIN_GIT.1);

        v_flex().size_full().items_center().justify_center().child(
            v_flex()
                .w(px(420.))
                .gap_3()
                .child(div().text_xl().child(t!("git_check.title").into_owned()))
                .children(error.map(|error| {
                    let found = (error.kind == GitErrorKind::GitTooOld)
                        .then(|| t!("git_check.found", version = error.stderr).into_owned());
                    v_flex()
                        .gap_1()
                        .child(error_message(error.kind))
                        .children(found)
                }))
                .child(
                    div()
                        .text_color(muted)
                        .child(t!("git_check.required", version = required).into_owned()),
                )
                .child(
                    div()
                        .text_color(muted)
                        .child(t!("git_check.path_label").into_owned()),
                )
                .child(Input::new(&self.path).id("git-path"))
                .child(
                    div().child(
                        Button::new("retry")
                            .primary()
                            .label(t!("git_check.retry").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                    ),
                ),
        )
    }
}
