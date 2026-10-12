# Technical requirements

Status: signed off 2026-10-12

The engineering blueprint. Tech context: Rust 1.97.1, GPUI through `gpui-kit` 0.7.3, system Git 2.39+ as the only Git backend, JSON files for app data, macOS 13+.

## Architecture

```
+--------------------------------------------------------------------+
|  gittree (app crate, GPUI)                                         |
|                                                                    |
|  Views: Welcome, Workspace (Sidebar, Graph, Detail, Staging,       |
|         Diff, Conflict), Dialogs, Palette, Settings                |
|      |  observe                         ^  cx.notify()             |
|      v                                  |                          |
|  AppState (Entity)  ---- owns ---->  RepoState (Entity, one per    |
|   settings, session,                  open repository): commits,   |
|   recent, trust                       refs, status, selection,     |
|                                       in-progress op, undo stack   |
|      |                                  |                          |
|      |   foreground executor (UI thread, never blocks)             |
+------|----------------------------------|--------------------------+
       |   background executor            |
       v                                  v
+--------------------------------------------------------------------+
|  gittree-core (library crate, no UI)                               |
|                                                                    |
|  GitRunner  -> spawns `git` with a fixed environment, captures     |
|                stdout/stderr, records to the activity log          |
|  parse::*   -> log, refs, status v2, diff, conflict markers        |
|  graph      -> lane layout, incremental                            |
|  patch      -> partial patches for hunk and line staging           |
|  ops        -> one function per user action, built on GitRunner    |
|  journal    -> undo entries and snapshots                          |
|  store      -> JSON settings, session, recent, trust               |
+--------------------------------------------------------------------+
       |                         |                       |
       v                         v                       v
   git executable         .git/gittree/           ~/Library/Application
   (child process)        journal + backups       Support/GitTree/*.json
```

The keystone is `RepoState`. It is the single source of truth for one open repository, held in a GPUI entity. Views never run Git and never hold their own copy of repository data. They read `RepoState` and call its methods. Each method starts work on the background executor through `gittree-core`, and when the result comes back on the foreground it updates `RepoState` and notifies observers. Every change to a repository goes through one function, `RepoState::run_op`, which queues it, records the journal entry, runs it, and refreshes. Undo, progress, error reporting, the activity log and the trust check all hang off that one path.

## Data models

Conventions:

- Object IDs are lowercase hex strings exactly as Git prints them (40 characters for SHA-1 repositories, 64 for SHA-256). Type alias `Oid = Arc<str>`. Short form for display is the first 7 characters.
- Times are Unix seconds, UTC, as `i64`. Converted to local time only for display.
- Paths inside a repository are stored as raw bytes from Git (`Vec<u8>` wrapped in `RepoPath`), because Git paths need not be UTF-8. Display uses lossy conversion.
- Journal and snapshot IDs are UUID v4 strings.
- No soft delete and no tenancy. In-memory types are plain structs. Persisted types derive `Serialize` and `Deserialize` and carry a `version: u32` field at the top level of each file.

### Repository

| Field | Type | Notes |
|-------|------|-------|
| path | PathBuf | working tree root, canonical |
| git_dir | PathBuf | from `git rev-parse --git-dir` |
| common_dir | PathBuf | from `git rev-parse --git-common-dir`; journal lives under it |
| trusted | bool | from the trust list, see FR-SAFE-04 |
| head | Head | `Branch(name)`, `Detached(oid)` or `Unborn(name)` |
| object_format | enum | `Sha1` or `Sha256` |

### Commit

| Field | Type | Notes |
|-------|------|-------|
| oid | Oid | primary key |
| parents | SmallVec<[Oid; 2]> | first parent first |
| author_name | Arc<str> | interned, many commits share one author |
| author_email | Arc<str> | interned |
| author_time | i64 | |
| summary | String | first line only; the body is loaded on selection |

Held in a `Vec<Commit>` in load order, with a `HashMap<Oid, u32>` index from ID to row.

### GraphRow

One per commit, same index as the commit list.

| Field | Type | Notes |
|-------|------|-------|
| lane | u16 | column of this commit's dot |
| color | u8 | index into the theme's 8 lane colours |
| edges | SmallVec<[Edge; 4]> | segments crossing or ending in this row |

`Edge { from_lane: u16, to_lane: u16, color: u8, kind: Through | ToParent | FromChild }`.

### Ref

| Field | Type | Notes |
|-------|------|-------|
| name | String | full name, for example `refs/heads/main` |
| kind | enum | `Local`, `Remote`, `Tag`, `Stash`, `Head` |
| target | Oid | peeled to the commit for annotated tags |
| upstream | Option<String> | local branches only |
| ahead | u32 | against upstream, 0 if none |
| behind | u32 | against upstream |
| is_current | bool | |

Index: `HashMap<Oid, Vec<RefId>>` for labels on graph rows.

### StatusEntry

| Field | Type | Notes |
|-------|------|-------|
| path | RepoPath | |
| orig_path | Option<RepoPath> | for renames and copies |
| index | Change | `Unmodified`, `Added`, `Modified`, `Deleted`, `Renamed`, `Copied`, `TypeChange` |
| worktree | Change | same enum, plus `Untracked` |
| conflict | Option<ConflictKind> | `BothModified`, `BothAdded`, `DeletedByUs`, `DeletedByThem`, `AddedByUs`, `AddedByThem`, `BothDeleted` |

A file with both an index and a worktree change appears in both the staged and unstaged lists.

### FileDiff

| Field | Type | Notes |
|-------|------|-------|
| old_path | Option<RepoPath> | none for added files |
| new_path | Option<RepoPath> | none for deleted files |
| kind | enum | `Text`, `Binary { old_size, new_size }`, `ModeOnly`, `TooLarge { lines }` |
| header | Vec<u8> | the raw `diff --git` header lines, kept to rebuild patches |
| hunks | Vec<Hunk> | |
| no_newline_old | bool | "No newline at end of file" on the old side |
| no_newline_new | bool | same for the new side |

`Hunk { old_start, old_len, new_start, new_len, section: String, lines: Vec<DiffLine> }`.

`DiffLine { kind: Context | Add | Remove, old_no: Option<u32>, new_no: Option<u32>, text: Vec<u8> }`. Line endings are kept in `text` so CRLF files survive a rebuild.

### Stash

| Field | Type | Notes |
|-------|------|-------|
| index | u32 | position, `stash@{index}` |
| oid | Oid | the stash commit |
| message | String | |
| time | i64 | |
| base | Oid | the commit it was made on, used to place it in the graph |

### Remote

| Field | Type | Notes |
|-------|------|-------|
| name | String | |
| fetch_url | String | |
| push_url | String | equals `fetch_url` unless set separately |

### InProgressOp

| Field | Type | Notes |
|-------|------|-------|
| kind | enum | `Merge`, `Rebase`, `CherryPick`, `Revert` |
| step | Option<(u32, u32)> | current and total, rebase only |
| onto | Option<String> | branch or short ID, for the banner text |
| conflicted | u32 | count of unresolved files |

Detected from files in the Git directory: `MERGE_HEAD`, `rebase-merge/` or `rebase-apply/`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`.

### Conflict

| Field | Type | Notes |
|-------|------|-------|
| path | RepoPath | |
| kind | ConflictKind | from StatusEntry |
| regions | Vec<Region> | empty for binary or delete conflicts |

`Region { ours: Range<usize>, base: Option<Range<usize>>, theirs: Range<usize>, choice: Unresolved | Ours | Theirs | Both | Edited(String) }`, with ranges as byte offsets into the working file.

### JournalEntry

One JSON object per line in `<common_dir>/gittree/journal.jsonl`.

| Field | Type | Notes |
|-------|------|-------|
| id | String | UUID v4 |
| time | i64 | |
| action | Action | tagged enum with the inputs needed for redo, for example `Commit { message, amend }`, `Reset { target, mode }` |
| label | String | text for the undo tooltip, for example "Hard reset main to 3f2a9c1" |
| head_before | HeadState | symbolic ref (if any) and commit ID |
| head_after | HeadState | |
| refs_before | Vec<(String, Option<Oid>)> | only refs the action touched; `None` means the ref did not exist |
| refs_after | Vec<(String, Option<Oid>)> | |
| snapshot | Option<Snapshot> | present when the action could destroy work |
| undone | bool | set by undo, cleared by redo |

### Snapshot

| Field | Type | Notes |
|-------|------|-------|
| tree_ref | Option<String> | `refs/gittree/snapshots/<id>`, pointing at a `git stash create` commit (index and tracked changes) |
| patch_file | Option<String> | `<common_dir>/gittree/backups/<id>/discard.patch`, for hunk and line discards |
| untracked_dir | Option<String> | `<common_dir>/gittree/backups/<id>/untracked/`, copies of untracked files removed by the action |
| remote_ref | Option<(String, Oid)> | the remote branch and its old commit, for force-push |

Retention: on opening a repository, entries older than 30 days or beyond the newest 200 are removed, along with their refs and backup folders.

### Settings

`settings.json` in the app data folder.

| Field | Type | Default |
|-------|------|---------|
| version | u32 | 1 |
| theme | `System`, `Light` or `Dark` | `System` |
| ui_font_size | f32 | 13.0 |
| diff_font_family | String | system monospace |
| diff_font_size | f32 | 12.0 |
| diff_view | `Inline` or `Split` | `Inline` |
| pull_mode | `FfOnly`, `Merge` or `Rebase` | `FfOnly` |
| git_path | Option<PathBuf> | none, meaning "find `git` on PATH" |
| clone_dir | PathBuf | `~/Developer` if it exists, else the home folder |
| auto_fetch_minutes | u32 | 0 (off); used by FR-SYNC-05 |

### Session

`session.json`.

| Field | Type | Notes |
|-------|------|-------|
| version | u32 | 1 |
| window | Bounds | position, size, maximised flag |
| open_repo | Option<PathBuf> | reopened at launch if it still exists |
| sidebar_width | f32 | |
| detail_size | f32 | |
| drafts | Map<PathBuf, String> | unsent commit message per repository |

### RecentRepo

`recent.json`: a list of up to 20 `{ path: PathBuf, name: String, last_opened: i64 }`, newest first.

### TrustList

`trust.json`: `{ version: 1, paths: [PathBuf] }`, canonical working tree paths.

### ActivityRecord

In memory, last 500, also appended to the log file.

| Field | Type | Notes |
|-------|------|-------|
| time | i64 | |
| args | Vec<String> | the Git arguments; URLs are logged with any embedded password removed |
| duration_ms | u32 | |
| exit_code | i32 | |
| stderr | String | capped at 8 KB |

## API / command surface

There is no network API. The surface is the Rust API of `gittree-core`, called only by `RepoState`. Style: plain functions that take a `&GitRunner` and return `Result<T, GitError>`.

Error format:

```rust
pub struct GitError {
    pub kind: GitErrorKind,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stderr: String,
}
pub enum GitErrorKind {
    GitNotFound, GitTooOld, NotARepository, Untrusted,
    IndexLocked, Conflict, DirtyWorkTree, NonFastForward,
    AuthFailed, Network, RefExists, RefNotFound,
    PatchDoesNotApply, Cancelled, Other,
}
```

`kind` is derived from exit codes and porcelain output first, and from `stderr` text (with `LC_ALL=C`) only where Git offers nothing else. Each kind maps to one plain-language message in `locales/en.yml`.

`GitRunner` environment for every call: `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, `GIT_PAGER=cat`. Read calls add `GIT_OPTIONAL_LOCKS=0`. Network calls add `GIT_ASKPASS`, `SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE=force` and `GITTREE_ASKPASS_SOCK`. Untrusted repositories add `-c core.hooksPath=/dev/null -c core.fsmonitor=false -c core.sshCommand=ssh -c credential.helper=` and refuse every function marked "write" below with `Untrusted`.

Auth column is not applicable (single user). "Trusted" below means the function needs a trusted repository.

### Setup

- `find_git(custom: Option<&Path>) -> GitVersion`: locates Git and parses `git --version` (FR-REPO-07). Errors: `GitNotFound`, `GitTooOld`.
- `open(path) -> Repository`: `git rev-parse --show-toplevel --git-dir --git-common-dir --show-object-format` (FR-REPO-01). Errors: `NotARepository`.
- `init(path) -> Repository`: `git init` (FR-REPO-03). Write.
- `clone(url, dest, on_progress, cancel) -> Repository`: `git clone --progress` (FR-REPO-02). Long-running, emits progress parsed from stderr. Errors: `AuthFailed`, `Network`, `Cancelled`.

### Reading

- `stream_log(on_batch, cancel)`: `git log --exclude=refs/gittree/* --all --date-order -z --format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s`, delivering batches of 2,000 commits (FR-GRAPH-01, FR-GRAPH-02). Long-running.
- `list_refs() -> Vec<Ref>`: `git for-each-ref` with a format giving name, peeled target, upstream, ahead/behind counts (FR-GRAPH-03, FR-BRANCH-01).
- `head() -> Head`: `git symbolic-ref -q HEAD`, falling back to `git rev-parse HEAD`.
- `status() -> Vec<StatusEntry>`: `git status --porcelain=v2 -z --untracked-files=all` (FR-STAGE-01, FR-GRAPH-04).
- `commit_details(oid) -> CommitDetails`: `git show -s --format=...` for the full message plus committer and dates (FR-GRAPH-07).
- `commit_files(oid) -> Vec<FileChange>`: `git diff-tree -r -z --name-status -M --root`, against the first parent for merges (FR-GRAPH-07).
- `diff_commit_file(oid, path) -> FileDiff`: `git show --format= --patch --no-color --no-ext-diff --no-textconv` (FR-DIFF-01).
- `diff_worktree(path, staged: bool) -> FileDiff`: `git diff [--cached]` with the same flags. Untracked files are shown as all-added by reading the file (FR-DIFF-01, FR-DIFF-09).
- `search(query) -> Vec<Oid>`: matching on message, author or hash runs in memory over loaded commits. A `file:` term runs `git log --format=%H -- <path>` (FR-GRAPH-05).
- `list_stashes() -> Vec<Stash>`: `git stash list -z --format=...` (FR-STASH-01).
- `list_remotes() -> Vec<Remote>`: `git remote -v` (FR-SYNC-04).
- `in_progress() -> Option<InProgressOp>`: reads marker files in the Git directory (FR-MERGE-07).
- `conflicts() -> Vec<Conflict>`: conflicted paths from status, regions parsed from the working files (FR-MERGE-05).

### Staging and committing (all write, trusted)

- `stage(paths)` and `unstage(paths)`: `git add -A -- <paths>` and `git restore --staged -- <paths>` (or `git rm --cached` in a repository with no commits) (FR-STAGE-01).
- `apply_selection(diff, selection, target)`: builds a partial patch from a hunk or a set of lines and runs `git apply --cached` (stage), `git apply --cached --reverse` (unstage) or `git apply --reverse` (discard), each after a `--check` pass (FR-STAGE-02, FR-STAGE-03, FR-STAGE-04). Errors: `PatchDoesNotApply`.
- `discard_files(paths)`: `git restore --source=HEAD --staged --worktree` for tracked files, delete for untracked ones, after the snapshot is written (FR-STAGE-04).
- `commit(message, amend: bool) -> Oid`: `git commit -F -` with the message on stdin, plus `--amend` when set (FR-STAGE-05, FR-STAGE-06).

### Branches, tags, stash (all write, trusted)

- `create_branch(name, at, checkout: bool)`, `rename_branch(old, new)`, `delete_branch(name, force: bool)`: `git branch` and `git switch -c` (FR-BRANCH-02). Errors: `RefExists`, `RefNotFound`.
- `unmerged_commits(branch) -> Vec<Oid>`: read helper for the delete confirmation (FR-BRANCH-02).
- `checkout(target)`: `git switch <branch>`, `git switch --track <remote>/<branch>` or `git switch --detach <oid>` (FR-BRANCH-03). Errors: `DirtyWorkTree`.
- `create_tag(name, at, message: Option)`, `delete_tag(name)`, `push_tag(remote, name)`, `delete_remote_tag(remote, name)` (FR-BRANCH-04).
- `stash_push(message, include_untracked)`, `stash_apply(index)`, `stash_pop(index)`, `stash_drop(index)` (FR-STASH-01). Errors: `Conflict`.

### Merge, rebase, reset (all write, trusted)

- `merge(branch, no_ff: bool)`: `git merge` (FR-MERGE-01). Errors: `Conflict` (expected: the app moves to the conflict view).
- `rebase(onto)`, `rebase_continue()`, `rebase_skip()`: `git rebase` (FR-MERGE-02).
- `cherry_pick(oid, mainline: Option<u8>)`, `revert(oid, mainline: Option<u8>)` (FR-MERGE-03).
- `reset(target, mode)`: `git reset --soft`, `--mixed` or `--hard` (FR-MERGE-04).
- `continue_op()` and `abort_op()`: the `--continue` or `--abort` form for whichever operation is in progress (FR-MERGE-07).
- `resolve_file(path, text)`: writes the resolved content and runs `git add` (FR-MERGE-05).
- `resolve_side(path, side)`: `git checkout --ours` or `--theirs`, then `git add` (FR-MERGE-05).

### Remotes (network, trusted)

- `fetch(on_progress, cancel)`: `git fetch --all --prune --progress` (FR-SYNC-01). Long-running.
- `pull(mode, on_progress, cancel)`: `git pull --ff-only`, `--no-rebase` or `--rebase` (FR-SYNC-01). Long-running. Errors: `NonFastForward`, `Conflict`.
- `push(remote, branch, set_upstream, force_with_lease, on_progress, cancel)`: `git push --progress`, plus `-u` or `--force-with-lease=<branch>:<expected-oid>` (FR-SYNC-01, FR-SYNC-02, FR-SYNC-06). Long-running. Errors: `NonFastForward`, `AuthFailed`.
- `add_remote(name, url)`, `rename_remote(old, new)`, `set_remote_url(name, url)`, `remove_remote(name)` (FR-SYNC-04).

### Journal

- `journal::begin(action, touches) -> PendingEntry`: records the "before" state and writes any snapshot (FR-UNDO-02).
- `journal::finish(pending) -> JournalEntry`: records the "after" state and appends the line.
- `journal::can_undo(entry) -> Result<(), Reason>`: true only when HEAD and every ref in `refs_after` still match.
- `journal::undo(entry)` and `journal::redo(entry)` (FR-UNDO-01).
- `journal::snapshots() -> Vec<JournalEntry>` and `journal::restore(entry, as_branch: Option<String>)` (FR-UNDO-02).
- `journal::prune()`: retention.

How each action is undone:

| Action | Undo |
|--------|------|
| commit | `git reset --soft <head_before>` |
| amend | `git reset --soft <head_before>` (the old commit is still in the object store) |
| checkout | switch back to `head_before` |
| branch or tag create / rename / delete | `git update-ref` or `git branch -m` back to `refs_before` |
| merge, rebase, cherry-pick, revert | `git reset --hard <head_before>` (the tree is clean straight after these) |
| reset | `git reset --hard <head_before>`, then `git stash apply --index <tree_ref>` if a snapshot exists |
| discard | `git apply <patch_file>` and copy back `untracked_dir`, or apply `tree_ref` for whole-file discards |
| stash push | `git stash pop` |
| stash drop | `git stash store <oid>` |

## Permissions

Single user. The only gate is repository trust: read functions work everywhere with the safe overrides, while write and network functions need `trusted == true`. Repositories created by `init` or `clone` are added to the trust list automatically. Enforcement is in `GitRunner`, not in views, so no caller can skip it.

## Screens and key components

### Git check

- Purpose: block the app until a usable Git is found (FR-REPO-07).
- Components: message with found and required versions, path field, Retry button.
- States: checking, failed. On success it is never shown.

### Welcome

- Purpose: entry point when no repository is open (FR-REPO-01 to FR-REPO-04).
- Components: three actions (Open, Clone, Init) and the recent list showing name, path, last-opened time. The window accepts a dropped folder.
- States: empty (no recents: one line of guidance), list, missing-folder rows greyed.

### Clone dialog

- Purpose: FR-REPO-02.
- Components: URL field, destination folder field with a Browse button, progress bar with Git's phase text, Cancel.
- States: input, cloning, error with Details.

### Workspace

The main window for an open repository. Built with GPUI Kit's resizable panels.

- Toolbar: repository name, current branch, Undo, Redo, Fetch, Pull, Push (with ahead/behind badges), Branch, Stash, a progress area, the search field (FR-UNDO-01, FR-SYNC-01, FR-STATE-02, FR-GRAPH-05).
- Banner strip (below the toolbar, shown when relevant): untrusted repository with a Trust button (FR-SAFE-04), operation in progress with Continue and Abort (FR-MERGE-07), detached HEAD with "Create branch here" (FR-BRANCH-03).
- Sidebar: filter box, then collapsible groups for Local, Remotes, Tags, Stashes. Context menus per item (FR-BRANCH-01 to FR-BRANCH-04, FR-STASH-01).
- Graph: the virtual list of rows with the custom lane element, ref labels, columns. Work-in-progress row on top. Context menu on rows (FR-GRAPH-01 to FR-GRAPH-08).
- Detail panel: for a commit, the header fields, file list and diff (FR-GRAPH-07). For the work-in-progress row, the staging view.
- States: loading (skeleton rows until the first batch), empty repository, loaded, error.

### Staging view

- Purpose: FR-STAGE-01 to FR-STAGE-06.
- Components: Unstaged list and Staged list (status icon, path, stage or unstage button per row, "Stage all" and "Unstage all"), the diff of the selected file with per-hunk buttons and line selection, the commit box (summary with counter, description, Amend toggle, Commit button).
- States: no changes (empty state), changes, committing, conflicted files present (they sort first and open the conflict view).

### Diff view

- Purpose: FR-DIFF-01 to FR-DIFF-03, FR-DIFF-09.
- Components: file header (path, status, inline/split toggle), a virtual list of diff lines with two gutters, hunk headers with Stage / Unstage / Discard when the diff is from the working tree. Lines are selectable by click, shift-click and drag in the gutter.
- States: loading, text, binary, too large with "Load all", empty (file unchanged).

### Conflict view

- Purpose: FR-MERGE-05.
- Components: conflicted file list with resolved/unresolved marks. For the selected file: each region showing the current side and the incoming side with buttons (Take current, Take incoming, Take both), an editable result pane, "Mark resolved". Whole-file buttons for binary and delete conflicts.
- States: unresolved, partly resolved, all resolved (prompts Continue).

### Dialogs

Create branch, rename branch, delete branch, create tag, merge, rebase, reset (three modes with explanations), stash, checkout with local changes, force-push, discard, remove remote, add or edit remote, credential prompt (FR-SYNC-03), error details (FR-STATE-03). Every destructive one has Cancel as the default button (FR-SAFE-01).

### Command palette

- Purpose: FR-CMD-01.
- Components: input, fuzzy-ranked list of commands with shortcuts, then matching branches and tags.
- States: empty query (recent commands), results, no results.

### Settings window

- Purpose: FR-SET-01, FR-SET-02.
- Components: one page of labelled controls for each field of Settings, each with reset.

### Snapshots list

- Purpose: FR-UNDO-02.
- Components: a list of journal entries that have a snapshot (label, time), with Restore and "Restore to new branch".
- States: empty, list.

### Default keyboard shortcuts

| Action | Shortcut |
|--------|----------|
| Command palette | Cmd+Shift+P |
| Open repository | Cmd+O |
| Clone | Cmd+Shift+O |
| Settings | Cmd+, |
| Undo / redo | Cmd+Z / Cmd+Shift+Z |
| Search commits | Cmd+F |
| Refresh | Cmd+R |
| Fetch | Cmd+Shift+F |
| Pull | Cmd+Shift+L |
| Push | Cmd+Shift+K |
| New branch | Cmd+B |
| Stash | Cmd+Shift+S |
| Focus graph / changes | Cmd+1 / Cmd+2 |
| Stage or unstage selection | Space |
| Commit | Cmd+Enter |
| Toggle inline / split diff | Cmd+Shift+D |
| Move selection | Up / Down, Page Up / Page Down, Home / End |
| Close dialog or palette | Escape |

Shortcuts are GPUI actions bound in one keymap module, so FR-CMD-03 can later load overrides from `keymap.json`.

## State management

- Source of truth: `AppState` (settings, session, recents, trust, Git version) and one `RepoState` per open repository. Both are GPUI entities. Views hold handles and observe.
- Refresh: `RepoState::refresh(scope)` with scopes `Status`, `Refs`, `Log`, `All`. Triggers: after every `run_op`, a debounced file-watcher signal (300 ms), window focus, manual Cmd+R. A watcher event under the Git directory refreshes refs and log; one in the working tree refreshes status only.
- Stale results: each refresh carries a generation counter per scope. A result older than the current generation is dropped.
- Write queue: `run_op` runs one changing command at a time per repository. While one runs, the watcher is muted for that repository and a single refresh follows.
- Graph data: commits and rows are append-only during a load. A reload (refs changed) builds a new list in the background and swaps it in whole, keeping the selection by commit ID and the scroll position by the top visible commit.
- Caching: the diff for the selected file only. The Tree-sitter parse of the visible file only.
- Undo ownership: the journal in `gittree-core`. `RepoState` holds the in-memory undo and redo stacks, rebuilt from the journal file on open. Text fields have their own local text undo from GPUI Kit, and Cmd+Z goes to a focused text field first.
- Persistence: settings are saved on change. Session is saved on quit and every 30 seconds if changed. Commit message drafts are saved with the session.

## Background and long-running tasks

- Log loading: starts on open and on ref change. Reports the commit count. Cancelled when superseded. Failure shows the error state in the graph with Retry.
- Status refresh: triggered as above. No progress UI (it must finish in 300 ms). Failure with `IndexLocked` retries once after 200 ms, then stays silent until the next trigger.
- Clone, fetch, pull, push: user-triggered. Progress comes from Git's stderr percentage lines. Cancel sends SIGTERM to the process group, then SIGKILL after 2 seconds. No automatic retry. Failure shows a toast with Details.
- File watcher: runs for the life of the open repository. If the watcher fails to start, the app falls back to refreshing on window focus and says so once in a toast.
- Journal pruning: once per repository open, on the background executor. Failures are logged, never shown.
- Askpass listener: a socket accept loop that lives only while a network command runs.

## Security

- Command execution: Git is always started with an argument vector, never through a shell. User-supplied values (branch names, messages, URLs, paths) are separate arguments, and a `--` separator precedes any path list. Commit messages go on stdin.
- Input validation: branch and tag names are checked with `git check-ref-format` before use. Names and URLs starting with `-` are rejected, which closes option-injection through a hostile remote or branch name. Clone URLs must use `https://`, `ssh://`, the `user@host:path` form, or `file://` with a local path. Git's `ext::` and `fd::` transports are refused.
- Untrusted repositories: read-only, with hooks, fsmonitor, a custom SSH command and credential helpers all disabled, and with `--no-ext-diff --no-textconv` on every diff (FR-SAFE-04).
- Secrets: none stored. Text typed in the credential dialog is passed to the askpass helper and dropped. It is never logged, and the dialog field is a password field. URLs are logged with embedded credentials removed.
- Local endpoint: the askpass socket is created with mode 0600 inside a per-user temporary folder with mode 0700, exists only during a network command, and accepts only a one-time token passed to the helper through its environment.
- File access: the app writes only inside the repository (through Git), inside `<common_dir>/gittree/`, and inside its own data and log folders. Restoring an untracked-file backup refuses any path that would land outside the working tree.
- Network: none besides Git's own (FR-SAFE-03).
- OWASP items that apply to a local desktop app: injection (covered above), sensitive data exposure (secrets rule), and insecure deserialisation (JSON files are parsed with typed `serde` structs, and a file that fails to parse is renamed aside and replaced with defaults).

## Logging and observability

- Logged: app start and version, Git version, each Git command (arguments, duration, exit code, capped stderr), errors with context, watcher start and stop.
- Where: `~/Library/Logs/GitTree/gittree.log` through `tracing`, plus the in-memory activity list.
- Never logged: passwords, tokens, passphrases, credential-dialog input, embedded URL credentials, file contents, diff contents.
- No remote reporting of any kind.

## Testing strategy

- Unit tests in `gittree-core`, no Git needed: parsers (log, refs, status v2, diff, conflict markers) against fixture bytes, including paths with spaces, quotes, newlines or non-UTF-8 bytes; graph layout against small hand-drawn histories (linear, one merge, octopus merge, criss-cross, two roots); the patch builder.
- Integration tests in `gittree-core`, against real Git in temporary folders: every function in the command surface has at least one test that performs the action and compares with Git's own output. Required suites:
  - Line staging: a table of cases (single add, single remove, mixed hunk, first line, last line, file without trailing newline, CRLF file, renamed file, new file) each checked by comparing `git show :path` with the expected content.
  - Undo round trips: for each action in FR-UNDO-01, capture refs, the index, file hashes, then run the action, undo, and compare. Then redo and compare with the post-action capture.
  - Trust: a repository with a marker-writing hook and fsmonitor command, browsed untrusted, leaves no marker.
  - Safety: after the whole suite, the fake home folder's `.gitconfig` is unchanged. Tests run with `HOME` and `GIT_CONFIG_GLOBAL` pointed at a temporary folder.
- UI tests in `gittree` with GPUI Kit's `test-support`: open a fixture repository, select the work-in-progress row, stage a file, commit, then check the graph has a new first row. Keep these few: they are slow to write and tied to the kit's API.
- Performance: `cargo run --release -p gittree --bin bench -- <path>` loads the big repo and prints time to first batch, time to full load, peak memory, and frame times for a scripted scroll. Run by hand at M0 and M8, results recorded in `PROGRESS.md`.
- Test data: `scripts/make-big-repo.sh` builds the 100,000-commit repository with `git fast-import` in about a minute. Small fixtures are built in code by test helpers.
- Running: `cargo test --workspace`. CI runs the same.

## Environment and configuration

- `GITTREE_GIT`: path to the Git executable, overriding the setting and PATH. Default unset.
- `GITTREE_DATA_DIR`: overrides the app data folder. Default `~/Library/Application Support/GitTree`. Used by tests.
- `GITTREE_LOG`: log level (`error`, `warn`, `info`, `debug`). Default `info`.
- `GITTREE_ASKPASS_SOCK` and `GITTREE_ASKPASS_TOKEN`: set by the app for its own askpass helper. Never set by users.

No secrets exist, so none can be in the repo. A command-line path argument opens that repository: `gittree /path/to/repo`.

## Migrations and seed data

- Each JSON file has a `version` field. On load, an older version is upgraded in memory by a chain of small functions and written back. A newer version than the app knows is left untouched and defaults are used for the session, so an older build never destroys a newer build's file.
- The journal is line-based. Lines that fail to parse are skipped and logged.
- No seed data. There is nothing to import from other Git clients in v1.

---

### Open questions

None.

### Assumptions confirmed

Defaults taken under the blanket sign-off on 2026-10-12.

1. **IDs and times : RESOLVED.** Git's hex IDs as strings, Unix seconds in UTC, UUID v4 for journal entries.
2. **Delete model : RESOLVED.** Hard delete everywhere, with the 30-day snapshot as the safety net.
3. **API style : RESOLVED.** An in-process Rust API with one typed error. No IPC besides the askpass socket.
4. **State ownership : RESOLVED.** One `RepoState` entity per repository. Views hold no repository data.
5. **Long-running operations : RESOLVED.** Log loading, clone, fetch, pull, push. Everything else must finish fast enough to need no progress UI.
6. **Journal location : RESOLVED.** Inside the repository's Git directory under `gittree/`, with snapshot refs under `refs/gittree/snapshots/`.
