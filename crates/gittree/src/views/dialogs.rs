use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::{App, ParentElement, Window};
use rust_i18n::t;

/// Asks before an action that loses work. Enter and Escape both cancel:
/// only a click on the action button goes ahead (FR-SAFE-01).
pub fn confirm_destructive(
    title: String,
    text: String,
    action: String,
    on_confirm: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_confirm = Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let on_confirm = on_confirm.clone();
        alert
            .title(title.clone())
            .description(text.clone())
            // Enter is the dialog's OK. Here it only closes the dialog.
            .on_ok(|_, _, _| true)
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .primary()
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm")
                            .danger()
                            .label(action.clone())
                            .on_click(move |_, window, cx| {
                                on_confirm(cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

// ---------------------------------------------------------------------------
// Form dialogs: branches, tags and stash.

use std::cell::Cell;

use gittree_core::model::Oid;
use gittree_core::ops::branch::Target;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Entity, Focusable, FontWeight, IntoElement, SharedString, Styled, div,
    px,
};

use super::ui::ic;
use crate::palette::{Palette, palette};
use crate::repo_state::{Op, RepoState, target_name};

/// A labelled control with an optional line of help.
fn field(label: String, control: impl IntoElement, help: Option<String>, p: Palette) -> AnyElement {
    v_flex()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.text_secondary)
                .child(label),
        )
        .child(control)
        .children(help.map(|help| {
            div()
                .text_size(px(12.))
                .text_color(p.text_tertiary)
                .child(help)
        }))
        .into_any_element()
}

/// The commit a new branch or tag starts at.
fn commit_chip(text: String, p: Palette) -> AnyElement {
    h_flex()
        .h(px(28.))
        .px(px(10.))
        .gap_2()
        .rounded(px(6.))
        .bg(p.bg_panel)
        .border_1()
        .border_color(p.border)
        .child(ic("commit", 14., p.text_tertiary))
        .child(div().flex_1().truncate().child(text))
        .into_any_element()
}

/// A dialog with a body and two buttons. Enter and the primary button run
/// `on_ok`, which returns false to keep the dialog open.
#[allow(clippy::too_many_arguments)]
fn form(
    title: String,
    description: Option<String>,
    ok: String,
    focus: Option<Entity<InputState>>,
    body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    on_ok: impl Fn(&mut Window, &mut App) -> bool + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_ok = Rc::new(on_ok);
    let first = Cell::new(true);
    window.open_alert_dialog(cx, move |alert, window, cx| {
        if first.replace(false)
            && let Some(input) = &focus
        {
            input.focus_handle(cx).focus(window, cx);
        }
        let (enter, click) = (on_ok.clone(), on_ok.clone());
        let mut alert = alert.width(px(460.)).title(title.clone());
        if let Some(description) = &description {
            alert = alert.description(description.clone());
        }
        alert
            .child(v_flex().gap(px(14.)).child(body(window, cx)))
            .on_ok(move |_, window, cx| enter(window, cx))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("ok").primary().label(ok.clone()).on_click(
                        move |_, window, cx| {
                            if click(window, cx) {
                                window.close_dialog(cx);
                            }
                        },
                    )),
            )
    });
}

fn text_input(
    value: &str,
    placeholder: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = value.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

/// A checkbox whose state lives outside any view.
fn toggle(id: &'static str, label: String, state: &Entity<bool>, cx: &App) -> Checkbox {
    let handle = state.clone();
    Checkbox::new(id)
        .label(label)
        .checked(*state.read(cx))
        .on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            handle.update(cx, |state, _| *state = checked);
            window.refresh();
        })
}

pub fn create_branch(
    repo: Entity<RepoState>,
    at: String,
    at_label: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name = text_input("", t!("branch.name_hint").into_owned(), window, cx);
    let checkout = cx.new(|_| true);
    let (body_name, body_checkout) = (name.clone(), checkout.clone());
    form(
        t!("branch.create_title").into_owned(),
        None,
        t!("branch.create").into_owned(),
        Some(name.clone()),
        move |_, cx| {
            let p = palette(cx);
            v_flex()
                .gap(px(14.))
                .child(field(
                    t!("branch.name").into_owned(),
                    Input::new(&body_name).id("branch-name").small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("branch.start_at").into_owned(),
                    commit_chip(at_label.clone(), p),
                    None,
                    p,
                ))
                .child(toggle(
                    "branch-checkout",
                    t!("branch.checkout_new").into_owned(),
                    &body_checkout,
                    cx,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let typed = name.read(cx).value().trim().to_owned();
            if typed.is_empty() {
                return false;
            }
            let op = Op::CreateBranch {
                name: typed,
                at: at.clone(),
                checkout: *checkout.read(cx),
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

pub fn rename_branch(repo: Entity<RepoState>, old: String, window: &mut Window, cx: &mut App) {
    let name = text_input(&old, String::new(), window, cx);
    let body_name = name.clone();
    form(
        t!("branch.rename_title").into_owned(),
        Some(t!("branch.rename_text", name = old).into_owned()),
        t!("branch.rename").into_owned(),
        Some(name.clone()),
        move |_, cx| {
            field(
                t!("branch.new_name").into_owned(),
                Input::new(&body_name).id("branch-name").small(),
                None,
                palette(cx),
            )
        },
        move |_, cx| {
            let new = name.read(cx).value().trim().to_owned();
            if new.is_empty() || new == old {
                return false;
            }
            let op = Op::RenameBranch {
                old: old.clone(),
                new,
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

/// Asks before a branch goes. When it has commits of its own they are
/// named, and a tick is needed before the button works (FR-BRANCH-02).
pub fn delete_branch(
    repo: Entity<RepoState>,
    name: String,
    lost: Vec<(Oid, String)>,
    window: &mut Window,
    cx: &mut App,
) {
    if lost.is_empty() {
        let op = Op::DeleteBranch {
            name: name.clone(),
            force: false,
        };
        confirm_destructive(
            t!("branch.delete_title", name = name).into_owned(),
            t!("branch.delete_merged").into_owned(),
            t!("branch.delete").into_owned(),
            move |cx| repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx)),
            window,
            cx,
        );
        return;
    }
    let understood = cx.new(|_| false);
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let p = palette(cx);
        let ticked = *understood.read(cx);
        let (repo, name) = (repo.clone(), name.clone());
        let branch = name.clone();
        alert
            .width(px(460.))
            .title(t!("branch.delete_title", name = name).into_owned())
            .description(t!("branch.delete_unmerged", count = lost.len()).into_owned())
            .child(
                v_flex()
                    .gap_2()
                    .children(lost.iter().take(5).map(|(oid, summary)| {
                        commit_chip(format!("{}  {summary}", &oid[..7.min(oid.len())]), p)
                    }))
                    .children((lost.len() > 5).then(|| {
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_tertiary)
                            .child(t!("branch.and_more", count = lost.len() - 5).into_owned())
                    }))
                    .child(toggle(
                        "understood",
                        t!("branch.understood").into_owned(),
                        &understood,
                        cx,
                    )),
            )
            // Enter only closes: losing commits takes a click.
            .on_ok(|_, _, _| true)
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .primary()
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm")
                            .danger()
                            .disabled(!ticked)
                            .label(t!("branch.delete").into_owned())
                            .on_click(move |_, window, cx| {
                                let op = Op::DeleteBranch {
                                    name: branch.clone(),
                                    force: true,
                                };
                                repo.update(cx, |repo, cx| repo.run_op(op, cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

pub fn create_tag(
    repo: Entity<RepoState>,
    at: String,
    at_label: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name = text_input("", t!("tag.name_hint").into_owned(), window, cx);
    let message = text_input("", t!("tag.message_hint").into_owned(), window, cx);
    let (body_name, body_message) = (name.clone(), message.clone());
    form(
        t!("tag.create_title").into_owned(),
        None,
        t!("tag.create").into_owned(),
        Some(name.clone()),
        move |_, cx| {
            let p = palette(cx);
            v_flex()
                .gap(px(14.))
                .child(field(
                    t!("branch.name").into_owned(),
                    Input::new(&body_name).id("tag-name").small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("tag.at").into_owned(),
                    commit_chip(at_label.clone(), p),
                    None,
                    p,
                ))
                .child(field(
                    t!("tag.message").into_owned(),
                    Input::new(&body_message).id("tag-message").small(),
                    Some(t!("tag.message_help").into_owned()),
                    p,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let typed = name.read(cx).value().trim().to_owned();
            if typed.is_empty() {
                return false;
            }
            let note = message.read(cx).value().trim().to_owned();
            let op = Op::CreateTag {
                name: typed,
                at: at.clone(),
                message: (!note.is_empty()).then_some(note),
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

pub fn delete_tag(repo: Entity<RepoState>, name: String, window: &mut Window, cx: &mut App) {
    let op = Op::DeleteTag(name.clone());
    confirm_destructive(
        t!("tag.delete_title", name = name).into_owned(),
        t!("tag.delete_text").into_owned(),
        t!("tag.delete").into_owned(),
        move |cx| repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx)),
        window,
        cx,
    );
}

pub fn stash(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let count = repo.read(cx).status.len();
    let message = text_input("", String::new(), window, cx);
    let untracked = cx.new(|_| true);
    let (body_message, body_untracked) = (message.clone(), untracked.clone());
    form(
        t!("stash.title").into_owned(),
        Some(t!("stash.text", count = count).into_owned()),
        t!("stash.action").into_owned(),
        Some(message.clone()),
        move |_, cx| {
            v_flex()
                .gap(px(14.))
                .child(field(
                    t!("stash.message").into_owned(),
                    Input::new(&body_message).id("stash-message").small(),
                    None,
                    palette(cx),
                ))
                .child(toggle(
                    "stash-untracked",
                    t!("stash.untracked").into_owned(),
                    &body_untracked,
                    cx,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let op = Op::StashPush {
                message: message.read(cx).value().trim().to_owned(),
                include_untracked: *untracked.read(cx),
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

pub fn drop_stash(
    repo: Entity<RepoState>,
    index: u32,
    name: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    confirm_destructive(
        t!("stash.drop_title").into_owned(),
        t!("stash.drop_text", name = name).into_owned(),
        t!("stash.drop").into_owned(),
        move |cx| repo.update(cx, |repo, cx| repo.run_op(Op::StashDrop(index), cx)),
        window,
        cx,
    );
}

/// Local changes are in the way of a checkout: offer to stash them first
/// (FR-BRANCH-03).
pub fn checkout_blocked(
    repo: Entity<RepoState>,
    target: Target,
    window: &mut Window,
    cx: &mut App,
) {
    let name = target_name(&target);
    form(
        t!("checkout.blocked_title").into_owned(),
        Some(t!("checkout.blocked_text", target = name).into_owned()),
        t!("checkout.stash_and_switch").into_owned(),
        None,
        |_, cx| {
            let p = palette(cx);
            h_flex()
                .gap_2()
                .p(px(10.))
                .rounded(px(8.))
                .bg(p.accent_subtle)
                .text_size(px(12.))
                .child(ic("info", 14., p.info))
                .child(div().flex_1().child(t!("checkout.stash_help").into_owned()))
                .into_any_element()
        },
        move |_, cx| {
            let op = Op::StashAndSwitch(target.clone());
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

// ---------------------------------------------------------------------------
// Merge, rebase, reset, cherry-pick and revert.

use gittree_core::ops::merge::ResetMode;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};

use super::ui::{RefLabel, ref_label};

/// From one ref to another, as two labels with an arrow.
fn flow(from: &str, to: &str, p: Palette) -> AnyElement {
    h_flex()
        .justify_center()
        .gap(px(10.))
        .p_3()
        .rounded(px(8.))
        .bg(p.bg_panel)
        .border_1()
        .border_color(p.border)
        .child(ref_label(RefLabel::Local, from.to_owned(), p))
        .child(ic("chevron-right", 14., p.text_tertiary))
        .child(ref_label(RefLabel::Current, to.to_owned(), p))
        .into_any_element()
}

pub fn merge(
    repo: Entity<RepoState>,
    branch: String,
    into: String,
    window: &mut Window,
    cx: &mut App,
) {
    let no_ff = cx.new(|_| false);
    let (body_no_ff, from, to) = (no_ff.clone(), branch.clone(), into.clone());
    form(
        t!("merge.merge_title", into = into).into_owned(),
        Some(t!("merge.merge_text", branch = branch, into = into).into_owned()),
        t!("merge.action").into_owned(),
        None,
        move |_, cx| {
            v_flex()
                .gap(px(14.))
                .child(flow(&from, &to, palette(cx)))
                .child(toggle(
                    "no-ff",
                    t!("merge.no_ff").into_owned(),
                    &body_no_ff,
                    cx,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let op = Op::Merge {
                branch: branch.clone(),
                no_ff: *no_ff.read(cx),
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

pub fn rebase(
    repo: Entity<RepoState>,
    current: String,
    onto: String,
    window: &mut Window,
    cx: &mut App,
) {
    let (from, to) = (current.clone(), onto.clone());
    form(
        t!("merge.rebase_title", branch = current).into_owned(),
        Some(t!("merge.rebase_text", branch = current, onto = onto).into_owned()),
        t!("merge.rebase").into_owned(),
        None,
        move |_, cx| flow(&from, &to, palette(cx)),
        move |_, cx| {
            let op = Op::Rebase(onto.clone());
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

/// The three reset modes, one sentence each. Hard says how many changed
/// files go, and needs a click on the red button (FR-MERGE-04).
pub fn reset(
    repo: Entity<RepoState>,
    branch: String,
    target: String,
    window: &mut Window,
    cx: &mut App,
) {
    let mode = cx.new(|_| ResetMode::Mixed);
    let changed = repo.read(cx).status.len();
    let short: String = target.chars().take(7).collect();
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let p = palette(cx);
        let picked = *mode.read(cx);
        let option = |id: &'static str, value: ResetMode, title: String, text: String| {
            let on = picked == value;
            let handle = mode.clone();
            h_flex()
                .id(id)
                .gap(px(10.))
                .items_start()
                .p(px(10.))
                .rounded(px(8.))
                .cursor_pointer()
                .border_1()
                .border_color(if on { p.accent } else { p.border })
                .bg(if on { p.bg_selected } else { p.bg_panel })
                .on_click(move |_, window, cx| {
                    handle.update(cx, |mode, _| *mode = value);
                    window.refresh();
                })
                .child(
                    div()
                        .mt(px(2.))
                        .size(px(16.))
                        .flex_none()
                        .rounded_full()
                        .border_color(if on { p.accent } else { p.border_strong })
                        .when(on, |dot| dot.border_4().bg(p.on_accent))
                        .when(!on, |dot| dot.border_1()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if value == ResetMode::Hard {
                                    p.danger
                                } else {
                                    p.text
                                })
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.text_secondary)
                                .child(text),
                        ),
                )
        };
        let hard = picked == ResetMode::Hard;
        let (repo, target) = (repo.clone(), target.clone());
        let run = move |cx: &mut App| {
            let op = Op::Reset {
                target: target.clone(),
                mode: picked,
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
        };
        let enter = run.clone();
        let action = Button::new("confirm").on_click(move |_, window, cx| {
            run(cx);
            window.close_dialog(cx);
        });
        alert
            .width(px(480.))
            .title(t!("reset.title", branch = branch, target = short).into_owned())
            .child(
                v_flex()
                    .gap_2()
                    .child(option(
                        "reset-soft",
                        ResetMode::Soft,
                        t!("reset.soft").into_owned(),
                        t!("reset.soft_text").into_owned(),
                    ))
                    .child(option(
                        "reset-mixed",
                        ResetMode::Mixed,
                        t!("reset.mixed").into_owned(),
                        t!("reset.mixed_text").into_owned(),
                    ))
                    .child(option(
                        "reset-hard",
                        ResetMode::Hard,
                        t!("reset.hard").into_owned(),
                        t!("reset.hard_text").into_owned(),
                    ))
                    .when(hard, |body| {
                        body.child(
                            h_flex()
                                .gap_2()
                                .p(px(10.))
                                .rounded(px(8.))
                                .bg(p.danger_subtle)
                                .text_size(px(12.))
                                .child(ic("warning", 14., p.danger))
                                .child(
                                    div().flex_1().child(
                                        t!("reset.hard_warning", count = changed).into_owned(),
                                    ),
                                ),
                        )
                    }),
            )
            // Enter runs a soft or mixed reset. A hard one only closes.
            .on_ok(move |_, _, cx| {
                if !hard {
                    enter(cx);
                }
                true
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .label(t!("dialog.cancel").into_owned())
                            .when(hard, |button| button.primary())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(if hard {
                        action.danger().label(t!("reset.action_hard").into_owned())
                    } else {
                        action.primary().label(t!("reset.action").into_owned())
                    }),
            )
    });
}

/// Cherry-picks or reverts a commit. For a merge commit it first asks
/// which parent the changes are measured against (FR-MERGE-03).
pub fn pick(
    repo: Entity<RepoState>,
    oid: Oid,
    parents: Vec<Oid>,
    revert: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let op = move |mainline: Option<u32>| {
        let oid = oid.to_string();
        if revert {
            Op::Revert { oid, mainline }
        } else {
            Op::CherryPick { oid, mainline }
        }
    };
    if parents.len() < 2 {
        repo.update(cx, |repo, cx| repo.run_op(op(None), cx));
        return;
    }
    let mainline = cx.new(|_| 1u32);
    let body_mainline = mainline.clone();
    form(
        t!("merge.mainline_title").into_owned(),
        Some(t!("merge.mainline_text").into_owned()),
        if revert {
            t!("menu_item.revert").into_owned()
        } else {
            t!("menu_item.cherry_pick").into_owned()
        },
        None,
        move |_, cx| {
            let p = palette(cx);
            let picked = *body_mainline.read(cx);
            v_flex()
                .gap_2()
                .children(parents.iter().enumerate().map(|(ix, parent)| {
                    let n = ix as u32 + 1;
                    let handle = body_mainline.clone();
                    h_flex()
                        .id(("parent", ix))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(if picked == n { p.accent } else { p.border })
                        .bg(if picked == n {
                            p.bg_selected
                        } else {
                            p.bg_panel
                        })
                        .on_click(move |_, window, cx| {
                            handle.update(cx, |mainline, _| *mainline = n);
                            window.refresh();
                        })
                        .child(
                            t!("merge.parent", n = n, oid = &parent[..7.min(parent.len())])
                                .into_owned(),
                        )
                }))
                .into_any_element()
        },
        move |_, cx| {
            let chosen = *mainline.read(cx);
            repo.update(cx, |repo, cx| repo.run_op(op(Some(chosen)), cx));
            true
        },
        window,
        cx,
    );
}
