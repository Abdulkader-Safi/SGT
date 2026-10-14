//! Branches, tags, checkout and stash against real Git, each with its undo.

mod common;

use common::TestRepo;
use gittree_core::GitErrorKind;
use gittree_core::journal::{Action, Journal, JournalEntry};
use gittree_core::ops::branch::{self, Target};
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

/// Runs an action the way the app does: journal entry around it.
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

fn undo(repo: &TestRepo, entry: &JournalEntry) {
    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    assert_eq!(journal.can_undo(&runner, entry), Ok(()));
    journal.undo(&runner, entry).unwrap();
}

fn refs(repo: &TestRepo) -> String {
    repo.git(1, &["for-each-ref", "--format=%(refname) %(objectname)"])
}

fn head(repo: &TestRepo) -> String {
    repo.git(1, &["rev-parse", "--abbrev-ref", "HEAD"])
}

#[test]
fn a_branch_can_be_created_renamed_and_deleted_and_each_is_undone() {
    let repo = repo();
    let before = refs(&repo);

    let create = Action::CreateBranch {
        name: "feature/x".into(),
        at: "HEAD~1".into(),
        checkout: true,
    };
    let created = act(&repo, create, |r| {
        branch::create_branch(r, "feature/x", "HEAD~1", true)
    });
    assert_eq!(head(&repo), "feature/x");
    assert_eq!(repo.git(1, &["log", "-1", "--format=%s"]), "first");
    undo(&repo, &created);
    assert_eq!((head(&repo), refs(&repo)), ("main".into(), before.clone()));

    branch::create_branch(&repo.runner(true), "old", "HEAD", false).unwrap();
    let with_old = refs(&repo);
    let rename = Action::RenameBranch {
        old: "old".into(),
        new: "new".into(),
    };
    let renamed = act(&repo, rename, |r| branch::rename_branch(r, "old", "new"));
    assert!(refs(&repo).contains("refs/heads/new") && !refs(&repo).contains("refs/heads/old"));
    undo(&repo, &renamed);
    assert_eq!(refs(&repo), with_old);

    let delete = Action::DeleteBranch {
        name: "old".into(),
        force: false,
    };
    let deleted = act(&repo, delete, |r| branch::delete_branch(r, "old", false));
    assert_eq!(refs(&repo), before);
    undo(&repo, &deleted);
    assert_eq!(refs(&repo), with_old);
}

#[test]
fn an_unmerged_branch_names_its_commits_and_needs_force() {
    let repo = repo();
    repo.git(1, &["switch", "-q", "-c", "side"]);
    repo.write("b.txt", b"b\n");
    repo.commit_all("only on side");
    repo.git(1, &["switch", "-q", "main"]);
    let runner = repo.runner(true);

    let lost = branch::unmerged_commits(&runner, "side").unwrap();
    assert_eq!(lost.len(), 1);
    assert_eq!(lost[0].1, "only on side");
    assert!(
        branch::unmerged_commits(&runner, "main")
            .unwrap()
            .is_empty()
    );

    assert!(branch::delete_branch(&runner, "side", false).is_err());
    // The current branch is never deleted.
    assert!(branch::delete_branch(&runner, "main", true).is_err());
    let before = refs(&repo);
    let delete = Action::DeleteBranch {
        name: "side".into(),
        force: true,
    };
    let deleted = act(&repo, delete, |r| branch::delete_branch(r, "side", true));
    assert!(!refs(&repo).contains("side"));
    undo(&repo, &deleted);
    assert_eq!(refs(&repo), before);
}

#[test]
fn checkout_moves_to_branches_remote_branches_and_commits() {
    let repo = repo();
    repo.git(1, &["branch", "other", "HEAD~1"]);
    repo.git(1, &["update-ref", "refs/remotes/origin/topic", "HEAD~1"]);
    repo.git(1, &["config", "remote.origin.url", "/nowhere"]);
    repo.git(
        1,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let checkout = |target: Target| {
        let action = Action::Checkout {
            target: format!("{target:?}"),
        };
        act(&repo, action, |r| branch::checkout(r, &target))
    };

    let to_branch = checkout(Target::Branch("other".into()));
    assert_eq!(head(&repo), "other");
    undo(&repo, &to_branch);
    assert_eq!(head(&repo), "main");

    let first = repo.git(1, &["rev-parse", "HEAD~1"]);
    let detached = checkout(Target::Commit(first.as_str().into()));
    assert_eq!(head(&repo), "HEAD");
    assert_eq!(repo.git(1, &["rev-parse", "HEAD"]), first);
    undo(&repo, &detached);
    assert_eq!(head(&repo), "main");

    checkout(Target::Remote("origin/topic".into()));
    assert_eq!(head(&repo), "topic");
    assert_eq!(
        repo.git(1, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]),
        "origin/topic"
    );
}

#[test]
fn checkout_refuses_to_overwrite_local_changes() {
    let repo = repo();
    repo.git(1, &["branch", "other", "HEAD~1"]);
    repo.write("a.txt", b"mine\n");
    let error = branch::checkout(&repo.runner(true), &Target::Branch("other".into())).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::DirtyWorkTree);
    assert_eq!(repo.read("a.txt"), b"mine\n");
}

#[test]
fn tags_are_created_plain_or_annotated_and_deleted() {
    let repo = repo();
    let before = refs(&repo);
    let plain = Action::CreateTag {
        name: "v1".into(),
        at: "HEAD".into(),
        message: None,
    };
    let created = act(&repo, plain, |r| branch::create_tag(r, "v1", "HEAD", None));
    assert_eq!(repo.git(1, &["cat-file", "-t", "v1"]), "commit");
    undo(&repo, &created);
    assert_eq!(refs(&repo), before);

    branch::create_tag(&repo.runner(true), "v2", "HEAD~1", Some("--not an option")).unwrap();
    assert_eq!(repo.git(1, &["cat-file", "-t", "v2"]), "tag");
    assert_eq!(
        repo.git(1, &["tag", "-l", "--format=%(contents:subject)", "v2"]),
        "--not an option"
    );
    let with_tag = refs(&repo);
    let delete = Action::DeleteTag { name: "v2".into() };
    let deleted = act(&repo, delete, |r| branch::delete_tag(r, "v2"));
    assert_eq!(refs(&repo), before);
    // The annotated tag object comes back, not a plain tag.
    undo(&repo, &deleted);
    assert_eq!(refs(&repo), with_tag);
}

#[test]
fn a_stash_is_pushed_listed_applied_and_dropped() {
    let repo = repo();
    repo.write("a.txt", b"changed\n");
    repo.write("new.txt", b"new\n");
    let push = Action::StashPush {
        message: "wip".into(),
        include_untracked: true,
    };
    let pushed = act(&repo, push, |r| branch::stash_push(r, "wip", true));
    assert_eq!(repo.git(1, &["status", "--porcelain"]), "");
    let runner = repo.runner(true);
    let stashes = branch::list_stashes(&runner).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.ends_with("wip"));
    assert_eq!(&*stashes[0].base, repo.git(1, &["rev-parse", "HEAD"]));

    // Undo of the push pops it.
    undo(&repo, &pushed);
    assert_eq!(
        repo.git(1, &["status", "--porcelain"]),
        " M a.txt\n?? new.txt"
    );
    assert!(branch::list_stashes(&runner).unwrap().is_empty());

    branch::stash_push(&runner, "", true).unwrap();
    branch::stash_apply(&runner, 0).unwrap();
    assert_eq!(repo.read("a.txt"), b"changed\n");
    let stash = branch::list_stashes(&runner).unwrap().remove(0);
    let drop = Action::StashDrop {
        oid: stash.oid.clone(),
        message: stash.message.clone(),
    };
    let dropped = act(&repo, drop, |r| branch::stash_drop(r, 0));
    assert!(branch::list_stashes(&runner).unwrap().is_empty());
    undo(&repo, &dropped);
    assert_eq!(branch::list_stashes(&runner).unwrap()[0].oid, stash.oid);
}

#[test]
fn bad_names_and_untrusted_repositories_are_refused() {
    let repo = repo();
    let runner = repo.runner(true);
    for name in ["-x", "", "a..b", "with space", "end.lock"] {
        assert!(
            branch::create_branch(&runner, name, "HEAD", false).is_err(),
            "{name:?}"
        );
        assert!(
            branch::create_tag(&runner, name, "HEAD", None).is_err(),
            "{name:?}"
        );
    }
    assert!(branch::create_branch(&runner, "ok", "--orphan", false).is_err());

    let untrusted = repo.runner(false);
    let kind = |result: Result<(), GitError>| result.unwrap_err().kind;
    assert_eq!(
        kind(branch::create_branch(&untrusted, "x", "HEAD", false)),
        GitErrorKind::Untrusted
    );
    assert_eq!(
        kind(branch::stash_push(&untrusted, "", false)),
        GitErrorKind::Untrusted
    );
    assert_eq!(
        kind(branch::create_tag(&untrusted, "t", "HEAD", None)),
        GitErrorKind::Untrusted
    );
    assert_eq!(
        kind(branch::checkout(
            &untrusted,
            &Target::Commit("HEAD~1".into())
        )),
        GitErrorKind::Untrusted
    );
    assert_eq!(head(&repo), "main");
}

#[test]
fn a_stash_becomes_a_branch_and_an_upstream_can_be_set_and_unset() {
    let repo = repo();
    let runner = repo.runner(true);
    repo.write("a.txt", b"stashed\n");
    branch::stash_push(&runner, "wip", false).unwrap();
    repo.write("a.txt", b"two\n");
    // main moves on, so the branch must start where the stash was made.
    repo.write("b.txt", b"b\n");
    repo.commit_all("third");

    branch::stash_branch(&runner, "from-stash", 0).unwrap();
    assert_eq!(
        repo.git(1, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "from-stash"
    );
    assert_eq!(repo.read("a.txt"), b"stashed\n");
    assert_eq!(
        repo.git(1, &["rev-parse", "HEAD"]),
        repo.git(1, &["rev-parse", "main~1"])
    );
    assert!(branch::list_stashes(&runner).unwrap().is_empty());
    assert!(branch::stash_branch(&runner, "-x", 0).is_err());

    branch::set_upstream(&runner, "from-stash", Some("main")).unwrap();
    assert_eq!(
        repo.git(1, &["rev-parse", "--abbrev-ref", "from-stash@{upstream}"]),
        "main"
    );
    branch::set_upstream(&runner, "from-stash", None).unwrap();
    assert!(
        repo.runner(true)
            .read(["rev-parse", "--abbrev-ref", "from-stash@{upstream}"])
            .is_err()
    );
    assert!(branch::set_upstream(&runner, "from-stash", Some("--x")).is_err());
    assert!(branch::set_upstream(&repo.runner(false), "from-stash", Some("main")).is_err());
}
