# Progress

Milestone status, decisions, and deviations from `docs/spec/06-build-spec.md`.

## Status

| Milestone | Status |
|-----------|--------|
| M0: de-risk | Done 2026-10-12. All four targets met, no fallback needed |
| M1: app shell | Not started |
| M2 to M10 | Not started |

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
