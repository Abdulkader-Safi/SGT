use std::path::PathBuf;

use gittree_core::model::Head;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme, ResizablePanelEvent, ResizableState, Sizable, h_flex, h_resizable,
    resizable_panel, v_flex,
};
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, div, px,
};
use rust_i18n::t;

use super::commit_detail::CommitDetail;
use super::graph::GraphView;
use super::sidebar::Sidebar;
use crate::actions::{FocusGraph, Refresh, SearchCommits, TrustRepository};
use crate::app_state::AppState;
use crate::repo_state::{RepoState, Scope};

/// The main window for an open repository: toolbar, banner, sidebar, graph
/// and detail panel.
pub struct Workspace {
    app: Entity<AppState>,
    repo: Entity<RepoState>,
    pub path: PathBuf,
    panels: Entity<ResizableState>,
    sidebar: Entity<Sidebar>,
    graph: Entity<GraphView>,
    detail: Entity<CommitDetail>,
    search: Entity<InputState>,
    focus: FocusHandle,
}

impl Workspace {
    pub fn new(
        app: Entity<AppState>,
        repo: Entity<RepoState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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

        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("search.placeholder").into_owned())
        });
        cx.subscribe(
            &search,
            |this, search, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let query = search.read(cx).value().to_string();
                    this.repo.update(cx, |repo, cx| repo.set_search(query, cx));
                }
                // Enter goes to the next match and Shift+Enter to the one before.
                InputEvent::PressEnter { shift, .. } => {
                    let forward = !*shift;
                    this.repo
                        .update(cx, |repo, cx| repo.step_search(forward, cx));
                }
                _ => {}
            },
        )
        .detach();
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();

        let graph = cx.new(|cx| GraphView::new(repo.clone(), cx));
        // Keyboard focus starts on the graph, so arrows and shortcuts work
        // straight away.
        graph.read(cx).focus.clone().focus(window, cx);
        Self {
            path: repo.read(cx).repository.path.clone(),
            sidebar: cx.new(|cx| Sidebar::new(repo.clone(), window, cx)),
            detail: cx.new(|cx| CommitDetail::new(repo.clone(), cx)),
            graph,
            panels,
            search,
            focus: cx.focus_handle(),
            app,
            repo,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, warning) = (theme.muted_foreground, theme.border, theme.warning);
        let session = &self.app.read(cx).session;
        let (sidebar_width, detail_size) = (session.sidebar_width, session.detail_size);
        let repo = self.repo.read(cx);
        let head = match &repo.repository.head {
            Head::Branch(name) | Head::Unborn(name) => name.clone(),
            Head::Detached(oid) => t!("workspace.detached", oid = &oid[..7]).into_owned(),
        };
        let search = &repo.search;
        let search_count = (!search.query.trim().is_empty() && !search.running).then(|| {
            if search.matches.is_empty() {
                t!("search.none").into_owned()
            } else {
                t!(
                    "search.count",
                    at = search.at + 1,
                    count = search.matches.len()
                )
                .into_owned()
            }
        });

        let toolbar = h_flex()
            .h(px(40.))
            .flex_none()
            .px_3()
            .gap_3()
            .border_b_1()
            .border_color(border)
            .child(repo.repository.name())
            .child(div().text_color(muted).child(head))
            .child(div().flex_1())
            .children(search_count.map(|count| div().text_sm().text_color(muted).child(count)))
            .child(
                Input::new(&self.search)
                    .id("search")
                    .small()
                    .cleanable(true)
                    .w(px(280.)),
            );

        let banner = (!repo.repository.trusted).then(|| {
            h_flex()
                .flex_none()
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

        let middle = if repo.has_commits() || !repo.history.commits.is_empty() {
            self.graph.clone().into_any_element()
        } else {
            v_flex()
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
                .into_any_element()
        };

        v_flex()
            .id("workspace")
            .size_full()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| {
                this.repo
                    .update(cx, |repo, cx| repo.refresh(Scope::All, cx));
            }))
            .on_action(cx.listener(|this, _: &SearchCommits, window, cx| {
                this.search
                    .update(cx, |search, cx| search.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &FocusGraph, window, cx| {
                this.graph.read(cx).focus.clone().focus(window, cx);
            }))
            .child(toolbar)
            .children(banner)
            .child(
                div().flex_1().overflow_hidden().child(
                    h_resizable("workspace-panels")
                        .with_state(&self.panels)
                        .child(
                            resizable_panel()
                                .size(px(sidebar_width))
                                .size_range(px(160.)..px(480.))
                                .child(self.sidebar.clone()),
                        )
                        .child(resizable_panel().child(middle))
                        .child(
                            resizable_panel()
                                .size(px(detail_size))
                                .size_range(px(240.)..px(900.))
                                .child(self.detail.clone()),
                        ),
                ),
            )
    }
}
