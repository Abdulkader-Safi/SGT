//! Every command as a GPUI action, and the default keymap. Commands that a
//! later milestone builds are listed here already, so the menu bar and the
//! shortcuts have one home.

use gpui_kit::{App, AppContext, KeyBinding, PathPromptOptions, actions};

use crate::app_state::app_state;

actions!(
    gittree,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        OpenSettings,
        OpenRepository,
        CloneRepository,
        InitRepository,
        CloseRepository,
        CloseWindow,
        CommandPalette,
        SearchCommits,
        FocusGraph,
        FocusChanges,
        ToggleDiffView,
        Refresh,
        Fetch,
        Pull,
        Push,
        Stash,
        TrustRepository,
        NewBranch,
        Minimize,
        Zoom,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-o", OpenRepository, None),
        KeyBinding::new("cmd-shift-o", CloneRepository, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("cmd-f", SearchCommits, None),
        KeyBinding::new("cmd-1", FocusGraph, None),
        KeyBinding::new("cmd-2", FocusChanges, None),
        KeyBinding::new("cmd-shift-d", ToggleDiffView, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-shift-f", Fetch, None),
        KeyBinding::new("cmd-shift-l", Pull, None),
        KeyBinding::new("cmd-shift-k", Push, None),
        KeyBinding::new("cmd-shift-s", Stash, None),
        KeyBinding::new("cmd-b", NewBranch, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenSettings, cx| crate::views::settings::open(cx));
    cx.on_action(|_: &OpenRepository, cx| pick_folder(cx, Pick::Open));
    cx.on_action(|_: &InitRepository, cx| pick_folder(cx, Pick::Init));
    cx.on_action(|_: &CloseRepository, cx| app_state(cx).update(cx, |app, cx| app.close_repo(cx)));
    cx.on_action(|_: &TrustRepository, cx| app_state(cx).update(cx, |app, cx| app.trust_repo(cx)));
    cx.on_action(|_: &CloseWindow, cx| in_active_window(cx, |window| window.remove_window()));
    cx.on_action(|_: &Minimize, cx| in_active_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| in_active_window(cx, |window| window.zoom_window()));
}

fn in_active_window(cx: &mut App, act: impl FnOnce(&mut gpui_kit::Window)) {
    if let Some(window) = cx.active_window() {
        let _ = cx.update_window(window, |_, window, _| act(window));
    }
}

enum Pick {
    Open,
    Init,
}

/// Asks for a folder, then opens or initialises it.
fn pick_folder(cx: &mut App, pick: Pick) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });
    let app = app_state(cx);
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = chosen.await
            && let Some(path) = paths.into_iter().next()
        {
            app.update(cx, |app, cx| match pick {
                Pick::Open => app.open(path, cx),
                Pick::Init => app.init(path, cx),
            });
        }
    })
    .detach();
}
