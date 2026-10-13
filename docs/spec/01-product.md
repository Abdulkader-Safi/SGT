# Safi Git Tree (SGT)

Status: signed off 2026-10-12

## Mission

Build a desktop Git client that is native end to end: Rust, GPU-rendered with GPUI, no web view and no bundled browser. Every Git action goes through the Git already installed on the machine, so the app can never disagree with the command line.

It is a personal project first. Safi builds it, uses it daily, and publishes the source. There is no revenue goal for v1, which means no accounts, no telemetry, no licence server. That shapes the rest of the spec: anything that needs a backend is out of v1.

## One-liner

A fast native Git client for developers who want a visual commit graph and line-level staging without an Electron app or a subscription.

## The problem

A developer who works across several repositories runs Git dozens of times a day. The command line is exact but blind: you cannot see how eight branches relate from `git log`, and staging three lines out of a 40-line change with `git add -p` is slow and easy to get wrong.

The visual clients that fix this cost money or weight (prices from the Toolradar guide, which says it checked vendor pages in September 2026: https://toolradar.com/guides/best-git-gui-clients):

- GitKraken: free Community tier, Pro from $10 per seat per month. Built on Electron.
- Tower: from $69 per user per year. No Linux build.
- Fork: $59.99 once. No Linux build.
- Sublime Merge: $99 once for a personal licence.
- SmartGit: $88.80 per year, free for non-commercial use. Runs on the JVM.
- GitHub Desktop and Sourcetree: free, but neither has interactive rebase with a full graph, and Sourcetree has no Linux build.

The shared flaw is a trade. The fast native ones are paid and closed. The free ones are slow, limited, or tied to one host.

The closest open project is rgitui (https://github.com/noahbclarkson/rgitui), an MIT-licensed GPUI Git client. Its README says it is in early development with breaking changes expected, it uses libgit2 for local operations (so hooks and commit signing can behave differently from the command line), and it builds its own widget set. It proves the approach works. It does not yet fill the gap.

The gap: a free, source-available, native client with the full daily workflow (graph, line staging, conflicts, undo) that behaves exactly like the installed Git.

## The value

The "after" picture. Safi opens a client repo with 60,000 commits. The window appears in under half a second and the graph is on screen in under one. He scrolls two years back at 60 frames per second. In the working changes he selects four lines of a fix, stages only those, commits, then drags the rest into a stash. A rebase hits a conflict: the resolver shows both sides, he takes one, edits a line, continues. He hard-resets the wrong branch by mistake and presses Cmd+Z. Nothing was lost.

The interaction model that matters: every destructive action is undoable from the toolbar, and every dialog that can lose work defaults to Cancel.

Versus GitKraken, the user gets:

- A native binary under 40 MB with no browser runtime: starts in under half a second, uses a fraction of the memory.
- No account and no subscription: private and commercial repositories work the same as public ones.
- The installed Git does the work: hooks, credential helpers and commit signing behave as they do in the terminal.
- Source on GitHub: anyone can read what the app runs. An activity log shows every Git command it executed.

What it deliberately is not: a code-review tool, an AI assistant, or a terminal. Pull requests, issues and CI status stay in the browser. AI commit messages stay in whichever coding agent the developer already uses. Those may come in v2. They are not in v1.

## Product category

Desktop Git GUI client. It competes with Fork and Sublime Merge on speed, with GitKraken on the commit graph, and with GitHub Desktop on price. The white space is "native, free, full workflow": today you get at most two of those.

## Business model and licence

Free. Source published on GitHub at https://github.com/Abdulkader-Safi/SGT under Apache-2.0, the same licence as GPUI and GPUI Kit, so there is one licence to reason about. No paid tier is planned for v1. If one ever appears it would cover hosted features (review, sync), never the local client.

## Platforms

Day one: macOS 13 or later, Apple silicon and Intel. That is where it is built and used daily.

Later (v1.1): Windows 10/11 and Linux (Ubuntu 22.04+, Debian 12+). GPUI supports both, and v1 code keeps platform-specific parts in one module so the port is packaging work, not a rewrite.

## What success looks like

One year in, Safi has not opened another Git client in months. The repo has a tagged 1.0 with a signed macOS download, and the Windows and Linux builds exist.

Measurable signals:

- 30 consecutive working days with Safi Git Tree as the only Git GUI on Safi's machine.
- Zero open bugs that lose user work.
- The performance numbers in NFR-01 hold on a 100,000-commit repository.
- At least 10 outside users have filed an issue or a pull request.

## UI and product references

No screenshots are stored for v1. The layout follows the convention shared by Fork and GitKraken, which users already know:

- A left sidebar listing local branches, remotes, tags and stashes, each group collapsible, with the current branch marked and ahead/behind counts beside it.
- A central commit graph: coloured lanes on the left, then message with ref labels inline, author, date, short hash. A "work in progress" row sits on top when there are uncommitted changes.
- A right or bottom detail panel that shows the selected commit's files and diff, or the staging area when the work-in-progress row is selected.
- A top toolbar with undo, redo, fetch, pull, push, branch, stash. Keep these as labelled icon buttons, as in Fork.
- A command palette on Cmd+Shift+P, as in VS Code and Zed.

Visual style comes from GPUI Kit's default theme (shadcn-like: flat surfaces, 1 px borders, small radii). Do not invent a custom design system.

---

### Open questions

None.

### Assumptions confirmed

All items below are defaults taken under Safi's blanket sign-off on 2026-10-12. Change any of them by saying so and the affected stages get regenerated.

1. **Business model : RESOLVED.** Free and source-available under Apache-2.0, no revenue goal for v1.
2. **Scope : RESOLVED.** v1 is the local Git client only. Hosting integrations, the AI assistant, the built-in terminal and user accounts are v2 or later. Reason: each needs either a backend or a large separate subsystem, and none is needed to replace a Git client for daily use.
3. **Platform : RESOLVED.** macOS only for v1, Windows and Linux in v1.1.
4. **Language : RESOLVED.** English only for v1. Arabic and right-to-left layout are deferred because GPUI Kit's own docs say right-to-left is not automatic and has to be reviewed per screen. All strings still go through the i18n layer so Arabic can be added without touching views.
5. **Name : RESOLVED 2026-10-13.** The product is "Safi Git Tree", short form SGT. It replaces the working name "GitTree", which another commercial Git client already uses. Internal identifiers keep the old spelling: the crates `gittree` and `gittree-core`, the `GITTREE_*` environment variables, `refs/gittree/` and the `.git/gittree/` folder.
