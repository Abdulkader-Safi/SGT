//! Remotes and the network commands, against a bare repository on disk.

mod common;

use std::sync::atomic::AtomicBool;

use common::TestRepo;
use gittree_core::ops::remote::{self, NetEnv};
use gittree_core::store::PullMode;
use gittree_core::{GitErrorKind, GitRunner};

struct Setup {
    /// The bare repository that plays the server.
    origin: tempfile::TempDir,
    repo: TestRepo,
}

fn setup() -> Setup {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    repo.write("a.txt", b"one\n");
    repo.commit_all("first");
    let origin = tempfile::tempdir().unwrap();
    repo.git(
        1,
        &["init", "-q", "--bare", "-b", "main", origin.path().to_str().unwrap()],
    );
    Setup { origin, repo }
}

/// A commit made by someone else, pushed straight to the server.
fn pushed_by_someone_else(setup: &Setup, file: &str) {
    let other = tempfile::tempdir().unwrap();
    let url = setup.origin.path().to_str().unwrap();
    let dir = other.path().to_str().unwrap();
    setup.repo.git(1, &["clone", "-q", url, dir]);
    std::fs::write(other.path().join(file), "theirs\n").unwrap();
    for args in [
        vec!["-C", dir, "add", "-A"],
        vec![
            "-C",
            dir,
            "-c",
            "user.name=Bo",
            "-c",
            "user.email=bo@example.com",
            "commit",
            "-q",
            "-m",
            "theirs",
        ],
        vec!["-C", dir, "push", "-q", "origin", "main"],
    ] {
        setup.repo.git(2, &args);
    }
}

const NO_CANCEL: AtomicBool = AtomicBool::new(false);

#[test]
fn remotes_are_added_renamed_edited_and_removed() {
    let s = setup();
    let runner = s.repo.runner(true);
    let url = s.origin.path().to_str().unwrap();
    remote::add_remote(&runner, "origin", url).unwrap();
    let list = remote::list_remotes(&runner).unwrap();
    assert_eq!(
        (list[0].name.as_str(), list[0].fetch_url.as_str()),
        ("origin", url)
    );
    assert_eq!(list[0].push_url, url);

    remote::rename_remote(&runner, "origin", "upstream").unwrap();
    remote::set_remote_url(&runner, "upstream", "https://example.com/x.git").unwrap();
    assert_eq!(
        s.repo.git(1, &["remote", "-v"]).lines().next().unwrap(),
        "upstream\thttps://example.com/x.git (fetch)"
    );
    remote::remove_remote(&runner, "upstream").unwrap();
    assert!(remote::list_remotes(&runner).unwrap().is_empty());

    assert!(remote::add_remote(&runner, "bad", "ext::sh -c 'touch x'").is_err());
    assert!(remote::add_remote(&runner, "-bad", url).is_err());
    assert!(remote::add_remote(&s.repo.runner(false), "o", url).is_err());
}

#[test]
fn push_publishes_then_fetch_and_pull_bring_in_new_commits() {
    let s = setup();
    let runner = s.repo.runner(true);
    let env = NetEnv::new();
    remote::add_remote(&runner, "origin", s.origin.path().to_str().unwrap()).unwrap();

    // FR-SYNC-06: one step publishes and sets the upstream.
    remote::push(
        &runner,
        "origin",
        "main",
        true,
        None,
        &env,
        &NO_CANCEL,
        |_| {},
    )
    .unwrap();
    assert_eq!(
        s.repo
            .git(1, &["rev-parse", "--abbrev-ref", "main@{upstream}"]),
        "origin/main"
    );

    pushed_by_someone_else(&s, "b.txt");
    remote::fetch(&runner, &env, &NO_CANCEL, |_| {}).unwrap();
    assert_eq!(
        s.repo.git(1, &["rev-list", "--count", "main..origin/main"]),
        "1"
    );
    remote::pull(&runner, PullMode::FfOnly, &env, &NO_CANCEL, |_| {}).unwrap();
    assert!(s.repo.path().join("b.txt").exists());
}

#[test]
fn a_push_behind_the_remote_is_refused_and_the_lease_guards_a_force_push() {
    let s = setup();
    let runner = s.repo.runner(true);
    let env = NetEnv::new();
    remote::add_remote(&runner, "origin", s.origin.path().to_str().unwrap()).unwrap();
    remote::push(
        &runner,
        "origin",
        "main",
        true,
        None,
        &env,
        &NO_CANCEL,
        |_| {},
    )
    .unwrap();
    let seen = s.repo.git(1, &["rev-parse", "origin/main"]);

    pushed_by_someone_else(&s, "b.txt");
    s.repo.write("a.txt", b"mine\n");
    s.repo.commit_all("mine");
    let error = remote::push(
        &runner,
        "origin",
        "main",
        false,
        None,
        &env,
        &NO_CANCEL,
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::NonFastForward);

    // The remote moved since the last fetch, so the lease does not hold.
    let stale = remote::push(
        &runner,
        "origin",
        "main",
        false,
        Some(&seen),
        &env,
        &NO_CANCEL,
        |_| {},
    );
    assert!(stale.is_err());
    // After a fetch the user has seen what will be replaced.
    remote::fetch(&runner, &env, &NO_CANCEL, |_| {}).unwrap();
    let now = s.repo.git(1, &["rev-parse", "origin/main"]);
    remote::push(
        &runner,
        "origin",
        "main",
        false,
        Some(&now),
        &env,
        &NO_CANCEL,
        |_| {},
    )
    .unwrap();
    assert_eq!(
        s.repo.git(1, &["rev-parse", "origin/main"]),
        s.repo.git(1, &["rev-parse", "main"])
    );
}

#[test]
fn pull_fast_forward_only_refuses_when_both_sides_moved() {
    let s = setup();
    let runner = s.repo.runner(true);
    let env = NetEnv::new();
    remote::add_remote(&runner, "origin", s.origin.path().to_str().unwrap()).unwrap();
    remote::push(
        &runner,
        "origin",
        "main",
        true,
        None,
        &env,
        &NO_CANCEL,
        |_| {},
    )
    .unwrap();
    pushed_by_someone_else(&s, "b.txt");
    s.repo.write("c.txt", b"mine\n");
    s.repo.commit_all("mine");

    let error = remote::pull(&runner, PullMode::FfOnly, &env, &NO_CANCEL, |_| {}).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::NonFastForward);
    remote::pull(&runner, PullMode::Rebase, &env, &NO_CANCEL, |_| {}).unwrap();
    assert!(s.repo.path().join("b.txt").exists() && s.repo.path().join("c.txt").exists());
    assert_eq!(s.repo.git(1, &["log", "-1", "--format=%s"]), "mine");
}

#[test]
fn tags_are_pushed_and_deleted_on_the_remote() {
    let s = setup();
    let runner = s.repo.runner(true);
    let env = NetEnv::new();
    let url = s.origin.path().to_str().unwrap();
    remote::add_remote(&runner, "origin", url).unwrap();
    s.repo.git(1, &["tag", "v1"]);
    remote::push_tag(&runner, "origin", "v1", false, &env, &NO_CANCEL).unwrap();
    assert!(
        s.repo
            .git(1, &["ls-remote", "--tags", url])
            .contains("refs/tags/v1")
    );
    remote::push_tag(&runner, "origin", "v1", true, &env, &NO_CANCEL).unwrap();
    assert_eq!(s.repo.git(1, &["ls-remote", "--tags", url]), "");
}

#[test]
fn clone_reports_progress_and_leaves_nothing_behind_when_it_fails() {
    let s = setup();
    let env = NetEnv::new();
    let url = s.origin.path().to_str().unwrap();
    s.repo.git(1, &["push", "-q", url, "main"]);
    let parent = tempfile::tempdir().unwrap();
    let runner = GitRunner::new("git", parent.path(), true);

    let dest = parent.path().join("copy");
    remote::clone(&runner, url, &dest, &env, &NO_CANCEL, |_| {}).unwrap();
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"one\n");

    let missing = parent.path().join("missing");
    let error = remote::clone(
        &runner,
        "/no/such/repository",
        &missing,
        &env,
        &NO_CANCEL,
        |_| {},
    );
    assert!(error.is_err() && !missing.exists());
    // A folder with something in it is never cloned into.
    assert!(remote::clone(&runner, url, &dest, &env, &NO_CANCEL, |_| {}).is_err());
    assert!(remote::clone(&runner, "ext::sh -c x", &missing, &env, &NO_CANCEL, |_| {}).is_err());

    // Cancelling stops Git and removes the partial folder.
    let cancelled = AtomicBool::new(true);
    let gone = parent.path().join("cancelled");
    let error = remote::clone(&runner, url, &gone, &env, &cancelled, |_| {}).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Cancelled);
    assert!(!gone.exists());
}
