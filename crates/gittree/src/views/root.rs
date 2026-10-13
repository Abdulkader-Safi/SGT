use std::path::PathBuf;

use gittree_core::GitError;
use gittree_core::store::WindowState;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::{
    AppContext, Context, Entity, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, WindowBounds, div, px,
};
use rust_i18n::t;

use super::git_check::GitCheckView;
use super::welcome::WelcomeView;
use super::workspace::Workspace;
use crate::app_state::{AppEvent, AppState, GitCheck};
use crate::error_message;

/// The content of the main window. Shows the Git check, the welcome screen
/// or the workspace, and turns app events into toasts and dialogs.
pub struct RootView {
    app: Entity<AppState>,
    git_check: Entity<GitCheckView>,
    welcome: Entity<WelcomeView>,
    workspace: Option<Entity<Workspace>>,
}

impl RootView {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_in(&app, window, |this, _, window, cx| {
            this.sync_workspace(window, cx);
            crate::theme::apply(cx);
            cx.notify();
        })
        .detach();
        cx.subscribe_in(
            &app,
            window,
            |_, app, event: &AppEvent, window, cx| match event {
                AppEvent::Error(error) => show_error(error.clone(), window, cx),
                AppEvent::NotARepository(path) => offer_init(app.clone(), path.clone(), window, cx),
                AppEvent::Notice(text) => {
                    window.push_notification(Notification::info(text.clone()), cx)
                }
            },
        )
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| crate::theme::apply(cx))
            .detach();
        cx.observe_window_bounds(window, |this, window, cx| {
            let state = window_state(window);
            this.app
                .update(cx, |app, _| app.update_session(|s| s.window = Some(state)));
        })
        .detach();

        let mut this = Self {
            git_check: cx.new(|cx| GitCheckView::new(app.clone(), window, cx)),
            welcome: cx.new(|cx| WelcomeView::new(app.clone(), cx)),
            workspace: None,
            app,
        };
        this.sync_workspace(window, cx);
        this
    }

    /// Keeps one workspace for the open repository, and none without one.
    fn sync_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.app.read(cx).repo.clone();
        let path = open
            .as_ref()
            .map(|repo| repo.read(cx).repository.path.clone());
        let shown = self.workspace.as_ref().map(|w| w.read(cx).path.clone());
        if path == shown {
            return;
        }
        let app = self.app.clone();
        self.workspace = open.map(|repo| cx.new(|cx| Workspace::new(app, repo, window, cx)));
    }
}

fn window_state(window: &Window) -> WindowState {
    let bounds = window.window_bounds();
    let origin = bounds.get_bounds().origin;
    // The position is the frame's, but the size is the content's: GPUI
    // reports the frame with its title bar and opens a window by content
    // size, so saving the frame would grow the window on every launch.
    let content = window.viewport_size();
    WindowState {
        x: origin.x.into(),
        y: origin.y.into(),
        width: content.width.into(),
        height: content.height.into(),
        maximized: !matches!(bounds, WindowBounds::Windowed(_)),
    }
}

/// A toast with the plain message, and Git's own words under Details
/// (FR-STATE-03).
fn show_error(error: GitError, window: &mut Window, cx: &mut gpui_kit::App) {
    let note = Notification::error(error_message(error.kind)).action(move |_, _, cx| {
        let error = error.clone();
        Button::new("details")
            .label(t!("error_details.button").into_owned())
            .on_click(cx.listener(move |note, _, window, cx| {
                show_details(error.clone(), window, cx);
                note.dismiss(window, cx);
            }))
    });
    window.push_notification(note, cx);
}

fn show_details(error: GitError, window: &mut Window, cx: &mut gpui_kit::App) {
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let output = if error.stderr.trim().is_empty() {
            t!("error_details.no_output").into_owned()
        } else {
            error.stderr.trim_end().to_owned()
        };
        let block = |label: String, text: String| {
            v_flex()
                .gap_1()
                .child(div().text_sm().text_color(muted).child(label))
                .child(div().text_sm().font_family(mono.clone()).child(text))
        };
        alert
            .width(px(560.))
            .title(t!("error_details.title").into_owned())
            .description(error_message(error.kind))
            .child(
                v_flex()
                    .gap_3()
                    .child(block(
                        t!("error_details.command").into_owned(),
                        format!("git {}", error.args.join(" ")),
                    ))
                    .child(block(t!("error_details.output").into_owned(), output)),
            )
            .ok_text(t!("error_details.close").into_owned())
    });
}

fn offer_init(app: Entity<AppState>, path: PathBuf, window: &mut Window, cx: &mut gpui_kit::App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (app, path) = (app.clone(), path.clone());
        alert
            .title(t!("not_a_repository.title").into_owned())
            .description(t!("not_a_repository.text", path = path.display()).into_owned())
            .confirm()
            .ok_text(t!("not_a_repository.create").into_owned())
            .on_ok(move |_, _, cx| {
                app.update(cx, |app, cx| app.init(path.clone(), cx));
                true
            })
    });
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.read(cx);
        let content = match (&app.git, &self.workspace) {
            (GitCheck::Checking, _) => div().into_any_element(),
            (GitCheck::Failed(_), _) => self.git_check.clone().into_any_element(),
            (GitCheck::Ready(_), Some(workspace)) => workspace.clone().into_any_element(),
            (GitCheck::Ready(_), None) => self.welcome.clone().into_any_element(),
        };
        let drop_target = cx.theme().drop_target;
        div()
            .id("root")
            .size_full()
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_target))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                // AppState reports a path that is not a repository folder.
                if let Some(path) = paths.paths().first().cloned() {
                    this.app.update(cx, |app, cx| app.open(path, cx));
                }
            }))
            .child(content)
    }
}
