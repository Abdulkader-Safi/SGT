use std::path::PathBuf;

use gittree_core::GitRunner;
use gittree_core::model::Head;
use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme, ResizablePanelEvent, ResizableState, Sizable, h_flex, h_resizable,
    resizable_panel, v_flex,
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use rust_i18n::t;

use super::graph::{GraphView, load_history};
use crate::actions::TrustRepository;
use crate::app_state::AppState;

/// The main window for an open repository. In M1 it is the frame only: the
/// toolbar shows the name and branch, the sidebar and detail panels are
/// empty, and the middle shows the M0 graph.
pub struct Workspace {
    app: Entity<AppState>,
    pub path: PathBuf,
    panels: Entity<ResizableState>,
    graph: Option<Entity<GraphView>>,
}

impl Workspace {
    pub fn new(
        app: Entity<AppState>,
        path: PathBuf,
        runner: GitRunner,
        has_commits: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let graph = has_commits.then(|| cx.new(|cx| GraphView::new(load_history(runner, cx), cx)));

        let panels = cx.new(|_| ResizableState::default());
        cx.subscribe(&panels, |this, panels, _: &ResizablePanelEvent, cx| {
            let sizes: Vec<f32> = panels
                .read(cx)
                .sizes()
                .iter()
                .map(|s| f32::from(*s))
                .collect();
            if let [sidebar, _, detail] = sizes[..] {
                this.app.update(cx, |app, _| {
                    app.update_session(|session| {
                        session.sidebar_width = sidebar;
                        session.detail_size = detail;
                    })
                });
            }
        })
        .detach();
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        Self {
            app,
            path,
            panels,
            graph,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, sidebar_bg, warning) = (
            theme.muted_foreground,
            theme.border,
            theme.sidebar,
            theme.warning,
        );
        let app = self.app.read(cx);
        let (sidebar_width, detail_size) = (app.session.sidebar_width, app.session.detail_size);
        let Some(open) = &app.repo else {
            return div().into_any_element();
        };
        let head = match &open.repository.head {
            Head::Branch(name) | Head::Unborn(name) => name.clone(),
            Head::Detached(oid) => t!("workspace.detached", oid = &oid[..7]).into_owned(),
        };

        let toolbar = h_flex()
            .h(px(40.))
            .px_3()
            .gap_3()
            .border_b_1()
            .border_color(border)
            .child(open.repository.name())
            .child(div().text_color(muted).child(head));

        let banner = (!open.repository.trusted).then(|| {
            h_flex()
                .px_3()
                .py_2()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .bg(warning.opacity(0.15))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .child(t!("workspace.untrusted").into_owned()),
                )
                .child(
                    Button::new("trust")
                        .small()
                        .label(t!("workspace.trust").into_owned())
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(TrustRepository), cx)
                        }),
                )
        });

        let middle = match &self.graph {
            Some(graph) => graph.clone().into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .child(t!("workspace.no_commits_title").into_owned()),
                )
                .child(
                    div()
                        .text_color(muted)
                        .child(t!("workspace.no_commits_text").into_owned()),
                )
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .child(toolbar)
            .children(banner)
            .child(
                div().flex_1().overflow_hidden().child(
                    h_resizable("workspace")
                        .with_state(&self.panels)
                        .child(
                            resizable_panel()
                                .size(px(sidebar_width))
                                .size_range(px(160.)..px(480.))
                                .child(div().size_full().bg(sidebar_bg)),
                        )
                        .child(resizable_panel().child(middle))
                        .child(
                            resizable_panel()
                                .size(px(detail_size))
                                .size_range(px(200.)..px(800.))
                                .child(div().size_full()),
                        ),
                ),
            )
            .into_any_element()
    }
}
