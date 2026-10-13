use std::path::PathBuf;

use gittree_core::model::Head;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    Disableable, ResizablePanelEvent, ResizableState, Sizable, h_flex, h_resizable,
    resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::commit_detail::CommitDetail;
use super::conflict::ConflictView;
use super::dialogs;
use super::graph::GraphView;
use super::sidebar::Sidebar;
use super::staging::StagingView;
use super::ui::{BannerKind, badge, banner, ic};
use crate::actions::{Commit, FocusChanges, FocusGraph, Refresh, SearchCommits, TrustRepository};
use crate::app_state::AppState;
use crate::palette::palette;
use crate::repo_state::{Op, RepoEvent};
use crate::repo_state::{RepoState, Scope};
use gpui_kit::component::input::{Redo, Undo};
use gpui_kit::component::tooltip::Tooltip;

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
    staging: Entity<StagingView>,
    conflict: Entity<ConflictView>,
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
        cx.subscribe_in(
            &repo,
            window,
            |_, repo, event: &RepoEvent, window, cx| match event {
                RepoEvent::ConfirmDeleteBranch { name, lost } => {
                    dialogs::delete_branch(repo.clone(), name.clone(), lost.clone(), window, cx)
                }
                RepoEvent::CheckoutBlocked(target) => {
                    dialogs::checkout_blocked(repo.clone(), target.clone(), window, cx)
                }
                _ => {}
            },
        )
        .detach();
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        // Coming back to the window: files may have changed meanwhile.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.repo.update(cx, |repo, cx| {
                    repo.refresh(Scope::Status, cx);
                    repo.refresh(Scope::Refs, cx);
                });
            }
        })
        .detach();

        let graph = cx.new(|cx| GraphView::new(repo.clone(), cx));
        // Keyboard focus starts on the graph, so arrows and shortcuts work
        // straight away.
        graph.read(cx).focus.clone().focus(window, cx);
        Self {
            path: repo.read(cx).repository.path.clone(),
            sidebar: cx.new(|cx| Sidebar::new(repo.clone(), window, cx)),
            detail: cx.new(|cx| CommitDetail::new(repo.clone(), cx)),
            staging: cx.new(|cx| StagingView::new(app.clone(), repo.clone(), window, cx)),
            conflict: cx.new(|cx| ConflictView::new(repo.clone(), window, cx)),
            graph,
            panels,
            search,
            focus: cx.focus_handle(),
            app,
            repo,
        }
    }
}

impl Workspace {
    /// Opens "Create branch", starting at HEAD.
    fn new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.read(cx);
        if !repo.repository.trusted || !repo.has_commits() {
            return;
        }
        let label = match &repo.repository.head {
            Head::Branch(name) | Head::Unborn(name) => name.clone(),
            Head::Detached(oid) => oid[..7.min(oid.len())].to_owned(),
        };
        dialogs::create_branch(self.repo.clone(), "HEAD".into(), label, window, cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = &self.app.read(cx).session;
        let (sidebar_width, detail_size) = (session.sidebar_width, session.detail_size);
        let repo = self.repo.read(cx);
        let trusted = repo.repository.trusted;
        let detached = matches!(repo.repository.head, Head::Detached(_));
        let (has_commits, has_changes) = (repo.has_commits(), !repo.status.is_empty());
        let busy = repo.busy;
        let undo_info = repo.undo_info.clone();
        let head = match &repo.repository.head {
            Head::Branch(name) | Head::Unborn(name) => name.clone(),
            Head::Detached(oid) => t!("workspace.detached", oid = &oid[..7]).into_owned(),
        };
        let current = repo.refs.iter().find(|r| r.is_current);
        let count = |n: Option<u32>| n.filter(|n| *n > 0).map(|n| n.to_string());
        let (ahead, behind) = (
            count(current.map(|r| r.ahead)),
            count(current.map(|r| r.behind)),
        );
        let search = &repo.search;
        let searching = !search.query.trim().is_empty();
        let search_count = (searching && !search.running).then(|| {
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

        // A toolbar button. One without a handler belongs to a later
        // milestone and shows greyed out.
        let tool = |id: &'static str, icon: &str, label: Option<String>, count: Option<String>| {
            h_flex()
                .id(id)
                .flex_none()
                .h(px(28.))
                .px_2()
                .gap(px(6.))
                .rounded(px(6.))
                .child(ic(icon, 16., p.text_secondary))
                .children(label.map(|label| {
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label)
                }))
                .children(count.map(|count| badge(count, p)))
        };
        let off = |button: gpui_kit::Stateful<gpui_kit::Div>| button.opacity(0.4);
        let divider = || div().flex_none().w(px(1.)).h(px(20.)).bg(p.border);
        // Fetch, Pull and Push need a remote, and wait for one another.
        let can_sync = trusted && !repo.remotes.is_empty() && !repo.net_running;
        let pull_mode = self.app.read(cx).settings.pull_mode;
        let repo_handle = self.repo.clone();
        let net = |button: gpui_kit::Stateful<gpui_kit::Div>, enabled: bool, op: Op| {
            if !enabled {
                return off(button);
            }
            let repo = repo_handle.clone();
            button
                .cursor_pointer()
                .hover(|style| style.bg(p.bg_hover))
                .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx)))
        };
        let running = repo.net_running.then(|| {
            let (phase, percent) = match &repo.progress {
                Some(progress) => (progress.phase.clone(), progress.percent),
                None => (t!("toolbar.working").into_owned(), None),
            };
            h_flex()
                .flex_none()
                .gap_2()
                .text_size(px(12.))
                .text_color(p.text_secondary)
                .child(match percent {
                    Some(percent) => format!("{phase} {percent}%"),
                    None => phase,
                })
                .child(
                    div()
                        .w(px(80.))
                        .h(px(4.))
                        .rounded(px(2.))
                        .bg(p.bg_hover)
                        .child(
                            div()
                                .h_full()
                                .rounded(px(2.))
                                .bg(p.accent)
                                .w(gpui_kit::relative(percent.unwrap_or(4) as f32 / 100.)),
                        ),
                )
                .child(
                    Button::new("cancel-network")
                        .xsmall()
                        .ghost()
                        .label(t!("sync.cancel").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.repo.update(cx, |repo, _| repo.cancel_network())
                        })),
                )
        });
        let toolbar = h_flex()
            .h(px(48.))
            .flex_none()
            // Room for the traffic lights.
            .pl(px(if crate::platform::MAC { 84. } else { 12. }))
            .pr_3()
            .gap(px(6.))
            .bg(p.bg_panel)
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .flex_none()
                    .h(px(28.))
                    .px_2()
                    .gap(px(6.))
                    .child(ic("folder", 16., p.text_secondary))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(repo.repository.name()),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .h(px(28.))
                    .px_2()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .bg(p.bg_elevated)
                    .border_1()
                    .border_color(p.border)
                    .child(ic(
                        "branch",
                        14.,
                        if detached { p.warning } else { p.accent },
                    ))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(head),
                    ),
            )
            .child(divider())
            .child({
                // The tooltip names what Undo would take back, or says why
                // it cannot (FR-UNDO-01).
                let tip = match (&undo_info.undo, undo_info.undo_blocked) {
                    (Some(_), true) => t!("undo.tip_blocked").into_owned(),
                    (Some(label), false) => t!("undo.tip_undo", label = label).into_owned(),
                    (None, _) => t!("undo.tip_none").into_owned(),
                };
                let can = trusted && undo_info.undo.is_some() && !undo_info.undo_blocked && !busy;
                let undo = tool("undo", "undo", None, None)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
                if can {
                    undo.cursor_pointer()
                        .hover(|style| style.bg(p.bg_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.repo.update(cx, |repo, cx| repo.run_op(Op::Undo, cx));
                        }))
                } else {
                    off(undo)
                }
            })
            .child({
                let tip = match &undo_info.redo {
                    Some(label) => t!("undo.tip_redo", label = label).into_owned(),
                    None => t!("undo.tip_none_redo").into_owned(),
                };
                let redo = tool("redo", "redo", None, None)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
                if trusted && undo_info.redo.is_some() && !busy {
                    redo.cursor_pointer()
                        .hover(|style| style.bg(p.bg_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.repo.update(cx, |repo, cx| repo.run_op(Op::Redo, cx));
                        }))
                } else {
                    off(redo)
                }
            })
            .child(divider())
            .child(net(
                tool(
                    "fetch",
                    "fetch",
                    Some(t!("toolbar.fetch").into_owned()),
                    None,
                ),
                can_sync,
                Op::Fetch,
            ))
            .child(net(
                tool(
                    "pull",
                    "pull",
                    Some(t!("toolbar.pull").into_owned()),
                    behind,
                ),
                can_sync,
                Op::Pull(pull_mode),
            ))
            .child(net(
                tool("push", "push", Some(t!("toolbar.push").into_owned()), ahead),
                can_sync && has_commits,
                Op::Push { force: false },
            ))
            .child(divider())
            .child({
                let button = tool(
                    "new-branch",
                    "branch",
                    Some(t!("toolbar.branch").into_owned()),
                    None,
                );
                if trusted && has_commits {
                    button
                        .cursor_pointer()
                        .hover(|style| style.bg(p.bg_hover))
                        .on_click(cx.listener(|this, _, window, cx| this.new_branch(window, cx)))
                } else {
                    off(button)
                }
            })
            .child({
                let button = tool(
                    "stash",
                    "stash",
                    Some(t!("toolbar.stash").into_owned()),
                    None,
                );
                if trusted && has_changes {
                    button
                        .cursor_pointer()
                        .hover(|style| style.bg(p.bg_hover))
                        .on_click(cx.listener(|this, _, window, cx| {
                            dialogs::stash(this.repo.clone(), window, cx)
                        }))
                } else {
                    off(button)
                }
            })
            .child(div().flex_1())
            .children(running)
            .when(repo.busy && !repo.net_running, |toolbar| {
                toolbar.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_secondary)
                        .child(t!("toolbar.working").into_owned()),
                )
            })
            .children(search_count.map(|count| {
                div()
                    .text_size(px(12.))
                    .text_color(p.text_secondary)
                    .child(count)
            }))
            .child(
                Input::new(&self.search)
                    .id("search")
                    .small()
                    .cleanable(true)
                    .prefix(ic("search", 14., p.text_tertiary))
                    .w(px(280.)),
            );

        let untrusted = (!trusted).then(|| {
            banner(
                BannerKind::Warning,
                "shield",
                t!("workspace.untrusted").into_owned(),
                p,
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
        // "Create branch here" joins this banner with the branch actions.
        let detached_banner = detached.then(|| {
            banner(
                BannerKind::Info,
                "info",
                t!("workspace.detached_banner").into_owned(),
                p,
            )
            .child(
                Button::new("branch-here")
                    .small()
                    .disabled(!trusted)
                    .label(t!("workspace.branch_here").into_owned())
                    .on_click(cx.listener(|this, _, window, cx| this.new_branch(window, cx))),
            )
        });

        let progress = repo.in_progress.clone().map(|op| {
            use gittree_core::ops::merge::OpKind;
            let mut text = match op.kind {
                OpKind::Merge => t!("banner.merge"),
                OpKind::Rebase => t!("banner.rebase"),
                OpKind::CherryPick => t!("banner.cherry_pick"),
                OpKind::Revert => t!("banner.revert"),
            }
            .into_owned();
            if let Some(name) = &op.onto {
                text.push_str(&t!("banner.onto", name = name));
            }
            if let Some((at, total)) = op.step {
                text.push_str(&t!("banner.step", at = at, total = total));
            }
            text.push_str(". ");
            text.push_str(&if op.conflicted > 0 {
                t!("banner.conflicts", count = op.conflicted)
            } else {
                t!("banner.no_conflicts")
            });
            let act = |id: &'static str, key: &str, op: Op| {
                Button::new(id)
                    .small()
                    .disabled(busy || !trusted)
                    .label(t!(key).into_owned())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx))
                    }))
            };
            banner(BannerKind::Info, "merge", text, p)
                .child(act("abort-op", "banner.abort", Op::AbortOp).ghost())
                .when(op.kind == OpKind::Rebase, |banner| {
                    banner.child(act("skip-op", "banner.skip", Op::SkipOp).ghost())
                })
                .child(
                    act("continue-banner", "banner.continue", Op::ContinueOp)
                        .disabled(busy || !trusted || op.conflicted > 0),
                )
        });

        let right = if repo.resolving && repo.selection.wip {
            self.conflict.clone().into_any_element()
        } else if repo.selection.wip {
            self.staging.clone().into_any_element()
        } else {
            self.detail.clone().into_any_element()
        };

        v_flex()
            .id("workspace")
            .size_full()
            .bg(p.bg_app)
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| {
                this.repo
                    .update(cx, |repo, cx| repo.refresh(Scope::All, cx));
            }))
            .on_action(cx.listener(|this, _: &SearchCommits, window, cx| {
                this.search
                    .update(cx, |search, cx| search.focus(window, cx));
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::NewBranch, window, cx| {
                    this.new_branch(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::actions::Stash, window, cx| {
                dialogs::stash(this.repo.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::actions::Fetch, _, cx| {
                this.repo.update(cx, |repo, cx| repo.run_op(Op::Fetch, cx))
            }))
            .on_action(cx.listener(|this, _: &crate::actions::Pull, _, cx| {
                let mode = this.app.read(cx).settings.pull_mode;
                this.repo
                    .update(cx, |repo, cx| repo.run_op(Op::Pull(mode), cx))
            }))
            .on_action(cx.listener(|this, _: &crate::actions::Push, _, cx| {
                this.repo
                    .update(cx, |repo, cx| repo.run_op(Op::Push { force: false }, cx))
            }))
            .on_action(cx.listener(|this, _: &FocusGraph, window, cx| {
                this.graph.read(cx).focus.clone().focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusChanges, window, cx| {
                this.repo.update(cx, |repo, cx| repo.select_wip(cx));
                this.staging.read(cx).focus.clone().focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Commit, window, cx| {
                this.staging
                    .update(cx, |staging, cx| staging.commit(window, cx));
            }))
            // A focused text field takes Cmd+Z for its own text first. It
            // gets here only from anywhere else.
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                this.repo.update(cx, |repo, cx| repo.run_op(Op::Undo, cx));
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                this.repo.update(cx, |repo, cx| repo.run_op(Op::Redo, cx));
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::ShowSnapshots, window, cx| {
                    dialogs::snapshots(this.repo.clone(), window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::CommandPalette, window, cx| {
                    super::palette::open(
                        this.repo.clone(),
                        this.graph.read(cx).focus.clone(),
                        window,
                        cx,
                    )
                }),
            )
            .child(toolbar)
            .children(untrusted)
            .children(progress)
            .children(detached_banner)
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
                        .child(resizable_panel().child(self.graph.clone()))
                        .child(
                            resizable_panel()
                                .size(px(detail_size))
                                .size_range(px(240.)..px(900.))
                                .child(right),
                        ),
                ),
            )
    }
}
