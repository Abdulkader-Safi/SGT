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
| M4: branches, tags, stash | Built, 93 tests pass. Waiting on the manual checks listed below |
| M5: merge, rebase, conflicts | Built, 103 tests pass. Waiting on the manual checks listed below |
| M6: remotes and sync | Built, 115 tests pass. Needs a check against a real server, see below |
| M7: undo, palette, keyboard | Built, 121 tests pass. The keyboard-only walkthrough needs doing by hand |
| M8: hardening and packaging | Built, 141 tests pass. The bench rerun and three checks by eye are still to do, see below |
| M9 and M10 | Not started |
| M11: MCP server | Step 1 built: the server and five tools. See below |
| M12: blame, file history, worktrees, agent sessions | Blame and file history built, 146 tests pass. Worktrees and agent sessions not started |

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

## Scope change: agents, AI and hosting (2026-10-13)

Safi asked for the features GitKraken has beyond a plain Git client, starting with MCP. They are in `03-requirements.md` as FR-AGENT-01 to 06 and in the build spec as M11 to M13. A built-in terminal was designed and then dropped: Safi does not want one in the app. The rule that shaped the list: nothing may need an account, a server or a stored secret. So AI runs through a command-line agent already on the Mac, and pull requests go through the `gh` command. Cloud patches, synced workspaces, team insights and single sign-on were left out because each needs a hosted service.

The design is section 8 of the Figma Design page. The Figma file also has a prototype now: five flows, with dialogs as overlays.

## M11: MCP server

### Step 1 (built)

`gittree mcp [folder]` serves MCP over stdio for one repository, with no window. It uses the same `GitRunner`, trust list and journal as the app.

- Tools: `git_status`, `git_log_or_diff`, `git_blame`, `git_branch` (list) and `git_add_or_commit`.
- A changing tool needs a trusted repository and "Read and change" under Settings, Coding agents. The default is "Read only".
- A commit made by an agent gets a journal entry, so the app's Undo takes it back.
- A revision that starts with a dash is refused, and a path always goes behind `--`.
- Output over 200,000 bytes is cut, with a note.

To connect Claude Code: `claude mcp add sgt -- /path/to/gittree mcp`, run in the repository folder.

Checked by tests: the protocol (initialize, notifications, unknown methods, one line in and one line out), every read tool in an untrusted repository, the two refusals for changes, an agent commit and its undo, and option injection through `revision` and `path`. Also tried by hand over a pipe against the built binary.

### Still to do

- `git_checkout`, `git_stash`, `git_worktree` and branch create and delete, once M4 has those operations.
- The MCP page in Settings from the design (per-tool switches, the copy-ready client command).
- The agent activity panel and the approval banner. Both need the server and a running app to talk to each other, which they do not yet: today the server is its own process.

## M4: branches, tags, stash

### What exists

- Core: create, rename and delete a branch, the list of commits only a branch has, checkout of a branch, a remote branch (with a tracking branch) or a commit, plain and annotated tags, and stash push, apply, pop, drop and list. Names and revisions that start with a dash are refused.
- Undo for checkout, branch create, rename and delete, tag create and delete, stash push and stash drop.
- App: the Branch and Stash toolbar buttons, Cmd+B and Cmd+Shift+S, "Create branch here" on the detached HEAD banner, right-click menus on sidebar branches, remote branches, tags and stashes and on graph rows, and the dialogs from the design (create and rename branch, delete branch with the named commits and a tick, create and delete tag, stash, drop stash, "Your changes are in the way" with Stash and switch).
- The sidebar lists every stash, not only the newest.

### Checked by tests

Each core operation with its undo, the unmerged-commit list, the refusal to overwrite local changes, bad names, and an untrusted repository. Through the app: the create-branch dialog with Enter, undo, the blocked checkout followed by Stash and switch, pop, an annotated tag, and deleting a merged branch through its confirmation.

### Still needs a check by hand

- The right-click menus themselves: they are built from the kit's menu and were not clicked in a test.
- Deleting an unmerged branch: the tick must enable the red button.
- Checking out a remote branch from the sidebar.

### Not in this milestone

- Pushing a tag and deleting a tag on the remote (FR-BRANCH-04) need the network work of M6.
- Cherry-pick, revert and reset in the commit menu come with M5.
- The MCP server does not have `git_checkout`, `git_stash` or branch create and delete yet, though the operations now exist.

## M5: merge, rebase, conflicts

### What exists

- Core: merge (with the option to force a merge commit), rebase with continue, skip and abort, cherry-pick and revert with a mainline parent for merge commits, reset in three modes, detection of an operation in progress from the Git folder (also one started in a terminal), the conflict marker parser, and resolving a file by content or by whole side.
- Git reports a conflict on stdout with exit code 1. The core turns that into the `Conflict` error kind by checking whether an operation was left in progress.
- Journal: an action that stops on a conflict is parked in `.git/gittree/in-progress.json` and written when the operation is continued to its end, so one Undo takes the whole merge back. Reset and rebase save the index and tracked changes first, in every mode, so Undo restores uncommitted work too.
- App: the banner for the operation in progress with Continue, Skip (rebase) and Abort, the conflict view (each region with Take current, Take incoming, Take both and Change, a result box that can be edited by hand, Mark resolved, whole-file buttons for binary and delete conflicts, and the "all resolved" state), and the dialogs for merge, rebase, reset and the mainline parent.
- Menus: "Merge into" and "Rebase onto" on sidebar branches, and Cherry-pick, Revert and "Reset to here" on graph rows.

### Checked by tests

Each core operation against real Git, with undo: a forced merge commit, a conflicting merge resolved by region and continued, abort, a rebase that stops and is skipped or continued, cherry-pick and revert including a merge commit, and all three reset modes with an uncommitted change restored on undo. Through the app: a conflicting merge opens the conflict view by itself, "Take both" and Mark resolved stage the file, Continue finishes the merge, and one Undo returns to the start.

### Still needs a check by hand

- The merge, rebase and reset dialogs and the mainline prompt, opened from the menus.
- The banner for a rebase started in a terminal.
- Editing the result box by hand, and a binary conflict.

### Differences from the design

- The conflict view shows one result box for the whole file, not a preview of only the conflicted lines.
- Region blocks show plain text, without line numbers or syntax colours.

## M6: remotes and sync

### The askpass spike (first task)

It works, with no fallback needed. Git and ssh are pointed at the app's own binary through `GIT_ASKPASS` and `SSH_ASKPASS`. Started that way the binary only passes the question over a Unix socket to the running app and prints the answer. The socket sits in a folder only the user can enter (mode 0700, socket 0600), exists only while a network command runs, and answers only a caller that knows a one-time token. A test runs real `git credential fill` through it and gets the user name and password back, and shows that Git fails instead of hanging once the listener is gone.

### What exists

- Core: a network runner that streams Git's progress lines and, on cancel, sends SIGTERM to the whole process group and SIGKILL after 2 seconds. Remote add, rename, edit and remove. Fetch with prune, pull in three modes, push that publishes a branch with no upstream, force-push with a lease on the commit last fetched, tag push and delete, and clone that removes a partial folder. URLs using the `ext::` or `fd::` transports are refused.
- App: the Fetch, Pull and Push toolbar buttons with ahead and behind counts, Cmd+Shift+F, L and K, progress with Git's phase text and a Cancel button, the credential dialog, the force-push confirmation, add, edit and remove remote from the Remotes group menu, push and delete of a tag on the remote, and Clone from the Welcome screen with progress, cancel and an error to retry from.
- A clone is trusted when it finishes, because the app made it.
- A pull that stops on conflicts opens the conflict view like a merge does.

### Checked by tests

Against a bare repository on disk: remotes, publish, fetch, pull, a push that is behind (refused as non-fast-forward), a lease that is stale (refused) and one that holds, pull refusing to fast-forward and then rebasing, tags, clone with a missing source, a non-empty destination, a forbidden URL and a cancel. Through the app: add remote, push, fetch showing "behind 1", pull, and clone opening the result trusted.

### Not checked: needs a real server

No test talks to a real host. Please try by hand:

- An HTTPS remote that asks for a password, and an SSH key with a passphrase. The dialog should appear, and Cancel should stop the command.
- Cancel on a slow clone or fetch: Git should stop within 2 seconds.
- A force-push to a real remote.

### Deviations

- The force-push dialog says how many commits will be replaced. It does not list them.
- Push does not ask which remote to use: it takes the upstream's remote, else `origin`, else the only one.
- Headless tests switch the credential listener off, as they do the file watcher, because both answer from their own threads and GPUI's test scheduler does not allow that.

## M7: undo, palette, keyboard

### What exists

- Redo. It brings undone actions back in the order they happened, and a new action ends the redo run. For a commit, merge, rebase, cherry-pick or revert, redo points the branch back at the commit the action made, so the ID is the same and conflicts are not met twice.
- The Undo and Redo toolbar buttons, with a tooltip that names the action. Undo is greyed out with the reason when the repository changed outside the app. Cmd+Z and Cmd+Shift+Z go to a focused text field first.
- The snapshot list (Repository menu, Snapshots): every entry that saved something, with Restore and "Restore to new branch". A force-push records the commit it replaced on the remote, which can come back as a branch.
- Pruning on open: entries older than 30 days or beyond the newest 200 go, with their refs and backup folders.
- The command palette (Cmd+Shift+P): every command with its shortcut, then branches and tags by name. Choosing a branch or tag selects its commit.

### Checked by tests

Undo then redo for commit, branch create (with checkout), checkout, rename, delete, tag create and delete, hard reset and discard, each comparing refs, HEAD, the index and the file on disk. The order of undo and redo, undo refused after an outside commit, restore in place and as a branch, the force-push snapshot, and pruning at 30 days. Through the app: the toolbar state after a reset, undo, redo, a snapshot restored as a branch, undo blocked after a terminal commit, and the palette finding "new branch" and running it.

### Still needs doing by hand

- The keyboard-only walkthrough the exit criteria ask for: open, stage, commit and push without the mouse.
- The tooltips on the Undo and Redo buttons, and the snapshot list on screen.

### Deviations

- The snapshot list is a dialog, not its own window as in the design.
- "Restore to new branch" names the branch `snapshot-<id>` itself. It does not ask for a name.
- The palette opens with all commands listed. It does not keep a list of recent commands.
- A force-push cannot be undone, since it changed the remote. Its old commit is in the snapshot list.

## Scope change: Windows and Linux (2026-10-13)

The owner asked for the app to run on Windows and Linux, checked in CI. They were planned for v1.1. Spec files 01 to 06, the README and `CLAUDE.md` now name all three systems. Installers for Windows and Linux stay out of v1.

### What changed

- CI runs fmt, clippy and the tests on `macos-latest`, `ubuntu-latest` and `windows-latest`. The core crate's tests run first, because they finish in seconds.
- The core crate compiles for Windows. Git paths go through `model::os_str`, which takes raw bytes on Unix and UTF-8 on Windows.
- Askpass on Windows listens on a TCP port on 127.0.0.1, because the standard library has no Unix sockets there. The one-time token is the only guard on that port. It now comes from `getrandom` on every system.
- Cancelling a network command on Windows runs `taskkill /T /F` on the process tree. There is no polite first step as there is with SIGTERM.
- Shortcuts are bound with `secondary-`, which is Cmd on macOS and Ctrl elsewhere. Hints such as "⇧⌘O" are shown as "Ctrl+Shift+O" off macOS.
- Windows and Linux keep the system title bar. The toolbar under the traffic lights is macOS only.

### Known gaps on Windows and Linux

- No menu bar. The commands are in the palette and on their shortcuts.
- A path that is not UTF-8 cannot be named on Windows.
- Restoring a discarded symbolic link on Windows needs developer mode, and a link to a folder comes back as a file link.
- An untracked file never shows as executable on Windows.
- The Git install hint on Linux is the `apt` command only.
- The bench does not report peak memory on Windows.
- Nobody has run the app in a window on either system. CI only builds it and runs the tests.

### CI result

All three systems pass fmt, clippy and every test as of commit `Port step 4`. Getting there found one real bug on macOS: the askpass helper dropped an answer when the app replied before the helper had closed its side of the socket.

## M8: hardening and packaging

### Done

- Hard-coded strings: a sweep of every multi-word literal in `crates/gittree/src` found only log messages, test data and the three install commands in `platform`. Nothing to move.
- Contrast: `palette::tests::text_meets_the_contrast_rule_in_both_themes` checks every text colour against every surface it sits on, in both themes, at 4.5:1. It failed on the design's values, so some tokens moved (listed below).
- Reduce motion: the kit reads the system setting when it starts, and the app has no animation of its own.
- Network: the app's dependency tree has no HTTP client. The `http` crate is there only for the types GPUI's unused client interface names. Our own code opens one socket, the askpass listener.
- Crash safety: `a_kill_in_the_middle_of_an_action_leaves_a_repository_git_can_use` stops between an action and its journal entry. The repository passes `git fsck`, the snapshot ref is still there, and the next action is journaled and undone as usual.
- Packaging: `cargo bundle --release --bin gittree` in `crates/gittree` makes `Safi Git Tree.app` (34 MB, limit 40) and a 10 MB disk image. The binary starts from the bundle.
- `LICENSE` (Apache-2.0), `NOTICE`, and `deny.toml`. `cargo deny check licenses` passes.
- The README covers the `refs/gittree/` refs, the `.git/gittree/` folder, the data folders on each system, and packaging.

### Decisions

- The bundle was 60 MB with the kit's 35 grammars. It now builds only the 19 that `language_for` names, with link-time optimisation and stripped symbols.
- The app icon is drawn for the project: `assets/app-icon/icon.svg`, from which `icon.icns` is made with `rsvg-convert` and `iconutil`.
- The bundle identifier is `io.github.abdulkader-safi.sgt`.
- The orphan snapshot a crash leaves behind is not listed in Snapshots and is not pruned. It can be read with Git.

### Colour tokens changed for contrast

- Tertiary text: dark `6B7380` to `8A92A0`, light `8A919E` to `5E6572`. In the light theme it is now close to secondary text.
- A new `accent_fill` (dark `2F6FE4`) sits under white text, on primary buttons and the current-branch label. `accent` stays as the colour for accent text. White on the old dark accent was 3.2:1.
- Light accent `2563EB` to `2160E6`, light warning `9A6700` to `946300`.
- Three light lane colours are darker (orange, teal, yellow), and avatar initials are dark on the dark theme's lanes.
- The Figma file still has the old values.

### Still needs a check by hand

- The bench on the big repository. It measures a visible window, and the run on 2026-10-13 hung because the window was not being drawn.
- The new colours by eye, in both themes.
- Launching `Safi Git Tree.app` from Finder. The app is not signed, so the first launch needs a right-click and "Open".
- A network monitor run while browsing, to confirm what the dependency check says.
- `tree-sitter-graphql` has no licence field in its manifest (its LICENSE file is MIT). `cargo deny` warns about it. It comes in through the kit even though the app does not use that grammar.

## M12: blame, file history, worktrees and agent sessions

### Blame and file history (built)

- `ops::file::blame` reads `git blame --porcelain` and `ops::file::history` reads `git log --follow` with the name and status of the file in each commit. Both have parser tests and one test against a real repository with a rename.
- In the commit details, "Blame" and "History" above the file list open the file view for the selected file, as the selected commit has it. The view has the design's Diff, Blame and History switch. Diff goes back to the commit details.
- Blame names each commit on the first line of its run, with a colour bar per commit. A click picks the commit, and the footer has "Show commit", which selects it in the graph.
- History lists the commits that changed the file, with "renamed from" where it moved. Picking one shows what it did to the file in a diff pane below.
- `shows_blame_and_history_of_a_file` covers the state through the app.

### Differences from the design

- Blame and history start from a commit's file list only. There is no way in from the staging view or the command palette yet.
- Blame code has no syntax colours.
- The file's history stops at 500 commits, with no "load more".
- "Load all" on a cut diff does nothing useful in the history's diff pane.
- The view has not been looked at on screen. The headless test renders it without error.

### Not started

- Worktrees: list, add, remove.
- Agent sessions in a new worktree on a new branch.
