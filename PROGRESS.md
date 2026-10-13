# Progress

Milestone status, decisions, and deviations from `docs/spec/06-build-spec.md`.

## Name

Renamed on 2026-10-13 from the working name "GitTree" to "Safi Git Tree" (SGT). The window title, menus, messages, the data folder (`~/Library/Application Support/Safi Git Tree`) and the log folder (`~/Library/Logs/Safi Git Tree`) use the new name. Internal identifiers keep the old spelling: the crates, the `gittree` binary, the `GITTREE_*` environment variables, `refs/gittree/` and `.git/gittree/`. The folder on disk is unchanged. The GitHub repository was renamed from `git-tree` to `SGT`.

## Status

| Milestone | Status |
|-----------|--------|
| M0: de-risk | Done 2026-10-12. All four targets met, no fallback needed |
| M1: app shell | Built, 45 tests pass. Waiting on the manual checks listed below |
| M2: graph and read-only browsing | Built, 57 tests pass. Waiting on the manual checks listed below |
| M3: staging and commit | Built, 81 tests pass. Waiting on the manual checks listed below |
| M4 to M10 | Not started |

## M0: de-risk

### Result

The stack loads and draws a 100,000-commit graph well inside the targets. `gpui-kit` 0.7.3 builds cleanly on Rust 1.97.1 with `tree-sitter-languages`, and none of the three fallbacks (`gix`, straight segments, an older kit release) was needed.

### Bench numbers

Command: `cargo run --release -p gittree --bin bench -- target/big-repo`, three runs, 2026-10-12.

Machine: MacBook Pro, Apple M2 Pro, 16 GB, macOS 26.6.2, Git 2.55.0, external display at 100 Hz. This is not the reference machine (M1, 16 GB), which is slower. Rerun the bench on an M1 before relying on the smaller margins.

Repository: `scripts/make-big-repo.sh` output. 100,000 commits, 5,000 merges, 201 branches, no commit-graph file.

| Measure | Target | Measured |
|---------|--------|----------|
| First rows drawn, from process start | under 1,000 ms | 463 to 591 ms |
| Full load, from process start | under 5,000 ms | 613 to 747 ms |
| Frame to frame, 95th percentile, steady scroll | under 16 ms | 10.9 to 11.4 ms |
| Frame to frame, 95th percentile, one window per frame | under 16 ms | 10.4 to 11.0 ms |
| App work per frame, 95th percentile, steady scroll | | 5.3 to 5.6 ms |
| App work per frame, 95th percentile, one window per frame | | 7.3 to 7.6 ms |
| Peak memory (largest resident size) | under 400 MB | 125 to 127 MB |

Notes on reading these:

- "Frame to frame" cannot go below the display's refresh interval, which was 10 ms here. "App work per frame" is the time from the start of render to the end of paint, and is the number to watch on a 60 Hz or 120 Hz display.
- All runs were with a warm disk cache. One cold run, taken before Git was started ahead of the UI, drew first rows at 1,163 ms. A cold run has not been measured since that change.
- Of the roughly 550 ms to first rows, Git itself needs about 335 ms before it prints its first record: `git log --date-order` walks the whole history first when the repository has no commit-graph file. With a commit-graph Git can stream at once, but the app does not write one, because that would change a repository the user has not trusted.
- GPUI slows an unfocused window to 30 frames per second and draws nothing for a covered window. The bench turns the first off for its own window and reports the second as "pauses left out".

Stress check, not part of the targets: a 20,000-commit repository with 30 branches open at once and a merge every second commit (30 lanes, a long curve on every other row). App work per frame was 4.9 ms at the 95th percentile for the steady scroll and 7.3 ms for one window per frame. Curve painting is not a bottleneck.

### GPUI Kit components

Checked in the published source of `gpui-component` 0.7.3, the crate that `gpui-kit` 0.7.3 resolves to and that docs.rs builds its pages from.

| Needed | In 0.7.3 | Where |
|--------|----------|-------|
| Tree view | Yes | `component::tree` (`tree`, `Tree`, `TreeState`) |
| Command palette | Yes | `component::command` (`Command`, `CommandState`, `CommandGroup`) |
| Settings form | Yes | `component::setting` (`Settings`, `SettingPage`, `SettingGroup`) |
| Title bar | Yes | `component::title_bar` (`TitleBar`) |
| Resizable panels | Yes | `component::resizable` (`h_resizable`, `v_resizable`), plus `component::dock` |
| Context menu | Yes | `component::menu` (`ContextMenu`, `PopupMenu`) |
| Toast | Yes | `component::notification` (`Notification`, `NotificationList`) |
| Dialog | Yes | `component::dialog` (`Dialog`, `AlertDialog`) |

Nothing has to be built from primitives. Each one still needs its 0.7.3 docs read before use, in the milestone that uses it.

### Decisions

- The graph column of a row is a GPUI `canvas` element with a paint function, not a hand-written `Element` type. It does the same job with less code. Vertical edges are painted as quads and only forks and merges are paths.
- A lane that ends in a row stays taken until that row's parents are placed, so an edge to a parent never leaves through a lane that an edge from a child just came in on.
- `git log` starts before `gpui_kit::init` and the window, so Git runs while the UI sets itself up. M2 should keep this when it moves loading into `RepoState`.
- The M0 `GitRunner` only reads and treats every repository as untrusted (hooks, fsmonitor, ssh command and credential helper overridden). The trust list and write calls are M1 work.
- Tests that run Git use a fake home folder in `target/tmp/home`, set once per test binary in `crates/gittree-core/tests/common/mod.rs`.

### Deviations from the spec

- `crates/gittree/src/lib.rs` exists, though section 4 does not list it. The bench binary and the UI tests need a library target to reach the views.
- The spike shows three text columns (summary, author, short hash) next to the graph, although M0 says "no other UI". Without text the frame times would say little about the real graph view.
- `libc` is a direct dependency of the app crate, used only by the bench to read peak memory. It was already in the dependency tree.
- The row text uses no `locales/en.yml` yet. The spike has no strings of its own: it shows commit data and Git's error text. `rust-i18n` is wired up in M1.

### Notes for later milestones

- `GraphView` holds the commits and rows itself. M2 moves them into `RepoState`.
- Lane colours are a fixed list in `graph_element.rs`. M1 moves them to the theme.
- `block` 0.1.6, a dependency of GPUI on macOS, triggers a future-incompatibility warning on Rust 1.97.1. It builds today. Watch it when the toolchain is next raised.
- At 100,000 rows the list is 2.4 million pixels tall, which `f32` scroll offsets still place to a quarter of a pixel. A repository ten times larger would need a check.

## M1: app shell

### What exists

- Core: `GitRunner` (fixed environment, activity records, cancellation, trust gate), all 15 `GitErrorKind`s, `find_git`, `open`, `init`, `head`, and `store.rs` for settings, session, recents and trust.
- App: `AppState`, the Git check screen, the welcome screen, open by picker, drop or command-line path, init, the workspace frame with three resizable panels, session restore, the menu bar, the settings window, light and dark themes, the error toast with a Details dialog, file logging, the CI workflow.

### Checked by tests

- Trust (FR-SAFE-04): planted hooks and an fsmonitor command do not run when browsing untrusted, every write is refused before Git starts, and a control shows plain Git does run them. Trust is remembered across launches.
- Open from a command-line path, the offer to create a repository in a plain folder, init giving a trusted empty repository (FR-REPO-01, FR-REPO-03).
- Recents and the open repository come back after a restart (FR-REPO-04, FR-STATE-04).
- A missing or old Git blocks the app and clears when the path is fixed (FR-REPO-07).
- Settings persist (FR-SET-01). One message per error kind (FR-STATE-03).

### Still needs a check by hand

No acceptance box is ticked yet, because these were not exercised in a real window:

- The folder picker, and dropping a folder on the window.
- The menu bar: six menus, shortcuts, and whether items with no handler yet show greyed out.
- The settings window: each control, reset, and changes applying at once.
- Light theme, and "System" following macOS within 1 second. Dark was seen working.
- The error toast and its Details dialog.
- Quitting with Cmd+Q writing the session. The 30-second autosave was seen working.

### Decisions

- The JSON files are read on a plain thread at launch, in parallel with UI setup, and written by one background task in order.
- Closing the last window quits the app.
- The workspace's middle panel shows the M0 graph through the repository's trust-checked runner. The sidebar and detail panels are empty until M2.
- A failed reopen of the last session's repository is silent. A failed open the user asked for is not.
- The saved window size is the content size. GPUI reports the frame with its title bar but opens by content size, so saving the frame grew the window by 32 pixels per launch.

### Deviations from the spec

- No Clone button on the welcome screen yet. Clone is M6, and a button that does nothing is worse than none.
- `Commit.parents` and `GraphRow.edges` are `Vec`, and `Commit.summary` is `Arc<str>`, where stage 5 says `SmallVec` and `String`. Memory is at 127 MB of a 400 MB budget, and `Arc<str>` lets rows share the text with the UI without copying.
- The default diff font is "Menlo", standing in for "system monospace".
- The app no longer starts `git log` before the UI, because it must find Git and open the repository first. M2 should measure this again.
- A bare repository does not open. Git's own error is shown.

### Filters in untrusted repositories: confirmed and fixed (2026-10-13)

A planted-filter test confirmed it: with only the four overrides the spec listed, `git status` and `git diff` in an untrusted repository ran a clean filter and a textconv program from the repository's own config. Safi approved the fix.

`GitRunner` now reads the repository's config before each untrusted call and overrides every setting there that names a program, leaving the user's global and system config alone. It also adds `--no-ext-diff --no-textconv` to diff-family commands itself, because Git tries to run an empty textconv command instead of skipping it. Stage 5 of the spec was edited and the matching text in 06 brought in line. The trust test now plants a filter and a textconv driver next to the hooks and fsmonitor command.

Known cost: one extra `git config` call per Git call in an untrusted repository. Cache it per runner if a profile ever shows it.

## M2: graph and read-only browsing

### What exists

- `RepoState`: the commit list and graph rows, refs, the selection with its details, files and diff, search, and a file watcher debounced by 300 ms. Stale results are dropped, and a reload builds the new history aside and swaps it in whole, keeping the selection by commit ID.
- The graph view with ref labels, columns for summary, author, date and hash, selection, and arrow, Page Up, Page Down, Home and End.
- The sidebar with Local, Remotes, Tags and Stashes groups, a filter and ahead/behind counts.
- The commit detail panel: header, changed files, and an inline diff in a virtual list, with binary and too-large handling.
- Search in the toolbar: text, `author:`, `file:`, a count, Enter and Shift+Enter.

### Bench with the production graph view

`cargo run --release -p gittree --bin bench -- target/big-repo`, 2026-10-13, same machine as M0, display at 120 Hz.

| Measure | Target | Measured |
|---------|--------|----------|
| First rows drawn | under 1,000 ms | 607 ms |
| Full load | under 5,000 ms | 766 ms |
| Frame to frame, 95th percentile | under 16 ms | 8.6 to 9.3 ms |
| App work per frame, 95th percentile | | 2.8 to 6.2 ms |
| Peak memory | under 400 MB | 124 to 141 MB |

The load times are from one run of three. In the other two the bench window was covered by other windows when it opened, GPUI drew nothing until it was uncovered, and the bench reported 46 and 67 seconds. That is the bench measuring a hidden window, not slow loading, but it means the load numbers rest on a single clean run. Rerun with the window in front before trusting them.

### Checked by tests

History and refs match Git, selecting a commit loads details, files and the first diff, the selection stops at the ends, text and `file:` search, and a commit made outside the app appears after the refresh the watcher asks for, with the selection kept.

### Still needs a check by hand

- The watcher itself: make a commit in a terminal and see it appear within 1 second. Tests switch the watcher off, because its thread breaks GPUI's test scheduler.
- Clicking through the sidebar, file list, parent links and "Load all".
- Keyboard movement in the graph, Cmd+F, Cmd+R.
- How the diff and ref labels look in the light theme.

### Decisions and deviations

- The diff parser leaves paths empty and the caller fills them from the file list, which avoids reading Git's quoted path forms.
- Search covers the summary, not the full message, because only summaries are loaded.
- Any file change refreshes refs, and the log is read again only if refs or HEAD moved. Status refresh arrives with the staging view in M3.
- No horizontal scrolling in the diff yet: long lines are clipped.
- Tabs in diff lines are shown as four spaces.

## M3: staging and commit

### First task: the kit's highlighter

GPUI Kit's `SyntaxHighlighter` takes any text, so it can colour the lines of a diff in a custom view. `tree-sitter-highlight` did not need wiring in. All 19 languages of FR-DIFF-03 load with the kit's `tree-sitter-languages` feature, and a test checks each one.

### What exists

- Core: the status parser and `status`, `diff_worktree` (an untracked file is read from disk and shown as all added), the partial patch builder, `stage`, `unstage`, `apply_selection`, `discard_files`, `commit`, and the journal with `begin`, `finish`, `can_undo` and `undo`.
- `RepoState::run_op`: one change at a time per repository, trust checked first, a journal entry for commits and discards, the file watcher muted while it runs, and one refresh of status and refs when the queue is empty.
- The work-in-progress row above the graph, with the changed-file count.
- The staging view: Unstaged and Staged lists with per-file, selected and "all" buttons, the diff of one file, and the commit box with the summary counter, a description, Amend and the message draft kept per repository.
- The diff pane, shared with commit details: inline or split, syntax colours, hunk buttons, and lines picked by click, Shift-click or drag.
- Discard for files, hunks and lines, behind a dialog where Enter and Escape both cancel.
- Undo of the last commit or discard, from Edit > Undo or Cmd+Z when no text field has the focus.

### Checked by tests

- The line-staging table (single add, single remove, mixed hunk, first line, last line, no trailing newline, CRLF, new file), each compared with `git show :path`, with the file on disk unchanged and a full unstage bringing the index back. A renamed file has its own test (FR-STAGE-03).
- One of two hunks staged leaves only that hunk in `git diff --cached` (FR-STAGE-02).
- Whole files stage and unstage like Git, also before the first commit (FR-STAGE-01).
- A discarded file, deleted file and untracked file come back byte for byte on undo, with the index as it was. Discarded lines come back too (FR-STAGE-04).
- Commit and amend land at HEAD with the exact message, including one that starts with `--`. Undo takes each back, and is refused once HEAD has moved (FR-STAGE-05, FR-STAGE-06).
- An untrusted repository refuses every change before Git starts, and nothing is deleted.
- An untracked file with quotes, a tab and a non-ASCII letter in its name stages through the patch header the app writes.
- Headless UI: the work-in-progress row appears and goes, a file is staged and committed from the commit box, Amend prefills and replaces HEAD, Commit does nothing without a summary or staged files, Enter in the discard dialog cancels, and undo restores the file (FR-GRAPH-04).
- Split rows pair removed lines with added ones. Rust keywords, strings and comments get three different colours in both themes. An unknown file type is plain text (FR-DIFF-02, FR-DIFF-03).

### Still needs a check by hand

- Picking lines with the mouse: click, Shift-click and drag, then "Stage lines", "Unstage lines" and "Discard lines".
- Space on a selected file and on picked lines, and that a space typed in the commit message stays a space.
- Cmd+Enter to commit, from a text field and from the lists. Cmd+2 to open the changes.
- The counter turning to the warning colour at 73 characters.
- The amend warning when the last commit is already on its upstream.
- The message draft surviving a restart.
- The split view surviving a restart, and both views in the light theme.
- The toast after undo, and after an undo that is refused.
- Saving a file in an editor and seeing it appear in the lists within 1 second.

### Decisions

- The work-in-progress row sits above the commit list and does not scroll with it. It shows while there is something to commit, and stays while it is selected, so Cmd+2 can open the staging view on a clean tree to amend only a message.
- After a commit that leaves nothing to commit, the selection moves to the new commit.
- Staging and unstaging get no journal entry. They are not in the spec's undo table, and they are undone by their opposite.
- A failed op clears the ops queued behind it, because those were chosen against a state that did not come about.
- A partial patch gets a plain "modify this file" header whenever an unselected line stays behind, or the file was renamed. Without that, unstaging one line of a new or renamed file would undo the whole creation or rename.
- A diff that was cut for its size offers no hunk or line actions until "Load all", because the cut ends mid-hunk.
- Syntax colours come from parsing the old and new side of the hunks, not the whole file. A colour can be off where a construct began before the hunk.

### Deviations from the spec

- Undo for commits and discards is built now, not in M7, because FR-STAGE-04 needs it. Redo, the Undo and Redo toolbar buttons, the reason shown when undo is refused, the snapshot list and pruning stay in M7.
- `Journal::begin` takes the action and a label. The refs an action touches come from the action, not from a second argument.
- A snapshot stores its patch file and untracked folder as paths under `.git/gittree/`, not full paths. The files are found by the entry's ID, which must be a UUID, so a hand-edited journal cannot point undo at another folder.
- Discarding a file in a repository with no commit yet brings it back untracked on undo, since there is nothing to stash against.
- Discarding a file removes its staged changes too. That is what `git restore --source=HEAD --staged --worktree` does, and the dialog says so.
- With an unselected removed line between them, selected added lines land after it. Example: the hunk removes "b" and "c" and adds "B", and only "b" and "B" are staged. The index gets "c" then "B". This is the spec's patch rule, and Git's own interactive staging does the same.
- Every watcher event reads the status and the refs. The spec splits them by where the change happened. Both reads are cheap, so the split waits for a profile that asks for it.
- The file lists build every row each frame. That is fine at the 500 changed files of NFR-04.
- One UI test sleeps 400 ms, because the kit's dialog takes no clicks during its quarter-second entrance and that animation runs on real time.

## Design (2026-10-13)

The design lives in Figma: `https://www.figma.com/design/NWCW3JuU78OiVV6dOT9pzQ/SGT`. It has a Cover, Foundations (colour variables in Dark and Light, text styles, spacing), Components, and a Design page with every screen and state.

### Restyled to the design

Everything built in M1 to M3 now follows it:

- `palette.rs` holds the colour tokens for both themes, and `theme.rs` copies the matching ones into the kit's theme so its buttons, inputs and dialogs agree.
- `icons.rs` embeds 32 icons. 21 are Reicon outline icons. Branch, commit, merge and a few others are drawn for the app.
- `views/ui.rs` has the shared pieces: ref labels, status marks, badges, key hints, captions, banners and empty states.
- The toolbar sits under the traffic lights as one strip. Git check and Welcome get a plain title strip.
- Welcome, Git check, sidebar, graph (28 px rows, column titles, ring for a merge commit, dimmed rows while searching), commit details, the diff pane (gutters, hunk rows, inline and split segments, selection bar), the staging lists (row actions on hover) and the commit box.

### Shown but not working yet

These are in the design and on screen, greyed out, until their milestone: Redo, Fetch, Pull, Push, Branch and Stash in the toolbar, and "Clone repository" on Welcome.

### Not built yet

The design also covers screens whose features are not built: Clone (M6), context menus and the branch, tag and stash dialogs (M4), merge, rebase, reset and the conflict view (M5), force-push, remotes and the credential prompt (M6), the command palette and the snapshot list (M7). Each gets built to the design with its milestone.

### Differences from the design

- The interface font is the system font, not Inter. The diff font is the one from Settings (Menlo by default), not JetBrains Mono.
- The Settings window still uses the kit's own form layout.
- Toasts and dialogs are the kit's, recoloured through the theme.
- Diff lines are 22 px high, not 20, so the default diff font is not clipped.
