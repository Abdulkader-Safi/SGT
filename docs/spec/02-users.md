# Users and usage

Status: signed off 2026-10-12

## Personas

### Primary: "Sami," the multi-repo developer

- Background: full-stack developer and freelancer. Works in six to ten repositories a week across his own products and client work, some with 50,000+ commits.
- What drives him: speed and control. He wants to see the shape of a repository at a glance and make small, clean commits. He dislikes paying a subscription for a tool that wraps free software.
- Current workaround: the terminal for most things, plus the Git panel in his editor for staging. For history questions he pipes `git log --graph` through a pager. Untangling a bad rebase costs him 20 to 30 minutes a few times a month.
- Tech comfort: high. Installs from a DMG or builds from source, edits JSON config files, knows what a rebase is.
- Relationship to Git safety: "let me do dangerous things, but let me take them back."
- Devices and environment: MacBook with Apple silicon, 16 GB, macOS 14 or 15, external monitor. Git from Homebrew or Xcode. Often on hotel or mobile connections, so local operations must not wait on the network. English interface.

### Secondary: "Layla," the Git-cautious developer

- Background: two years into her first developer job. Comfortable with committing and pushing. Avoids rebase and has lost work to `git reset --hard` once.
- What drives her: not breaking things in front of the team. She wants to understand what an action will do before she does it.
- Current workaround: GitHub Desktop for everyday work, and asking a senior colleague when a merge conflicts.
- Tech comfort: medium. Installs apps, will not edit config files.
- Relationship to Git safety: "tell me in plain words what this button does."
- Devices and environment: company MacBook, macOS 14, English interface, Arabic as first language.

She matters because plain-language dialogs and undo make the app safer for everyone. She is not the beachhead because the first user is the builder, and Sami's needs (speed, scale, line staging) are the ones that decide the architecture.

### Not the target

- Teams that want code review, pull requests and CI status inside the Git client. They should use the browser or a client with hosting integrations.
- People who want an AI agent to write commits or resolve conflicts.
- Non-developers managing documents with Git.
- Anyone on Windows or Linux in v1.

The app will not add accounts, cloud sync or review features to please these groups.

## Roles and permissions

Single user. No accounts and no roles. The only permission concept is per repository: trusted or untrusted (see FR-SAFE-04).

## Jobs to be done

- When **Sami opens a repository he has not touched in a month**, he wants to **see which branches moved and where his work sits**, so he can **decide what to pull or rebase**.
- When **Sami has mixed two unrelated fixes in one file**, he wants to **stage only the lines for the first fix**, so he can **make two clean commits**.
- When **Sami's pull brings a conflict**, he wants to **see both sides and pick or edit per conflict**, so he can **finish the merge without opening an editor and hunting for markers**.
- When **Sami runs the wrong reset or discards the wrong file**, he wants to **undo it in one step**, so he can **carry on without digging in the reflog**.
- When **Sami needs to find the commit that touched a function**, he wants to **search by message, author, hash or file**, so he can **read the diff and its context in the graph**.
- When **Layla is about to force-push or hard-reset**, she wants to **read what will happen in plain words**, so she can **cancel if it is not what she meant**.
- When **Sami switches tasks mid-change**, he wants to **stash with a name and switch branch**, so he can **come back later without a throwaway commit**.
- When **Sami receives a repository as a zip from a client**, he wants to **browse it without its hooks or config running code**, so he can **inspect it safely first**.
- When **Sami wants a history clean-up before a pull request** (edge job, Should), he wants to **reorder and squash commits in a list**, so he can **push a tidy branch**.

## Primary user journey

1. **Discovery:** the GitHub repository, found through a post about GPUI apps or a "Git clients" list. The pitch that lands: "native Git client, opens 100,000 commits in a second, free."
2. **Setup:** download the app (or `cargo run --release` from source). No account and no wizard. On first launch the app checks that Git 2.39 or newer is installed. Under one minute.
3. **First use:** the welcome screen offers three actions: Open, Clone, Init. Sami opens an existing repository and the graph appears.
4. **Aha moment:** he scrolls a huge history with no stutter, clicks the work-in-progress row, selects three lines in a diff and presses Stage.
5. **Optional power-up:** the command palette and keyboard shortcuts. After a week he rarely touches the toolbar.
6. **Finish:** work leaves the app as a push to the remote. Nothing is exported in another form.
7. **Ongoing use:** the app stays open all day. Recent repositories reopen from the welcome screen and the last session's window is restored.

## Core loop

**The repeated action:** look at the graph -> change code elsewhere -> review the diff -> stage lines or files -> commit -> push.

**What pulls users back:**

- It is already open and already current: the file watcher refreshes status within a second of saving in the editor.
- The graph answers "where am I?" faster than any command.
- Undo means experiments with rebase and reset are cheap.

## Language and accessibility

- English only in v1. Every user-facing string goes through the i18n layer with keys in `locales/en.yml`, so Arabic can be added later.
- Right-to-left layout is out of scope for v1 (see 01, assumption 4).
- Dates show in the system locale's format and local time zone. Relative dates ("3 hours ago") for commits newer than 7 days.
- Accessibility target: every Must feature reachable by keyboard, text contrast of at least 4.5:1 in the built-in themes, and the system "reduce motion" setting honoured. Full screen-reader support for the graph is a Should.
- Fonts: system UI font for chrome, a monospace font (default: the system monospace, configurable) for diffs and hashes.

---

### Open questions

None.

### Assumptions confirmed

Defaults taken under the blanket sign-off on 2026-10-12.

1. **Primary persona : RESOLVED.** Modelled on the builder: an experienced developer on macOS with many repositories.
2. **Roles : RESOLVED.** Single user, no accounts.
3. **Languages : RESOLVED.** English only in v1, strings externalised for later translation.
4. **Accessibility level : RESOLVED.** Keyboard access and contrast are Musts. Screen-reader labelling of the custom graph is a Should.
