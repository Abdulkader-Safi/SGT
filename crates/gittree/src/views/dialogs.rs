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

/// Sets or clears the branch a local branch tracks (FR-BRANCH-05).
pub fn set_upstream(
    repo: Entity<RepoState>,
    branch: String,
    current: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let shown = current
        .as_deref()
        .map(|full| {
            full.strip_prefix("refs/remotes/")
                .unwrap_or(full)
                .to_owned()
        })
        .unwrap_or_else(|| format!("origin/{branch}"));
    let upstream = text_input(&shown, "origin/main".into(), window, cx);
    let body = upstream.clone();
    form(
        t!("upstream.title", branch = &branch).into_owned(),
        Some(t!("upstream.text").into_owned()),
        t!("upstream.action").into_owned(),
        Some(upstream.clone()),
        move |_, cx| {
            field(
                t!("upstream.field").into_owned(),
                Input::new(&body).id("upstream").small(),
                None,
                palette(cx),
            )
        },
        move |_, cx| {
            let value = upstream.read(cx).value().trim().to_owned();
            let op = Op::SetUpstream {
                branch: branch.clone(),
                upstream: (!value.is_empty()).then_some(value),
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

/// Changes the name and email commits get in this repository. The dialog
/// says it writes this repository's config only (FR-SET-05).
pub fn identity(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let (name, email) = repo.read(cx).identity.clone();
    let name = text_input(&name, "Ada Lovelace".into(), window, cx);
    let email = text_input(&email, "ada@example.com".into(), window, cx);
    let shown = (name.clone(), email.clone());
    form(
        t!("identity.title").into_owned(),
        Some(t!("identity.text").into_owned()),
        t!("identity.action").into_owned(),
        Some(name.clone()),
        move |_, cx| {
            let p = palette(cx);
            v_flex()
                .gap(px(14.))
                .child(field(
                    t!("identity.name").into_owned(),
                    Input::new(&shown.0).id("identity-name").small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("identity.email").into_owned(),
                    Input::new(&shown.1).id("identity-email").small(),
                    None,
                    p,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let name = name.read(cx).value().trim().to_owned();
            let email = email.read(cx).value().trim().to_owned();
            if name.is_empty() || email.is_empty() {
                return false;
            }
            repo.update(cx, |repo, cx| {
                repo.run_op(Op::SetIdentity { name, email }, cx)
            });
            true
        },
        window,
        cx,
    );
}

/// Names the branch a stash becomes (FR-STASH-02).
pub fn stash_branch(repo: Entity<RepoState>, index: u32, window: &mut Window, cx: &mut App) {
    let name = text_input("", t!("branch.name_hint").into_owned(), window, cx);
    let body = name.clone();
    form(
        t!("stash.branch_title").into_owned(),
        Some(t!("stash.branch_text").into_owned()),
        t!("branch.create").into_owned(),
        Some(name.clone()),
        move |_, cx| {
            field(
                t!("branch.name").into_owned(),
                Input::new(&body).id("stash-branch-name").small(),
                None,
                palette(cx),
            )
        },
        move |_, cx| {
            let name = name.read(cx).value().trim().to_owned();
            if name.is_empty() {
                return false;
            }
            let op = Op::StashBranch { name, index };
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

/// What a merge of `theirs` into HEAD would conflict on: `None` while Git
/// works it out.
type Prediction = Entity<Option<Result<Vec<gittree_core::model::RepoPath>, ()>>>;

/// Starts working out the conflicts in the background (FR-MERGE-06).
fn predict(
    repo: &Entity<RepoState>,
    theirs: String,
    window: &mut Window,
    cx: &mut App,
) -> Prediction {
    let state: Prediction = cx.new(|_| None);
    let runner = repo.read(cx).runner.clone();
    let task = cx.background_spawn(async move {
        gittree_core::ops::merge::predict_conflicts(&runner, &theirs).map_err(drop)
    });
    let handle = state.clone();
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| {
                handle.update(cx, |state, _| *state = Some(result));
                window.refresh();
            });
        })
        .detach();
    state
}

/// The prediction as a line, and the files when there are any. A failed
/// prediction shows nothing: the merge itself will say what is wrong.
fn prediction_note(state: &Prediction, cx: &App) -> AnyElement {
    let p = palette(cx);
    let line = |icon: &str, text: String, color| {
        h_flex()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(color)
            .child(ic(icon, 14., color))
            .child(text)
    };
    match state.read(cx) {
        None => line(
            "refresh",
            t!("merge.predicting").into_owned(),
            p.text_tertiary,
        )
        .into_any_element(),
        Some(Ok(files)) if files.is_empty() => {
            line("check", t!("merge.no_conflicts").into_owned(), p.success).into_any_element()
        }
        Some(Ok(files)) => v_flex()
            .gap(px(4.))
            .child(line(
                "warning",
                t!("merge.conflicts_ahead", count = files.len()).into_owned(),
                p.warning,
            ))
            .children(files.iter().take(8).map(|file| {
                div()
                    .pl(px(20.))
                    .truncate()
                    .font_family(super::graph::mono(cx))
                    .text_size(px(12.))
                    .text_color(p.text_secondary)
                    .child(file.to_string())
            }))
            .into_any_element(),
        Some(Err(())) => div().into_any_element(),
    }
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
    let conflicts = predict(&repo, branch.clone(), window, cx);
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
                .child(prediction_note(&conflicts, cx))
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
    let conflicts = predict(&repo, onto.clone(), window, cx);
    form(
        t!("merge.rebase_title", branch = current).into_owned(),
        Some(t!("merge.rebase_text", branch = current, onto = onto).into_owned()),
        t!("merge.rebase").into_owned(),
        None,
        move |_, cx| {
            v_flex()
                .gap(px(14.))
                .child(flow(&from, &to, palette(cx)))
                .child(prediction_note(&conflicts, cx))
                .into_any_element()
        },
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

// ---------------------------------------------------------------------------
// Remotes, credentials and clone.

use crate::app_state::AppState;
use crate::repo_state::CredentialRequest;

/// Git or ssh asks for a user name, password or passphrase. What is typed
/// goes to Git once and is dropped (FR-SYNC-03).
pub fn credential(request: CredentialRequest, window: &mut Window, cx: &mut App) {
    let secret = request.secret;
    let input = cx.new(|cx| InputState::new(window, cx).masked(secret));
    let first = Cell::new(true);
    let request = Rc::new(request);
    window.open_alert_dialog(cx, move |alert, window, cx| {
        if first.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let p = palette(cx);
        let answer = {
            let (request, input) = (request.clone(), input.clone());
            move |cx: &mut App| {
                let typed = input.read(cx).value().to_string();
                let _ = request.reply.send(Some(typed));
            }
        };
        let (enter, click) = (answer.clone(), answer);
        let (cancel, closed) = (request.clone(), request.clone());
        alert
            .width(px(460.))
            .title(t!("credential.title").into_owned())
            .description(request.question.clone())
            .child(
                v_flex()
                    .gap(px(10.))
                    .child(Input::new(&input).id("credential").small())
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(p.text_tertiary)
                            .child(ic("lock", 12., p.text_tertiary))
                            .child(div().flex_1().child(t!("credential.note").into_owned())),
                    ),
            )
            .on_ok(move |_, _, cx| {
                enter(cx);
                true
            })
            // Escape, or the dialog closing any other way, cancels the
            // command. A second send after an answer is ignored.
            .on_cancel(move |_, _, _| {
                let _ = closed.reply.send(None);
                true
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(move |_, window, cx| {
                                let _ = cancel.reply.send(None);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("ok")
                            .primary()
                            .label(t!("credential.ok").into_owned())
                            .on_click(move |_, window, cx| {
                                click(cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

pub fn force_push(
    repo: Entity<RepoState>,
    branch: String,
    behind: u32,
    window: &mut Window,
    cx: &mut App,
) {
    confirm_destructive(
        t!("push.force_title", branch = branch).into_owned(),
        t!("push.force_text", count = behind).into_owned(),
        t!("push.force").into_owned(),
        move |cx| repo.update(cx, |repo, cx| repo.run_op(Op::Push { force: true }, cx)),
        window,
        cx,
    );
}

/// Adds a remote, or with `existing` changes the URL of one.
pub fn remote(
    repo: Entity<RepoState>,
    existing: Option<(String, String)>,
    window: &mut Window,
    cx: &mut App,
) {
    let editing = existing.is_some();
    let (old_name, old_url) = existing.unwrap_or_default();
    let name = text_input(&old_name, "origin".into(), window, cx);
    let url = text_input(&old_url, String::new(), window, cx);
    let (body_name, body_url) = (name.clone(), url.clone());
    form(
        if editing {
            t!("remote.edit_title").into_owned()
        } else {
            t!("remote.add_title").into_owned()
        },
        None,
        if editing {
            t!("remote.save").into_owned()
        } else {
            t!("remote.add").into_owned()
        },
        Some(if editing { url.clone() } else { name.clone() }),
        move |_, cx| {
            let p = palette(cx);
            v_flex()
                .gap(px(14.))
                .child(field(
                    t!("branch.name").into_owned(),
                    Input::new(&body_name).id("remote-name").small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("remote.url").into_owned(),
                    Input::new(&body_url).id("remote-url").small(),
                    Some(t!("remote.url_help").into_owned()),
                    p,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let new_name = name.read(cx).value().trim().to_owned();
            let new_url = url.read(cx).value().trim().to_owned();
            if new_name.is_empty() || new_url.is_empty() {
                return false;
            }
            repo.update(cx, |repo, cx| {
                if !editing {
                    repo.run_op(
                        Op::AddRemote {
                            name: new_name,
                            url: new_url,
                        },
                        cx,
                    );
                    return;
                }
                if new_url != old_url {
                    repo.run_op(
                        Op::SetRemoteUrl {
                            name: old_name.clone(),
                            url: new_url,
                        },
                        cx,
                    );
                }
                if new_name != old_name {
                    repo.run_op(
                        Op::RenameRemote {
                            old: old_name.clone(),
                            new: new_name,
                        },
                        cx,
                    );
                }
            });
            true
        },
        window,
        cx,
    );
}

pub fn remove_remote(repo: Entity<RepoState>, name: String, window: &mut Window, cx: &mut App) {
    let op = Op::RemoveRemote(name.clone());
    confirm_destructive(
        t!("remote.remove_title", name = name).into_owned(),
        t!("remote.remove_text").into_owned(),
        t!("remote.remove").into_owned(),
        move |cx| repo.update(cx, |repo, cx| repo.run_op(op.clone(), cx)),
        window,
        cx,
    );
}

/// The clone dialog: URL and destination, then progress with Cancel, or
/// the error with another try (FR-REPO-02).
pub fn clone_repository(app: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let base = app.read(cx).settings.clone_dir.clone();
    let url = text_input("", "git@github.com:user/project.git".into(), window, cx);
    let dest = text_input(&base.display().to_string(), String::new(), window, cx);
    let first = Cell::new(true);
    window.open_alert_dialog(cx, move |alert, window, cx| {
        if first.replace(false) {
            url.focus_handle(cx).focus(window, cx);
        }
        let p = palette(cx);
        let state = app.read(cx);
        let job = state.clone.as_ref();
        let running = job.is_some_and(|job| job.running());
        let failed = job
            .and_then(|job| job.error.as_ref())
            .map(|error| crate::error_message(error.kind));
        let start = {
            let (app, url, dest) = (app.clone(), url.clone(), dest.clone());
            move |cx: &mut App| {
                let source = url.read(cx).value().trim().to_owned();
                if source.is_empty() {
                    return;
                }
                // Into a folder named after the repository.
                let name = source
                    .trim_end_matches('/')
                    .rsplit(['/', ':'])
                    .next()
                    .unwrap_or("repository")
                    .trim_end_matches(".git")
                    .to_owned();
                let folder = std::path::PathBuf::from(dest.read(cx).value().trim()).join(name);
                app.update(cx, |app, cx| app.clone_repo(source, folder, cx));
            }
        };
        let (enter, click) = (start.clone(), start);
        let (stop, closed) = (app.clone(), app.clone());
        let progress = job.filter(|_| running).map(|job| {
            let (phase, percent) = match &job.progress {
                Some(progress) => (progress.phase.clone(), progress.percent),
                None => (t!("clone.starting").into_owned(), None),
            };
            v_flex()
                .gap_2()
                .p(px(14.))
                .rounded(px(8.))
                .bg(p.bg_panel)
                .border_1()
                .border_color(p.border)
                .child(
                    h_flex()
                        .text_size(px(12.))
                        .text_color(p.text_secondary)
                        .child(div().flex_1().child(phase))
                        .children(percent.map(|percent| format!("{percent}%"))),
                )
                .child(
                    div().h(px(4.)).rounded(px(2.)).bg(p.bg_hover).child(
                        div()
                            .h_full()
                            .rounded(px(2.))
                            .bg(p.accent)
                            .w(gpui_kit::relative(percent.unwrap_or(4) as f32 / 100.)),
                    ),
                )
        });
        alert
            .width(px(480.))
            .title(t!("clone.title").into_owned())
            .child(
                v_flex()
                    .gap(px(14.))
                    .child(field(
                        t!("clone.url").into_owned(),
                        Input::new(&url).id("clone-url").small().disabled(running),
                        Some(failed.unwrap_or_else(|| t!("remote.url_help").into_owned())),
                        p,
                    ))
                    .child(field(
                        t!("clone.dest").into_owned(),
                        Input::new(&dest).id("clone-dest").small().disabled(running),
                        Some(t!("clone.dest_help").into_owned()),
                        p,
                    ))
                    .children(progress),
            )
            .on_ok(move |_, _, cx| {
                if !running {
                    enter(cx);
                }
                // The dialog stays to show progress. It closes itself when
                // the repository opens.
                false
            })
            .on_cancel(move |_, _, cx| {
                closed.update(cx, |app, cx| app.cancel_clone(cx));
                true
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel")
                            .label(t!("dialog.cancel").into_owned())
                            .on_click(move |_, window, cx| {
                                stop.update(cx, |app, cx| app.cancel_clone(cx));
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("ok")
                            .primary()
                            .loading(running)
                            .disabled(running)
                            .label(t!("clone.action").into_owned())
                            .on_click(move |_, _, cx| click(cx)),
                    ),
            )
    });
}

/// The snapshot list: what was saved before a risky action, with Restore
/// and "Restore to new branch" (FR-UNDO-02).
pub fn snapshots(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let p = palette(cx);
        let state = repo.read(cx);
        let (busy, trusted) = (state.busy, state.repository.trusted);
        let rows = state.snapshots.iter().enumerate().map(|(ix, entry)| {
            let snapshot = entry.snapshot.clone().unwrap_or_default();
            // A commit a force-push replaced has no files to put back in
            // place: it can only become a branch.
            let in_place = snapshot.remote_ref.is_none();
            let has_commit = snapshot.tree_ref.is_some() || snapshot.remote_ref.is_some();
            let restore = |id: &'static str, label: String, branch: Option<String>| {
                let (repo, entry_id) = (repo.clone(), entry.id.clone());
                Button::new((id, ix))
                    .xsmall()
                    .label(label)
                    .disabled(busy || !trusted)
                    .on_click(move |_, _, cx| {
                        let op = Op::RestoreSnapshot {
                            id: entry_id.clone(),
                            branch: branch.clone(),
                        };
                        repo.update(cx, |repo, cx| repo.run_op(op, cx))
                    })
            };
            let short: String = entry.id.chars().take(8).collect();
            h_flex()
                .min_h(px(48.))
                .px_3()
                .py_1()
                .gap_3()
                .border_b_1()
                .border_color(p.border)
                .child(ic("clock", 14., p.text_secondary))
                .child(
                    v_flex()
                        .flex_1()
                        .overflow_hidden()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(entry.label.clone()),
                        )
                        .child(div().text_size(px(12.)).text_color(p.text_tertiary).child(
                            format!(
                                "{}{}",
                                crate::local_time(entry.time),
                                if entry.undone {
                                    format!(" · {}", t!("snapshots.undone"))
                                } else {
                                    String::new()
                                }
                            ),
                        )),
                )
                .when(has_commit, |row| {
                    row.child(restore(
                        "restore-branch",
                        t!("snapshots.restore_branch").into_owned(),
                        Some(format!("snapshot-{short}")),
                    ))
                })
                .when(in_place, |row| {
                    row.child(
                        restore("restore", t!("snapshots.restore").into_owned(), None).primary(),
                    )
                })
        });
        let empty = state.snapshots.is_empty().then(|| {
            v_flex()
                .items_center()
                .gap_1()
                .p_6()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(t!("snapshots.empty_title").into_owned()),
                )
                .child(
                    div()
                        .text_center()
                        .text_color(p.text_secondary)
                        .child(t!("snapshots.empty_text").into_owned()),
                )
        });
        alert
            .width(px(600.))
            .title(t!("snapshots.title").into_owned())
            .description(t!("snapshots.text").into_owned())
            .child(
                v_flex()
                    .max_h(px(360.))
                    .overflow_hidden()
                    .rounded(px(8.))
                    .bg(p.bg_panel)
                    .border_1()
                    .border_color(p.border)
                    .children(rows)
                    .children(empty),
            )
            .ok_text(t!("snapshots.close").into_owned())
    });
}

/// Every Git command the app ran for this repository in this session,
/// newest first: when, how long, how it ended, and what Git said on a
/// failure (FR-STATE-05). A list taken when the dialog opens.
pub fn activity(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let mut records = repo.read(cx).runner.activity();
    records.reverse();
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let p = palette(cx);
        let font = super::graph::mono(cx);
        let rows = records.iter().enumerate().map(|(ix, record)| {
            let failed = record.exit_code != 0;
            let ended = if record.exit_code == -1 {
                t!("activity.stopped").into_owned()
            } else {
                t!("activity.exit", code = record.exit_code).into_owned()
            };
            v_flex()
                .id(("activity", ix))
                .px_3()
                .py(px(6.))
                .gap(px(2.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    h_flex()
                        .gap_3()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .font_family(font.clone())
                                .child(format!("git {}", record.args.join(" "))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(p.text_tertiary)
                                .child(format!("{} ms", record.duration_ms)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(if failed { p.danger } else { p.text_tertiary })
                                .child(ended),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.text_tertiary)
                        .child(crate::local_time(record.time)),
                )
                .when(failed && !record.stderr.trim().is_empty(), |row| {
                    let said: String = record
                        .stderr
                        .trim()
                        .lines()
                        .take(4)
                        .collect::<Vec<_>>()
                        .join("\n");
                    row.child(
                        div()
                            .font_family(font.clone())
                            .text_size(px(11.))
                            .text_color(p.danger)
                            .child(said),
                    )
                })
        });
        let empty = records.is_empty().then(|| {
            div()
                .p_6()
                .text_center()
                .text_color(p.text_secondary)
                .child(t!("activity.empty").into_owned())
        });
        alert
            .width(px(720.))
            .title(t!("activity.title").into_owned())
            .description(t!("activity.text").into_owned())
            .child(
                v_flex()
                    .id("activity-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .rounded(px(8.))
                    .bg(p.bg_panel)
                    .border_1()
                    .border_color(p.border)
                    .children(rows)
                    .children(empty),
            )
            .ok_text(t!("snapshots.close").into_owned())
    });
}

/// Where HEAD has been, with "Create branch" on each step (FR-UNDO-03).
// ponytail: the last 200 steps of HEAD's reflog. Branch reflogs and paging
// wait until someone needs them.
pub fn reflog(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let entries: Entity<Option<Vec<gittree_core::ops::read::ReflogEntry>>> = cx.new(|_| None);
    let runner = repo.read(cx).runner.clone();
    let task = cx.background_spawn(async move {
        gittree_core::ops::read::reflog(&runner, 200).unwrap_or_default()
    });
    let handle = entries.clone();
    window
        .spawn(cx, async move |cx| {
            let read = task.await;
            let _ = cx.update(|window, cx| {
                handle.update(cx, |entries, _| *entries = Some(read));
                window.refresh();
            });
        })
        .detach();
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let p = palette(cx);
        let state = repo.read(cx);
        let can_branch = state.repository.trusted && !state.busy;
        let font = super::graph::mono(cx);
        let note = |text: String| {
            div()
                .p_6()
                .text_center()
                .text_color(p.text_secondary)
                .child(text)
        };
        let list = match entries.read(cx) {
            None => note(t!("reflog.loading").into_owned()).into_any_element(),
            Some(read) if read.is_empty() => {
                note(t!("reflog.empty").into_owned()).into_any_element()
            }
            Some(read) => v_flex()
                .id("reflog")
                .max_h(px(400.))
                .overflow_y_scroll()
                .children(read.iter().enumerate().map(|(ix, entry)| {
                    let short: String = entry.oid.chars().take(7).collect();
                    let (repo, oid, label) = (repo.clone(), entry.oid.to_string(), short.clone());
                    h_flex()
                        .min_h(px(40.))
                        .px_3()
                        .gap_3()
                        .border_b_1()
                        .border_color(p.border)
                        .child(
                            div()
                                .flex_none()
                                .font_family(font.clone())
                                .text_size(px(12.))
                                .text_color(p.text_secondary)
                                .child(short),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .overflow_hidden()
                                .child(div().truncate().child(entry.message.clone()))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(p.text_tertiary)
                                        .child(crate::local_time(entry.time)),
                                ),
                        )
                        .child(
                            Button::new(("reflog-branch", ix))
                                .xsmall()
                                .label(t!("reflog.branch").into_owned())
                                .disabled(!can_branch)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    create_branch(
                                        repo.clone(),
                                        oid.clone(),
                                        label.clone(),
                                        window,
                                        cx,
                                    )
                                }),
                        )
                }))
                .into_any_element(),
        };
        alert
            .width(px(600.))
            .title(t!("reflog.title").into_owned())
            .description(t!("reflog.text").into_owned())
            .child(
                div()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(p.bg_panel)
                    .border_1()
                    .border_color(p.border)
                    .child(list),
            )
            .ok_text(t!("snapshots.close").into_owned())
    });
}

/// The files that differ between two branches, tags or commits
/// (FR-DIFF-07). Both ends can be edited, and Compare reads the list again.
// ponytail: a list of files. Opening a file's diff from here is not built:
// the commit details already show diffs one commit at a time.
pub fn compare(
    repo: Entity<RepoState>,
    from: String,
    to: String,
    window: &mut Window,
    cx: &mut App,
) {
    type Files = Option<Result<Vec<gittree_core::model::FileChange>, String>>;
    let files: Entity<Files> = cx.new(|_| None);
    let from_input = text_input(&from, String::new(), window, cx);
    let to_input = text_input(&to, String::new(), window, cx);
    let runner = repo.read(cx).runner.clone();
    let load = Rc::new({
        let files = files.clone();
        move |from: String, to: String, window: &mut Window, cx: &mut App| {
            files.update(cx, |files, _| *files = None);
            let runner = runner.clone();
            let task = cx.background_spawn(async move {
                gittree_core::ops::read::compare(&runner, &from, &to)
                    .map_err(|error| error.stderr.trim().to_owned())
            });
            let files = files.clone();
            window
                .spawn(cx, async move |cx| {
                    let read = task.await;
                    let _ = cx.update(|window, cx| {
                        files.update(cx, |files, _| *files = Some(read));
                        window.refresh();
                    });
                })
                .detach();
        }
    });
    load(from, to, window, cx);
    let shown = (from_input.clone(), to_input.clone(), files.clone());
    form(
        t!("compare.title").into_owned(),
        None,
        t!("compare.action").into_owned(),
        Some(from_input.clone()),
        move |_, cx| {
            let p = palette(cx);
            let (from, to, files) = &shown;
            let font = super::graph::mono(cx);
            let note = |text: String, color| {
                div()
                    .py_2()
                    .text_size(px(12.))
                    .text_color(color)
                    .child(text)
            };
            let list = match files.read(cx) {
                None => {
                    note(t!("compare.loading").into_owned(), p.text_tertiary).into_any_element()
                }
                Some(Err(problem)) => note(problem.clone(), p.danger).into_any_element(),
                Some(Ok(list)) if list.is_empty() => {
                    note(t!("compare.same").into_owned(), p.text_tertiary).into_any_element()
                }
                Some(Ok(list)) => v_flex()
                    .gap(px(4.))
                    .child(note(
                        t!("compare.count", count = list.len()).into_owned(),
                        p.text_secondary,
                    ))
                    .child(
                        v_flex()
                            .id("compare-files")
                            .max_h(px(300.))
                            .overflow_y_scroll()
                            .children(list.iter().map(|file| {
                                h_flex()
                                    .gap(px(8.))
                                    .h(px(24.))
                                    .text_size(px(12.))
                                    .child(
                                        super::ui::status_mark(file.change, false, p)
                                            .font_family(font.clone()),
                                    )
                                    .child(
                                        div().flex_1().truncate().font_family(font.clone()).child(
                                            match &file.orig_path {
                                                Some(old) => format!("{old} → {}", file.path),
                                                None => file.path.to_string(),
                                            },
                                        ),
                                    )
                            })),
                    )
                    .into_any_element(),
            };
            v_flex()
                .gap(px(14.))
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(div().flex_1().child(field(
                            t!("compare.from").into_owned(),
                            Input::new(from).id("compare-from").small(),
                            None,
                            p,
                        )))
                        .child(div().flex_1().child(field(
                            t!("compare.to").into_owned(),
                            Input::new(to).id("compare-to").small(),
                            None,
                            p,
                        ))),
                )
                .child(list)
                .into_any_element()
        },
        move |window, cx| {
            let from = from_input.read(cx).value().trim().to_owned();
            let to = to_input.read(cx).value().trim().to_owned();
            load(from, to, window, cx);
            // The list updates in place. Cancel closes the dialog.
            false
        },
        window,
        cx,
    );
}

/// One commit in the interactive rebase dialog.
struct PlanRow {
    oid: String,
    summary: String,
    step: gittree_core::ops::merge::Step,
    /// The new message, used when the step is Reword.
    message: Entity<InputState>,
}

/// Interactive rebase of the commits after `base` (FR-REWRITE-01). The
/// list is the plan: nothing changes until Rebase is pressed.
// ponytail: the step is one button that cycles through the six, and order
// changes by Up and Down. A menu per row and drag to reorder would be
// kinder with many commits.
pub fn interactive_rebase(
    repo: Entity<RepoState>,
    base: String,
    window: &mut Window,
    cx: &mut App,
) {
    use gittree_core::ops::merge::{self, Step, TodoItem};
    type Plan = Entity<Option<Result<Vec<PlanRow>, String>>>;
    let plan: Plan = cx.new(|_| None);
    let runner = repo.read(cx).runner.clone();
    let task = cx.background_spawn({
        let base = base.clone();
        async move { merge::rebase_commits(&runner, &base).map_err(|e| e.stderr) }
    });
    let handle = plan.clone();
    window
        .spawn(cx, async move |cx| {
            let read = task.await;
            let _ = cx.update(|window, cx| {
                let rows = read.map(|commits| {
                    commits
                        .into_iter()
                        .map(|(oid, summary)| PlanRow {
                            message: text_input(&summary, String::new(), window, cx),
                            oid,
                            summary,
                            step: Step::Pick,
                        })
                        .collect()
                });
                handle.update(cx, |plan, _| *plan = Some(rows));
                window.refresh();
            });
        })
        .detach();
    let name = |step: Step| match step {
        Step::Pick => t!("rebase_plan.pick"),
        Step::Reword => t!("rebase_plan.reword"),
        Step::Edit => t!("rebase_plan.edit"),
        Step::Squash => t!("rebase_plan.squash"),
        Step::Fixup => t!("rebase_plan.fixup"),
        Step::Drop => t!("rebase_plan.drop"),
    };
    let shown = plan.clone();
    let short: String = base.chars().take(7).collect();
    form(
        t!("rebase_plan.title", base = &short).into_owned(),
        Some(t!("rebase_plan.text").into_owned()),
        t!("merge.rebase").into_owned(),
        None,
        move |_, cx| {
            let p = palette(cx);
            let font = super::graph::mono(cx);
            let note = |text: String, color| div().py_2().text_color(color).child(text);
            let rows = match shown.read(cx) {
                None => {
                    return note(t!("rebase_plan.loading").into_owned(), p.text_tertiary)
                        .into_any_element();
                }
                Some(Err(problem)) => return note(problem.clone(), p.danger).into_any_element(),
                Some(Ok(rows)) => rows,
            };
            let count = rows.len();
            let list = rows.iter().enumerate().map(|(ix, row)| {
                let change =
                    |id: &'static str, label: String, act: fn(&mut Vec<PlanRow>, usize)| {
                        let plan = shown.clone();
                        Button::new((id, ix)).xsmall().label(label).on_click(
                            move |_, window, cx| {
                                plan.update(cx, |plan, _| {
                                    if let Some(Ok(rows)) = plan {
                                        act(rows, ix)
                                    }
                                });
                                window.refresh();
                            },
                        )
                    };
                let gone = row.step == Step::Drop;
                v_flex()
                    .gap(px(4.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                change("plan-step", name(row.step).into_owned(), |rows, ix| {
                                    rows[ix].step = match rows[ix].step {
                                        Step::Pick => Step::Reword,
                                        Step::Reword => Step::Edit,
                                        Step::Edit => Step::Squash,
                                        Step::Squash => Step::Fixup,
                                        Step::Fixup => Step::Drop,
                                        Step::Drop => Step::Pick,
                                    }
                                })
                                .w(px(76.)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(font.clone())
                                    .text_size(px(12.))
                                    .text_color(p.text_tertiary)
                                    .child(row.oid.chars().take(7).collect::<String>()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_color(if gone { p.text_tertiary } else { p.text })
                                    .when(gone, |text| text.line_through())
                                    .child(row.summary.clone()),
                            )
                            .child(
                                change("plan-up", "↑".into(), |rows, ix| rows.swap(ix, ix - 1))
                                    .disabled(ix == 0),
                            )
                            .child(
                                change("plan-down", "↓".into(), |rows, ix| rows.swap(ix, ix + 1))
                                    .disabled(ix + 1 == count),
                            ),
                    )
                    .when(row.step == Step::Reword, |row_box| {
                        row_box.child(div().pl(px(84.)).child(Input::new(&row.message).small()))
                    })
            });
            v_flex()
                .id("rebase-plan")
                .max_h(px(380.))
                .overflow_y_scroll()
                .children(list)
                .into_any_element()
        },
        move |_, cx| {
            let Some(Ok(rows)) = plan.read(cx) else {
                return false;
            };
            let items: Vec<TodoItem> = rows
                .iter()
                .map(|row| TodoItem {
                    oid: row.oid.clone(),
                    step: row.step,
                    message: row.message.read(cx).value().to_string(),
                })
                .collect();
            let op = Op::InteractiveRebase {
                base: base.clone(),
                items,
            };
            repo.update(cx, |repo, cx| repo.run_op(op, cx));
            true
        },
        window,
        cx,
    );
}

use gittree_core::agent::{self, AgentKind};
use gpui_kit::component::input::{Textarea, TextareaState};

/// "Start an agent session": the agent, its task, and the branch and
/// folder it gets (FR-AGENT-04).
pub fn start_session(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let kind = cx.new(|_| AgentKind::Claude);
    let task = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 8));
    let custom = text_input("", "my-agent --flag".into(), window, cx);
    let branch = text_input("", "agent/".into(), window, cx);
    let folder = text_input("", String::new(), window, cx);
    let at = match &repo.read(cx).repository.head {
        gittree_core::model::Head::Branch(name) | gittree_core::model::Head::Unborn(name) => {
            name.clone()
        }
        gittree_core::model::Head::Detached(oid) => oid.chars().take(7).collect(),
    };
    let root = repo.read(cx).repository.path.clone();
    let body = (
        kind.clone(),
        task.clone(),
        custom.clone(),
        branch.clone(),
        folder.clone(),
    );
    form(
        t!("agents.start_title").into_owned(),
        Some(t!("agents.start_text").into_owned()),
        t!("agents.start").into_owned(),
        None,
        move |_, cx| {
            let p = palette(cx);
            let (kind, task, custom, branch, folder) = &body;
            let picked = *kind.read(cx);
            let chips = h_flex()
                .gap(px(2.))
                .children(AgentKind::ALL.into_iter().enumerate().map(|(ix, value)| {
                    let on = picked == value;
                    let handle = kind.clone();
                    div()
                        .id(("agent", ix))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .border_1()
                        .border_color(if on { p.border_strong } else { p.border })
                        .bg(if on { p.bg_elevated } else { p.bg_panel })
                        .text_color(if on { p.text } else { p.text_secondary })
                        .on_click(move |_, window, cx| {
                            handle.update(cx, |kind, _| *kind = value);
                            window.refresh();
                        })
                        .child(value.label())
                }));
            v_flex()
                .gap(px(14.))
                .child(field(t!("agents.agent").into_owned(), chips, None, p))
                .when(picked == AgentKind::Other, |form| {
                    form.child(field(
                        t!("agents.command").into_owned(),
                        Input::new(custom).id("agent-command").small(),
                        Some(t!("agents.command_help").into_owned()),
                        p,
                    ))
                })
                .child(field(
                    t!("agents.task").into_owned(),
                    Textarea::new(task).small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("agents.branch").into_owned(),
                    Input::new(branch).id("agent-branch").small(),
                    Some(t!("agents.branch_help", at = &at).into_owned()),
                    p,
                ))
                .child(field(
                    t!("agents.folder").into_owned(),
                    Input::new(folder).id("agent-folder").small(),
                    Some(t!("agents.folder_help").into_owned()),
                    p,
                ))
                .into_any_element()
        },
        move |_, cx| {
            let text = task.read(cx).value().trim().to_owned();
            if text.is_empty() {
                return false;
            }
            let typed =
                |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().to_owned();
            let name = match typed(&branch, cx) {
                name if name.is_empty() => agent::branch_for(&text),
                name => name,
            };
            let path = match typed(&folder, cx) {
                path if path.is_empty() => agent::folder_for(&root, &name),
                path => path.into(),
            };
            let (kind, command) = (*kind.read(cx), typed(&custom, cx));
            repo.update(cx, |repo, cx| {
                repo.start_session(kind, &command, &text, name, path, cx)
            });
            true
        },
        window,
        cx,
    );
}

/// "Create pull request" from the current branch into the default one.
pub fn create_pull_request(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let Some((head, base)) = repo.read(cx).pull_request_branches() else {
        return;
    };
    // The newest commit's summary is the usual title.
    let summary = repo
        .read(cx)
        .history
        .commits
        .first()
        .map(|commit| commit.summary.to_string())
        .unwrap_or_default();
    let title = text_input(&summary, String::new(), window, cx);
    let body = cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 10));
    let draft = cx.new(|_| false);
    let writing = cx.new(|_| false);
    // "Write from commits": the AI helper writes both from the branch's
    // commit messages (FR-AGENT-05).
    let write = {
        let (repo, base, title, body, writing) = (
            repo.clone(),
            base.clone(),
            title.clone(),
            body.clone(),
            writing.clone(),
        );
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            let settings = &crate::app_state::app_state(cx).read(cx).settings;
            let argv = match agent::ask_command(settings.ai_agent, &settings.ai_command) {
                Ok(argv) => argv,
                Err(problem) => {
                    return window.push_notification(
                        gpui_kit::component::notification::Notification::info(problem),
                        cx,
                    );
                }
            };
            writing.update(cx, |writing, _| *writing = true);
            let (runner, base) = (repo.read(cx).runner.clone(), base.clone());
            let task = cx.background_spawn(async move {
                let messages = gittree_core::ops::read::branch_messages(&runner, &base)
                    .map_err(|error| error.stderr)?;
                agent::ask(
                    &argv,
                    &agent::pull_request_prompt(&messages),
                    &std::sync::atomic::AtomicBool::new(false),
                )
            });
            let (title, body, writing) = (title.clone(), body.clone(), writing.clone());
            window
                .spawn(cx, async move |cx| {
                    let answer = task.await;
                    let _ = cx.update(|window, cx| {
                        writing.update(cx, |writing, _| *writing = false);
                        match answer {
                            Ok(text) => {
                                let (first, rest) = text.split_once('\n').unwrap_or((&text, ""));
                                let (first, rest) =
                                    (first.trim().to_owned(), rest.trim().to_owned());
                                title.update(cx, |input, cx| input.set_value(first, window, cx));
                                body.update(cx, |input, cx| input.set_value(rest, window, cx));
                            }
                            Err(problem) => window.push_notification(
                                gpui_kit::component::notification::Notification::info(problem),
                                cx,
                            ),
                        }
                        window.refresh();
                    });
                })
                .detach();
        }
    };
    let write = Rc::new(write);
    let shown = (title.clone(), body.clone(), draft.clone());
    form(
        t!("launchpad.create").into_owned(),
        None,
        t!("launchpad.create").into_owned(),
        Some(title.clone()),
        move |_, cx| {
            let p = palette(cx);
            let (title, body, draft) = &shown;
            v_flex()
                .gap(px(14.))
                .child(flow(&head, &base, p))
                .child(field(
                    t!("launchpad.pr_title").into_owned(),
                    Input::new(title).id("pr-title").small(),
                    None,
                    p,
                ))
                .child(field(
                    t!("launchpad.pr_body").into_owned(),
                    Textarea::new(body).small(),
                    None,
                    p,
                ))
                .child(
                    h_flex()
                        .justify_between()
                        .child(toggle(
                            "pr-draft",
                            t!("launchpad.pr_draft").into_owned(),
                            draft,
                            cx,
                        ))
                        .child({
                            let (write, busy) = (write.clone(), *writing.read(cx));
                            Button::new("pr-write")
                                .small()
                                .loading(busy)
                                .disabled(busy)
                                .label(if busy {
                                    t!("ai.writing").into_owned()
                                } else {
                                    t!("launchpad.pr_write").into_owned()
                                })
                                .on_click(move |event, window, cx| write(event, window, cx))
                        }),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .child(t!("launchpad.pr_note").into_owned()),
                )
                .into_any_element()
        },
        move |_, cx| {
            let name = title.read(cx).value().trim().to_owned();
            if name.is_empty() {
                return false;
            }
            let (text, as_draft) = (body.read(cx).value().to_string(), *draft.read(cx));
            repo.update(cx, |repo, cx| {
                repo.create_pull_request(name, text, as_draft, cx)
            });
            true
        },
        window,
        cx,
    );
}

/// The commits an agent proposes for the uncommitted work: a message to
/// edit and the files for each. Nothing is committed until it is applied.
// ponytail: whole files, in the agent's order. Moving a file or a hunk to
// another commit by dragging, as the design shows, is not built.
pub fn commit_plan(repo: Entity<RepoState>, window: &mut Window, cx: &mut App) {
    let Some(plan) = repo.read(cx).ai.plan.clone() else {
        return;
    };
    let messages: Vec<Entity<InputState>> = plan
        .iter()
        .map(|commit| text_input(&commit.message, String::new(), window, cx))
        .collect();
    let files: usize = plan.iter().map(|commit| commit.files.len()).sum();
    let font = super::graph::mono(cx);
    let shown = (plan.clone(), messages.clone());
    form(
        t!("ai.plan_title").into_owned(),
        Some(t!("ai.plan_count", files = files, commits = plan.len()).into_owned()),
        t!("ai.plan_apply", count = plan.len()).into_owned(),
        messages.first().cloned(),
        move |_, cx| {
            let p = palette(cx);
            let (plan, messages) = &shown;
            let commits = plan
                .iter()
                .zip(messages)
                .enumerate()
                .map(|(ix, (commit, message))| {
                    let paths = commit.files.iter().map(|path| {
                        div()
                            .pl(px(28.))
                            .truncate()
                            .font_family(font.clone())
                            .text_size(px(12.))
                            .text_color(p.text_secondary)
                            .child(path.to_string())
                    });
                    v_flex()
                        .gap(px(4.))
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .w(px(20.))
                                        .flex_none()
                                        .text_size(px(12.))
                                        .text_color(p.text_tertiary)
                                        .child((ix + 1).to_string()),
                                )
                                .child(div().flex_1().child(Input::new(message).small())),
                        )
                        .children(paths)
                });
            v_flex()
                .gap(px(14.))
                .child(
                    v_flex()
                        .id("commit-plan")
                        .gap(px(12.))
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .children(commits),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_tertiary)
                        .child(t!("ai.plan_note").into_owned()),
                )
                .into_any_element()
        },
        move |_, cx| {
            let mut plan = plan.clone();
            for (commit, message) in plan.iter_mut().zip(&messages) {
                commit.message = message.read(cx).value().trim().to_owned();
                if commit.message.is_empty() {
                    return false;
                }
            }
            repo.update(cx, |repo, cx| repo.apply_plan(plan, cx));
            true
        },
        window,
        cx,
    );
}
