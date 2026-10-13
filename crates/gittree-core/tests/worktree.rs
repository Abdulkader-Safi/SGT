//! Worktrees against a real repository.

mod common;

use common::TestRepo;
use gittree_core::ops::setup::canonical;
use gittree_core::ops::worktree::{add, list, remove};

#[test]
fn a_worktree_is_added_listed_and_removed_only_when_clean() {
    let repo = TestRepo::new();
    repo.write("a.txt", b"one\n");
    repo.commit_all("first");
    let runner = repo.runner(true);
    let outside = tempfile::tempdir().unwrap();
    let path = canonical(outside.path()).join("wt");

    add(&runner, &path, "agent/keys", "HEAD").unwrap();
    let found = list(&runner).unwrap();
    assert_eq!(found.len(), 2);
    assert!(found[0].main && found[0].branch.as_deref() == Some("main"));
    assert_eq!(canonical(&found[1].path), path);
    assert_eq!(found[1].branch.as_deref(), Some("agent/keys"));
    assert_eq!(found[1].head, found[0].head);

    // Options and relative paths are refused before Git runs.
    assert!(add(&runner, &path.join("x"), "-b", "HEAD").is_err());
    assert!(add(&runner, &path.join("x"), "ok", "--force").is_err());
    assert!(add(&runner, "relative".as_ref(), "ok", "HEAD").is_err());
    // An untrusted repository is read-only.
    assert!(add(&repo.runner(false), &path.join("y"), "nope", "HEAD").is_err());

    // Work in the worktree keeps it from being removed.
    std::fs::write(path.join("new.txt"), "work").unwrap();
    assert!(remove(&runner, &path).is_err());
    assert!(path.join("new.txt").exists());
    std::fs::remove_file(path.join("new.txt")).unwrap();
    remove(&runner, &path).unwrap();
    assert_eq!(list(&runner).unwrap().len(), 1);
    // The branch stays.
    assert_eq!(
        repo.git(1, &["branch", "--list", "agent/keys"]).trim(),
        "agent/keys"
    );
}
