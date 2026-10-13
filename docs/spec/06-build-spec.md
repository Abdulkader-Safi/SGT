# Build spec: Safi Git Tree

> **This is the hand-off document for Claude Code.** It is self-contained: build from this file alone. It pulls together the product, the users, the requirements, plus the tech decisions from stages 1 to 5. Where it says "see Stage N", that is optional background.

Status: signed off 2026-10-12. Sources: 01 to 05 as of that date.

## 1. What we're building

Safi Git Tree (SGT) is a native desktop Git client for macOS, written in Rust and rendered with GPUI. It is for developers who work across many repositories and want a visual commit graph, line-level staging, in-app conflict resolution and undo for every local action, without an Electron app, an account or a subscription. Its main difference: it is native and free, and every Git action is done by the Git already installed on the machine, so the app always agrees with the command line. The mission is a personal, source-available tool (Apache-2.0) with no backend, no telemetry and no network use beyond Git's own.

## 2. Stack (decided, see Stage 4)

| Layer | Choice |
|-------|--------|
| Language | Rust 1.97.1 stable, edition 2024, pinned in `rust-toolchain.toml` |
| UI framework | GPUI through `gpui-kit = "0.7.3"`, default features plus `tree-sitter-languages`; `test-support` in dev. Import GPUI only through `gpui_kit` re-exports. Never add the `gpui` crate directly |
| Git access | System `git` 2.39+ as a child process for every read and write. `gix` 0.89 only as the M0 fallback for reading the commit list |
| Commit graph | Streamed `git log`, incremental lane layout in the core crate, custom GPUI element inside a uniform virtual list |
| Diff and patch engine | Git computes diffs; the core crate parses unified output and builds partial patches for `git apply`. `similar` 2.7 for word-level highlights (post-MVP) |
| Syntax highlighting | Tree-sitter grammars from GPUI Kit. Fallback: `tree-sitter-highlight` at Tree-sitter 0.26 |
| Conflict resolution | Parse conflict markers from working files (`merge`, `diff3`, `zdiff3` styles) |
| Undo and snapshots | Append-only JSON-lines journal in `<git-common-dir>/gittree/`, snapshot refs under `refs/gittree/snapshots/` |
| Credentials | None stored. Own binary doubles as `GIT_ASKPASS` / `SSH_ASKPASS` helper over a Unix socket |
| File watching | `notify` 8.2, debounced 300 ms |
| Async | GPUI foreground and background executors, `smol` 2 primitives. No tokio |
| Persistence | JSON files with `serde` 1.0 and `serde_json` in `~/Library/Application Support/Safi Git Tree/`, found with `dirs`, written by temp-file-and-rename |
| Internationalisation | `rust-i18n` 4.2, `locales/en.yml` |
| Errors and logging | `thiserror` in core, `anyhow` in the app, `tracing` to `~/Library/Logs/Safi Git Tree/gittree.log` |
| Testing | `cargo test`, real Git in `tempfile` folders, a few headless UI tests, a bench binary |
| CI | GitHub Actions on `macos-latest`: fmt, clippy with warnings denied, test |
| Distribution | `cargo-bundle` 0.12 for `Safi Git Tree.app`. Signed universal DMG later |
| Backend, database, auth, hosting, payments, analytics, AI | None |

## 3. Architecture and principles

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
   (child process)        journal + backups       Support/Safi Git Tree/*.json
```

`RepoState` is the single source of truth for one open repository. Views never run Git and hold no repository data of their own. Every change to a repository goes through `RepoState::run_op`, which queues it, writes the journal entry, runs it through `gittree-core`, and refreshes.

**Non-negotiable principles:**

1. Git does the work. Every read and write of repository data is a `git` child process. No libgit2, and no in-process reimplementation of a Git command.
2. The UI thread never waits. No Git call, file read or JSON write on the foreground executor.
3. One write path. Every changing action goes through `run_op`: one at a time per repository, journaled, trust-checked, logged.
4. Snapshot before destruction. Hard reset, discard, rebase, force-push, branch delete and stash drop each save what they could destroy before they run.
5. Machine output only. Parse `--porcelain=v2`, `-z` and explicit `--format` output. Never parse text meant for people, except to classify an error when Git offers nothing else.
6. Fixed environment. Every Git call sets `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, `GIT_PAGER=cat`. Read calls add `GIT_OPTIONAL_LOCKS=0`.
7. No shell. Git is started with an argument vector. User values are separate arguments, with `--` before path lists. Names and URLs starting with `-` are rejected.
8. Untrusted means read-only. A repository the app did not create or clone runs no hooks or helper programs and accepts no changes until the user trusts it. Enforced in `GitRunner`.
9. Leave the user's setup alone. Never write global Git config, `~/.ssh` or a credential store. Never store or log a secret.
10. No network except Git's. No telemetry, update check or crash upload.
11. Strings come from `locales/en.yml`. Destructive dialogs default to Cancel and say in plain words what will be lost.

## 4. Repository structure

```
SGT/
  Cargo.toml                 workspace: members crates/*
  Cargo.lock                 committed
  rust-toolchain.toml        1.97.1
  LICENSE                    Apache-2.0
  NOTICE                     third-party attributions
  README.md
  PROGRESS.md                milestone status, decisions, deviations
  .github/workflows/ci.yml   fmt, clippy, test on macos-latest
  docs/spec/                 these six files
  scripts/
    make-big-repo.sh         100,000-commit test repository via git fast-import
  crates/
    gittree-core/            no UI dependency
      src/
        lib.rs
        runner.rs            GitRunner: env, spawn, cancel, activity log, trust gate
        error.rs             GitError, GitErrorKind
        model.rs             Oid, RepoPath, Commit, Ref, StatusEntry, FileDiff, ...
        parse/               log.rs, refs.rs, status.rs, diff.rs, conflict.rs
        graph.rs             lane layout
        patch.rs             partial patch builder
        ops/                 setup.rs, read.rs, stage.rs, branch.rs, merge.rs, remote.rs
        journal.rs           entries, snapshots, undo, redo, prune
        store.rs             settings, session, recent, trust (JSON)
      tests/                 integration tests against real Git
    gittree/                 the app
      Cargo.toml             bundle metadata for cargo-bundle
      locales/en.yml
      src/
        main.rs              app start, askpass helper mode, CLI path argument
        app_state.rs         AppState entity
        repo_state.rs        RepoState entity, run_op, refresh, watcher
        actions.rs           GPUI actions and the default keymap
        menus.rs             native menu bar
        platform/            macOS-specific code, askpass socket
        views/
          git_check.rs  welcome.rs  workspace.rs  toolbar.rs  banners.rs
          sidebar.rs  graph.rs  graph_element.rs  commit_detail.rs
          staging.rs  diff.rs  conflict.rs  palette.rs  settings.rs
          snapshots.rs  dialogs/
        bin/bench.rs         performance measurements
      tests/                 headless UI tests
```

## 5. Data model

Conventions: object IDs are Git's lowercase hex strings (`Oid = Arc<str>`). Times are Unix seconds in UTC as `i64`. Repository paths are raw bytes (`RepoPath`). Journal IDs are UUID v4. Persisted JSON files carry a top-level `version: u32`. Full field tables are in Stage 5.

In memory (read from Git, never persisted):

- Repository: `path`, `git_dir`, `common_dir`, `trusted`, `head` (`Branch`, `Detached` or `Unborn`), `object_format`.
- Commit: `oid`, `parents` (first parent first), `author_name`, `author_email` (both interned), `author_time`, `summary`. Stored in a `Vec` in load order with an ID-to-row map.
- GraphRow: `lane`, `color` (index into 8 lane colours), `edges` (each with `from_lane`, `to_lane`, `color`, `kind`).
- Ref: `name`, `kind` (`Local`, `Remote`, `Tag`, `Stash`, `Head`), `target` (peeled), `upstream`, `ahead`, `behind`, `is_current`.
- StatusEntry: `path`, `orig_path`, `index` change, `worktree` change, `conflict` kind.
- FileDiff: `old_path`, `new_path`, `kind` (`Text`, `Binary`, `ModeOnly`, `TooLarge`), raw `header`, `hunks`, the two no-trailing-newline flags. Hunk: starts, lengths, section text, lines. DiffLine: `kind`, `old_no`, `new_no`, `text` bytes with line ending kept.
- Stash: `index`, `oid`, `message`, `time`, `base`.
- Remote: `name`, `fetch_url`, `push_url`.
- InProgressOp: `kind` (`Merge`, `Rebase`, `CherryPick`, `Revert`), `step`, `onto`, `conflicted` count.
- Conflict: `path`, `kind`, `regions` (each with `ours`, optional `base`, `theirs` byte ranges and a `choice`).
- ActivityRecord: `time`, `args` (URL passwords removed), `duration_ms`, `exit_code`, `stderr` capped at 8 KB. Last 500 kept.

Persisted inside the repository, under `<common_dir>/gittree/`:

- JournalEntry (one JSON line in `journal.jsonl`): `id`, `time`, `action` (tagged enum holding redo inputs), `label`, `head_before`, `head_after`, `refs_before`, `refs_after`, optional `snapshot`, `undone`.
- Snapshot: `tree_ref` (`refs/gittree/snapshots/<id>`, a `git stash create` commit), `patch_file` (`backups/<id>/discard.patch`), `untracked_dir` (`backups/<id>/untracked/`), `remote_ref` (remote branch and old commit, for force-push). Retention: 30 days or 200 entries, pruned when the repository is opened.

Persisted in the app data folder:

- Settings (`settings.json`): `theme` (default `System`), `ui_font_size` (13), `diff_font_family`, `diff_font_size` (12), `diff_view` (`Inline`), `pull_mode` (`FfOnly`), `git_path`, `clone_dir`, `auto_fetch_minutes` (0).
- Session (`session.json`): `window` bounds, `open_repo`, `sidebar_width`, `detail_size`, `drafts` (commit message per repository).
- RecentRepo (`recent.json`): up to 20 of `path`, `name`, `last_opened`.
- TrustList (`trust.json`): canonical paths of trusted repositories.

## 6. API / command surface

An in-process Rust API in `gittree-core`. Functions take a `&GitRunner` and return `Result<T, GitError>`. `GitError { kind, args, exit_code, stderr }` with kinds: `GitNotFound`, `GitTooOld`, `NotARepository`, `Untrusted`, `IndexLocked`, `Conflict`, `DirtyWorkTree`, `NonFastForward`, `AuthFailed`, `Network`, `RefExists`, `RefNotFound`, `PatchDoesNotApply`, `Cancelled`, `Other`. Each kind has one plain-language message in `locales/en.yml`.

Untrusted repositories: `GitRunner` adds `-c core.hooksPath=/dev/null -c core.fsmonitor=false -c core.sshCommand=ssh -c credential.helper=` and returns `Untrusted` for every write or network function. It also reads the repository's own config before each call and overrides every setting there that names a program (`filter.*.clean`, `smudge` and `process`, `merge.*.driver`, `core.pager`, `pager.*`, `alias.*`, `gpg.program` and the like), because those names are chosen by the repository and no fixed list can cover them. Settings from the user's global and system config are left alone. `GitRunner` itself adds `--no-ext-diff --no-textconv` to `diff`, `show`, `log`, `diff-tree`, `diff-index`, `diff-files` and `blame` in an untrusted repository, since an empty diff driver cannot be set through config. Every diff call passes `--no-color --no-ext-diff --no-textconv`.

Setup:

- `find_git(custom) -> GitVersion`: locate Git, parse `git --version`, require 2.39.
- `open(path) -> Repository`: `git rev-parse --show-toplevel --git-dir --git-common-dir --show-object-format`.
- `init(path) -> Repository`: `git init`. Adds the path to the trust list.
- `clone(url, dest, on_progress, cancel) -> Repository`: `git clone --progress`. Long-running. Adds to the trust list.

Reading:

- `stream_log(on_batch, cancel)`: `git log --exclude=refs/gittree/* --all --date-order -z --format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s`, in batches of 2,000.
- `list_refs()`: `git for-each-ref` with name, peeled target, upstream, ahead/behind.
- `head()`: `git symbolic-ref -q HEAD`, else `git rev-parse HEAD`.
- `status()`: `git status --porcelain=v2 -z --untracked-files=all`.
- `commit_details(oid)`: `git show -s --format=...`.
- `commit_files(oid)`: `git diff-tree -r -z --name-status -M --root`, first parent for merges.
- `diff_commit_file(oid, path)`: `git show --format= --patch`.
- `diff_worktree(path, staged)`: `git diff [--cached]`. Untracked files are read and shown as all-added.
- `search(query)`: message, author or hash in memory; `file:` terms through `git log --format=%H -- <path>`.
- `list_stashes()`, `list_remotes()`, `in_progress()`, `conflicts()`.

Staging and committing (write):

- `stage(paths)`: `git add -A -- <paths>`. `unstage(paths)`: `git restore --staged -- <paths>`, or `git rm --cached` before the first commit.
- `apply_selection(diff, selection, target)`: build a partial patch, run `git apply --cached --check` then the real apply. Stage: `--cached`. Unstage: `--cached --reverse`. Discard: `--reverse`.
- `discard_files(paths)`: snapshot first, then `git restore --source=HEAD --staged --worktree`, and delete untracked files.
- `commit(message, amend)`: `git commit -F -` with the message on stdin.

Partial patch rule, for a selection of lines within a hunk. Forward patches (staging): keep context lines and selected lines, drop unselected `+` lines, turn unselected `-` lines into context. Reverse patches (unstaging and discard): keep context lines and selected lines, turn unselected `+` lines into context, drop unselected `-` lines. Then recompute the hunk header counts, drop hunks with no change left, and keep the original file header and any "No newline at end of file" marker that still applies.

Branches, tags, stash (write):

- `create_branch(name, at, checkout)`, `rename_branch(old, new)`, `delete_branch(name, force)`, `unmerged_commits(branch)`.
- `checkout(target)`: `git switch <branch>`, `git switch --track <remote>/<branch>` or `git switch --detach <oid>`.
- `create_tag(name, at, message)`, `delete_tag(name)`, `push_tag(remote, name)`, `delete_remote_tag(remote, name)`.
- `stash_push(message, include_untracked)`, `stash_apply(index)`, `stash_pop(index)`, `stash_drop(index)`.

Merge, rebase, reset (write):

- `merge(branch, no_ff)`, `rebase(onto)`, `rebase_continue()`, `rebase_skip()`.
- `cherry_pick(oid, mainline)`, `revert(oid, mainline)`.
- `reset(target, mode)` with `--soft`, `--mixed` or `--hard`.
- `continue_op()`, `abort_op()` for whichever operation is in progress.
- `resolve_file(path, text)`: write the result, `git add`. `resolve_side(path, side)`: `git checkout --ours` or `--theirs`, then `git add`.

Remotes (network, write, long-running, cancellable):

- `fetch(...)`: `git fetch --all --prune --progress`.
- `pull(mode, ...)`: `git pull --ff-only`, `--no-rebase` or `--rebase`.
- `push(remote, branch, set_upstream, force_with_lease, ...)`: `git push --progress`, with `-u` or `--force-with-lease=<branch>:<expected-oid>`.
- `add_remote`, `rename_remote`, `set_remote_url`, `remove_remote`.

Journal:

- `begin(action, touches)`, `finish(pending)`, `can_undo(entry)`, `undo(entry)`, `redo(entry)`, `snapshots()`, `restore(entry, as_branch)`, `prune()`.
- Undo is allowed only when HEAD and every ref in `refs_after` still match the repository. Redo runs the action again from its recorded inputs.

| Action | Undo |
|--------|------|
| commit, amend | `git reset --soft <head_before>` |
| checkout | switch back to `head_before` |
| branch or tag create / rename / delete | `git update-ref` or `git branch -m` back to `refs_before` |
| merge, rebase, cherry-pick, revert | `git reset --hard <head_before>` |
| reset | `git reset --hard <head_before>`, then `git stash apply --index <tree_ref>` if a snapshot exists |
| discard | `git apply <patch_file>` plus copying back `untracked_dir`, or applying `tree_ref` |
| stash push | `git stash pop` |
| stash drop | `git stash store <oid>` |

Graph layout (in `graph.rs`), run per commit in the order `git log` delivers them:

1. Keep a list of active lanes, each either free or reserved for a commit ID that a child is waiting for.
2. The commit takes the leftmost lane reserved for it. If none is reserved, it takes the first free lane and a new colour. Other lanes reserved for the same commit end in this row (merge-in edges) and become free.
3. The first parent inherits the commit's lane and colour. Each further parent reuses a lane already reserved for it, or takes the first free lane with a new colour.
4. Every other active lane passes straight through the row.
5. Emit a `GraphRow` with the commit's lane and its edges. Colours cycle through 8 theme colours.

## 7. Screens

- Git check: shown only when Git is missing or older than 2.39. Found and required versions, a path field, Retry. Blocks everything else.
- Welcome: Open, Clone, Init, plus the recent list (name, path, last opened). Accepts a dropped folder. States: no recents (one line of guidance), list, missing-folder rows greyed with Remove.
- Clone dialog: URL, destination with Browse, progress bar with Git's phase text, Cancel. States: input, cloning, error with Details.
- Workspace (main window, resizable panels):
  - Toolbar: repository name, current branch, Undo, Redo, Fetch, Pull, Push with ahead/behind badges, Branch, Stash, progress area, search field.
  - Banner strip: untrusted repository with Trust; operation in progress with Continue and Abort; detached HEAD with "Create branch here".
  - Sidebar: filter box and collapsible groups (Local, Remotes, Tags, Stashes) with context menus.
  - Graph: virtual list with the custom lane element, ref labels, columns for summary, author, date, short hash. Work-in-progress row on top. Context menu per row.
  - Detail panel: commit header, file list, diff. For the work-in-progress row it shows the staging view.
  - States: loading (skeleton rows), empty repository, loaded, error with Retry.
- Staging view: Unstaged and Staged lists with per-row and "all" buttons, the selected file's diff with per-hunk buttons and line selection, the commit box (summary with a 72-character counter, description, Amend toggle, Commit). States: no changes, changes, committing, conflicts present.
- Diff view: file header with the inline/split toggle, virtual list of lines with two gutters, hunk headers with Stage / Unstage / Discard for working-tree diffs. Lines select by click, shift-click or gutter drag. States: loading, text, binary, too large with "Load all", unchanged.
- Conflict view: conflicted file list, and per region the current side, the incoming side, buttons (Take current, Take incoming, Take both), an editable result, "Mark resolved". Whole-file buttons for binary and delete conflicts. States: unresolved, partly resolved, all resolved.
- Dialogs: create / rename / delete branch, create tag, merge, rebase, reset with three explained modes, stash, checkout with local changes, force-push, discard, add or edit remote, remove remote, credential prompt, error details. Destructive ones default to Cancel.
- Command palette: input, fuzzy-ranked commands with shortcuts, then matching branches and tags. States: recent commands, results, no results.
- Settings window: one page, one labelled control per Settings field, each with reset.
- Snapshots list: journal entries that have a snapshot, with Restore and "Restore to new branch". States: empty, list.
- Minimum window size: 900 by 600.

Default shortcuts: palette Cmd+Shift+P, open Cmd+O, clone Cmd+Shift+O, settings Cmd+, , undo Cmd+Z, redo Cmd+Shift+Z, search Cmd+F, refresh Cmd+R, fetch Cmd+Shift+F, pull Cmd+Shift+L, push Cmd+Shift+K, new branch Cmd+B, stash Cmd+Shift+S, focus graph Cmd+1, focus changes Cmd+2, stage or unstage selection Space, commit Cmd+Enter, toggle diff view Cmd+Shift+D, arrows / Page Up / Page Down / Home / End to move, Escape to close.

## 8. Build plan (milestones)

"Reference machine": Apple silicon MacBook, M1, 16 GB. "Big repo": the output of `scripts/make-big-repo.sh` (100,000 commits, 200 branches, a merge about every 20 commits).

### M0: de-risk (do this first)

A throwaway spike of the top risk: can this stack load and draw a 100,000-commit graph fast enough, and does GPUI Kit build and offer the components the app needs?

- **Tasks:**
  - Create the workspace and both crates. Add `gpui-kit = "0.7.3"`, open a window, commit `Cargo.lock`.
  - Write `scripts/make-big-repo.sh` with `git fast-import`.
  - Implement `stream_log`, the log parser and the lane layout in the core crate, with unit tests for four histories: linear, single-merge, octopus, criss-cross.
  - Draw the graph in a uniform virtual list with a custom element painting dots and curved edges. No other UI.
  - Write `bin/bench.rs` and record the numbers in `PROGRESS.md`.
  - Check docs.rs for `gpui-kit` 0.7.3 and list in `PROGRESS.md` whether each of these exists: tree view, command palette, settings form, title bar, resizable panels, context menu, toast, dialog. Anything missing gets built from primitives in the milestone that needs it.
- **Exit criteria:** on the reference machine with the big repo: first rows drawn under 1 second, full load under 5 seconds, scrolling at 60 frames per second (95th-percentile frame under 16 ms), memory under 400 MB. Fallbacks, in order: read the commit list with `gix` if `git log` is the bottleneck; draw straight segments if curve painting is; pin an earlier GPUI Kit 0.7 release if 0.7.3 does not build. If none of the three gets within the targets, stop and report back before any feature work.

### M1: app shell

- **Builds:** FR-REPO-01, FR-REPO-03, FR-REPO-04, FR-REPO-07, FR-SET-01, FR-SET-02, FR-STATE-01, FR-STATE-03, FR-STATE-04, FR-CMD-04, FR-SAFE-03, FR-SAFE-04
- **Tasks:**
  - `GitRunner` with the fixed environment, activity records, cancellation, plus the trust gate with untrusted overrides.
  - `GitError` classification and the message table in `locales/en.yml`.
  - `store.rs`: settings, session, recents, trust, with atomic writes and version fields.
  - `AppState`, the Git check screen, the welcome screen, open by picker / drop / CLI argument, init.
  - Workspace frame with resizable panels and empty placeholders, session restore, native menu bar.
  - Settings window. Light and dark themes that follow the system.
  - Error toast with a Details dialog.
  - CI workflow.
- **Exit criteria:** tests pass. A tester can open a repository, initialise one, reopen both, change settings and see them persist. The trust test (planted hook and fsmonitor command leave no marker) passes.

### M2: graph and read-only browsing

- **Builds:** FR-GRAPH-01, FR-GRAPH-02, FR-GRAPH-03, FR-GRAPH-05, FR-GRAPH-07, FR-BRANCH-01, FR-DIFF-01, FR-DIFF-09, FR-REPO-06
- **Tasks:**
  - `RepoState` with refresh scopes, generation counters and the debounced file watcher.
  - Production graph view from the M0 spike: columns, ref labels, selection, keyboard movement.
  - Sidebar with groups, filter, ahead/behind.
  - Commit detail panel, file list, diff parser, inline diff view with virtualised lines, plus binary and too-large handling.
  - Search with `author:` and `file:` prefixes.
- **Exit criteria:** tests pass. Browsing the big repo meets the M0 numbers with the full UI. A commit made in a terminal appears within 1 second.

### M3: staging and commit

- **Builds:** FR-GRAPH-04, FR-STAGE-01, FR-STAGE-02, FR-STAGE-03, FR-STAGE-04, FR-STAGE-05, FR-STAGE-06, FR-DIFF-02, FR-DIFF-03
- **Tasks:**
  - First task: confirm GPUI Kit's highlighter can colour arbitrary lines in a custom view. If not, wire `tree-sitter-highlight` directly.
  - `run_op` and the journal's `begin` / `finish`, with snapshots for discard.
  - Work-in-progress row and the staging view.
  - The patch builder with its table-driven test suite (single add, single remove, mixed hunk, first line, last line, no trailing newline, CRLF, renamed file, new file).
  - Hunk and line versions of stage, unstage, discard. Commit and amend with the summary counter and the message draft.
  - Split diff view and syntax highlighting.
- **Exit criteria:** tests pass, including every line-staging case. This completes the POC: open a repository, read the graph, stage chosen lines, commit.

### M4: branches, tags, stash

- **Builds:** FR-BRANCH-02, FR-BRANCH-03, FR-BRANCH-04, FR-STASH-01, FR-SAFE-01
- **Tasks:**
  - Branch create, rename, delete with the unmerged-commits confirmation.
  - Checkout of local branch, remote branch and commit, with the "Stash and switch" dialog and the detached HEAD banner.
  - Tags: lightweight, annotated, delete, push.
  - Stash push, apply, pop, drop; stashes in the sidebar and graph.
  - A shared confirmation dialog component whose default is Cancel, used by every destructive action from here on.
- **Exit criteria:** tests pass. Each action's result equals the matching Git command in an integration test.

### M5: merge, rebase, conflicts

- **Builds:** FR-MERGE-01, FR-MERGE-02, FR-MERGE-03, FR-MERGE-04, FR-MERGE-05, FR-MERGE-07, FR-GRAPH-08
- **Tasks:**
  - In-progress detection and the banner with Continue and Abort.
  - Merge, rebase with continue / skip / abort, cherry-pick, revert (with mainline choice for merges), reset with the three explained modes.
  - Conflict marker parser for the three styles, the conflict view, whole-file resolution.
  - The commit context menu.
- **Exit criteria:** tests pass. A scripted two-conflict merge can be resolved in the app and the result matches the expected file. A rebase started in a terminal is picked up by the banner.

### M6: remotes and sync

- **Builds:** FR-REPO-02, FR-SYNC-01, FR-SYNC-02, FR-SYNC-03, FR-SYNC-04, FR-SYNC-06, FR-STATE-02, FR-SAFE-02
- **Tasks:**
  - First task: the askpass spike. Helper mode in `main.rs`, the socket with a one-time token, the credential dialog. Prove it with an HTTPS remote needing a password and an SSH key with a passphrase. Fallback if it cannot be made reliable: show Git's error and tell the user to authenticate once in a terminal.
  - Progress parsing from stderr, toolbar progress, cancel by SIGTERM then SIGKILL.
  - Clone dialog, fetch, pull with the three modes, push, publish branch, force-push with lease and its snapshot.
  - Remote management. URL validation.
  - A test that the fake home's Git config is unchanged after the whole suite.
- **Exit criteria:** tests pass against local `file://` remotes. Manual check against one real HTTPS remote and one SSH remote, recorded in `PROGRESS.md`.

### M7: undo, palette, keyboard

- **Builds:** FR-UNDO-01, FR-UNDO-02, FR-CMD-01, FR-CMD-02
- **Tasks:**
  - Journal `undo`, `redo`, `can_undo`, `prune`, covering every action in the undo table. Round-trip test per action.
  - Toolbar buttons with tooltips naming the action, Cmd+Z routing (focused text field first).
  - Snapshots list with Restore and "Restore to new branch".
  - All actions registered as GPUI actions in one keymap module. Command palette over them plus branches and tags.
  - A keyboard-only pass over every Must feature.
- **Exit criteria:** tests pass. Every undo round trip returns refs, the index, every file to the captured state. The keyboard-only walkthrough (open, stage, commit, push) works.

### M8: hardening and packaging

- **Builds:** no new requirement IDs. Verifies NFR-01 to NFR-09.
- **Tasks:**
  - Run the bench on the big repo and fix regressions against NFR-01.
  - Contrast check of both themes, reduce-motion check, a sweep for hard-coded strings.
  - Network monitor run for FR-SAFE-03. Crash-safety check: kill the app mid-operation and confirm Git can continue.
  - `cargo-bundle` config, app icon, `README.md` (including the `refs/gittree/` namespace and the `.git/gittree/` folder), `LICENSE`, `NOTICE`, `cargo deny check licenses`.
- **Exit criteria:** every box in section 9 is ticked. `Safi Git Tree.app` launches from Finder and is under 40 MB. This is the MVP.

### M9: history tools (post-MVP, Should)

- **Covers:** FR-REWRITE-01, FR-DIFF-04, FR-DIFF-05, FR-DIFF-06, FR-DIFF-07, FR-UNDO-03, FR-MERGE-06
- **Tasks:** interactive rebase list (sequence editor set to copy a prepared todo file; reword done as pick plus an `exec git commit --amend -F` line), word diff and whitespace toggle, blame, file history with `--follow`, compare two refs, reflog browser, conflict prediction with `git merge-tree --write-tree`.

### M10: comfort (post-MVP, Should)

- **Covers:** FR-REPO-05, FR-GRAPH-06, FR-STAGE-07, FR-STAGE-08, FR-STAGE-09, FR-BRANCH-05, FR-STASH-02, FR-SYNC-05, FR-CMD-03, FR-SET-03, FR-SET-04, FR-SET-05, FR-STATE-05, FR-STATE-06
- **Tasks:** repository tabs, hide/show branches and `change:` search, message history and co-authors, signing errors, add to `.gitignore`, set upstream, stash to branch, auto-fetch, `keymap.json`, high-contrast and user themes, zoom, identity display, activity log view, completion toasts.

## 9. Acceptance criteria

### POC (proves the core idea: M1 to M3)

- [ ] **FR-REPO-01** A repository opens from the folder picker, a dropped folder and a command-line path. A non-repository folder gets a clear message and an offer to initialise.
- [ ] **FR-REPO-03** Init creates `.git` in the chosen folder and shows the empty-repository state.
- [ ] **FR-REPO-04** The welcome screen lists recent repositories after a restart, newest first, with missing folders greyed and removable.
- [ ] **FR-REPO-06** A file saved elsewhere or a commit made in a terminal shows in the app within 1 second.
- [ ] **FR-REPO-07** With Git missing or older than 2.39, a blocking screen shows found and required versions and a custom path field.
- [ ] **FR-GRAPH-01** The graph shows coloured lanes with curved forks and merges, and each row's summary, author, date, hash all match `git log`.
- [ ] **FR-GRAPH-02** On the big repo the first screen draws in under 1 second, the full history loads in under 5, and scrolling holds 60 frames per second.
- [ ] **FR-GRAPH-03** Labels for local branch, remote branch, tag, HEAD appear on the right rows and look distinct, with the current branch marked.
- [ ] **FR-GRAPH-04** A work-in-progress row with the changed-file count appears only when there are uncommitted changes, and opens the staging view.
- [ ] **FR-GRAPH-05** Search matches message, author or hash, supports `author:` and `file:`, shows a count, and Enter / Shift+Enter move between matches.
- [ ] **FR-GRAPH-07** The detail panel matches `git show --format=fuller`, lists changed files, shows a file's diff, and parent hashes are clickable.
- [ ] **FR-STAGE-01** Unstaged and staged lists show each file's status, and staging or unstaging one file, a selection or all files matches `git status`.
- [ ] **FR-STAGE-02** Staging one of two hunks leaves exactly that hunk in `git diff --cached`.
- [ ] **FR-STAGE-03** Staging lines 2 and 4 of a five-line addition stages exactly those two lines and leaves the file on disk unchanged.
- [ ] **FR-STAGE-04** Discarding a file, hunk or lines asks first, restores HEAD content, and Undo brings the discarded content back byte for byte, including untracked files.
- [ ] **FR-STAGE-05** Commit is disabled with nothing staged or an empty summary, the counter warns past 72 characters, and the commit lands at HEAD with the exact message.
- [ ] **FR-STAGE-06** Amend prefills the last message, replaces HEAD keeping its parent, and warns when the commit is already on its upstream.
- [ ] **FR-DIFF-01** The inline diff shows old and new line numbers, coloured changes and hunk headers, matching `git diff` with 3 context lines.
- [ ] **FR-DIFF-02** The split view aligns old and new sides, and the chosen view survives a restart.
- [ ] **FR-DIFF-03** Diffs are syntax-highlighted for the 19 listed languages in both themes, and unknown types show as plain text.
- [ ] **FR-DIFF-09** Binary files show sizes instead of content, diffs over 20,000 lines load 2,000 with "Load all", and neither freezes the window beyond 100 ms.
- [ ] **FR-BRANCH-01** The sidebar lists local branches, remotes, tags, stashes, all matching Git, marks the current branch, shows ahead/behind, filters by name, and clicking selects the commit.
- [ ] **FR-CMD-04** A native menu bar has six menus (File, Edit, View, Repository, Branch, Window) covering every command, with standard Mac items behaving normally.
- [ ] **FR-SET-01** Each setting applies without restart, persists, can be reset to its default.
- [ ] **FR-SET-02** Dark and light themes ship, and "System" follows macOS appearance within 1 second.
- [ ] **FR-SAFE-03** A network monitor shows zero outbound connections from the app while using every local feature.
- [ ] **FR-SAFE-04** An untrusted repository with a planted hook and fsmonitor command can be browsed without either running, every changing action is disabled until the user trusts it, and the choice persists.
- [ ] **FR-STATE-01** Each of the five empty states shows what is empty and the next action.
- [ ] **FR-STATE-03** Every failed Git command shows a plain-language message with the command line and stderr under Details.
- [ ] **FR-STATE-04** After quit and relaunch the same repository is open with the same window and panel layout.

### MVP (shippable v1: M4 to M8)

- [ ] **FR-REPO-02** Clone from an HTTPS or SSH URL shows Git's progress, opens the result, and Cancel stops Git within 2 seconds and removes the partial folder.
- [ ] **FR-GRAPH-08** The commit context menu offers checkout, create branch, create tag, cherry-pick, revert, reset to here and copy hash, each matching its Git command, with inapplicable items disabled.
- [ ] **FR-BRANCH-02** Branches can be created, renamed or deleted; deleting an unmerged branch needs a second confirmation naming the commits; the current branch cannot be deleted.
- [ ] **FR-BRANCH-03** Checkout works for local branches, remote branches (creating a tracking branch) and commits, offers "Stash and switch" when local changes conflict, and shows a detached HEAD banner.
- [ ] **FR-BRANCH-04** Lightweight and annotated tags can be created, deleted and pushed, and deleting offers to remove the remote tag.
- [ ] **FR-MERGE-01** The merge dialog names both branches, the result equals `git merge`, and an option forces a merge commit.
- [ ] **FR-MERGE-02** Rebase onto a branch works, and the three controls (Continue, Skip, Abort) behave like the matching `git rebase` flags.
- [ ] **FR-MERGE-03** Cherry-pick and revert create the expected commits and ask for the mainline parent on merge commits.
- [ ] **FR-MERGE-04** Reset offers three modes (soft, mixed, hard) with one plain sentence each, hard states how many changed files will be removed, and each equals its `git reset` flag.
- [ ] **FR-MERGE-05** Conflicts can be resolved per region (current, incoming, both, or hand edit) and per file, leaving no markers and the file staged.
- [ ] **FR-MERGE-07** A banner shows any in-progress merge, rebase, cherry-pick or revert, including ones started outside the app, with progress, Continue (disabled while conflicts remain) and Abort.
- [ ] **FR-UNDO-01** Undo and redo work for every listed action, return refs, the index, every file to the prior state, name the action in the tooltip, and are disabled with a reason when the repository changed outside the app.
- [ ] **FR-UNDO-02** Destructive actions save a snapshot kept 30 days, listed with its time and restorable after a restart, including the old remote commit after a force-push.
- [ ] **FR-STASH-01** Stash with an optional message and untracked files, then apply, pop or drop; stashes show in the sidebar and graph; a conflicting pop keeps the stash and opens the conflict view.
- [ ] **FR-SYNC-01** Fetch (all, prune), pull (fast-forward only by default, or merge, or rebase) and push each show progress, can be cancelled, and report their result.
- [ ] **FR-SYNC-02** Force-push uses `--force-with-lease`, confirms with Cancel as default, names the remote branch and the commits replaced, and is refused when the remote moved.
- [ ] **FR-SYNC-03** Git's own credential setup is used, username / password / passphrase prompts appear as app dialogs, nothing typed is stored, and no operation hangs waiting for a terminal.
- [ ] **FR-SYNC-04** Remotes can be listed, added, renamed, edited or removed (with confirmation), matching `git remote -v`.
- [ ] **FR-SYNC-06** Pushing a branch with no upstream publishes it and sets the upstream in one step.
- [ ] **FR-CMD-01** Cmd+Shift+P opens a palette that fuzzy-finds every command (with shortcut shown) and branches and tags by name.
- [ ] **FR-CMD-02** Core actions have shortcuts shown in menus, and open, stage, commit, push can be done with the keyboard alone.
- [ ] **FR-SAFE-01** Every work-losing dialog defaults to Cancel on Enter and Escape and names what will be lost.
- [ ] **FR-SAFE-02** Global Git config and `~/.ssh` are byte-identical after a full test session, and repository config changes only through explicit actions.
- [ ] **FR-STATE-02** Operations over 300 ms show progress, the window stays readable, and network operations cancel within 2 seconds.
- [ ] **NFR-01** Performance on the reference machine with the big repo: start under 500 ms, first graph screen under 1 second, full load under 5 seconds, 60 frames per second scrolling, status under 300 ms at 10,000 files, 5,000-line diff under 200 ms, memory under 400 MB.
- [ ] **NFR-02** No repository content or secret leaves the machine or reaches a log, and untrusted repositories cannot run code.
- [ ] **NFR-03** Everything except the four network operations (clone, fetch, pull, push) works offline, and network failures never freeze the window.
- [ ] **NFR-04** The app handles 100,000 commits, 5,000 refs, 50,000 tracked files, 500 changed files and 20,000-line diffs.
- [ ] **NFR-05** Runs on macOS 13 or later on Apple silicon and Intel, with macOS-specific code confined to the `platform` module.
- [ ] **NFR-06** English strings all come from `locales/en.yml`, every Must feature works by keyboard, contrast is at least 4.5:1, and reduce-motion is honoured.
- [ ] **NFR-07** The app bundle is under 40 MB with no web view.
- [ ] **NFR-08** Every repository change is made by the installed Git and equals the matching command, with hooks and signing behaving as on the command line in trusted repositories.
- [ ] **NFR-09** No action loses work without a snapshot, and killing the app mid-operation leaves a repository Git can continue from.

## 10. Standard states

- Undo and redo: FR-UNDO-01, FR-UNDO-02.
- Empty states: FR-STATE-01.
- Loading and progress: FR-STATE-02, FR-GRAPH-02.
- Error states and messages: FR-STATE-03, FR-REPO-07.
- Confirm before destructive actions: FR-SAFE-01, FR-SYNC-02.
- Save and reopen: FR-STATE-04, plus the commit message draft kept per repository (FR-STAGE-05).
- Search and filter: FR-GRAPH-05, the sidebar filter (FR-BRANCH-01), FR-CMD-01.
- Keyboard shortcuts: FR-CMD-02. Rebinding (FR-CMD-03) is post-MVP.
- Settings that persist: FR-SET-01.
- First run: FR-REPO-07 and the welcome screen. Onboarding tutorial: out of scope.
- Offline or bad connection: NFR-03, FR-STATE-02, FR-STATE-03.
- Permissions per role: out of scope (single user). Repository trust is FR-SAFE-04.
- Notifications: in-app toasts only (FR-STATE-06, post-MVP). System notifications: out of scope.
- Import and export: out of scope.
- Delete and restore: FR-UNDO-02, FR-STAGE-04.
- Audit log: FR-STATE-05 (post-MVP view; the records and log file exist from M1).
- Multi-language and RTL: out of scope for v1. Strings externalised under NFR-06.
- Accessibility: NFR-06.
- Mobile layout: out of scope. Minimum window 900 by 600.
- Rate limits and quotas: out of scope, no service is called.

## 11. Out of scope for MVP

Do not build these, even if they look easy:

- Hosting integrations: GitHub, GitLab, Bitbucket, Azure DevOps, pull requests, issues, CI status.
- Any AI feature: chat, agent mode, generated commit messages.
- A built-in terminal.
- User accounts, sign-in, any server.
- Arabic or any right-to-left layout. Any language besides English.
- Windows and Linux builds or packaging.
- Worktrees, submodules, Git LFS management, bisect, Git Flow, patch create and apply.
- Partial clone options (shallow, sparse, blobless).
- Identity profiles, SSH key management, external diff or merge tools, custom commands.
- Auto-update, system notifications, onboarding tutorial.
- Image diff and Markdown preview (FR-DIFF-08), quick history edits from the context menu (FR-REWRITE-02), partial stash (FR-STASH-03).
- Storing credentials or tokens in any form.

Repositories that use worktrees, submodules or LFS must still open and behave correctly for everything in scope.

## 12. Top risks and mitigations

1. Graph speed and drawing in GPUI. Mitigation: M0 proves it before anything else, with `gix` and straight-segment fallbacks.
2. GPUI Kit churn (a 0.x library on a GPUI snapshot, several releases a week). Mitigation: exact pin, committed lockfile, upgrades as separate commits, M0 component check.
3. Line staging correctness. Mitigation: `git apply --check` before every apply, snapshot before every discard, the table-driven suite in M3.
4. Undo correctness. Mitigation: undo refused unless the repository matches the journal's "after" state, a round-trip test per action in M7.
5. Parsing Git output. Mitigation: porcelain and `-z` formats, `LC_ALL=C`, the 2.39 minimum, tests with hostile file names.
6. Credential prompts. Mitigation: the askpass spike is the first task of M6, with a documented fallback.
7. Untrusted repositories. Mitigation: read-only mode with a fixed set of read commands and every known hook and helper disabled, plus the planted-hook test in M1, extended in M2 with a planted filter and textconv driver.
8. Name clash: resolved. The working name "GitTree" clashed with another commercial Git client, so the product is now "Safi Git Tree" (SGT). The display name lives in one constant and the bundle metadata.

## 13. Instructions for Claude Code

- Build milestone by milestone, in order. Finish M0 before any feature work.
- Keep a `PROGRESS.md` at the repo root: milestone status, decisions made, deviations from this spec and why.
- At the end of each milestone, run the tests and tick the acceptance boxes it covers. Don't move on with a failing box.
- Don't add features outside this spec. If something seems missing, note it in `PROGRESS.md` and ask.
- If a spec decision turns out to be wrong in practice, stop. Explain why. Don't silently swap a stack choice.
- Commit after each step (each finished task within a milestone), with a message naming the milestone and the step. Push to `origin` (`https://github.com/Abdulkader-Safi/SGT.git`, branch `main`) only when the GitHub CI needs to run, not after every commit.
- Keep secrets out of the repo. There are none in v1; the environment variables are `GITTREE_GIT`, `GITTREE_DATA_DIR`, `GITTREE_LOG`, and the two askpass variables the app sets for itself.
- GPUI Kit's API moves fast. Before using a component, read its docs for 0.7.3 on docs.rs instead of relying on memory, and prefer a kit component over a hand-built one when it exists.
- Tests that touch Git run with `HOME` and `GIT_CONFIG_GLOBAL` pointed at a temporary folder, so they never read or write the developer's real config.
- All user-facing text is plain English in sentence case, with no em dashes, and lives in `locales/en.yml`.

---

*Companion files in `docs/spec/`: 01-product, 02-users, 03-requirements, 04-tech-decisions, 05-tech-requirements. This build spec is the entry point.*
