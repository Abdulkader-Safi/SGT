# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

M0 (the graph performance spike) is done. The workspace exists with both crates: the core crate has a read-only `GitRunner`, the log parser, `stream_log` and the lane layout, and the app draws the commit graph and nothing else. `PROGRESS.md` has the bench numbers and decisions. Next is M1.

`docs/spec/06-build-spec.md` is the entry point: it is self-contained, and files 01 to 05 are the reasoning behind it. Build milestone by milestone, and follow its section 13.

## What this is

GitTree (working name) is a native macOS Git client in Rust, rendered with GPUI through the `gpui-kit` crate. v1 is a local client only: no backend, accounts, telemetry, hosting integrations, AI features or terminal. Section 11 of the build spec lists what not to build.

## Commands

```sh
cargo test --workspace                       # all tests
cargo test -p gittree-core                   # core crate only (fast, no UI build)
cargo test -p gittree-core <test_name>       # a single test
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p gittree -- /path/to/repo        # run the app on a repository
scripts/make-big-repo.sh                     # build the 100,000-commit test repository in target/big-repo
cargo run --release -p gittree --bin bench -- <path>   # performance numbers for NFR-01
```

Tests that touch Git must run with `HOME` and `GIT_CONFIG_GLOBAL` pointed at a temporary folder, so they never read or write the developer's real config.

## Architecture

A Cargo workspace with two crates:

- `crates/gittree-core`: Git access, parsers, graph lane layout, partial patch builder, undo journal, JSON stores. No UI dependency, so its tests run without compiling GPUI.
- `crates/gittree`: the GPUI app. Views, `AppState`, and one `RepoState` entity per open repository.

`RepoState` is the single source of truth for a repository. Views hold no repository data and never run Git. Every change to a repository goes through `RepoState::run_op`, which queues it (one writer at a time), writes the journal entry, runs it through `gittree-core`, and refreshes. Undo, progress, error reporting, the activity log and the trust check all depend on that single path, so do not add a second one.

All Git work is done by the installed `git` executable (2.39+) as a child process through `GitRunner`. There is no libgit2 and no in-process reimplementation of Git commands.

## Rules that hold in every milestone

- The UI thread never waits on Git, file reads or JSON writes. Use the background executor.
- Parse machine output only: `--porcelain=v2`, `-z`, explicit `--format`. Every Git call sets `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, `GIT_PAGER=cat`, and read calls add `GIT_OPTIONAL_LOCKS=0`.
- Start Git with an argument vector, never a shell. Put `--` before path lists. Reject names and URLs that start with `-`.
- Snapshot before any action that can destroy work (hard reset, discard, rebase, force-push, branch delete, stash drop).
- A repository the app did not create or clone is untrusted: read-only, with hooks and helper programs disabled. `GitRunner` enforces this, not the views.
- Never write global Git config, `~/.ssh` or a credential store. Never store or log a secret. No network use besides Git's own.
- Import GPUI only through `gpui_kit` re-exports. Never add the `gpui` crate directly: the kit pins its own GPUI snapshot (`gpui-pre`) and a second copy will not compile together with it.
- GPUI Kit is pinned at 0.7.3 and its API changes often. Read the docs for that exact version on docs.rs before using a component. Upgrade only as a separate commit.
- `git log --all` must be preceded by `--exclude=refs/gittree/*`, because the app keeps snapshot refs under `refs/gittree/snapshots/`.

## Working conventions

- Keep `PROGRESS.md` at the repo root: milestone status, decisions, deviations from the spec with reasons.
- Do not add features outside the spec. If a spec decision proves wrong, stop and explain instead of swapping a stack choice.
- To change scope, edit the earliest affected stage file (01 to 05) and regenerate 06. Do not patch 06 by hand.
- Commit after each step (each finished task within a milestone), with a message that names the milestone and the step. Do not push after every commit. Push to `origin main` only when the GitHub CI needs to run, for example to check a finished milestone.
- User-facing strings live in `crates/gittree/locales/en.yml`. Destructive dialogs default to Cancel and say what will be lost.
- Prose in docs, comments, commit messages and UI text: plain English, sentence-case headings, straight quotes, no em dashes.
- Never put a link to the gittree.app website in any project file.
