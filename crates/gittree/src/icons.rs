//! The app's icons, embedded in the binary. Most are Reicon outline icons.
//! The Git ones (branch, commit, merge), the sidebar ones and a few others
//! are drawn for the app.

use gpui_kit::component::Icon;

/// An icon by name. It takes the text colour of where it is used. An
/// unknown name gives an empty icon.
pub fn icon(name: &str) -> Icon {
    let bytes: &[u8] = match name {
        "branch" => include_bytes!("../assets/icons/branch.svg"),
        "check" => include_bytes!("../assets/icons/check.svg"),
        "chevron-down" => include_bytes!("../assets/icons/chevron-down.svg"),
        "chevron-right" => include_bytes!("../assets/icons/chevron-right.svg"),
        "clock" => include_bytes!("../assets/icons/clock.svg"),
        "clone" => include_bytes!("../assets/icons/clone.svg"),
        "close" => include_bytes!("../assets/icons/close.svg"),
        "commit" => include_bytes!("../assets/icons/commit.svg"),
        "copy" => include_bytes!("../assets/icons/copy.svg"),
        "fetch" => include_bytes!("../assets/icons/fetch.svg"),
        "file" => include_bytes!("../assets/icons/file.svg"),
        "folder" => include_bytes!("../assets/icons/folder.svg"),
        "info" => include_bytes!("../assets/icons/info.svg"),
        "inline" => include_bytes!("../assets/icons/inline.svg"),
        "lock" => include_bytes!("../assets/icons/lock.svg"),
        "merge" => include_bytes!("../assets/icons/merge.svg"),
        "minus" => include_bytes!("../assets/icons/minus.svg"),
        "more" => include_bytes!("../assets/icons/more.svg"),
        "plus" => include_bytes!("../assets/icons/plus.svg"),
        "pull" => include_bytes!("../assets/icons/pull.svg"),
        "push" => include_bytes!("../assets/icons/push.svg"),
        "redo" => include_bytes!("../assets/icons/redo.svg"),
        "remote" => include_bytes!("../assets/icons/remote.svg"),
        "search" => include_bytes!("../assets/icons/search.svg"),
        "settings" => include_bytes!("../assets/icons/settings.svg"),
        "sidebar-left" => include_bytes!("../assets/icons/sidebar-left.svg"),
        "sidebar-right" => include_bytes!("../assets/icons/sidebar-right.svg"),
        "shield" => include_bytes!("../assets/icons/shield.svg"),
        "split" => include_bytes!("../assets/icons/split.svg"),
        "stash" => include_bytes!("../assets/icons/stash.svg"),
        "tag" => include_bytes!("../assets/icons/tag.svg"),
        "trash" => include_bytes!("../assets/icons/trash.svg"),
        "undo" => include_bytes!("../assets/icons/undo.svg"),
        "warning" => include_bytes!("../assets/icons/warning.svg"),
        _ => return Icon::default(),
    };
    Icon::default().data(bytes)
}
