//! Merge, rebase, cherry-pick, revert, reset and conflicts against real Git.

mod common;

use common::TestRepo;
use gittree_core::GitErrorKind;
use gittree_core::journal::{Action, Journal};
use gittree_core::ops::merge::{self, OpKind, ResetMode, Side};
use gittree_core::parse::conflict::{Choice, parse_conflicts, resolve};

/// main and side both change line 2 of f.txt, so they conflict. side also
/// has a commit that does not.
fn diverged() -> TestRepo {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    repo.write("f.txt", b"one\ntwo\nthree\n");
    repo.commit_all("base");
    repo.git(1, &["switch", "-q", "-c", "side"]);
    repo.write("f.txt", b"one\nSIDE\nthree\n");
    repo.commit_all("side changes two");
    repo.write("other.txt", b"other\n");
    repo.commit_all("side adds a file");
    repo.git(1, &["switch", "-q", "main"]);
    repo.write("f.txt", b"one\nMAIN\nthree\n");
    repo.commit_all("main changes two");
    repo
}

fn git_dir(repo: &TestRepo) -> std::path::PathBuf {
    repo.path().join(".git")
}

fn head(repo: &TestRepo) -> String {
    repo.git(1, &["rev-parse", "HEAD"])
}

#[test]
fn a_clean_merge_can_force_a_merge_commit_and_be_undone() {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    repo.write("a.txt", b"a\n");
    repo.commit_all("base");
    repo.git(1, &["switch", "-q", "-c", "side"]);
    repo.write("b.txt", b"b\n");
    repo.commit_all("on side");
    repo.git(1, &["switch", "-q", "main"]);
    let (runner, dir, before) = (repo.runner(true), git_dir(&repo), head(&repo));
    let journal = Journal::new(&dir);

    let action = Action::Merge {
        branch: "side".into(),
        no_ff: true,
    };
    let pending = journal.begin(&runner, action, "Merge".into()).unwrap();
    merge::merge(&runner, &dir, "side", true).unwrap();
    let entry = journal.finish(&runner, pending).unwrap();
    assert_eq!(
        repo.git(1, &["rev-list", "--parents", "-1", "HEAD"])
            .split(' ')
            .count(),
        3
    );
    journal.undo(&runner, &entry).unwrap();
    assert_eq!(head(&repo), before);

    // Without the option it fast-forwards.
    merge::merge(&runner, &dir, "side", false).unwrap();
    assert_eq!(head(&repo), repo.git(1, &["rev-parse", "side"]));
}

#[test]
fn a_conflicting_merge_is_resolved_by_region_and_continued() {
    let repo = diverged();
    let (runner, dir, before) = (repo.runner(true), git_dir(&repo), head(&repo));
    let journal = Journal::new(&dir);
    let action = Action::Merge {
        branch: "side".into(),
        no_ff: false,
    };
    let pending = journal.begin(&runner, action, "Merge side".into()).unwrap();
    let error = merge::merge(&runner, &dir, "side", false).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Conflict);
    journal.park(pending).unwrap();

    let state = merge::in_progress(&runner, &dir).unwrap();
    assert_eq!((state.kind, state.conflicted), (OpKind::Merge, 1));
    assert_eq!(state.onto.as_deref(), Some("side"));
    // Continue is refused while a file is still in conflict.
    assert!(merge::continue_op(&runner, &dir, OpKind::Merge).is_err());

    let content = repo.read("f.txt");
    let regions = parse_conflicts(&content);
    assert_eq!(regions.len(), 1);
    let fixed = resolve(&content, &regions, &[Choice::Both]);
    merge::resolve_file(&runner, &"f.txt".into(), &fixed).unwrap();
    assert_eq!(repo.read("f.txt"), b"one\nMAIN\nSIDE\nthree\n");
    assert_eq!(merge::in_progress(&runner, &dir).unwrap().conflicted, 0);

    merge::continue_op(&runner, &dir, OpKind::Merge).unwrap();
    assert!(merge::in_progress(&runner, &dir).is_none());
    assert!(repo.path().join("other.txt").exists());
    let entry = journal
        .finish(&runner, journal.take_parked().unwrap())
        .unwrap();
    assert!(journal.take_parked().is_none());
    journal.undo(&runner, &entry).unwrap();
    assert_eq!(head(&repo), before);
    assert_eq!(repo.read("f.txt"), b"one\nMAIN\nthree\n");
}

#[test]
fn abort_puts_the_repository_back() {
    let repo = diverged();
    let (runner, dir, before) = (repo.runner(true), git_dir(&repo), head(&repo));
    assert!(merge::merge(&runner, &dir, "side", false).is_err());
    merge::abort_op(&runner, OpKind::Merge).unwrap();
    assert!(merge::in_progress(&runner, &dir).is_none());
    assert_eq!(head(&repo), before);
    assert_eq!(repo.git(1, &["status", "--porcelain"]), "");
}

#[test]
fn a_rebase_stops_on_a_conflict_and_can_skip_or_continue() {
    let repo = diverged();
    repo.git(1, &["switch", "-q", "side"]);
    let (runner, dir) = (repo.runner(true), git_dir(&repo));
    let error = merge::rebase(&runner, &dir, "main").unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Conflict);
    let state = merge::in_progress(&runner, &dir).unwrap();
    assert_eq!(state.kind, OpKind::Rebase);
    assert_eq!(state.step, Some((1, 2)));

    // Skipping drops the conflicting commit. The second one applies.
    merge::rebase_skip(&runner, &dir).unwrap();
    assert!(merge::in_progress(&runner, &dir).is_none());
    assert_eq!(
        repo.git(1, &["log", "--format=%s", "-2"]),
        "side adds a file\nmain changes two"
    );

    // The same again, this time taking the incoming side.
    repo.git(1, &["reset", "-q", "--hard", "ORIG_HEAD"]);
    assert!(merge::rebase(&runner, &dir, "main").is_err());
    // In a rebase "theirs" is the commit being replayed.
    merge::resolve_side(&runner, &"f.txt".into(), Side::Theirs).unwrap();
    merge::continue_op(&runner, &dir, OpKind::Rebase).unwrap();
    assert_eq!(repo.read("f.txt"), b"one\nSIDE\nthree\n");
    assert_eq!(repo.git(1, &["rev-list", "--count", "main..HEAD"]), "2");
}

#[test]
fn cherry_pick_and_revert_make_commits_and_ask_for_a_mainline_on_merges() {
    let repo = diverged();
    let (runner, dir) = (repo.runner(true), git_dir(&repo));
    let pick = repo.git(1, &["rev-parse", "side"]);
    merge::cherry_pick(&runner, &dir, &pick, None).unwrap();
    assert_eq!(
        repo.git(1, &["log", "-1", "--format=%s"]),
        "side adds a file"
    );
    assert!(repo.path().join("other.txt").exists());

    merge::revert(&runner, &dir, "HEAD", None).unwrap();
    assert!(!repo.path().join("other.txt").exists());
    assert!(
        repo.git(1, &["log", "-1", "--format=%s"])
            .starts_with("Revert")
    );

    // A merge commit has two parents: Git needs to be told which to keep.
    repo.git(1, &["switch", "-q", "-c", "third", "HEAD~2"]);
    repo.write("third.txt", b"3\n");
    repo.commit_all("third");
    repo.git(1, &["switch", "-q", "main"]);
    repo.git(1, &["merge", "-q", "--no-ff", "-m", "Merge third", "third"]);
    assert!(merge::revert(&runner, &dir, "HEAD", None).is_err());
    merge::revert(&runner, &dir, "HEAD", Some(1)).unwrap();
    assert!(!repo.path().join("third.txt").exists());
}

#[test]
fn reset_has_three_modes_and_undo_brings_everything_back() {
    for (mode, status_after) in [
        (ResetMode::Soft, "M  f.txt"),
        (ResetMode::Mixed, " M f.txt"),
        (ResetMode::Hard, ""),
    ] {
        let repo = diverged();
        // An uncommitted change that a hard reset would destroy.
        repo.write("f.txt", b"one\nMAIN\nthree\nextra\n");
        let (runner, dir, before) = (repo.runner(true), git_dir(&repo), head(&repo));
        let journal = Journal::new(&dir);
        let action = Action::Reset {
            target: "HEAD~1".into(),
            mode,
        };
        let pending = journal.begin(&runner, action, "Reset".into()).unwrap();
        merge::reset(&runner, "HEAD~1", mode).unwrap();
        let entry = journal.finish(&runner, pending).unwrap();
        assert_eq!(
            head(&repo),
            repo.git(1, &["rev-parse", &format!("{before}~1")])
        );
        if mode == ResetMode::Hard {
            assert_eq!(repo.git(1, &["status", "--porcelain"]), status_after);
            assert_eq!(repo.read("f.txt"), b"one\ntwo\nthree\n");
        }

        journal.undo(&runner, &entry).unwrap();
        assert_eq!(head(&repo), before, "{mode:?}");
        assert_eq!(repo.read("f.txt"), b"one\nMAIN\nthree\nextra\n", "{mode:?}");
        assert_eq!(
            repo.git(1, &["status", "--porcelain"]),
            " M f.txt",
            "{mode:?}"
        );
    }
}

#[test]
fn conflicts_are_predicted_without_changing_anything() {
    let repo = diverged();
    let runner = repo.runner(true);
    let before = repo.git(1, &["status", "--porcelain"]);
    assert_eq!(
        merge::predict_conflicts(&runner, "side").unwrap(),
        ["f.txt".into()]
    );
    assert_eq!(head(&repo), repo.git(1, &["rev-parse", "main"]));
    assert_eq!(repo.git(1, &["status", "--porcelain"]), before);
    assert!(merge::in_progress(&runner, &git_dir(&repo)).is_none());
    // A merge with no overlap predicts none.
    assert!(
        merge::predict_conflicts(&runner, "HEAD~1")
            .unwrap()
            .is_empty()
    );
    assert!(merge::predict_conflicts(&runner, "--output=x").is_err());
    assert!(merge::predict_conflicts(&repo.runner(false), "side").is_err());
}

#[test]
fn an_untrusted_repository_refuses_all_of_it() {
    let repo = diverged();
    let (runner, dir) = (repo.runner(false), git_dir(&repo));
    let untrusted = |result: Result<(), gittree_core::GitError>| {
        assert_eq!(result.unwrap_err().kind, GitErrorKind::Untrusted)
    };
    untrusted(merge::merge(&runner, &dir, "side", false));
    untrusted(merge::rebase(&runner, &dir, "side"));
    untrusted(merge::cherry_pick(&runner, &dir, "side", None));
    untrusted(merge::reset(&runner, "HEAD~1", ResetMode::Hard));
    untrusted(merge::resolve_file(&runner, &"f.txt".into(), b"x"));
    assert_eq!(repo.read("f.txt"), b"one\nMAIN\nthree\n");
    // Reading the state is always allowed.
    assert!(merge::in_progress(&runner, &dir).is_none());
}
