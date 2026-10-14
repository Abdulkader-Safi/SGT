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
        Commit,
        StageSelection,
        ShowSnapshots,
        ShowReflog,
        ShowActivity,
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

/// Every command the palette offers: its name, icon, shortcut and action.
pub fn commands() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    Box<dyn gpui_kit::Action>,
)> {
    vec![
        ("menu.open", "folder", "⌘O", Box::new(OpenRepository)),
        ("menu.clone", "clone", "⇧⌘O", Box::new(CloneRepository)),
        ("menu.init", "plus", "", Box::new(InitRepository)),
        (
            "menu.close_repository",
            "close",
            "",
            Box::new(CloseRepository),
        ),
        ("menu.settings", "settings", "⌘,", Box::new(OpenSettings)),
        ("menu.refresh", "fetch", "⌘R", Box::new(Refresh)),
        ("menu.fetch", "fetch", "⇧⌘F", Box::new(Fetch)),
        ("menu.pull", "pull", "⇧⌘L", Box::new(Pull)),
        ("menu.push", "push", "⇧⌘K", Box::new(Push)),
        ("menu.new_branch", "branch", "⌘B", Box::new(NewBranch)),
        ("menu.stash", "stash", "⇧⌘S", Box::new(Stash)),
        ("menu.commit", "commit", "⌘↵", Box::new(Commit)),
        ("menu.focus_graph", "commit", "⌘1", Box::new(FocusGraph)),
        ("menu.focus_changes", "file", "⌘2", Box::new(FocusChanges)),
        ("menu.search", "search", "⌘F", Box::new(SearchCommits)),
        ("menu.toggle_diff", "split", "⇧⌘D", Box::new(ToggleDiffView)),
        ("snapshots.menu", "clock", "", Box::new(ShowSnapshots)),
        ("reflog.menu", "clock", "", Box::new(ShowReflog)),
        ("activity.menu", "info", "", Box::new(ShowActivity)),
        ("menu.trust", "shield", "", Box::new(TrustRepository)),
    ]
}

pub fn init(cx: &mut App) {
    // "secondary" is Cmd on macOS and Ctrl on Windows and Linux.
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-o", OpenRepository, None),
        KeyBinding::new("secondary-shift-o", CloneRepository, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
        KeyBinding::new("secondary-shift-p", CommandPalette, None),
        KeyBinding::new("secondary-f", SearchCommits, None),
        KeyBinding::new("secondary-1", FocusGraph, None),
        KeyBinding::new("secondary-2", FocusChanges, None),
        KeyBinding::new("secondary-shift-d", ToggleDiffView, None),
        KeyBinding::new("secondary-r", Refresh, None),
        KeyBinding::new("secondary-shift-f", Fetch, None),
        KeyBinding::new("secondary-shift-l", Pull, None),
        KeyBinding::new("secondary-shift-k", Push, None),
        KeyBinding::new("secondary-shift-s", Stash, None),
        KeyBinding::new("secondary-b", NewBranch, None),
        KeyBinding::new("secondary-m", Minimize, None),
        KeyBinding::new("secondary-enter", Commit, None),
        // Only while the changed files or their diff have the focus, so a
        // space typed into the commit message stays a space.
        KeyBinding::new("space", StageSelection, Some("Changes")),
    ]);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenSettings, cx| crate::views::settings::open(cx));
    cx.on_action(|_: &ToggleDiffView, cx| {
        use gittree_core::store::DiffView::{Inline, Split};
        app_state(cx).update(cx, |app, cx| {
            app.update_settings(
                |settings| {
                    settings.diff_view = if settings.diff_view == Split {
                        Inline
                    } else {
                        Split
                    }
                },
                cx,
            )
        })
    });
    cx.on_action(|_: &OpenRepository, cx| pick_folder(cx, Pick::Open));
    cx.on_action(|_: &InitRepository, cx| pick_folder(cx, Pick::Init));
    cx.on_action(|_: &CloseRepository, cx| app_state(cx).update(cx, |app, cx| app.close_repo(cx)));
    cx.on_action(|_: &TrustRepository, cx| app_state(cx).update(cx, |app, cx| app.trust_repo(cx)));
    cx.on_action(|_: &CloseWindow, cx| in_active_window(cx, |window| window.remove_window()));
    cx.on_action(|_: &Minimize, cx| in_active_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| in_active_window(cx, |window| window.zoom_window()));
}

/// The bindings in `keymap.json`: an object from keystrokes to a command,
/// such as `{"secondary-shift-k": "Push"}`. Names are the actions in this
/// file, with or without the `gittree::` prefix. Returns the bindings and
/// a line for each entry that could not be read.
fn parse_keymap(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    if text.trim().is_empty() {
        return (Vec::new(), Vec::new());
    }
    let map: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(text) {
        Ok(map) => map,
        Err(error) => {
            return (
                Vec::new(),
                vec![format!("keymap.json is not a JSON object: {error}")],
            );
        }
    };
    let mut bindings = Vec::new();
    let mut problems = Vec::new();
    for (keys, value) in map {
        match value.as_str() {
            Some(name) if name.contains("::") => bindings.push((keys, name.to_owned())),
            Some(name) => bindings.push((keys, format!("gittree::{name}"))),
            None => problems.push(format!("{keys}: the command must be a name in quotes")),
        }
    }
    (bindings, problems)
}

/// Adds the user's own shortcuts from `keymap.json` (FR-CMD-03). They come
/// after the defaults, so a key bound in both runs the user's command.
// ponytail: a binding is added, not swapped: the old keys of a command
// still work, and the palette and menus show the default keys. Problems
// go to the log, not to a toast.
pub fn load_keymap(text: &str, cx: &mut App) {
    let (bindings, mut problems) = parse_keymap(text);
    let mut loaded = Vec::new();
    for (keys, name) in bindings {
        let built = cx
            .build_action(&name, None)
            .map_err(|error| error.to_string());
        let binding = built.and_then(|action| {
            KeyBinding::load(
                &keys,
                action,
                None,
                false,
                None,
                cx.keyboard_mapper().as_ref(),
            )
            .map_err(|error| error.to_string())
        });
        match binding {
            Ok(binding) => loaded.push(binding),
            Err(error) => problems.push(format!("{keys}: {error}")),
        }
    }
    for problem in problems {
        tracing::warn!(%problem, "keymap.json");
    }
    cx.bind_keys(loaded);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_entries_name_a_command_with_or_without_its_prefix() {
        let (bindings, problems) =
            parse_keymap(r#"{"secondary-k": "Push", "alt-p": "gittree::Pull", "ctrl-x": 3}"#);
        assert_eq!(
            bindings,
            [
                ("secondary-k".to_owned(), "gittree::Push".to_owned()),
                ("alt-p".to_owned(), "gittree::Pull".to_owned()),
            ]
        );
        assert_eq!(problems.len(), 1);
        assert_eq!(parse_keymap("  ").1.len(), 0);
        assert_eq!(parse_keymap("[1]").1.len(), 1);
    }
}
