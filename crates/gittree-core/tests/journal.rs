//! Undo then redo for each kind of action, the order of the two stacks,
//! snapshots and pruning.

mod common;

use common::TestRepo;
use gittree_core::journal::{Action, Blocked, Journal, JournalEntry};
use gittree_core::ops::merge::ResetMode;
use gittree_core::ops::{branch, merge, stage};
use gittree_core::{GitError, GitRunner};

fn repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    repo.write("a.txt", b"one\n");
    repo.commit_all("first");
    repo.write("a.txt", b"two\n");
    repo.commit_all("second");
    repo
}

/// Refs, HEAD, the index and the files on disk, as one string.
fn state(repo: &TestRepo) -> String {
    [
        repo.git(
            1,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
                "refs/tags",
                "refs/stash",
            ],
        ),
        repo.git(1, &["rev-parse", "--abbrev-ref", "HEAD"]),
        repo.git(1, &["status", "--porcelain"]),
        repo.git(1, &["ls-files", "-s"]),
        String::from_utf8_lossy(&repo.read("a.txt")).into_owned(),
    ]
    .join("\n--\n")
}

fn act(
    repo: &TestRepo,
    action: Action,
    run: impl FnOnce(&GitRunner) -> Result<(), GitError>,
) -> JournalEntry {
    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    let pending = journal.begin(&runner, action, "test".into()).unwrap();
    run(&runner).unwrap();
    journal.finish(&runner, pending).unwrap()
}

/// Runs the action, then checks that undo returns to the start and redo to
/// the state after it.
fn round_trip(
    name: &str,
    repo: &TestRepo,
    action: Action,
    run: impl FnOnce(&GitRunner) -> Result<(), GitError>,
) {
    let runner = repo.runner(true);
    let dir = repo.path().join(".git");
    let journal = Journal::new(&dir);
    let before = state(repo);
    act(repo, action, run);
    let after = state(repo);
    assert_ne!(before, after, "{name}: the action changed nothing");

    let entry = journal.next_undo().unwrap();
    assert_eq!(journal.can_undo(&runner, &entry), Ok(()), "{name}");
    journal.undo(&runner, &entry).unwrap();
    assert_eq!(state(repo), before, "{name}: after undo");

    let entry = journal.next_redo().expect("an undone entry can be redone");
    assert!(journal.can_redo(&runner, &entry), "{name}");
    journal.redo(&runner, &dir, &entry).unwrap();
    assert_eq!(state(repo), after, "{name}: after redo");
    assert!(journal.next_redo().is_none());
}

#[test]
fn every_action_survives_undo_then_redo() {
    let r = repo();
    r.write("a.txt", b"three\n");
    r.git(1, &["add", "a.txt"]);
    let commit = Action::Commit {
        message: "third".into(),
        amend: false,
    };
    round_trip("commit", &r, commit, |g| {
        stage::commit(g, "third", false).map(drop)
    });

    let create = Action::CreateBranch {
        name: "topic".into(),
        at: "HEAD~1".into(),
        checkout: true,
    };
    round_trip("create branch", &r, create, |g| {
        branch::create_branch(g, "topic", "HEAD~1", true)
    });

    let checkout = Action::Checkout {
        target: "main".into(),
    };
    round_trip("checkout", &r, checkout, |g| {
        branch::checkout(g, &branch::Target::Branch("main".into()))
    });

    let rename = Action::RenameBranch {
        old: "topic".into(),
        new: "renamed".into(),
    };
    round_trip("rename", &r, rename, |g| {
        branch::rename_branch(g, "topic", "renamed")
    });

    let delete = Action::DeleteBranch {
        name: "renamed".into(),
        force: true,
    };
    round_trip("delete branch", &r, delete, |g| {
        branch::delete_branch(g, "renamed", true)
    });

    let tag = Action::CreateTag {
        name: "v1".into(),
        at: "HEAD".into(),
        message: None,
    };
    round_trip("tag", &r, tag, |g| {
        branch::create_tag(g, "v1", "HEAD", None)
    });
    round_trip(
        "delete tag",
        &r,
        Action::DeleteTag { name: "v1".into() },
        |g| branch::delete_tag(g, "v1"),
    );

    let target = r.git(1, &["rev-parse", "HEAD~1"]);
    let reset = Action::Reset {
        target: target.clone(),
        mode: ResetMode::Hard,
    };
    round_trip("reset", &r, reset, |g| {
        merge::reset(g, &target, ResetMode::Hard)
    });

    r.write("a.txt", b"edited\n");
    let discard = Action::DiscardFiles {
        tracked: vec!["a.txt".into()],
        untracked: Vec::new(),
    };
    round_trip("discard", &r, discard, |g| {
        stage::discard_files(g, &["a.txt".into()], &[], true)
    });
}

#[test]
fn undo_and_redo_keep_their_order_and_a_new_action_ends_redo() {
    let r = repo();
    let (runner, dir) = (r.runner(true), r.path().join(".git"));
    let journal = Journal::new(&dir);
    for name in ["a", "b", "c"] {
        let action = Action::CreateBranch {
            name: name.into(),
            at: "HEAD".into(),
            checkout: false,
        };
        act(&r, action, |g| {
            branch::create_branch(g, name, "HEAD", false)
        });
    }
    let name = |entry: Option<JournalEntry>| match entry.map(|e| e.action) {
        Some(Action::CreateBranch { name, .. }) => name,
        _ => "none".to_owned(),
    };
    // Undo takes the newest first.
    for expected in ["c", "b"] {
        let entry = journal.next_undo().unwrap();
        assert_eq!(name(Some(entry.clone())), expected);
        journal.undo(&runner, &entry).unwrap();
    }
    // Redo brings them back in the order they happened.
    assert_eq!(name(journal.next_redo()), "b");
    let entry = journal.next_redo().unwrap();
    journal.redo(&runner, &dir, &entry).unwrap();
    assert_eq!(name(journal.next_redo()), "c");
    assert_eq!(name(journal.next_undo()), "b");

    // Something new happens: "c" can no longer be redone.
    let action = Action::CreateBranch {
        name: "d".into(),
        at: "HEAD".into(),
        checkout: false,
    };
    act(&r, action, |g| branch::create_branch(g, "d", "HEAD", false));
    assert!(journal.next_redo().is_none());
    assert_eq!(name(journal.next_undo()), "d");
}

#[test]
fn undo_is_refused_once_the_repository_moved() {
    let r = repo();
    r.write("a.txt", b"three\n");
    r.git(1, &["add", "a.txt"]);
    let commit = Action::Commit {
        message: "third".into(),
        amend: false,
    };
    let entry = act(&r, commit, |g| stage::commit(g, "third", false).map(drop));
    r.write("b.txt", b"b\n");
    r.commit_all("made in a terminal");
    let journal = Journal::new(&r.path().join(".git"));
    assert_eq!(
        journal.can_undo(&r.runner(true), &entry),
        Err(Blocked::RepositoryChanged)
    );
}

#[test]
fn a_snapshot_restores_in_place_or_as_a_branch() {
    let r = repo();
    let runner = r.runner(true);
    let journal = Journal::new(&r.path().join(".git"));
    r.write("a.txt", b"work in progress\n");
    let target = r.git(1, &["rev-parse", "HEAD~1"]);
    let reset = Action::Reset {
        target: target.clone(),
        mode: ResetMode::Hard,
    };
    act(&r, reset, |g| merge::reset(g, &target, ResetMode::Hard));
    assert_eq!(r.read("a.txt"), b"one\n");

    let snapshots = journal.snapshots();
    assert_eq!(snapshots.len(), 1);
    journal
        .restore(&runner, &snapshots[0], Some("rescued"))
        .unwrap();
    assert_eq!(
        r.git_bytes(&["show", "rescued:a.txt"]),
        b"work in progress\n"
    );
    // The working tree was left alone.
    assert_eq!(r.read("a.txt"), b"one\n");

    // The commit a force-push replaced can only come back as a branch.
    let replaced = r.git(1, &["rev-parse", "HEAD"]);
    let push = Action::ForcePush {
        remote: "origin".into(),
        branch: "main".into(),
        replaced: replaced.as_str().into(),
    };
    let entry = act(&r, push, |_| Ok(()));
    assert_eq!(journal.can_undo(&runner, &entry), Err(Blocked::NotUndoable));
    journal
        .restore(&runner, &entry, Some("was-on-origin"))
        .unwrap();
    assert_eq!(r.git(1, &["rev-parse", "was-on-origin"]), replaced);
}

#[test]
fn prune_drops_old_entries_with_their_refs_and_backups() {
    let r = repo();
    let runner = r.runner(true);
    let dir = r.path().join(".git");
    let journal = Journal::new(&dir);
    r.write("a.txt", b"edited\n");
    let target = r.git(1, &["rev-parse", "HEAD~1"]);
    let reset = Action::Reset {
        target: target.clone(),
        mode: ResetMode::Hard,
    };
    let old = act(&r, reset, |g| merge::reset(g, &target, ResetMode::Hard));
    let snapshot_ref = format!("refs/gittree/snapshots/{}", old.id);
    assert!(!r.git(1, &["for-each-ref", &snapshot_ref]).is_empty());

    // Within 30 days nothing goes.
    assert_eq!(journal.prune(&runner, old.time + 29 * 86_400).unwrap(), 0);
    assert_eq!(journal.prune(&runner, old.time + 31 * 86_400).unwrap(), 1);
    assert!(journal.entries().is_empty());
    assert_eq!(r.git(1, &["for-each-ref", &snapshot_ref]), "");
}
