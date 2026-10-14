use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Task, Window, div,
};

use crate::zoom::px;
use rust_i18n::t;

use super::graph::mono;
use super::ui::{empty_state, ic};
use crate::APP_NAME;
use crate::actions::{CloneRepository, InitRepository, OpenRepository, OpenSettings};
use crate::app_state::AppState;
use crate::app_state::GitCheck;
use crate::palette::palette;

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
        let p = palette(cx);
        let mono = mono(cx);
        let app = self.app.read(cx);
        let recent = app.recent.repos.clone();
        let git = match &app.git {
            GitCheck::Ready(version) => version.text.replace("git version", "Git"),
            _ => String::new(),
        };

        // One of the three big actions on the left.
        let action = |id: &'static str,
                      icon: &str,
                      label: String,
                      hint: String,
                      key: &str,
                      primary: bool| {
            let (foreground, soft) = if primary {
                (p.on_accent, p.on_accent)
            } else {
                (p.text, p.text_tertiary)
            };
            h_flex()
                .id(id)
                .h(px(52.))
                .px(px(14.))
                .gap_3()
                .rounded(px(10.))
                .when(primary, |row| {
                    row.bg(p.accent_fill)
                        .hover(|style| style.bg(p.accent_fill_hover))
                })
                .when(!primary, |row| {
                    row.bg(p.bg_elevated)
                        .border_1()
                        .border_color(p.border)
                        .hover(|style| style.bg(p.bg_hover))
                })
                .child(ic(
                    icon,
                    18.,
                    if primary {
                        p.on_accent
                    } else {
                        p.text_secondary
                    },
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(foreground)
                                .child(label),
                        )
                        .child(div().text_size(px(12.)).text_color(soft).child(hint)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(soft)
                        .child(key.to_owned()),
                )
        };
        let left = v_flex()
            .w(px(400.))
            .h_full()
            .flex_none()
            .p(px(40.))
            .gap(px(28.))
            .bg(p.bg_panel)
            .border_r_1()
            .border_color(p.border)
            .child(
                v_flex()
                    .gap(px(14.))
                    .child(
                        h_flex()
                            .size(px(40.))
                            .justify_center()
                            .rounded(px(10.))
                            .bg(p.accent_subtle)
                            .child(ic("branch", 22., p.accent)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(APP_NAME),
                            )
                            .child(
                                div()
                                    .text_color(p.text_secondary)
                                    .child(t!("welcome.tagline").into_owned()),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(10.))
                    .child(
                        action(
                            "open",
                            "folder",
                            t!("welcome.open").into_owned(),
                            t!("welcome.open_hint").into_owned(),
                            "⌘O",
                            true,
                        )
                        .cursor_pointer()
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(OpenRepository), cx)
                        }),
                    )
                    .child(
                        action(
                            "clone",
                            "clone",
                            t!("welcome.clone").into_owned(),
                            t!("welcome.clone_hint").into_owned(),
                            "⇧⌘O",
                            false,
                        )
                        .cursor_pointer()
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(CloneRepository), cx)
                        }),
                    )
                    .child(
                        action(
                            "init",
                            "plus",
                            t!("welcome.init").into_owned(),
                            t!("welcome.init_hint").into_owned(),
                            "",
                            false,
                        )
                        .cursor_pointer()
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(InitRepository), cx)
                        }),
                    ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(12.))
                    .child(
                        h_flex()
                            .id("welcome-settings")
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_color(p.text_secondary)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenSettings), cx)
                            })
                            .child(ic("settings", 14., p.text_tertiary))
                            .child(t!("welcome.settings").into_owned()),
                    )
                    .child(div().text_color(p.text_tertiary).child("·"))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(11.))
                            .text_color(p.text_tertiary)
                            .child(git),
                    ),
            );

        let rows = recent.iter().enumerate().map(|(ix, repo)| {
            let missing = self.missing.contains(&repo.path);
            let path = repo.path.clone();
            let app = self.app.clone();
            let row = h_flex()
                .id(("recent", ix))
                .group("recent-row")
                .w_full()
                .h(px(52.))
                .px(px(14.))
                .gap_3()
                .rounded(px(8.))
                .child(
                    h_flex()
                        .flex_none()
                        .size(px(32.))
                        .justify_center()
                        .rounded(px(8.))
                        .bg(p.bg_elevated)
                        .border_1()
                        .border_color(p.border)
                        .child(if missing {
                            ic("warning", 16., p.warning)
                        } else {
                            ic("folder", 16., p.text_secondary)
                        }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .overflow_hidden()
                        .when(missing, |text| text.opacity(0.55))
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(repo.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(mono.clone())
                                .text_size(px(11.))
                                .text_color(p.text_tertiary)
                                .child(repo.path.display().to_string()),
                        ),
                );
            let remove = {
                let (app, path) = (app.clone(), path.clone());
                Button::new(("remove", ix))
                    .small()
                    .ghost()
                    .label(t!("welcome.remove").into_owned())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        app.update(cx, |app, cx| app.remove_recent(&path, cx));
                    })
            };
            if missing {
                row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.warning)
                        .child(t!("welcome.missing").into_owned()),
                )
                .child(remove)
            } else {
                row.cursor_pointer()
                    .hover(|style| style.bg(p.bg_hover))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_secondary)
                            .child(last_opened(repo.last_opened)),
                    )
                    .child(
                        div()
                            .invisible()
                            .group_hover("recent-row", |style| style.visible())
                            .child(remove),
                    )
                    .on_click(move |_, _, cx| {
                        app.update(cx, |app, cx| app.open(path.clone(), cx));
                    })
            }
        });

        let right = if recent.is_empty() {
            empty_state(
                "folder",
                t!("welcome.no_recent_title").into_owned(),
                t!("welcome.no_recent").into_owned(),
                p,
            )
            .into_any_element()
        } else {
            v_flex()
                .id("recent-list")
                .size_full()
                .p(px(40.))
                .gap_4()
                .child(
                    h_flex()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("welcome.recent").into_owned()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.text_tertiary)
                                .child(t!("welcome.newest_first").into_owned()),
                        ),
                )
                .child(v_flex().gap(px(2.)).children(rows))
                .overflow_y_scrollbar()
                .into_any_element()
        };
        h_flex()
            .size_full()
            .items_start()
            .bg(p.bg_app)
            .text_size(px(13.))
            .child(left)
            .child(div().flex_1().h_full().overflow_hidden().child(right))
    }
}
