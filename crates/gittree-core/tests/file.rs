//! Blame and file history against a real repository.

mod common;

use common::TestRepo;
use gittree_core::model::Change;
use gittree_core::ops::file::{blame, history};

#[test]
fn blame_and_history_follow_a_file_through_a_rename() {
    let repo = TestRepo::new();
    repo.write("old.txt", b"one\ntwo\nthree\nfour\nfive\nsix\n");
    repo.git(100, &["add", "-A"]);
    repo.git(100, &["commit", "-q", "-m", "first"]);
    repo.git(200, &["mv", "old.txt", "new.txt"]);
    repo.git(200, &["commit", "-q", "-m", "move"]);
    repo.write("new.txt", b"one\nTWO\nthree\nfour\nfive\nsix\n");
    repo.git(300, &["add", "-A"]);
    repo.git(300, &["commit", "-q", "-m", "shout"]);
    let runner = repo.runner(false);

    let lines = blame(&runner, "HEAD", &"new.txt".into()).unwrap();
    let brief: Vec<_> = lines
        .iter()
        .map(|l| {
            (
                &*l.summary,
                l.time,
                String::from_utf8_lossy(&l.text).into_owned(),
            )
        })
        .collect();
    assert_eq!(brief.len(), 6);
    assert_eq!(brief[0], ("first", 100, "one".to_owned()));
    assert_eq!(brief[1], ("shout", 300, "TWO".to_owned()));
    assert_eq!(&*lines[1].author, "Ada");
    assert!(blame(&runner, "HEAD", &"missing.txt".into()).is_err());

    let commits = history(&runner, "HEAD", &"new.txt".into(), 100).unwrap();
    let brief: Vec<_> = commits
        .iter()
        .map(|c| {
            (
                &*c.commit.summary,
                c.file.change,
                c.file.path.to_string(),
                c.file.orig_path.as_ref().map(|p| p.to_string()),
            )
        })
        .collect();
    assert_eq!(
        brief,
        [
            ("shout", Change::Modified, "new.txt".to_owned(), None),
            (
                "move",
                Change::Renamed,
                "new.txt".to_owned(),
                Some("old.txt".to_owned())
            ),
            ("first", Change::Added, "old.txt".to_owned(), None),
        ]
    );
    assert_eq!(
        history(&runner, "HEAD", &"new.txt".into(), 1)
            .unwrap()
            .len(),
        1
    );
}
