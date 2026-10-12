use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use rust_i18n::t;

use crate::APP_NAME;
use crate::actions::{InitRepository, OpenRepository};
use crate::app_state::AppState;

/// The screen with no repository open: Open, Create and the recent list
/// (FR-REPO-01, FR-REPO-03, FR-REPO-04).
pub struct WelcomeView {
    app: Entity<AppState>,
    /// Recent repositories whose folder is gone.
    missing: HashSet<PathBuf>,
    _check: Task<()>,
}

impl WelcomeView {
    pub fn new(app: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&app, |this, _, cx| {
            this.check_folders(cx);
            cx.notify();
        })
        .detach();
        let mut this = Self {
            app,
            missing: HashSet::new(),
            _check: Task::ready(()),
        };
        this.check_folders(cx);
        this
    }

    /// Looks for the recent folders on disk, off the UI thread.
    fn check_folders(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self
            .app
            .read(cx)
            .recent
            .repos
            .iter()
            .map(|repo| repo.path.clone())
            .collect();
        let missing = cx.background_spawn(async move {
            paths
                .into_iter()
                .filter(|path| !path.is_dir())
                .collect::<HashSet<_>>()
        });
        self._check = cx.spawn(async move |this, cx| {
            let missing = missing.await;
            let _ = this.update(cx, |this, cx| {
                if this.missing != missing {
                    this.missing = missing;
                    cx.notify();
                }
            });
        });
    }
}

fn last_opened(time: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    match (now - time).max(0) / 86_400 {
        0 => t!("welcome.today").into_owned(),
        1 => t!("welcome.yesterday").into_owned(),
        days => t!("welcome.days_ago", count = days).into_owned(),
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, border) = (theme.muted_foreground, theme.list_hover, theme.border);
        let recent = self.app.read(cx).recent.repos.clone();

        let rows = recent.into_iter().enumerate().map(|(ix, repo)| {
            let missing = self.missing.contains(&repo.path);
            let path = repo.path.clone();
            let app = self.app.clone();
            let row = h_flex()
                .id(("recent", ix))
                .w_full()
                .px_3()
                .py_2()
                .gap_3()
                .rounded_md()
                .child(
                    v_flex()
                        .flex_1()
                        .overflow_hidden()
                        .child(div().truncate().child(repo.name.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(muted)
                                .child(repo.path.display().to_string()),
                        ),
                );
            if missing {
                row.opacity(0.5)
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("welcome.missing").into_owned()),
                    )
                    .child(
                        Button::new(("remove", ix))
                            .small()
                            .ghost()
                            .label(t!("welcome.remove").into_owned())
                            .on_click(move |_, _, cx| {
                                app.update(cx, |app, cx| app.remove_recent(&path, cx));
                            }),
                    )
            } else {
                row.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(last_opened(repo.last_opened)),
                    )
                    .on_click(move |_, _, cx| {
                        app.update(cx, |app, cx| app.open(path.clone(), cx));
                    })
            }
        });
        let has_recent = !self.app.read(cx).recent.repos.is_empty();

        v_flex()
            .size_full()
            .items_center()
            .pt_16()
            .gap_6()
            .child(div().text_3xl().child(APP_NAME))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("open")
                            .primary()
                            .label(t!("welcome.open").into_owned())
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenRepository), cx)
                            }),
                    )
                    .child(
                        Button::new("init")
                            .label(t!("welcome.init").into_owned())
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(InitRepository), cx)
                            }),
                    ),
            )
            .child(if has_recent {
                v_flex()
                    .id("recent-list")
                    .w(px(560.))
                    .flex_1()
                    .mb_6()
                    .gap_1()
                    .child(
                        div()
                            .pb_2()
                            .border_b_1()
                            .border_color(border)
                            .text_sm()
                            .text_color(muted)
                            .child(t!("welcome.recent").into_owned()),
                    )
                    .children(rows)
                    .overflow_y_scrollbar()
                    .into_any_element()
            } else {
                div()
                    .w(px(420.))
                    .text_center()
                    .text_color(muted)
                    .child(t!("welcome.no_recent").into_owned())
                    .into_any_element()
            })
    }
}
