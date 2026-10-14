//! The command palette: every command with its shortcut, then the branches
//! and tags by name (FR-CMD-01).

use std::cell::Cell;

use gittree_core::model::{Oid, RefKind};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{WindowExt, h_flex};
use gpui_kit::{App, AppContext, Entity, FocusHandle, ParentElement, Styled, Window, div};

use crate::zoom::px;
use rust_i18n::t;

use super::ui::{ic, kbd};
use crate::actions::commands;
use crate::palette::palette;
use crate::repo_state::RepoState;

/// Opens the palette. A command runs as if it came from `focus`, so the
/// workspace's own handlers receive it.
pub fn open(repo: Entity<RepoState>, focus: FocusHandle, window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| CommandState::new(window, cx));
    // Branches and tags, with the commit each one names.
    let refs: Vec<(RefKind, String, Oid)> = repo
        .read(cx)
        .refs
        .iter()
        .filter(|r| r.kind != RefKind::Stash)
        .map(|r| (r.kind, r.short_name().to_owned(), r.target.clone()))
        .collect();
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            state.update(cx, |state, cx| state.focus(window, cx));
        }
        let p = palette(cx);
        let row = move |icon: &'static str, label: String, hint: String| {
            move |_: &mut Window, _: &mut App| {
                h_flex()
                    .w_full()
                    .gap(px(10.))
                    .child(ic(icon, 14., p.text_secondary))
                    .child(div().flex_1().truncate().child(label.clone()))
                    .children((!hint.is_empty()).then(|| kbd(hint.clone(), p)))
            }
        };
        let command_items = commands().into_iter().map(|(key, icon, shortcut, _)| {
            let label = t!(key).trim_end_matches('…').to_owned();
            CommandItem::new()
                .label(label.clone())
                .child(row(icon, label, shortcut.to_owned()))
        });
        let ref_items = refs.iter().map(|(kind, name, _)| {
            let icon = match kind {
                RefKind::Tag => "tag",
                RefKind::Remote => "remote",
                _ => "branch",
            };
            CommandItem::new()
                .label(name.clone())
                .child(row(icon, name.clone(), String::new()))
        });
        let (repo, focus, refs) = (repo.clone(), focus.clone(), refs.clone());
        dialog
            .w(crate::zoom::scaled(620., cx))
            .close_button(false)
            .child(
                Command::new(&state)
                    .placeholder(t!("palette.placeholder").into_owned())
                    .bordered(false)
                    .group(
                        CommandGroup::new()
                            .label(t!("palette.commands").into_owned())
                            .items(command_items),
                    )
                    .group(
                        CommandGroup::new()
                            .label(t!("palette.refs").into_owned())
                            .items(ref_items),
                    )
                    .empty(move |_, _, _| {
                        div()
                            .p_6()
                            .text_center()
                            .text_color(p.text_secondary)
                            .child(t!("palette.empty").into_owned())
                    })
                    .on_cancel(|window, cx| window.close_dialog(cx))
                    .on_confirm(move |path, window, cx| {
                        window.close_dialog(cx);
                        // A group is a section. Commands come first.
                        if path.section == 0 {
                            if let Some((_, _, _, action)) = commands().into_iter().nth(path.row) {
                                focus.focus(window, cx);
                                window.dispatch_action(action, cx);
                            }
                        } else if let Some((_, _, oid)) = refs.get(path.row) {
                            repo.update(cx, |repo, cx| repo.select_oid(oid, cx));
                        }
                    }),
            )
    });
}
