//! The native menu bar. An item whose action has no handler yet belongs to a
//! later milestone.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, Menu, MenuItem, OsAction, SystemMenuType};
use rust_i18n::t;

use crate::actions::*;

pub fn init(cx: &mut App) {
    let item = |key: &str, action: Box<dyn gpui_kit::Action>| MenuItem::Action {
        name: t!(key).into_owned().into(),
        action,
        os_action: None,
        checked: false,
        disabled: false,
    };
    let os_item = |key: &str, action: Box<dyn gpui_kit::Action>, os_action| MenuItem::Action {
        name: t!(key).into_owned().into(),
        action,
        os_action: Some(os_action),
        checked: false,
        disabled: false,
    };
    let menu = |key: &str, items: Vec<MenuItem>| Menu::new(t!(key).into_owned()).items(items);
    let app_menu = |items: Vec<MenuItem>| Menu::new(crate::APP_NAME).items(items);
    let separator = MenuItem::separator;

    cx.set_menus([
        // The first menu is the app menu, whatever it is called.
        app_menu(vec![
            item("menu.settings", Box::new(OpenSettings)),
            separator(),
            MenuItem::os_submenu(t!("menu.services").into_owned(), SystemMenuType::Services),
            separator(),
            item("menu.hide", Box::new(Hide)),
            item("menu.hide_others", Box::new(HideOthers)),
            item("menu.show_all", Box::new(ShowAll)),
            separator(),
            item("menu.quit", Box::new(Quit)),
        ]),
        menu(
            "menu.file",
            vec![
                item("menu.open", Box::new(OpenRepository)),
                item("menu.clone", Box::new(CloneRepository)),
                item("menu.init", Box::new(InitRepository)),
                separator(),
                item("menu.close_repository", Box::new(CloseRepository)),
                item("menu.close_window", Box::new(CloseWindow)),
            ],
        ),
        menu(
            "menu.edit",
            vec![
                os_item("menu.undo", Box::new(Undo), OsAction::Undo),
                os_item("menu.redo", Box::new(Redo), OsAction::Redo),
                separator(),
                os_item("menu.cut", Box::new(Cut), OsAction::Cut),
                os_item("menu.copy", Box::new(Copy), OsAction::Copy),
                os_item("menu.paste", Box::new(Paste), OsAction::Paste),
                os_item("menu.select_all", Box::new(SelectAll), OsAction::SelectAll),
            ],
        ),
        menu(
            "menu.view",
            vec![
                item("menu.command_palette", Box::new(CommandPalette)),
                item("menu.search", Box::new(SearchCommits)),
                separator(),
                item("menu.focus_graph", Box::new(FocusGraph)),
                item("menu.focus_changes", Box::new(FocusChanges)),
                item("menu.toggle_diff", Box::new(ToggleDiffView)),
            ],
        ),
        menu(
            "menu.repository",
            vec![
                item("menu.refresh", Box::new(Refresh)),
                separator(),
                item("menu.commit", Box::new(Commit)),
                item("menu.stage_selection", Box::new(StageSelection)),
                separator(),
                item("menu.fetch", Box::new(Fetch)),
                item("menu.pull", Box::new(Pull)),
                item("menu.push", Box::new(Push)),
                separator(),
                item("menu.stash", Box::new(Stash)),
                item("snapshots.menu", Box::new(ShowSnapshots)),
                item("reflog.menu", Box::new(ShowReflog)),
                item("activity.menu", Box::new(ShowActivity)),
                separator(),
                item("menu.trust", Box::new(TrustRepository)),
            ],
        ),
        menu(
            "menu.branch",
            vec![item("menu.new_branch", Box::new(NewBranch))],
        ),
        menu(
            "menu.window",
            vec![
                item("menu.minimize", Box::new(Minimize)),
                item("menu.zoom", Box::new(Zoom)),
            ],
        ),
    ]);
}
