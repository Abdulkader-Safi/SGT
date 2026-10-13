# Tech decisions

Status: signed off 2026-10-12

For each decision: what was chosen, what else was considered, and why, traced to a requirement (Stage 3) or the mission and users (Stages 1 and 2).

> Constraints going in: the stack is fixed by Safi: Rust with GPUI and GPUI Kit. One developer plus Claude Code. Budget is zero. macOS first. No existing code to reuse: the repository is empty.
>
> Changes: none yet.

Versions below were read from crates.io on 2026-10-12.

## Overall architecture

- Choice: one native desktop process. A Cargo workspace with two crates: `gittree-core` (Git access, graph layout, diff parsing, patch building, undo journal; no UI dependency) and `gittree` (the GPUI app). All repository work is done by running the installed `git` executable as a child process on background threads, and the UI thread never waits on Git.
- Considered: a single crate (dropped: core logic would need the whole UI stack compiled to run a unit test, and GPUI builds are slow). A workspace of eight or nine small crates, as rgitui does (dropped: no second consumer exists for any of them). A client/daemon split (dropped: nothing needs to outlive the window).
- Why: FR-GRAPH-02 and NFR-01 need the UI thread free. NFR-08 needs real Git. The two-crate split lets the risky logic (line staging, graph layout, undo) be tested in milliseconds without a window.

## Language and toolchain

- Choice: Rust 1.97.1 stable, edition 2024, pinned in `rust-toolchain.toml`.
- Considered: tracking latest stable unpinned (dropped: GPUI Kit is edition 2024 and moves fast, so a pinned toolchain removes one variable).
- Why: fixed constraint. 1.97.1 is what is installed on the build machine.
- Landmines: none known. Tree-sitter 0.27 needs Rust 1.90, which is already satisfied.

## UI framework

- Choice: GPUI through `gpui-kit = "0.7.3"` with exact versions locked by a committed `Cargo.lock`. Features: the default set (component layer and Lucide icons) plus `tree-sitter-languages` and `test-support` for tests. The app imports GPUI only through `gpui_kit` re-exports.
- Considered: depending on `gpui` from crates.io directly (dropped: the published `gpui` crate is 0.2.2 from October 2025 and stale, while GPUI Kit pins its own snapshot, `gpui-pre =0.3.9`. Adding `gpui` as well would compile two incompatible GPUIs). A git dependency on the Zed repository (dropped: unpinned and heavy). Writing widgets by hand as rgitui does (dropped: GPUI Kit already has these components: list, table, dock, menu, popover, input, editor).
- Why: fixed constraint, and it serves NFR-07 (no web view) and FR-GRAPH-02 (GPU rendering, virtual lists).
- Landmines:
  - GPUI Kit released 0.7.2 and 0.7.3 on the same day (2026-10-12). Expect breaking changes between minor versions. Upgrade only on purpose, one version at a time.
  - `gpui-pre` is a snapshot of GPUI published by the GPUI Kit team, not an official Zed release. API docs for it are on docs.rs under `gpui-pre`, and Zed's own GPUI docs may not match.
  - Right-to-left layout is not automatic in GPUI Kit (its i18n docs say to review each screen). This is why Arabic is out of v1.
  - Confirmed present in GPUI Kit's docs: virtual list, data table, dock layout with resizable panels and tab groups, code editor with Tree-sitter highlighting, text view, menu, popover, tooltip, scrollbar, input, button, theme. Not confirmed from the pages read: a tree view, a command palette, a settings form, a title bar. M0 checks docs.rs for each and the app builds any missing one from primitives.
  - Sources: https://github.com/longbridge/gpui-kit and https://crates.io/crates/gpui-kit

## Git access

- Choice: the system `git` executable (2.39 or newer) for every read and every write, started with `std::process::Command` from GPUI's background executor. Machine-readable output only: `--porcelain=v2`, `-z`, explicit `--format` strings with unit-separator fields.
- Considered:
  - `git2` 0.21 (libgit2 bindings). Dropped: libgit2 does not run hooks, credential helpers, signing or LFS filters the way Git does, so results can differ from the command line. That breaks NFR-08.
  - `gix` 0.89 (pure Rust). Dropped as the primary: fast for reading, but its write side does not cover rebase, merge with conflicts, or stash. Kept as the named fallback for loading the commit list if `git log` proves too slow in M0.
  - Mixing a library for reads with the CLI for writes. Dropped for v1: two code paths to keep consistent, and no measured need yet.
- Why: NFR-08 (same results as the command line), FR-SYNC-03 (the user's credential setup just works), FR-STAGE-08 (signing follows Git config). One backend is also the least code.
- Landmines:
  - A background `git status` takes the index lock and can make the user's own terminal command fail. Set `GIT_OPTIONAL_LOCKS=0` on every read command.
  - Git must never wait for terminal input. Set `GIT_TERMINAL_PROMPT=0` always, and route prompts through an askpass helper (see "Credentials").
  - Git's messages are localised. Set `LC_ALL=C` so any error text the app matches is stable. Prefer exit codes and porcelain output over matching text.
  - Commands that open an editor (merge, rebase continue, revert) must run with `GIT_EDITOR=true` or an explicit message.
  - `git log --all` would include the app's own snapshot refs. Pass `--exclude=refs/gittree/*` before `--all`.

## Commit graph

- Choice: commits streamed from one `git log` process in date order, lane layout computed in `gittree-core` as rows arrive, and drawn by a custom GPUI element inside a uniform-height virtual list. Only visible rows are painted. Edges are quadratic curves painted as paths.
- Considered: GPUI Kit's data table for the whole view (dropped for the graph column: lanes cross row boundaries and need custom painting. The other columns can still reuse table cells if that proves simpler). Pre-rendering the graph to an image (dropped: memory and blur at zoom).
- Why: FR-GRAPH-01, FR-GRAPH-02, NFR-01, NFR-04. This is the top technical risk and is proven in M0.

## Diff and patch engine

- Choice: Git computes diffs (`git diff`, `git show` with `--no-color --no-ext-diff --no-textconv`). `gittree-core` parses the unified output into hunks and lines. Hunk and line staging build a partial patch in memory and feed it to `git apply --cached` (with `--reverse` for unstaging, and without `--cached` for discard). Word-level highlighting within a changed line uses the `similar` crate, 2.7, the version GPUI Kit already depends on.
- Considered: computing diffs in-process with `similar` or `imara-diff` (dropped: results could differ from Git's and would ignore the user's diff settings). Driving `git add -p` interactively (dropped: screen-scraping a prompt).
- Why: FR-STAGE-02, FR-STAGE-03, FR-STAGE-04, FR-DIFF-01, NFR-08.
- Landmines: a wrong partial patch can stage the wrong lines without an error. Every patch is checked with `git apply --cached --check` first, and the patch builder has its own test suite (Stage 5).

## Syntax highlighting

- Choice: Tree-sitter grammars bundled by GPUI Kit's `tree-sitter-languages` feature, using its highlighter for diff lines.
- Considered: `syntect` 5.3 (dropped: a second highlighting engine and theme format next to the one the UI kit already has). No highlighting (dropped: FR-DIFF-03 is a Must).
- Why: FR-DIFF-03. The feature flags cover every language in that requirement.
- Landmines: GPUI Kit's highlighter is designed for its editor component. If it cannot be called for arbitrary lines in a custom diff view, the fallback is `tree-sitter-highlight` used directly at the Tree-sitter version GPUI Kit pins (0.26, not the newer 0.27). Checked in M3's first task.

## Conflict resolution

- Choice: parse conflict markers from the working file (supporting the `merge`, `diff3` and `zdiff3` styles), present each region with its two sides, and write the resolved text back before `git add`. Whole-file choices use `git checkout --ours` or `--theirs`.
- Considered: rebuilding the three versions from index stages 1 to 3 and merging in-process (dropped: more code, and it would ignore what Git already merged cleanly).
- Why: FR-MERGE-05.

## Undo and snapshots

- Choice: an append-only journal per repository at `<git-common-dir>/gittree/journal.jsonl`. Each entry records the action, the refs before and after, and what is needed to restore the working tree: a snapshot commit made with `git stash create` and held by a private ref under `refs/gittree/snapshots/`, a saved patch for discards, and file copies for untracked files. Undo runs only if the repository still matches the entry's "after" state.
- Considered: relying on the reflog alone (dropped: it does not cover uncommitted changes, which is exactly what hard reset and discard destroy). A SQLite database in the app data folder (dropped: a text file inside `.git` needs no dependency, travels with the repository and is deleted with it).
- Why: FR-UNDO-01, FR-UNDO-02, FR-STAGE-04, NFR-09.
- Landmines: `git stash create` does not include untracked files, hence the separate copies. Private refs are never pushed by default refspecs, but they do show in `git for-each-ref`, so the namespace is documented in the README.

## Credentials

- Choice: none stored. Git's own credential helper and the SSH agent do the work. For prompts, the app sets `GIT_ASKPASS` and `SSH_ASKPASS` (with `SSH_ASKPASS_REQUIRE=force`) to its own executable. Started that way, the binary runs in a tiny helper mode: it sends the prompt text to the main app over a Unix domain socket, the app shows a dialog, and the helper prints the answer to stdout.
- Considered: the `keyring` crate 4.2 to store tokens (dropped for v1: nothing to store until hosting integrations exist). A separate helper binary (dropped: one binary is simpler to bundle and sign).
- Why: FR-SYNC-03, FR-SAFE-02, NFR-02.
- Landmines: the socket path goes in an environment variable and the socket file is created with mode 0600 in a per-user temporary folder. The Rust standard library has no Unix sockets on Windows, so there the listener is a TCP port on 127.0.0.1 picked by the system. Any local program can connect to that port, so the one-time token is the only guard on Windows: it is 32 random bytes from the system and travels only in the environment of the Git process the app started.

## File watching

- Choice: `notify` 8.2 (already in the dependency tree through GPUI Kit), watching the working tree and the `.git` folder, with events debounced by 300 ms.
- Considered: polling `git status` on a timer (dropped: either slow to react or wasteful). Git's own fsmonitor (dropped: it is user configuration the app must not change, FR-SAFE-02).
- Why: FR-REPO-06.
- Landmines: the app's own Git commands touch `.git` and would trigger refresh loops. The refresh is suppressed while an app-started command is running and one refresh is run when it ends. Paths ignored by `.gitignore` (such as `target/` or `node_modules/`) produce floods of events, so events are coalesced into a single "something changed" signal.

## Async and concurrency

- Choice: GPUI's foreground and background executors, with `smol` 2 primitives where a channel or timer is needed. Per repository, changing commands run one at a time through a queue. Read commands run concurrently and carry a generation number so stale results are dropped.
- Considered: `tokio` 1.53 (dropped: a second runtime next to GPUI's for no gain).
- Why: NFR-01, NFR-09 (no two writers racing for the index lock).

## Persistence and configuration

- Choice: JSON files written with `serde` 1.0 and `serde_json`, in `~/Library/Application Support/Safi Git Tree/` (found with the `dirs` crate). Files are written to a temporary name and renamed, so a crash cannot leave half a file.
- Considered: `rusqlite` 0.40 (dropped: five small documents do not need a database). macOS user defaults (dropped: not portable to v1.1 platforms).
- Why: FR-SET-01, FR-STATE-04, FR-REPO-04, FR-SAFE-04.

## Internationalisation

- Choice: `rust-i18n` 4.2 with `locales/en.yml`, wired the way GPUI Kit documents (`rust_i18n::extend!(gpui_component)` before `gpui_kit::init`).
- Considered: `fluent` 0.17 (dropped: GPUI Kit already uses rust-i18n, so one system covers both the kit's strings and the app's).
- Why: NFR-06.

## Errors and logging

- Choice: `thiserror` for the typed `GitError` in the core crate, `anyhow` at the app boundary, `tracing` with a file writer for logs at `~/Library/Logs/Safi Git Tree/gittree.log`, truncated at startup when over 5 MB.
- Considered: `log` plus a custom file writer (dropped: GPUI Kit already brings `tracing`).
- Why: FR-STATE-03, FR-STATE-05.

## Testing

- Choice: `cargo test`. Core tests build real temporary repositories with the `tempfile` crate and the system Git, then assert on Git's own output. A small number of headless UI tests use GPUI Kit's `test-support` feature. Performance is measured by a bench binary against the big repo.
- Considered: mocking Git (dropped: the point of the tests is that the app and Git agree). `insta` snapshot tests for parsers (dropped for now: plain assertions are enough, add it if parser fixtures grow).
- Why: NFR-08, NFR-09, and every acceptance line in Stage 3 that says "equals the Git command".

## CI/CD

- Choice: GitHub Actions on `macos-latest`, `ubuntu-latest` and `windows-latest` for every push: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`. Releases are built locally in v1.
- Considered: a three-OS matrix now (dropped until v1.1: the other platforms are not supported yet, and a red build nobody fixes teaches people to ignore CI).
- Why: one developer, zero budget. GitHub Actions is free for public repositories.

## Distribution

- Choice: `cargo-bundle` 0.12 to produce `Safi Git Tree.app` for local use. For the first public release: a universal binary (`aarch64-apple-darwin` plus `x86_64-apple-darwin` joined with `lipo`), signed and notarised with an Apple Developer ID, shipped as a DMG on GitHub Releases.
- Considered: `cargo-packager` 0.11.8 (dropped: last released November 2025, while cargo-bundle shipped in September 2026). Homebrew cask (later, once there is a signed release).
- Why: NFR-05, NFR-07.
- Landmines: an unsigned app is blocked by Gatekeeper on other people's Macs. Signing needs a paid Apple Developer membership. This blocks public distribution, not development.

## Layers that do not exist in v1

No backend, database server, auth, hosting, payments, email, analytics, AI models or search service. The app is a single local binary (Stage 1 mission, FR-SAFE-03).

## Reused from existing work

Nothing from Safi's earlier projects applies: the other project in the workspace is a C++ engine. Reused from outside: GPUI Kit's components and theme, and two ideas from rgitui (lane-based graph with curved edges, three-pane conflict view) as proof they work in GPUI. No code is copied from it.

## Cost

Zero to build and run. No hosting, API or model costs at any scale, because there is no server. The only possible cost is the Apple Developer Program at $99 per year, needed for a signed public release and not before.

## Licensing

The project is Apache-2.0. Dependencies and their licences: GPUI Kit, gpui-pre and similar are Apache-2.0, and smol is dual Apache-2.0 or MIT. These are MIT or Apache-2.0: serde, anyhow, thiserror, tempfile, dirs, uuid. rust-i18n and the Tree-sitter grammars are MIT. notify is CC0-1.0. All are compatible with Apache-2.0 distribution. Git itself is GPL-2.0 but is run as a separate program and never linked or bundled, so its licence does not attach to the app.

To verify at build time: run `cargo deny check licenses` once before the first public release, and include a `NOTICE` file with third-party attributions, as Apache-2.0 requires.

## Top technical risks

1. **Graph speed and drawing in GPUI** : 100,000 rows with custom-painted lanes at 60 frames per second is unproven with this exact stack. Spike: M0 builds only this, against the big repo, and measures it. If loading is slow, switch the commit-list read to `gix`. If curve painting is slow, draw straight segments.
2. **GPUI Kit churn** : a 0.x library with several releases a week, built on a GPUI snapshot. Spike: M0 confirms a clean build and lists which needed components exist. Mitigation after that: exact pin, committed lockfile, upgrades as their own commits.
3. **Line staging correctness** : a bad partial patch silently stages or discards the wrong lines, which is data loss. Mitigation: `--check` before every apply, a snapshot before every discard, and a table-driven test suite that compares the index with expected content for adds, deletes, mixed hunks, files with no trailing newline, and CRLF files.
4. **Undo correctness** : restoring refs and files to a wrong state is worse than no undo. Mitigation: undo is refused unless the repository matches the journal's "after" state, each undoable action has a round-trip test (do, undo, compare with a full snapshot of refs, index, files), and redo is implemented as "run the action again" from recorded inputs.
5. **Parsing Git output** : formats differ by Git version, by locale, and by user config. Mitigation: porcelain and `-z` formats only, `LC_ALL=C`, explicit `--format`, a minimum Git version check (FR-REPO-07), tests with file names containing spaces, quotes, newlines, or non-ASCII characters.
6. **Credential prompts** : the askpass round trip between Git, a helper process and a dialog is easy to deadlock. Spike: the first task of M6 is a fetch from a password-protected HTTPS remote and an SSH remote with a passphrase-protected key. Fallback: show Git's error and tell the user to authenticate once in a terminal.
7. **Untrusted repositories** : Git config offers many ways to run a program, and the list of overrides may be incomplete. Mitigation: untrusted mode is read-only and runs a fixed, small set of read commands with every known hook and helper disabled. The test in FR-SAFE-04 plants hooks and helpers and checks none fire.
8. **Name clash** : RESOLVED 2026-10-13. The working name "GitTree" clashed with another commercial Git client, so the product was renamed "Safi Git Tree" (SGT). The display name lives in one constant and the bundle metadata.

---

### Open questions

None.

### Assumptions confirmed

Defaults taken under the blanket sign-off on 2026-10-12.

1. **"GPUI kit" : RESOLVED.** Read as the `gpui-kit` crate from Longbridge (the project formerly known by its component crate, `gpui-component`), version 0.7.3.
2. **Git backend : RESOLVED.** System Git for everything, `gix` only as a measured fallback for reading the commit list.
3. **Storage : RESOLVED.** JSON files, no database.
4. **Secrets : RESOLVED.** Not stored. Prompts are relayed to Git.
5. **Distribution : RESOLVED.** Local `.app` during development. Signed DMG on GitHub Releases for the first public version, which needs an Apple Developer membership at that point.
6. **CI : RESOLVED.** GitHub Actions on macOS, Linux and Windows.
