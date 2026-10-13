# Functional requirements

Status: signed off 2026-10-12

Requirements say what the product must do, not how. Tech choices are in Stage 4. Each traces to the product (Stage 1) and the journey or core loop (Stage 2).

Priorities: Must is required for v1. Should is built after the Musts, in the post-MVP milestones. Could is optional. Won't is excluded from v1.

"Reference machine" below means an Apple silicon MacBook (M1, 16 GB RAM, SSD) on macOS 14. "Big repo" means the synthetic test repository with 100,000 commits and 200 branches, built by `scripts/make-big-repo.sh`.

## Core features (MVP)

### Repositories (REPO)

- **FR-REPO-01 [Must]** The app must open a local repository from a folder picker, from a folder dropped on the window, or from a path given on the command line.
  *Acceptance:* given a folder containing a Git repository, when it is opened by each of the three routes, then the graph for that repository is shown. Given a folder that is not a repository, then a message says so and offers "Initialise a repository here".
- **FR-REPO-02 [Must]** The app must clone a repository from an HTTPS or SSH URL into a chosen folder, showing progress and allowing cancel.
  *Acceptance:* given a valid public URL, when Clone is confirmed, then a progress bar shows Git's reported percentage, and on completion the new repository opens. When Cancel is pressed mid-clone, then the Git process stops within 2 seconds and the partial folder is removed.
- **FR-REPO-03 [Must]** The app must initialise a new repository in a chosen folder.
  *Acceptance:* given an empty folder, when Init is confirmed, then `.git` exists and the app shows the empty-repository state (FR-STATE-01).
- **FR-REPO-04 [Must]** The app must keep a list of recently opened repositories and show it on the welcome screen.
  *Acceptance:* after opening three repositories and restarting, the welcome screen lists all three, most recent first. A repository whose folder no longer exists is shown greyed with a Remove action.
- **FR-REPO-05 [Should]** The app should open several repositories as tabs in one window and restore them at launch.
- **FR-REPO-06 [Must]** The app must notice changes made outside the app (edits in an editor, Git commands in a terminal) and refresh.
  *Acceptance:* given an open repository, when a tracked file is saved in another program, then the work-in-progress row and file list update within 1 second. When `git commit` is run in a terminal, then the new commit appears in the graph within 1 second.
- **FR-REPO-07 [Must]** The app must check at startup that Git 2.39 or newer is installed and usable.
  *Acceptance:* given no Git on the PATH or an older version, when the app starts, then a blocking screen states the found version, the required version, a link to install instructions, and a field to set a custom Git path. Nothing else in the app is reachable until the check passes.

### Commit graph (GRAPH)

- **FR-GRAPH-01 [Must]** The app must show history as a graph with one row per commit: coloured branch lanes with curved fork and merge lines, then summary, author, date, short hash.
  *Acceptance:* given a repository with two branches merged once, then the graph shows two lanes in different colours that fork at the branch point and join at the merge commit, and each row's columns match `git log` for that commit.
- **FR-GRAPH-02 [Must]** The app must stay responsive on very large histories by loading and drawing incrementally.
  *Acceptance:* given the big repo on the reference machine, when it is opened, then the first screen of rows is drawn in under 1 second, scrolling holds 60 frames per second, and jumping to the oldest commit works as soon as loading finishes (under 5 seconds).
- **FR-GRAPH-03 [Must]** The app must label commits with the refs that point at them: local branches, remote branches, tags, HEAD. The four kinds must be visually distinct.
  *Acceptance:* given a commit pointed at by `main`, `origin/main` and tag `v1.0`, then all three labels appear on that row, and the checked-out branch is marked as current.
- **FR-GRAPH-04 [Must]** The app must show a work-in-progress row at the top of the graph whenever there are uncommitted changes, with the count of changed files.
  *Acceptance:* given a clean repository, no such row exists. When one file is edited, then the row appears with "1" and selecting it opens the staging view.
- **FR-GRAPH-05 [Must]** The app must search commits by summary text, author, or hash, and support the prefixes `author:` and `file:`.
  *Acceptance:* given the query `author:sami fix`, then only commits by an author matching "sami" whose message contains "fix" are highlighted, a match count is shown, and Enter / Shift+Enter move between matches. Given `file:src/main.rs`, then only commits touching that path match.
- **FR-GRAPH-06 [Should]** The app should let the user hide and show individual branches in the graph, and support a `change:` search prefix that finds commits adding or removing a given string.
- **FR-GRAPH-07 [Must]** The app must show details for the selected commit: full message, author with date, committer with date, full hash, parents (clickable), the list of changed files with status, and the diff for a selected file.
  *Acceptance:* given a selected commit, then every field matches `git show --format=fuller` for it, and clicking a parent hash selects and scrolls to that commit.
- **FR-GRAPH-08 [Must]** The app must offer a context menu on a commit with: checkout, create branch here, create tag here, cherry-pick, revert, reset current branch to here, copy hash.
  *Acceptance:* each menu item performs the same change as the matching Git command, and items that cannot apply (for example cherry-pick of the current HEAD) are disabled.

### Staging and commits (STAGE)

- **FR-STAGE-01 [Must]** The app must list changed files in two groups, unstaged and staged, with each file's status (added, modified, deleted, renamed, untracked, conflicted), and stage or unstage a single file, a selection, or all files.
  *Acceptance:* given three modified files, when "Stage all" is pressed, then all three move to the staged group and `git status` agrees.
- **FR-STAGE-02 [Must]** The app must stage and unstage individual hunks.
  *Acceptance:* given a file with two hunks, when the first is staged, then `git diff --cached` shows only the first hunk and `git diff` shows only the second.
- **FR-STAGE-03 [Must]** The app must stage and unstage individual lines or a selected range of lines.
  *Acceptance:* given a hunk that adds five lines, when lines 2 and 4 are selected and staged, then `git diff --cached` shows exactly those two additions and the file on disk is unchanged.
- **FR-STAGE-04 [Must]** The app must discard changes for a file, a hunk, or selected lines, after confirmation, and keep a backup so the discard can be undone.
  *Acceptance:* given a modified file, when Discard is confirmed, then the file matches HEAD. When Undo is pressed, then the discarded content is back, byte for byte. The same holds for an untracked file.
- **FR-STAGE-05 [Must]** The app must create a commit from a summary and an optional description, with a live character counter on the summary that warns past 72.
  *Acceptance:* the Commit button is disabled when nothing is staged or the summary is empty. Given a 73-character summary, the counter turns to the warning colour and the commit is still allowed. After committing, the new commit is at HEAD with the exact message entered.
- **FR-STAGE-06 [Must]** The app must amend the last commit, changing its message, its content, or both.
  *Acceptance:* given the "Amend" toggle is on, the message box is filled with the last commit's message, and committing replaces HEAD with a commit that has the same parent. If the commit being amended already exists on its upstream, a warning says a force-push will be needed.
- **FR-STAGE-07 [Should]** The app should offer recent commit messages, message templates, and co-author trailers.
- **FR-STAGE-08 [Should]** The app should sign commits when the user's Git config asks for it (GPG, SSH or X.509), and show a clear error when signing fails.
- **FR-STAGE-09 [Should]** The app should add a file or pattern to `.gitignore` from a file's context menu.

### Diff (DIFF)

- **FR-DIFF-01 [Must]** The app must show a file's changes as an inline (unified) diff with old and new line numbers, hunk headers, and colour on added and removed lines.
  *Acceptance:* given any text change, the lines shown match `git diff` with 3 lines of context.
- **FR-DIFF-02 [Must]** The app must offer a split (side by side) view of the same diff, with the choice remembered.
  *Acceptance:* toggling the view shows old on the left and new on the right with changed lines aligned, and the choice survives a restart.
- **FR-DIFF-03 [Must]** The app must syntax-highlight diff content for at least: Rust, JavaScript, TypeScript, TSX, Python, Go, C, C++, Java, PHP, Ruby, HTML, CSS, JSON, YAML, TOML, Markdown, SQL, Bash.
  *Acceptance:* given a `.rs` file diff, keywords and strings are coloured differently from each other and from comments in both light and dark themes. A file type outside the list shows as plain text with no error.
- **FR-DIFF-04 [Should]** The app should highlight changed words within changed lines, and offer a toggle to ignore whitespace.
- **FR-DIFF-05 [Should]** The app should show blame for a file: per line, the commit, author and date, with a click to select that commit.
- **FR-DIFF-06 [Should]** The app should show the history of one file, following renames.
- **FR-DIFF-07 [Should]** The app should compare any two branches, tags or commits and list the files that differ.
- **FR-DIFF-08 [Could]** The app could show image changes side by side and render Markdown files as a preview.
- **FR-DIFF-09 [Must]** The app must handle files it cannot or should not render as text.
  *Acceptance:* a binary file shows "Binary file" with old and new sizes. A diff over 20,000 lines shows the first 2,000 with a "Load all" button. Neither case freezes the window for more than 100 ms.

### Branches and tags (BRANCH)

- **FR-BRANCH-01 [Must]** The app must show a sidebar listing local branches, remote branches grouped by remote, tags, and stashes, with the current branch marked and ahead/behind counts against its upstream.
  *Acceptance:* the lists match `git branch -a`, `git tag` and `git stash list`. Given a branch 2 ahead and 1 behind its upstream, the sidebar shows "2" up and "1" down. Clicking an entry selects its commit in the graph. A filter box narrows the lists by name.
- **FR-BRANCH-02 [Must]** The app must create, rename, or delete local branches.
  *Acceptance:* creating a branch from the sidebar or a commit's menu makes it at the chosen commit, with an option to check it out. Deleting a branch that is not fully merged needs a second confirmation that names the commits that would become unreachable. The current branch cannot be deleted.
- **FR-BRANCH-03 [Must]** The app must check out a local branch, a remote branch (creating a local tracking branch), or a commit (detached HEAD), and warn before overwriting local changes.
  *Acceptance:* given uncommitted changes that conflict with the target, when checkout is requested, then a dialog offers "Stash and switch" and "Cancel" and nothing is lost on either path. Given a detached HEAD, a banner says so and offers "Create branch here".
- **FR-BRANCH-04 [Must]** The app must create lightweight and annotated tags, delete tags, and push a tag to a remote.
  *Acceptance:* an annotated tag created in the app shows its message in `git show <tag>`. Deleting a tag removes it locally and offers to delete it on the remote.
- **FR-BRANCH-05 [Should]** The app should set or change a branch's upstream in one step.

### Merge and rebase (MERGE)

- **FR-MERGE-01 [Must]** The app must merge a chosen branch into the current one, fast-forwarding when possible, with a dialog that names both branches.
  *Acceptance:* the dialog reads "Merge feature into main". After confirming, the result equals `git merge feature`. An option forces a merge commit.
- **FR-MERGE-02 [Must]** The app must rebase the current branch onto a chosen branch, with three controls while it is in progress: continue, skip, abort.
  *Acceptance:* given a rebase that stops on a conflict, the in-progress banner (FR-MERGE-07) shows the step number of total, and each of the three buttons does what the matching `git rebase` flag does.
- **FR-MERGE-03 [Must]** The app must cherry-pick and revert a selected commit.
  *Acceptance:* cherry-picking a commit from another branch creates a new commit on the current branch with the same change. Reverting creates a commit that undoes it. For a merge commit the app asks which parent is the mainline.
- **FR-MERGE-04 [Must]** The app must reset the current branch to a chosen commit in soft, mixed, or hard mode, with each mode explained in one plain sentence.
  *Acceptance:* the dialog text says what happens to commits, staged changes and files for each mode. Hard mode states how many files have uncommitted changes that will be removed, and the result of each mode equals the matching `git reset` flag.
- **FR-MERGE-05 [Must]** The app must resolve conflicts in the app: list conflicted files, and for each conflict take the current side, take the incoming side, take both, or edit the result by hand, then mark the file resolved.
  *Acceptance:* given a file with two conflicts, when "current" is taken for the first and "both" for the second and the file is marked resolved, then the file on disk has no conflict markers, contains the chosen text, and is staged. Whole-file "take current" and "take incoming" also work, including for binary files.
- **FR-MERGE-06 [Should]** The app should predict conflicts before a merge or rebase starts, and list the files that would conflict.
- **FR-MERGE-07 [Must]** The app must show a banner whenever a merge, rebase, cherry-pick, or revert is in progress, including one started outside the app, with the operation's name, its progress, and Continue and Abort.
  *Acceptance:* given `git rebase` started in a terminal and stopped on a conflict, when the app is opened, then the banner is shown. Continue is disabled while any file is still conflicted. Abort returns the repository to the state before the operation.

### History rewriting (REWRITE)

- **FR-REWRITE-01 [Should]** The app should offer interactive rebase as a list of commits that can be reordered and marked pick, reword, edit, squash, fixup or drop, with a preview before anything changes.
- **FR-REWRITE-02 [Could]** The app could offer quick reword, squash-into-parent and drop from a commit's context menu.

### Undo and recovery (UNDO)

- **FR-UNDO-01 [Must]** The app must undo and redo, from the toolbar and with Cmd+Z / Cmd+Shift+Z, these actions made in the app: commit, amend, checkout, branch create / rename / delete, tag create / delete, merge, rebase, cherry-pick, revert, reset, discard, stash push, stash drop.
  *Acceptance:* for each listed action, performing it then pressing Undo returns HEAD, every branch and tag, the index and the working files to the state before it, and Redo applies it again. The undo button's tooltip names the action it will undo. If the repository changed outside the app since the action, Undo is disabled with a tooltip saying why.
- **FR-UNDO-02 [Must]** The app must take a snapshot before any action that can destroy work (hard reset, discard, rebase, force-push, branch delete, stash drop) and keep it for 30 days.
  *Acceptance:* after a hard reset that removed uncommitted changes and a restart of the app, a "Snapshots" list shows an entry for that reset with its time, and "Restore" brings the changes back. After a force-push, the previous remote commit is in the list and can be restored to a new branch.
- **FR-UNDO-03 [Should]** The app should show the reflog and create a branch from any entry.

### Stash (STASH)

- **FR-STASH-01 [Must]** The app must stash changes with an optional message and an option to include untracked files, and apply, pop, or drop a stash.
  *Acceptance:* stashes appear in the sidebar and as rows in the graph. Selecting one shows its diff. Pop on a stash that conflicts leaves the stash in place and opens the conflict view.
- **FR-STASH-02 [Should]** The app should create a branch from a stash.
- **FR-STASH-03 [Could]** The app could stash only selected files or lines.

### Remotes and sync (SYNC)

- **FR-SYNC-01 [Must]** The app must fetch (all remotes, with prune), pull, and push, each with progress and cancel. Pull mode is a setting: fast-forward only (default), merge, or rebase.
  *Acceptance:* after Fetch, remote branch labels match the remote. Pull in fast-forward-only mode on a diverged branch stops with a message offering merge or rebase. Push sends the current branch and reports the result as a toast.
- **FR-SYNC-02 [Must]** The app must force-push only with `--force-with-lease`, behind a confirmation whose default button is Cancel.
  *Acceptance:* given a rejected push, the app offers force-push. The dialog names the remote branch and the number of remote commits that will be replaced. If the remote moved since the last fetch, the push is refused and the app says to fetch first.
- **FR-SYNC-03 [Must]** The app must authenticate through the user's existing Git setup (credential helper, SSH agent), and when Git needs a username, password, token or passphrase, ask for it in an app dialog. The app never stores what is typed.
  *Acceptance:* given an HTTPS remote with no stored credential, when Fetch runs, then a dialog asks for username and password, and after entry the fetch completes. Given an SSH key with a passphrase and no agent, the dialog asks for the passphrase. No Git operation ever hangs waiting for terminal input.
- **FR-SYNC-04 [Must]** The app must list, add, rename, edit the URL of, and remove remotes.
  *Acceptance:* each action changes `git remote -v` to match. Removing a remote needs confirmation.
- **FR-SYNC-05 [Should]** The app should fetch in the background on an interval set in settings (off, 5, 15 or 60 minutes).
- **FR-SYNC-06 [Must]** The app must publish a new local branch by pushing it and setting its upstream in one step.
  *Acceptance:* given a branch with no upstream, Push asks which remote (if more than one), pushes, and afterwards the sidebar shows ahead/behind counts for the branch.

### Commands and keyboard (CMD)

- **FR-CMD-01 [Must]** The app must have a command palette, opened with Cmd+Shift+P, that fuzzy-searches every app command, and branches and tags by name (choosing one selects it in the graph).
  *Acceptance:* typing "pus" lists Push first. Typing a branch name lists that branch, and Enter scrolls the graph to it. Each command row shows its shortcut.
- **FR-CMD-02 [Must]** The app must have keyboard shortcuts for the core actions (the table in Stage 5), shown beside their menu items, and every Must feature must be reachable without a mouse.
  *Acceptance:* a tester can open a repository, stage a file, commit it, then push, using only the keyboard.
- **FR-CMD-03 [Should]** The app should let the user rebind any shortcut through a `keymap.json` file.
- **FR-CMD-04 [Must]** The app must provide a native macOS menu bar with these menus: File, Edit, View, Repository, Branch, Window, covering the same commands.
  *Acceptance:* every command in the palette is also in a menu, and standard items (Quit, Close Window, Copy, Paste, Undo) behave as on any Mac app.

### Settings and themes (SET)

- **FR-SET-01 [Must]** The app must have a settings window for: theme, UI font size, diff font and size, default diff view, pull mode, Git executable path, default clone folder. Changes apply at once and persist.
  *Acceptance:* changing each setting takes effect without restart and is still set after restart. Each setting has a "Reset to default" action.
- **FR-SET-02 [Must]** The app must ship a dark and a light theme and follow the system appearance by default.
  *Acceptance:* with theme set to "System", switching macOS between light and dark switches the app within 1 second.
- **FR-SET-03 [Should]** The app should ship a high-contrast theme and load user themes from JSON files.
- **FR-SET-04 [Should]** The app should zoom the whole interface from 50% to 300%.
- **FR-SET-05 [Should]** The app should show the Git identity (name and email) used for the open repository and let the user change it for that repository after a clear confirmation.

### Safety and privacy (SAFE)

- **FR-SAFE-01 [Must]** Every dialog for an action that can lose work must default to Cancel and state the consequence in plain words.
  *Acceptance:* in the dialogs for hard reset, discard, force-push, branch delete, stash drop, remote remove and abort, pressing Enter or Escape cancels, and the body text names what will be lost.
- **FR-SAFE-02 [Must]** The app must not change the user's global Git config, SSH folder or credential store, and must change a repository's config only through an explicit user action that says so.
  *Acceptance:* after a full test session, `~/.gitconfig` and `~/.ssh` are byte-identical to before. Adding a remote and setting an upstream are the only v1 actions that write repository config.
- **FR-SAFE-03 [Must]** The app must make no network connection except those Git itself makes to the user's remotes. No telemetry, no analytics, no update check, no crash upload.
  *Acceptance:* with a network monitor running, using every local feature produces zero outbound connections from the app.
- **FR-SAFE-04 [Must]** The app must treat a repository it did not create or clone as untrusted until the user trusts it, and in untrusted mode it must be read-only and must not run the repository's hooks or configured helper programs.
  *Acceptance:* given a repository with a `post-checkout` hook and a `core.fsmonitor` command that each write a marker file, when it is opened and browsed (graph, diffs, status) without being trusted, then no marker file is created, and every action that changes the repository is disabled with a "Trust this repository" prompt. After the user trusts it, all actions work, and the choice persists.

### App states (STATE)

- **FR-STATE-01 [Must]** The app must show a purposeful empty state wherever there is nothing to show: welcome screen with no recent repositories, a repository with no commits, no uncommitted changes, no search results, no stashes.
  *Acceptance:* each of the five states shows one sentence saying what is empty and the next action to take (for example "No commits yet. Stage files and make the first commit.").
- **FR-STATE-02 [Must]** The app must show progress for any operation that takes longer than 300 ms, and let the user cancel network operations.
  *Acceptance:* during a fetch, a progress indicator with the operation name is visible in the toolbar, the rest of the window stays usable for reading, and Cancel stops the Git process within 2 seconds.
- **FR-STATE-03 [Must]** The app must report every failed Git command with a plain-language summary and the exact Git output available on request. No failure is silent.
  *Acceptance:* given a push rejected by the remote, a toast says "Push rejected: the remote has commits you do not have. Fetch first." and "Details" shows the command line and Git's stderr.
- **FR-STATE-04 [Must]** The app must restore the previous session at launch: window size and position, the open repository, panel sizes, the selected diff view.
  *Acceptance:* after quit and relaunch, the same repository is open with the same layout.
- **FR-STATE-05 [Should]** The app should keep an activity log listing each Git command it ran, with time, duration, exit code and output, viewable in the app.
- **FR-STATE-06 [Should]** The app should show a toast when a background operation finishes or fails.

### Agents, AI and hosting (AGENT)

Added on 2026-10-13, after v1 scope was set, at the owner's request. These come after the MVP. None of them adds an account, a server or a stored secret: AI runs through a command-line agent already installed on the Mac, and hosting goes through the `gh` command.

- **FR-AGENT-01 [Should]** The app should offer an MCP server over stdio (`gittree mcp`), so a coding agent can read the repository and, when allowed, change it through the app's own Git path.
  *Acceptance:* an MCP client lists the tools and reads status, log, diff and blame. In an untrusted repository, or with access set to read only, every changing tool is refused. A commit made by an agent has a journal entry and can be undone in the app.
- **FR-AGENT-02 [Should]** The app should list every call an agent made, and ask before each change when that setting is on.
- **FR-AGENT-03 [Should]** The app should show blame for a file and the history of one file (this replaces FR-DIFF-05 and FR-DIFF-06 as the place they are built).
- **FR-AGENT-04 [Should]** The app should list, add and remove worktrees, and start an agent session in a new worktree on a new branch.
- **FR-AGENT-05 [Could]** The app could write a commit message, explain a commit, suggest a conflict result and plan commits from uncommitted work, by running a command-line agent the user has installed. Nothing is sent anywhere by the app itself.
- **FR-AGENT-06 [Could]** The app could list pull requests and issues and create a pull request, through the `gh` command the user has signed in to.

## Standard states

- Undo and redo: FR-UNDO-01, FR-UNDO-02.
- Empty states: FR-STATE-01.
- Loading and progress: FR-STATE-02, FR-GRAPH-02.
- Error states and messages: FR-STATE-03, FR-REPO-07.
- Confirm before destructive actions: FR-SAFE-01, FR-SYNC-02.
- Autosave or save-and-reopen: FR-STATE-04 for the session. The commit message draft is kept per repository until committed (part of FR-STAGE-05).
- Search and filter: FR-GRAPH-05, the sidebar filter in FR-BRANCH-01, FR-CMD-01.
- Keyboard shortcuts: FR-CMD-02, FR-CMD-03.
- Settings that persist: FR-SET-01.
- First-run and onboarding: FR-REPO-07 and the welcome screen (FR-REPO-04, FR-STATE-01). No tutorial: out of scope, the audience knows Git.
- Offline or bad-connection behaviour: NFR-03, FR-STATE-02 (cancel), FR-STATE-03 (errors).
- Permissions per role: out of scope, single user. The trust level per repository is FR-SAFE-04.
- Notifications: in-app toasts only (FR-STATE-06). System notifications are out of scope for v1.
- Import and export: out of scope. Patches (create and apply) are deferred to v2.
- Delete and restore: FR-UNDO-02 (snapshots), FR-STAGE-04 (discard backup).
- Audit log: FR-STATE-05.
- Multi-language and RTL: out of scope for v1 (English only). Strings are externalised under NFR-06.
- Accessibility: NFR-06.
- Mobile layout: out of scope, desktop app. Minimum window size is 900 by 600.
- Rate limits and quotas: out of scope, no service is called.

## Account and auth

None. The app has no accounts, no sign-in and no server. Authentication to Git remotes is handled by Git and the user's existing credential helper or SSH agent (FR-SYNC-03).

## Data the product handles

- Repository: a Git repository on the user's disk. Owned by the user. The app reads it and changes it only through Git. Never copied or uploaded.
- Commit: read from Git and held in memory while the repository is open. Not persisted by the app.
- Ref: branches, remote branches, tags, HEAD. Read from Git, held in memory.
- Working status: the staged and unstaged file list. Read from Git, refreshed on change.
- Diff: file changes split into hunks and lines. Computed by Git on demand, cached in memory for the selected item only.
- Stash: read from Git.
- Remote: name and URLs, read from the repository's Git config.
- Operation in progress: a merge, rebase, cherry-pick or revert that has not finished. Read from the `.git` folder.
- Conflict: conflicted files and their conflict regions, parsed from the working files.
- Journal entry: a record of each action the app performed, with the before and after state needed for undo. Stored inside the repository's `.git` folder. Kept 30 days or 200 entries, whichever is smaller.
- Snapshot: the saved state behind a journal entry: Git objects held by private refs, plus backup copies of untracked files. Same location and retention as the journal.
- Settings: user preferences. One JSON file in the app's data folder. Kept until the user deletes it.
- Session: window geometry, open repository, panel layout. One JSON file in the app's data folder.
- Recent repositories: paths and last-opened times, up to 20.
- Trust list: paths of repositories the user trusted.
- Activity log: the Git commands run, in memory for the session (last 500) and written to a log file capped at 5 MB.

## Integrations

- System Git: every read and write of repository data goes through the `git` executable, version 2.39 or newer. Direction: both. Auth: none.
- Git credential helper and SSH agent: used by Git itself for remotes. The app only relays prompts (FR-SYNC-03).
- macOS: file dialogs, menu bar, light/dark appearance, file-system change events.

No web service, hosting provider or AI model is called in v1.

## Non-functional requirements

- **NFR-01 Performance:** on the reference machine with the big repo: cold start to a usable window under 500 ms; first screen of the graph under 1 second and the full graph loaded under 5 seconds; scrolling at 60 frames per second (95th-percentile frame under 16 ms); status refresh under 300 ms in a working tree of 10,000 files; a 5,000-line diff displayed under 200 ms; memory under 400 MB with the big repo open.
- **NFR-02 Privacy and security:** repository content never leaves the machine through the app. No secrets are stored or logged. No network connections other than Git's own. Untrusted repositories cannot run code (FR-SAFE-04).
- **NFR-03 Availability / offline:** every feature except the four network operations (clone, fetch, pull, push) works with no network. A network failure produces an error within Git's own timeout and never freezes the window.
- **NFR-04 Scale:** 100,000 commits, 5,000 refs, 50,000 tracked files, 500 changed files in the working tree, single diffs up to 20,000 lines.
- **NFR-05 Platforms:** macOS 13 or later on Apple silicon and Intel for v1. Code that is specific to macOS lives in one module, so Windows and Linux builds can follow in v1.1.
- **NFR-06 Accessibility and locale:** English. All user-facing strings come from a locale file, none are hard-coded in views. Every Must feature works by keyboard. Text contrast is at least 4.5:1 in built-in themes. The system "reduce motion" setting turns off animations.
- **NFR-07 Footprint:** the app bundle is under 40 MB and contains no web view or browser runtime.
- **NFR-08 Git fidelity:** every change to a repository is made by the installed Git, so the result of any app action equals the result of the matching Git command, and hooks, filters, commit signing all behave as on the command line (in trusted repositories).
- **NFR-09 Reliability:** no app action loses committed or uncommitted work without a snapshot (FR-UNDO-02). A crash of the app never leaves the repository in a state Git itself cannot continue from.

## Out of scope for MVP

- **Hosting integrations** (GitHub, GitLab, Bitbucket, Azure DevOps: browse, pull requests, issues, CI status) : need accounts and four provider APIs. v2.
- **AI assistant** (chat, agent mode, generated commit messages) : a separate product surface. v2 at the earliest.
- Built-in terminal: a terminal emulator is a large subsystem and every user already has one. v2.
- User accounts and sign-in: nothing in v1 needs them. Only if hosting integrations arrive.
- Arabic and right-to-left layout: needs per-screen layout review in the UI kit. v1.1 or v2.
- Windows and Linux builds: v1.1.
- Worktrees, submodules, Git LFS management, bisect, Git Flow, patches: each works from the terminal meanwhile, and the app must not break repositories that use them. v2, in that rough order.
- Partial clone options (shallow, sparse, blobless): v2. Repositories cloned that way elsewhere still open.
- Profiles for multiple identities, SSH key management, external diff and merge tools, custom commands: v2.
- Auto-update: conflicts with FR-SAFE-03 unless opt-in. Decide with the first public release.
- System notifications, onboarding tutorial, mobile layout: not planned.

---

### Open questions

None.

### Assumptions confirmed

Defaults taken under the blanket sign-off on 2026-10-12.

1. **v1 cut line : RESOLVED.** 54 Must requirements covering the daily local workflow. Interactive rebase, blame, file history, compare, tabs, reflog: all are Should and come straight after the MVP.
2. **Undo scope : RESOLVED.** Local actions only. Push and fetch are not undoable. A force-push keeps the old remote commit as a snapshot instead.
3. **Credentials : RESOLVED.** The app stores no secrets in v1 and relies on the user's Git credential helper, so no keychain integration is needed yet.
4. **Trust model : RESOLVED.** Repositories the app cloned or initialised are trusted automatically. Any other repository opens read-only until trusted.
5. **Performance baseline : RESOLVED.** M1 MacBook with 16 GB and a 100,000-commit synthetic repository.
6. **Minimum Git : RESOLVED.** 2.39, which every supported macOS can install and which has the commands the app relies on.
