# Safi Git Tree (SGT)

A native Git client for macOS, Windows and Linux, written in Rust and drawn with GPUI. It shows the commit graph, branches, commit details and diffs. Every Git action is done by the `git` already installed on your machine, so the app always agrees with the command line.

There is no account, no server and no telemetry. The only network use is Git's own.

## Status

Early development. What works today:

- Open a repository from the folder picker, by dropping a folder on the window, or with a path on the command line. Create a new repository.
- A commit graph with coloured lanes and curved forks and merges, with labels for branches, remote branches, tags and HEAD. A 100,000-commit repository loads in under a second on an M2 Pro.
- A sidebar listing local branches, remotes, tags and stashes, with a name filter.
- Commit details: message, author, committer, parents, changed files and an inline diff per file.
- Search by message, author or hash, plus `author:` and `file:` searches.
- Staging: whole files, single hunks or chosen lines. Unstage and discard the same way, with a confirmation before anything is discarded.
- Commit and amend, with the message kept as a draft until it is used.
- Inline and split diffs with syntax colours for 19 languages.
- Branches, tags and stashes: create, rename, delete, check out, with right-click menus and dialogs.
- Merge, rebase, cherry-pick, revert and reset, with a conflict view that resolves each region or the whole file.
- Clone, fetch, pull and push with progress and cancel, force-push with a lease, remotes, and sign-in prompts as app dialogs.
- Undo and redo for commits, discards, checkouts, merges, rebases, resets, and branch, tag and stash changes.
- Snapshots of anything a risky action could lose, kept for 30 days.
- A command palette (Cmd+Shift+P).
- An MCP server (`gittree mcp`), so a coding agent can read status, history, diffs and blame, and stage and commit when you allow it.
- The view updates by itself when the repository changes on disk.
- Settings, light and dark themes, and the window layout restored at launch.

Not built yet: packaging as an app bundle, blame and file history, worktrees and agent sessions, AI helpers, pull requests. `docs/spec/06-build-spec.md` has the full plan and `PROGRESS.md` tracks where it stands.

## Requirements

- macOS 13 or later, Windows 10/11, or Linux (Ubuntu 22.04+, Debian 12+)
- Git 2.39 or later
- Rust 1.97.1 to build (pinned in `rust-toolchain.toml`, so `rustup` installs it for you)
- macOS: Xcode command line tools
- Linux: the libraries GPUI links against (the `apt-get install` line in `.github/workflows/ci.yml` lists them)

## Build and run

```sh
cargo run -p gittree                      # start on the welcome screen
cargo run -p gittree -- /path/to/repo     # open a repository
```

The binary and the crates are still called `gittree`, from the project's working name.

## Tests

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Tests that run Git use a fake home folder under `target/`, so they never read or change your Git config.

To measure performance, build the large test repository and run the bench on it:

```sh
scripts/make-big-repo.sh
cargo run --release -p gittree --bin bench -- target/big-repo
```

## Trust

A repository that SGT did not create is opened read-only until you press "Trust repository". While it is untrusted, SGT stops Git from running anything the repository names: hooks, the fsmonitor command, filters, diff and merge drivers, pagers, aliases and credential helpers. Programs set in your own global Git config are left alone.

## What it writes, and where

- Settings, the recent list, the session and the trust list: `~/Library/Application Support/Safi Git Tree/`
- The log: `~/Library/Logs/Safi Git Tree/gittree.log`
- Inside a repository, once undo is built: snapshot refs under `refs/gittree/snapshots/` and a journal in `.git/gittree/`

It never writes your global Git config, `~/.ssh` or a credential store.

Environment variables: `GITTREE_GIT` (path to the Git program), `GITTREE_DATA_DIR` (a different data folder), `GITTREE_LOG` (`error`, `warn`, `info` or `debug`).

## Layout

- `crates/gittree-core`: Git access, parsers, graph layout and the JSON stores. No UI code, so its tests are fast.
- `crates/gittree`: the app.
- `docs/spec/`: the product and technical spec, in six files.

## Licence

Apache-2.0.
