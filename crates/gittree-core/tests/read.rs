mod common;

use common::TestRepo;
use gittree_core::model::{Change, DiffKind, LineKind, Oid, RefKind};
use gittree_core::ops::read::{
    LARGE_DIFF_SHOWN, branch_messages, commit_details, commit_files, compare, diff_commit_file,
    list_refs, reflog,
};

fn write(repo: &TestRepo, name: &str, content: &[u8]) {
    std::fs::write(repo.path().join(name), content).unwrap();
}

fn commit_all(repo: &TestRepo, time: i64, message: &str) -> Oid {
    repo.git(time, &["add", "-A"]);
    repo.git(time, &["commit", "-q", "-m", message]);
    repo.git(0, &["rev-parse", "HEAD"]).into()
}

#[test]
fn lists_refs_like_git() {
    let repo = TestRepo::new();
    write(&repo, "a.txt", b"one\n");
    let first = commit_all(&repo, 1, "first");
    repo.git(2, &["tag", "light"]);
    repo.git(2, &["tag", "-a", "v1", "-m", "version one"]);
    repo.git(0, &["update-ref", "refs/remotes/origin/main", &first]);
    repo.git(0, &["config", "remote.origin.url", "file:///nowhere"]);
    repo.git(
        0,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    repo.git(0, &["branch", "--set-upstream-to=origin/main"]);
    write(&repo, "a.txt", b"two\n");
    let second = commit_all(&repo, 3, "second");
    repo.git(0, &["branch", "topic", &first]);
    repo.git(0, &["update-ref", "refs/gittree/snapshots/x", &first]);

    let refs = list_refs(&repo.runner(false)).unwrap();
    let find = |name: &str| refs.iter().find(|r| r.name == name).unwrap();

    let main = find("refs/heads/main");
    assert!(main.is_current);
    assert_eq!((&main.target, main.ahead, main.behind), (&second, 1, 0));
    assert_eq!(main.upstream.as_deref(), Some("refs/remotes/origin/main"));
    assert!(!find("refs/heads/topic").is_current);
    assert_eq!(find("refs/remotes/origin/main").kind, RefKind::Remote);
    // Both tags point at the commit, though v1 is its own object.
    assert_eq!(find("refs/tags/light").target, first);
    assert_eq!(find("refs/tags/v1").target, first);
    assert!(refs.iter().all(|r| !r.name.starts_with("refs/gittree/")));
}

/// A name Git has to quote. Windows refuses a tab in a file name.
const ODD_NAME: &str = if cfg!(windows) {
    "new name.txt"
} else {
    "new\tname.txt"
};

#[test]
fn reads_commit_details_and_files() {
    let repo = TestRepo::new();
    write(&repo, "keep.txt", b"same\n");
    write(&repo, "old name.txt", b"a\nb\nc\nd\ne\nf\n");
    write(&repo, "gone.txt", b"bye\n");
    let first = commit_all(&repo, 100, "first");

    std::fs::rename(repo.path().join("old name.txt"), repo.path().join(ODD_NAME)).unwrap();
    std::fs::remove_file(repo.path().join("gone.txt")).unwrap();
    write(&repo, "keep.txt", b"changed\n");
    write(&repo, "added.txt", b"hi\n");
    let second = commit_all(&repo, 200, "second\n\nWith a body.\nOn two lines.");

    let runner = repo.runner(false);
    let details = commit_details(&runner, &second).unwrap();
    assert_eq!(details.oid, second);
    assert_eq!(details.parents, std::slice::from_ref(&first));
    assert_eq!(details.message, "second\n\nWith a body.\nOn two lines.");
    assert_eq!(
        (details.author_name.as_str(), details.author_time),
        ("Ada", 200)
    );
    assert_eq!(
        (details.committer_email.as_str(), details.commit_time),
        ("ada@example.com", 200)
    );

    let mut files = commit_files(&runner, &second).unwrap();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let summary: Vec<(Change, String, Option<String>)> = files
        .iter()
        .map(|f| {
            (
                f.change,
                f.path.to_string(),
                f.orig_path.as_ref().map(|p| p.to_string()),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (Change::Added, "added.txt".to_owned(), None),
            (Change::Deleted, "gone.txt".to_owned(), None),
            (Change::Modified, "keep.txt".to_owned(), None),
            (
                Change::Renamed,
                ODD_NAME.to_owned(),
                Some("old name.txt".to_owned())
            ),
        ]
    );

    // The root commit lists its files as added.
    let root = commit_files(&runner, &first).unwrap();
    assert!(root.len() == 3 && root.iter().all(|f| f.change == Change::Added));

    let modified = files.iter().find(|f| f.change == Change::Modified).unwrap();
    let diff = diff_commit_file(&runner, &second, modified, false, false).unwrap();
    assert_eq!(diff.kind, DiffKind::Text);
    let lines: Vec<(LineKind, &[u8])> = diff.hunks[0]
        .lines
        .iter()
        .map(|l| (l.kind, &l.text[..]))
        .collect();
    assert_eq!(
        lines,
        [
            (LineKind::Remove, &b"same\n"[..]),
            (LineKind::Add, &b"changed\n"[..])
        ]
    );

    let renamed = files.iter().find(|f| f.change == Change::Renamed).unwrap();
    let diff = diff_commit_file(&runner, &second, renamed, false, false).unwrap();
    assert!(diff.hunks.is_empty(), "a pure rename has no hunks");
    assert_eq!(diff.old_path.unwrap().to_string(), "old name.txt");
}

#[test]
fn a_merge_is_compared_with_its_first_parent() {
    let repo = TestRepo::new();
    write(&repo, "a.txt", b"base\n");
    commit_all(&repo, 1, "base");
    repo.git(0, &["switch", "-q", "-c", "side"]);
    write(&repo, "side.txt", b"side\n");
    commit_all(&repo, 2, "side");
    repo.git(0, &["switch", "-q", "main"]);
    write(&repo, "main.txt", b"main\n");
    commit_all(&repo, 3, "main");
    repo.git(4, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let merge: Oid = repo.git(0, &["rev-parse", "HEAD"]).into();

    let runner = repo.runner(false);
    let files = commit_files(&runner, &merge).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path.to_string(), "side.txt");
    let diff = diff_commit_file(&runner, &merge, &files[0], false, false).unwrap();
    assert_eq!(diff.hunks[0].lines[0].text, b"side\n");
}

#[test]
fn binary_files_report_sizes_and_huge_diffs_are_cut() {
    let repo = TestRepo::new();
    write(&repo, "image.bin", &[0u8, 1, 2, 3]);
    write(&repo, "big.txt", b"start\n");
    commit_all(&repo, 1, "first");
    write(&repo, "image.bin", &[0u8; 10]);
    let big: String = (0..25_000).map(|n| format!("line {n}\n")).collect();
    write(&repo, "big.txt", big.as_bytes());
    let second = commit_all(&repo, 2, "second");

    let runner = repo.runner(false);
    let files = commit_files(&runner, &second).unwrap();
    let file = |name: &str| files.iter().find(|f| f.path.to_string() == name).unwrap();

    let binary = diff_commit_file(&runner, &second, file("image.bin"), false, false).unwrap();
    assert_eq!(
        binary.kind,
        DiffKind::Binary {
            old_size: 4,
            new_size: 10
        }
    );

    let count = |load_all| {
        let diff = diff_commit_file(&runner, &second, file("big.txt"), load_all, false).unwrap();
        (
            diff.kind.clone(),
            diff.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
        )
    };
    assert_eq!(
        count(false),
        (DiffKind::TooLarge { lines: 25_001 }, LARGE_DIFF_SHOWN)
    );
    assert_eq!(count(true), (DiffKind::Text, 25_001));
}

#[test]
fn the_reflog_lists_where_head_has_been_newest_first() {
    let repo = TestRepo::new();
    assert!(reflog(&repo.runner(false), 10).unwrap().is_empty());
    write(&repo, "a.txt", b"one\n");
    let first = commit_all(&repo, 1_000, "first");
    write(&repo, "a.txt", b"two\n");
    let second = commit_all(&repo, 2_000, "second");
    repo.git(3_000, &["reset", "-q", "--hard", "HEAD~1"]);

    // Read-only, so an untrusted repository shows it too.
    let entries = reflog(&repo.runner(false), 10).unwrap();
    let seen: Vec<(&str, i64, &str)> = entries
        .iter()
        .map(|entry| (&*entry.oid, entry.time, entry.message.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            (&*first, 3_000, "reset: moving to HEAD~1"),
            (&*second, 2_000, "commit: second"),
            (&*first, 1_000, "commit (initial): first"),
        ]
    );
    assert_eq!(reflog(&repo.runner(false), 1).unwrap().len(), 1);
}

#[test]
fn compares_any_two_revisions_by_file() {
    let repo = TestRepo::new();
    write(&repo, "a.txt", b"one\n");
    write(&repo, "b.txt", b"bee\n");
    commit_all(&repo, 1, "first");
    repo.git(1, &["tag", "v1"]);
    write(&repo, "a.txt", b"two\n");
    std::fs::remove_file(repo.path().join("b.txt")).unwrap();
    write(&repo, "c.txt", b"new\n");
    commit_all(&repo, 2, "second");

    let files = compare(&repo.runner(false), "v1", "HEAD").unwrap();
    let seen: Vec<(Change, String)> = files
        .iter()
        .map(|file| (file.change, file.path.to_string()))
        .collect();
    assert_eq!(
        seen,
        [
            (Change::Modified, "a.txt".into()),
            (Change::Deleted, "b.txt".into()),
            (Change::Added, "c.txt".into()),
        ]
    );
    assert!(
        compare(&repo.runner(false), "HEAD", "HEAD")
            .unwrap()
            .is_empty()
    );
    assert!(compare(&repo.runner(false), "--output=x", "HEAD").is_err());
    assert!(compare(&repo.runner(false), "nope", "HEAD").is_err());
}

#[test]
fn whitespace_only_changes_can_be_left_out() {
    let repo = TestRepo::new();
    write(&repo, "a.rs", b"fn main() {\n    run();\n}\n");
    commit_all(&repo, 1, "first");
    write(&repo, "a.rs", b"fn main() {\n        run();\n}\n");
    let second = commit_all(&repo, 2, "indent");
    let runner = repo.runner(false);
    let file = &commit_files(&runner, &second).unwrap()[0];
    assert_eq!(
        diff_commit_file(&runner, &second, file, false, false)
            .unwrap()
            .hunks
            .len(),
        1
    );
    assert!(
        diff_commit_file(&runner, &second, file, false, true)
            .unwrap()
            .hunks
            .is_empty()
    );
}

#[test]
fn a_branch_lists_the_messages_its_base_does_not_have() {
    let repo = TestRepo::new();
    write(&repo, "a.txt", b"a\n");
    commit_all(&repo, 1, "on main");
    repo.git(1, &["switch", "-q", "-c", "topic"]);
    write(&repo, "a.txt", b"b\n");
    commit_all(&repo, 2, "Add b\n\nBecause.");
    write(&repo, "a.txt", b"c\n");
    commit_all(&repo, 3, "Add c");
    let out = String::from_utf8(branch_messages(&repo.runner(false), "main").unwrap()).unwrap();
    assert!(out.contains("Add c") && out.contains("Because.") && !out.contains("on main"));
}
