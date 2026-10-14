mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use common::TestRepo;
use gittree_core::model::Commit;
use gittree_core::ops::read::{LOG_BATCH, stream_log};
use gittree_core::{GitErrorKind, GitRunner};

fn load(repo: &TestRepo) -> Vec<Commit> {
    let mut commits = Vec::new();
    stream_log(
        &repo.runner(false),
        &[],
        |batch| commits.extend(batch),
        &AtomicBool::new(false),
    )
    .unwrap();
    commits
}

#[test]
fn matches_git_and_skips_snapshot_refs() {
    let repo = TestRepo::new();
    repo.git(100, &["commit", "-q", "--allow-empty", "-m", "first"]);
    repo.git(0, &["switch", "-q", "-c", "side"]);
    repo.git(200, &["commit", "-q", "--allow-empty", "-m", "on side"]);
    repo.git(0, &["switch", "-q", "main"]);
    repo.git(300, &["commit", "-q", "--allow-empty", "-m", "on main"]);
    repo.git(400, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);

    // A commit only a snapshot ref can reach must stay out of the history.
    let tree = repo.git(0, &["rev-parse", "HEAD^{tree}"]);
    let hidden = repo.git(500, &["commit-tree", "-m", "snapshot", &tree]);
    repo.git(0, &["update-ref", "refs/gittree/snapshots/x", &hidden]);

    let commits = load(&repo);
    let oid = |rev: &str| repo.git(0, &["rev-parse", rev]);
    let summaries: Vec<&str> = commits.iter().map(|c| &*c.summary).collect();
    assert_eq!(summaries, ["merge side", "on main", "on side", "first"]);

    let merge = &commits[0];
    assert_eq!(&*merge.oid, oid("main"));
    let parents: Vec<&str> = merge.parents.iter().map(|p| &**p).collect();
    assert_eq!(parents, [oid("main^1"), oid("main^2")]);
    assert_eq!(&*merge.author_name, "Ada");
    assert_eq!(&*merge.author_email, "ada@example.com");
    assert_eq!(merge.author_time, 400);
    assert!(commits[3].parents.is_empty());
}

#[test]
fn an_empty_repository_has_no_commits() {
    assert!(load(&TestRepo::new()).is_empty());
}

/// A straight line of `count` commits, built with fast-import.
fn long_history(count: usize) -> TestRepo {
    let repo = TestRepo::new();
    let mut stream = String::new();
    for n in 1..=count {
        stream += &format!(
            "commit refs/heads/main\nmark :{n}\ncommitter Ada <ada@example.com> {n} +0000\ndata 1\nx\n"
        );
        if n > 1 {
            stream += &format!("from :{}\n", n - 1);
        }
        stream += "\n";
    }
    repo.git_with_input(0, &["fast-import", "--quiet"], stream.as_bytes());
    repo
}

#[test]
fn delivers_batches_and_stops_when_cancelled() {
    let repo = long_history(LOG_BATCH * 2 + 500);
    let runner = repo.runner(false);

    let mut sizes = Vec::new();
    stream_log(
        &runner,
        &[],
        |batch| sizes.push(batch.len()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sizes, [LOG_BATCH, LOG_BATCH, 500]);

    let cancel = AtomicBool::new(false);
    let mut batches = 0;
    let result = stream_log(
        &runner,
        &[],
        |_| {
            batches += 1;
            cancel.store(true, Ordering::Relaxed);
        },
        &cancel,
    );
    assert_eq!(result.unwrap_err().kind, GitErrorKind::Cancelled);
    assert_eq!(batches, 1);
}

#[test]
fn a_folder_that_is_not_a_repository_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = stream_log(
        &GitRunner::new("git", dir.path(), false),
        &[],
        |_| {},
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::NotARepository, "{error}");
}

#[test]
fn a_hidden_branch_leaves_out_only_its_own_commits() {
    let repo = TestRepo::new();
    repo.git(100, &["commit", "-q", "--allow-empty", "-m", "first"]);
    repo.git(0, &["switch", "-q", "-c", "side"]);
    repo.git(
        200,
        &["commit", "-q", "--allow-empty", "-m", "only on side"],
    );
    repo.git(0, &["switch", "-q", "main"]);
    repo.git(300, &["commit", "-q", "--allow-empty", "-m", "on main"]);
    let mut commits = Vec::new();
    stream_log(
        &repo.runner(false),
        &["refs/heads/side".to_owned(), "--output=x".to_owned()],
        |batch| commits.extend(batch),
        &AtomicBool::new(false),
    )
    .unwrap();
    let summaries: Vec<&str> = commits.iter().map(|c| &*c.summary).collect();
    assert_eq!(summaries, ["on main", "first"]);
}
