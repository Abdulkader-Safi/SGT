use gittree_core::model::{Ref, RefKind};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
};

use crate::zoom::px;
use rust_i18n::t;

use gittree_core::ops::branch::Target;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};

use super::dialogs;
use super::ui::{caption, ic};
use crate::icons::icon;
use crate::palette::palette;
use crate::repo_state::Op;
use crate::repo_state::RepoState;

const GROUPS: [(RefKind, &str); 4] = [
    (RefKind::Local, "sidebar.local"),
    (RefKind::Remote, "sidebar.remotes"),
    (RefKind::Tag, "sidebar.tags"),
    (RefKind::Stash, "sidebar.stashes"),
];

/// Branches, remote branches, tags and stashes, with a name filter
/// (FR-BRANCH-01).
pub struct Sidebar {
    repo: Entity<RepoState>,
    filter: Entity<InputState>,
    collapsed: [bool; GROUPS.len()],
}

impl Sidebar {
    pub fn new(repo: Entity<RepoState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("sidebar.filter").into_owned()));
        cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            repo,
            filter,
            collapsed: [false; GROUPS.len()],
        }
    }

    fn row(
        &self,
        ix: usize,
        r: &Ref,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let repo = self.repo.clone();
        let target = r.target.clone();
        let name: SharedString = r.short_name().to_owned().into();
        let hidden = self.repo.read(cx).hidden.contains(&r.name);
        let icon = match r.kind {
            RefKind::Local => "branch",
            RefKind::Remote => "remote",
            RefKind::Tag => "tag",
            RefKind::Stash => "stash",
        };
        let track = [("↑", r.ahead), ("↓", r.behind)]
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(arrow, count)| format!("{arrow}{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        h_flex()
            .id(("ref", ix))
            .w_full()
            .h(px(28.))
            .pl(px(30.))
            .pr_3()
            .gap_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(p.bg_selected))
            .when(!selected, |row| row.hover(|style| style.bg(p.bg_hover)))
            .when(r.is_current, |row| row.font_weight(FontWeight::MEDIUM))
            .on_click(move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&target, cx)))
            .child(ic(
                icon,
                14.,
                if r.is_current {
                    p.accent
                } else {
                    p.text_tertiary
                },
            ))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(hidden, |name| name.text_color(p.text_tertiary).italic())
                    .child(name),
            )
            .when(!track.is_empty(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(p.text_secondary)
                        .child(track),
                )
            })
            .context_menu({
                let repo = self.repo.clone();
                let (kind, current, target) = (r.kind, r.is_current, r.target.clone());
                let behind = r.behind;
                let short = r.short_name().to_owned();
                let upstream = r.upstream.clone();
                let full_name = r.name.clone();
                move |menu, _, cx| {
                    let trusted = repo.read(cx).repository.trusted;
                    let head_name = match &repo.read(cx).repository.head {
                        gittree_core::model::Head::Branch(name)
                        | gittree_core::model::Head::Unborn(name) => name.clone(),
                        gittree_core::model::Head::Detached(_) => "HEAD".to_owned(),
                    };
                    let run = |op: Op| {
                        let repo = repo.clone();
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                            repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx))
                        }
                    };
                    let item = |icon_name: &str, key: &str| {
                        PopupMenuItem::new(t!(key).into_owned()).icon(crate::icons::icon(icon_name))
                    };
                    let branch_here = {
                        let (repo, at, label) = (repo.clone(), target.to_string(), short.clone());
                        item("branch", "menu_item.create_branch")
                            .disabled(!trusted)
                            .on_click(move |_, window, cx| {
                                dialogs::create_branch(
                                    repo.clone(),
                                    at.clone(),
                                    label.clone(),
                                    window,
                                    cx,
                                )
                            })
                    };
                    // HEAD always shows, so the current branch cannot hide.
                    let hide_item = {
                        let (repo, name) = (repo.clone(), full_name.clone());
                        let shown = !repo.read(cx).hidden.contains(&name);
                        PopupMenuItem::new(if shown {
                            t!("menu_item.hide").into_owned()
                        } else {
                            t!("menu_item.show").into_owned()
                        })
                        .disabled(current)
                        .on_click(move |_, _, cx| {
                            repo.update(cx, |repo, cx| repo.toggle_hidden(name.clone(), cx))
                        })
                    };
                    match kind {
                        RefKind::Local => {
                            let (rename_repo, rename_name) = (repo.clone(), short.clone());
                            let (delete_repo, delete_name) = (repo.clone(), short.clone());
                            menu.item(
                                item("check", "menu_item.checkout")
                                    .disabled(!trusted || current)
                                    .on_click(run(Op::Checkout(Target::Branch(short.clone())))),
                            )
                            .item(branch_here)
                            .item({
                                let (repo, name) = (repo.clone(), short.clone());
                                PopupMenuItem::new(t!("push.force_item").into_owned())
                                    .icon(crate::icons::icon("push"))
                                    .disabled(!trusted || !current || behind == 0)
                                    .on_click(move |_, window, cx| {
                                        dialogs::force_push(
                                            repo.clone(),
                                            name.clone(),
                                            behind,
                                            window,
                                            cx,
                                        )
                                    })
                            })
                            .separator()
                            .item({
                                let (repo, from, into) =
                                    (repo.clone(), short.clone(), head_name.clone());
                                PopupMenuItem::new(
                                    t!("menu_item.merge_into", branch = head_name).into_owned(),
                                )
                                .icon(crate::icons::icon("merge"))
                                .disabled(!trusted || current)
                                .on_click(move |_, window, cx| {
                                    dialogs::merge(
                                        repo.clone(),
                                        from.clone(),
                                        into.clone(),
                                        window,
                                        cx,
                                    )
                                })
                            })
                            .item({
                                let (repo, onto, mine) =
                                    (repo.clone(), short.clone(), head_name.clone());
                                PopupMenuItem::new(
                                    t!("menu_item.rebase_onto", branch = head_name).into_owned(),
                                )
                                .icon(crate::icons::icon("fetch"))
                                .disabled(!trusted || current)
                                .on_click(move |_, window, cx| {
                                    dialogs::rebase(
                                        repo.clone(),
                                        mine.clone(),
                                        onto.clone(),
                                        window,
                                        cx,
                                    )
                                })
                            })
                            .separator()
                            .item({
                                let (repo, name, upstream) =
                                    (repo.clone(), short.clone(), upstream.clone());
                                item("remote", "menu_item.set_upstream")
                                    .disabled(!trusted)
                                    .on_click(move |_, window, cx| {
                                        dialogs::set_upstream(
                                            repo.clone(),
                                            name.clone(),
                                            upstream.clone(),
                                            window,
                                            cx,
                                        )
                                    })
                            })
                            .item(
                                item("file", "menu_item.rename")
                                    .disabled(!trusted)
                                    .on_click(move |_, window, cx| {
                                        dialogs::rename_branch(
                                            rename_repo.clone(),
                                            rename_name.clone(),
                                            window,
                                            cx,
                                        )
                                    }),
                            )
                            .item(
                                item("trash", "menu_item.delete_branch")
                                    .disabled(!trusted || current)
                                    .on_click(move |_, _, cx| {
                                        let name = delete_name.clone();
                                        delete_repo
                                            .update(cx, |repo, cx| repo.delete_branch(name, cx))
                                    }),
                            )
                            .separator()
                            .item(hide_item)
                        }
                        RefKind::Remote => menu
                            .item(
                                item("check", "menu_item.checkout")
                                    .disabled(!trusted)
                                    .on_click(run(Op::Checkout(Target::Remote(short.clone())))),
                            )
                            .item(branch_here)
                            .separator()
                            .item(hide_item),
                        RefKind::Tag => {
                            let (tag_repo, tag_name) = (repo.clone(), short.clone());
                            menu.item(
                                item("check", "menu_item.checkout_commit")
                                    .disabled(!trusted)
                                    .on_click(run(Op::Checkout(Target::Commit(target.clone())))),
                            )
                            .item(branch_here)
                            .separator()
                            .item({
                                let tag = short.clone();
                                PopupMenuItem::new(t!("push.tag").into_owned())
                                    .icon(crate::icons::icon("push"))
                                    .disabled(!trusted)
                                    .on_click(run(Op::PushTag { tag, delete: false }))
                            })
                            .item({
                                let tag = short.clone();
                                PopupMenuItem::new(t!("push.delete_remote_tag").into_owned())
                                    .icon(crate::icons::icon("remote"))
                                    .disabled(!trusted)
                                    .on_click(run(Op::PushTag { tag, delete: true }))
                            })
                            .item(
                                item("trash", "menu_item.delete_tag")
                                    .disabled(!trusted)
                                    .on_click(move |_, window, cx| {
                                        dialogs::delete_tag(
                                            tag_repo.clone(),
                                            tag_name.clone(),
                                            window,
                                            cx,
                                        )
                                    }),
                            )
                        }
                        RefKind::Stash => menu,
                    }
                }
            })
    }

    /// One stash: click to see it, right-click to apply, pop or drop it.
    fn stash_row(
        &self,
        stash: &gittree_core::ops::branch::Stash,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let (repo, oid, index) = (self.repo.clone(), stash.oid.clone(), stash.index);
        let name: SharedString = stash.message.clone().into();
        h_flex()
            .id(("stash", index as usize))
            .w_full()
            .h(px(28.))
            .pl(px(30.))
            .pr_3()
            .gap_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(p.bg_selected))
            .when(!selected, |row| row.hover(|style| style.bg(p.bg_hover)))
            .on_click({
                let repo = repo.clone();
                move |_, _, cx| repo.update(cx, |repo, cx| repo.select_oid(&oid, cx))
            })
            .child(ic("stash", 14., p.text_tertiary))
            .child(div().flex_1().truncate().child(name.clone()))
            .context_menu(move |menu, _, cx| {
                let trusted = repo.read(cx).repository.trusted;
                let run = |op: Op| {
                    let repo = repo.clone();
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                        repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx))
                    }
                };
                let (drop_repo, drop_name) = (repo.clone(), name.clone());
                menu.item(
                    PopupMenuItem::new(t!("stash.apply").into_owned())
                        .icon(icon("check"))
                        .disabled(!trusted)
                        .on_click(run(Op::StashApply(index))),
                )
                .item(
                    PopupMenuItem::new(t!("stash.pop").into_owned())
                        .icon(icon("push"))
                        .disabled(!trusted)
                        .on_click(run(Op::StashPop(index))),
                )
                .item({
                    let repo = repo.clone();
                    PopupMenuItem::new(t!("stash.branch_item").into_owned())
                        .icon(icon("branch"))
                        .disabled(!trusted)
                        .on_click(move |_, window, cx| {
                            dialogs::stash_branch(repo.clone(), index, window, cx)
                        })
                })
                .separator()
                .item(
                    PopupMenuItem::new(t!("menu_item.drop_stash").into_owned())
                        .icon(icon("trash"))
                        .disabled(!trusted)
                        .on_click(move |_, window, cx| {
                            dialogs::drop_stash(
                                drop_repo.clone(),
                                index,
                                drop_name.clone(),
                                window,
                                cx,
                            )
                        }),
                )
            })
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let repo = self.repo.read(cx);
        let selected = repo.selection.oid.clone();

        let groups = GROUPS.iter().enumerate().map(|(group, (kind, title))| {
            let stash_rows = || -> Vec<gpui_kit::AnyElement> {
                repo.stashes
                    .iter()
                    .filter(|stash| {
                        needle.is_empty() || stash.message.to_lowercase().contains(&needle)
                    })
                    .map(|stash| {
                        let is_selected = selected.as_ref() == Some(&stash.oid);
                        self.stash_row(stash, is_selected, cx).into_any_element()
                    })
                    .collect()
            };
            let rows: Vec<_> = repo
                .refs
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == *kind && r.kind != RefKind::Stash)
                .filter(|(_, r)| {
                    needle.is_empty() || r.short_name().to_lowercase().contains(&needle)
                })
                .map(|(ix, r)| {
                    // The branch or tag that names the selected commit.
                    let is_selected = selected.as_ref() == Some(&r.target);
                    self.row(ix, r, is_selected, cx).into_any_element()
                })
                .collect();
            let rows = if *kind == RefKind::Stash {
                stash_rows()
            } else {
                rows
            };
            let collapsed = self.collapsed[group];
            let count = rows.len();
            let empty_stashes =
                (*kind == RefKind::Stash && count == 0 && needle.is_empty()).then(|| {
                    div()
                        .pl(px(30.))
                        .pr_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .child(t!("sidebar.no_stashes").into_owned())
                });
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .id(("group", group))
                        .h(px(26.))
                        .px_3()
                        .gap(px(6.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.collapsed[group] = !this.collapsed[group];
                            cx.notify();
                        }))
                        .child(ic(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                            p.text_tertiary,
                        ))
                        .child(div().flex_1().child(caption(t!(*title).to_uppercase(), p)))
                        .child(caption(count.to_string(), p))
                        .context_menu({
                            let repo = self.repo.clone();
                            let is_remotes = *kind == RefKind::Remote;
                            move |menu, _, cx| {
                                if !is_remotes {
                                    return menu;
                                }
                                let trusted = repo.read(cx).repository.trusted;
                                let remotes = repo.read(cx).remotes.clone();
                                let add = repo.clone();
                                let mut menu = menu.item(
                                    PopupMenuItem::new(t!("remote.add_item").into_owned())
                                        .icon(crate::icons::icon("plus"))
                                        .disabled(!trusted)
                                        .on_click(move |_, window, cx| {
                                            dialogs::remote(add.clone(), None, window, cx)
                                        }),
                                );
                                for remote in remotes {
                                    let (edit, remove) = (repo.clone(), repo.clone());
                                    let (name, url) = (remote.name.clone(), remote.fetch_url);
                                    let gone = name.clone();
                                    menu =
                                        menu.separator()
                                            .item(
                                                PopupMenuItem::new(
                                                    t!("remote.edit_item", name = name)
                                                        .into_owned(),
                                                )
                                                .disabled(!trusted)
                                                .on_click(move |_, window, cx| {
                                                    dialogs::remote(
                                                        edit.clone(),
                                                        Some((name.clone(), url.clone())),
                                                        window,
                                                        cx,
                                                    )
                                                }),
                                            )
                                            .item(
                                                PopupMenuItem::new(
                                                    t!("remote.remove_item", name = gone)
                                                        .into_owned(),
                                                )
                                                .disabled(!trusted)
                                                .on_click(move |_, window, cx| {
                                                    dialogs::remove_remote(
                                                        remove.clone(),
                                                        gone.clone(),
                                                        window,
                                                        cx,
                                                    )
                                                }),
                                            );
                                }
                                menu
                            }
                        }),
                )
                .when(!collapsed, |group| {
                    group.children(rows).children(empty_stashes)
                })
        });

        v_flex()
            .size_full()
            .bg(p.bg_panel)
            .border_r_1()
            .border_color(p.border)
            .child(div().p(px(10.)).child(
                Input::new(&self.filter).small().cleanable(true).prefix(ic(
                    "search",
                    14.,
                    p.text_tertiary,
                )),
            ))
            .child(
                v_flex()
                    .id("refs")
                    .flex_1()
                    .children(groups)
                    .overflow_y_scrollbar(),
            )
    }
}
