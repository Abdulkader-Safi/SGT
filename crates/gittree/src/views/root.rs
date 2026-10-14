use std::path::PathBuf;

use gittree_core::GitError;
use gittree_core::store::WindowState;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, WindowBounds, div,
};

use crate::zoom::px;
use rust_i18n::t;

use super::git_check::GitCheckView;
use super::welcome::WelcomeView;
use super::workspace::Workspace;
use crate::app_state::{AppEvent, AppState, GitCheck};
use crate::error_message;
use crate::repo_state::RepoState;

/// The content of the main window. Shows the Git check, the welcome screen
/// or the workspace, and turns app events into toasts and dialogs.
pub struct RootView {
    app: Entity<AppState>,
    git_check: Entity<GitCheckView>,
    welcome: Entity<WelcomeView>,
    /// The active tab's workspace.
    workspace: Option<Entity<Workspace>>,
    /// A workspace per open tab, so each keeps its own state.
    workspaces: Vec<(Entity<RepoState>, Entity<Workspace>)>,
    cloning: bool,
}

impl RootView {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_in(&app, window, |this, app, window, cx| {
            // The clone dialog stays open for progress, and goes when the
            // clone is through.
            let cloning = app.read(cx).clone.is_some();
            if std::mem::replace(&mut this.cloning, cloning) && !cloning {
                window.close_dialog(cx);
            }
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
                AppEvent::Credential(request) => {
                    super::dialogs::credential(request.clone(), window, cx)
                }
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
            workspaces: Vec::new(),
            cloning: false,
            app,
        };
        this.sync_workspace(window, cx);
        this
    }

    /// Keeps a workspace for each open tab and shows the active one.
    fn sync_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (repos, active) = {
            let app = self.app.read(cx);
            (app.repos.clone(), app.repo.clone())
        };
        self.workspaces.retain(|(repo, _)| repos.contains(repo));
        if let Some(repo) = &active
            && !self.workspaces.iter().any(|(open, _)| open == repo)
        {
            let app = self.app.clone();
            let workspace = cx.new(|cx| Workspace::new(app, repo.clone(), window, cx));
            self.workspaces.push((repo.clone(), workspace));
        }
        self.workspace = active.and_then(|repo| {
            let found = self.workspaces.iter().find(|(open, _)| *open == repo);
            found.map(|(_, workspace)| workspace.clone())
        });
    }

    /// A tab per open repository, shown when there are two or more.
    fn tabs(&self, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        let app = self.app.read(cx);
        if app.repos.len() < 2 {
            return None;
        }
        let p = crate::palette::palette(cx);
        let tabs = app.repos.iter().enumerate().map(|(ix, repo)| {
            let on = app.repo.as_ref() == Some(repo);
            let name = repo.read(cx).repository.name();
            let (pick, close) = (self.app.clone(), self.app.clone());
            h_flex()
                .id(("tab", ix))
                .h_full()
                .px_3()
                .gap_2()
                .max_w(px(220.))
                .border_r_1()
                .border_color(p.border)
                .text_size(px(12.))
                .text_color(if on { p.text } else { p.text_secondary })
                .when(on, |tab| {
                    tab.bg(p.bg_app).font_weight(gpui_kit::FontWeight::MEDIUM)
                })
                .when(!on, |tab| {
                    tab.cursor_pointer().hover(|style| style.bg(p.bg_hover))
                })
                .on_click(move |_, _, cx| pick.update(cx, |app, cx| app.activate(ix, cx)))
                .child(div().truncate().child(name))
                .child(
                    Button::new(("close-tab", ix))
                        .xsmall()
                        .ghost()
                        .label("×")
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            close.update(cx, |app, cx| app.close_tab(ix, cx))
                        }),
                )
        });
        Some(
            h_flex()
                .flex_none()
                .h(px(32.))
                // Room for the traffic lights, as the workspace toolbar has.
                .when(crate::platform::MAC, |bar| bar.pl(px(80.)))
                .bg(p.bg_panel)
                .border_b_1()
                .border_color(p.border)
                .children(tabs),
        )
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
            .width(crate::zoom::scaled(560., cx))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The zoom: every size the views give through `zoom::px` follows.
        window.set_rem_size(gpui_kit::px(crate::zoom::BASE * crate::zoom::level(cx)));
        let app = self.app.read(cx);
        let content = match (&app.git, &self.workspace) {
            (GitCheck::Checking, _) => div().into_any_element(),
            (GitCheck::Failed(_), _) => self.git_check.clone().into_any_element(),
            (GitCheck::Ready(_), Some(workspace)) => workspace.clone().into_any_element(),
            (GitCheck::Ready(_), None) => self.welcome.clone().into_any_element(),
        };
        let p = crate::palette::palette(cx);
        // The workspace has its toolbar under the traffic lights. The other
        // screens get a plain strip with the app's name.
        let strip = (crate::platform::MAC && self.workspace.is_none()).then(|| {
            div()
                .h(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .bg(p.bg_panel)
                .border_b_1()
                .border_color(p.border)
                .text_size(px(12.))
                .text_color(p.text_secondary)
                .child(crate::APP_NAME)
        });
        let tabs = self.tabs(cx);
        let content = v_flex()
            .size_full()
            .children(strip)
            .children(tabs)
            .child(div().flex_1().overflow_hidden().child(content));
        let drop_target = cx.theme().drop_target;
        div()
            .id("root")
            .size_full()
            .bg(p.bg_app)
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_target))
            .on_action(
                cx.listener(|this, _: &crate::actions::CloneRepository, window, cx| {
                    super::dialogs::clone_repository(this.app.clone(), window, cx)
                }),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                // AppState reports a path that is not a repository folder.
                if let Some(path) = paths.paths().first().cloned() {
                    this.app.update(cx, |app, cx| app.open(path, cx));
                }
            }))
            .child(content)
    }
}
