//! Staging, discarding and committing against real Git.

mod common;

use common::TestRepo;
use gittree_core::GitErrorKind;
use gittree_core::journal::{Action, Blocked, Journal};
use gittree_core::model::{Change, FileDiff, LineKind, StatusEntry};
use gittree_core::ops::read::{diff_worktree, status};
use gittree_core::ops::stage::{self, Target};
use gittree_core::patch::{self, Lines};

use LineKind::{Add, Remove};

fn entry(repo: &TestRepo, path: &str) -> StatusEntry {
    status(&repo.runner(true))
        .unwrap()
        .into_iter()
        .find(|entry| entry.path.to_string() == path)
        .unwrap_or_else(|| panic!("{path} is not in the status"))
}

fn diff(repo: &TestRepo, path: &str, staged: bool) -> FileDiff {
    diff_worktree(&repo.runner(true), &entry(repo, path), staged, false, false).unwrap()
}

/// The changed lines whose kind and text (without the line ending) are in
/// `picks`.
fn pick(diff: &FileDiff, picks: &[(LineKind, &str)]) -> Lines {
    let mut lines = Lines::new();
    for (h, hunk) in diff.hunks.iter().enumerate() {
        for (l, line) in hunk.lines.iter().enumerate() {
            let text = String::from_utf8_lossy(&line.text);
            let text = text.trim_end_matches(['\n', '\r']);
            if picks.contains(&(line.kind, text)) {
                lines.insert((h, l));
            }
        }
    }
    assert_eq!(lines.len(), picks.len(), "every pick must match one line");
    lines
}

struct Case {
    name: &'static str,
    /// The committed content. None for a file Git does not know yet.
    head: Option<&'static [u8]>,
    work: &'static [u8],
    picks: &'static [(LineKind, &'static str)],
    /// What the index must hold after staging the picks.
    staged: &'static [u8],
}

const CASES: &[Case] = &[
    Case {
        name: "single add",
        head: Some(b"a\nb\nc\n"),
        work: b"a\nb\nNEW\nc\nALSO\n",
        picks: &[(Add, "NEW")],
        staged: b"a\nb\nNEW\nc\n",
    },
    Case {
        name: "single remove",
        head: Some(b"a\nb\nc\nd\n"),
        work: b"a\nd\n",
        picks: &[(Remove, "c")],
        staged: b"a\nb\nd\n",
    },
    Case {
        name: "mixed hunk",
        head: Some(b"a\nb\nc\nd\n"),
        work: b"a\nB\nC\nd\nE\n",
        picks: &[(Remove, "b"), (Add, "B"), (Add, "E")],
        // The removal of "c" is left out, so "c" stays, ahead of the added
        // lines as in the diff.
        staged: b"a\nc\nB\nd\nE\n",
    },
    Case {
        name: "first line",
        head: Some(b"a\nb\nc\n"),
        work: b"ZERO\na\nb\nc\nLAST\n",
        picks: &[(Add, "ZERO")],
        staged: b"ZERO\na\nb\nc\n",
    },
    Case {
        name: "last line",
        head: Some(b"a\nb\nc\n"),
        work: b"ZERO\na\nb\nc\nLAST\n",
        picks: &[(Add, "LAST")],
        staged: b"a\nb\nc\nLAST\n",
    },
    Case {
        name: "no trailing newline",
        head: Some(b"a\nb\nc"),
        work: b"A\nb\nX\nc",
        picks: &[(Add, "X")],
        staged: b"a\nb\nX\nc",
    },
    Case {
        name: "CRLF",
        head: Some(b"a\r\nb\r\nc\r\n"),
        work: b"a\r\nB\r\nc\r\nd\r\n",
        picks: &[(Add, "d")],
        staged: b"a\r\nb\r\nc\r\nd\r\n",
    },
    Case {
        // FR-STAGE-03: lines 2 and 4 of a five-line addition.
        name: "new file",
        head: None,
        work: b"1\n2\n3\n4\n5\n",
        picks: &[(Add, "2"), (Add, "4")],
        staged: b"2\n4\n",
    },
];

#[test]
fn staging_chosen_lines_puts_exactly_those_in_the_index() {
    for case in CASES {
        let repo = TestRepo::new();
        repo.write("keep.txt", b"keep\n");
        if let Some(head) = case.head {
            repo.write("f.txt", head);
        }
        repo.commit_all("base");
        repo.write("f.txt", case.work);

        let unstaged = diff(&repo, "f.txt", false);
        let lines = pick(&unstaged, case.picks);
        stage::apply_selection(&repo.runner(true), &unstaged, &lines, Target::Stage)
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert_eq!(
            String::from_utf8_lossy(&repo.git_bytes(&["show", ":f.txt"])),
            String::from_utf8_lossy(case.staged),
            "{}: index",
            case.name
        );
        assert_eq!(repo.read("f.txt"), case.work, "{}: file on disk", case.name);

        // Unstaging everything that is staged brings the index back.
        let staged = diff(&repo, "f.txt", true);
        let all: Lines = (0..staged.hunks.len())
            .flat_map(|h| patch::hunk_lines(&staged, h))
            .collect();
        stage::apply_selection(&repo.runner(true), &staged, &all, Target::Unstage)
            .unwrap_or_else(|error| panic!("{}: unstage: {error}", case.name));
        match case.head {
            Some(head) => assert_eq!(repo.git_bytes(&["show", ":f.txt"]), head, "{}", case.name),
            None => assert_eq!(entry(&repo, "f.txt").worktree, Change::Untracked),
        }
    }
}

#[test]
fn unstaging_one_line_of_a_renamed_file_keeps_the_rename() {
    let repo = TestRepo::new();
    repo.write("old.txt", b"a\nb\nc\nd\ne\nf\n");
    repo.commit_all("base");
    repo.git(1, &["mv", "old.txt", "new.txt"]);
    repo.write("new.txt", b"a\nONE\nb\nc\nd\ne\nf\nTWO\n");
    repo.git(1, &["add", "-A"]);

    let entry = entry(&repo, "new.txt");
    assert_eq!(entry.index, Change::Renamed);
    assert_eq!(entry.orig_path, Some("old.txt".into()));
    let staged = diff(&repo, "new.txt", true);
    assert_eq!(staged.old_path, Some("old.txt".into()));
    let lines = pick(&staged, &[(Add, "TWO")]);
    stage::apply_selection(&repo.runner(true), &staged, &lines, Target::Unstage).unwrap();

    assert_eq!(
        repo.git_bytes(&["show", ":new.txt"]),
        b"a\nONE\nb\nc\nd\ne\nf\n"
    );
    assert_eq!(self::entry(&repo, "new.txt").index, Change::Renamed);
    assert_eq!(repo.read("new.txt"), b"a\nONE\nb\nc\nd\ne\nf\nTWO\n");
}

/// Twenty lines, with lines 2 and 18 changed: two hunks.
fn two_hunks(repo: &TestRepo) {
    let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
    repo.write("f.txt", (lines.join("\n") + "\n").as_bytes());
    repo.commit_all("base");
    let mut changed = lines.clone();
    changed[1] = "TOP".into();
    changed[17] = "BOTTOM".into();
    repo.write("f.txt", (changed.join("\n") + "\n").as_bytes());
}

#[test]
fn staging_one_of_two_hunks_leaves_the_other_unstaged() {
    // FR-STAGE-02.
    let repo = TestRepo::new();
    two_hunks(&repo);
    let unstaged = diff(&repo, "f.txt", false);
    assert_eq!(unstaged.hunks.len(), 2);
    let first = patch::hunk_lines(&unstaged, 0);
    stage::apply_selection(&repo.runner(true), &unstaged, &first, Target::Stage).unwrap();

    let cached = repo.git(1, &["diff", "--cached"]);
    assert!(cached.contains("+TOP") && !cached.contains("+BOTTOM"));
    let rest = repo.git(1, &["diff"]);
    assert!(rest.contains("+BOTTOM") && !rest.contains("+TOP"));

    // The second hunk moved up by nothing, and still stages cleanly.
    let unstaged = diff(&repo, "f.txt", false);
    let only = patch::hunk_lines(&unstaged, 0);
    stage::apply_selection(&repo.runner(true), &unstaged, &only, Target::Stage).unwrap();
    assert_eq!(repo.git(1, &["diff"]), "");
}

#[test]
fn whole_files_stage_and_unstage_like_git() {
    // FR-STAGE-01.
    let repo = TestRepo::new();
    for name in ["a.txt", "b.txt", "gone.txt"] {
        repo.write(name, b"one\n");
    }
    repo.commit_all("base");
    repo.write("a.txt", b"two\n");
    repo.write("b.txt", b"two\n");
    std::fs::remove_file(repo.path().join("gone.txt")).unwrap();
    repo.write("dir/new file.txt", b"new\n");

    let runner = repo.runner(true);
    let all: Vec<_> = status(&runner)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(all.len(), 4);
    stage::stage(&runner, &all).unwrap();
    assert_eq!(
        repo.git(1, &["status", "--porcelain"]),
        "M  a.txt\nM  b.txt\nA  \"dir/new file.txt\"\nD  gone.txt"
    );
    assert!(
        status(&runner)
            .unwrap()
            .iter()
            .all(|e| e.staged() && !e.unstaged())
    );

    stage::unstage(&runner, &all[..1], true).unwrap();
    assert!(
        repo.git(1, &["status", "--porcelain"])
            .starts_with(" M a.txt\n")
    );
    stage::unstage(&runner, &all, true).unwrap();
    assert!(
        status(&runner)
            .unwrap()
            .iter()
            .all(|e| !e.staged() && e.unstaged())
    );
}

#[test]
fn files_unstage_before_the_first_commit() {
    let repo = TestRepo::new();
    repo.write("a.txt", b"one\n");
    let runner = repo.runner(true);
    stage::stage(&runner, &["a.txt".into()]).unwrap();
    assert_eq!(entry(&repo, "a.txt").index, Change::Added);
    stage::unstage(&runner, &["a.txt".into()], false).unwrap();
    assert_eq!(entry(&repo, "a.txt").worktree, Change::Untracked);
    assert_eq!(repo.read("a.txt"), b"one\n");
}

#[test]
fn an_untracked_file_shows_as_all_added_whatever_its_name() {
    let repo = TestRepo::new();
    repo.write("base.txt", b"x\n");
    repo.commit_all("base");
    // Windows refuses a quote or a tab in a file name.
    let name = if cfg!(windows) {
        "we ird \u{e9} name.txt"
    } else {
        "we\"ird \u{e9}\tname.txt"
    };
    repo.write(name, b"one\ntwo");
    repo.write("bin.dat", b"a\0b");

    let text = diff(&repo, name, false);
    assert_eq!(text.hunks[0].lines.len(), 2);
    assert!(text.no_newline_new && text.old_path.is_none());
    // Git accepts the header this app wrote for the odd name.
    let all = patch::hunk_lines(&text, 0);
    stage::apply_selection(&repo.runner(true), &text, &all, Target::Stage).unwrap();
    assert_eq!(repo.git_bytes(&["show", &format!(":{name}")]), b"one\ntwo");

    let binary = diff(&repo, "bin.dat", false);
    assert_eq!(
        binary.kind,
        gittree_core::model::DiffKind::Binary {
            old_size: 0,
            new_size: 3
        }
    );
}

#[test]
fn a_discarded_file_comes_back_byte_for_byte() {
    // FR-STAGE-04, for a staged and edited file, a deleted file and an
    // untracked file.
    let repo = TestRepo::new();
    repo.write("f.txt", b"one\n");
    repo.write("gone.txt", b"here\n");
    repo.commit_all("base");
    repo.write("f.txt", b"staged\n");
    repo.git(1, &["add", "f.txt"]);
    repo.write("f.txt", b"staged\nand edited\r\n");
    std::fs::remove_file(repo.path().join("gone.txt")).unwrap();
    repo.write("dir/new.txt", b"untracked\n");
    let before = repo.git(1, &["status", "--porcelain"]);

    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    let tracked = vec!["f.txt".into(), "gone.txt".into()];
    let untracked = vec!["dir/new.txt".into()];
    let action = Action::DiscardFiles {
        tracked: tracked.clone(),
        untracked: untracked.clone(),
    };
    let pending = journal
        .begin(&runner, action, "Discard 3 files".into())
        .unwrap();
    stage::discard_files(&runner, &tracked, &untracked, true).unwrap();
    let entry = journal.finish(&runner, pending).unwrap();

    assert_eq!(repo.git(1, &["status", "--porcelain"]), "");
    assert_eq!(repo.read("f.txt"), b"one\n");
    assert!(!repo.path().join("dir/new.txt").exists());
    // The snapshot ref stays out of the history the app shows.
    assert_eq!(
        repo.git(
            1,
            &["log", "--exclude=refs/gittree/*", "--all", "--format=%s"]
        ),
        "base"
    );

    assert_eq!(journal.can_undo(&runner, &entry), Ok(()));
    journal.undo(&runner, &entry).unwrap();
    assert_eq!(repo.git(1, &["status", "--porcelain"]), before);
    assert_eq!(repo.read("f.txt"), b"staged\nand edited\r\n");
    assert_eq!(repo.git_bytes(&["show", ":f.txt"]), b"staged\n");
    assert!(!repo.path().join("gone.txt").exists());
    assert_eq!(repo.read("dir/new.txt"), b"untracked\n");

    let saved = journal.entries();
    assert!(saved.len() == 1 && saved[0].undone);
    assert_eq!(
        journal.can_undo(&runner, &saved[0]),
        Err(Blocked::AlreadyUndone)
    );
}

#[test]
fn discarded_lines_come_back() {
    let repo = TestRepo::new();
    two_hunks(&repo);
    let work = repo.read("f.txt");
    let unstaged = diff(&repo, "f.txt", false);
    let lines = patch::hunk_lines(&unstaged, 1);
    let patch = patch::build(&unstaged, &lines, true).unwrap();

    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    let action = Action::DiscardLines {
        path: "f.txt".into(),
        patch: patch.clone(),
    };
    let pending = journal
        .begin(&runner, action, "Discard lines".into())
        .unwrap();
    stage::apply_patch(&runner, &patch, Target::Discard).unwrap();
    let entry = journal.finish(&runner, pending).unwrap();
    let text = String::from_utf8(repo.read("f.txt")).unwrap();
    assert!(text.contains("TOP") && text.contains("line 18") && !text.contains("BOTTOM"));

    journal.undo(&runner, &entry).unwrap();
    assert_eq!(repo.read("f.txt"), work);
}

#[test]
fn commit_and_amend_land_at_head_and_undo_takes_them_back() {
    // FR-STAGE-05 and FR-STAGE-06.
    let repo = TestRepo::new();
    // The runner sets no author, so the repository needs one.
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    let commit = |message: &str, amend: bool| {
        let action = Action::Commit {
            message: message.into(),
            amend,
        };
        let pending = journal.begin(&runner, action, "Commit".into()).unwrap();
        let oid = stage::commit(&runner, message, amend).unwrap();
        (oid, journal.finish(&runner, pending).unwrap())
    };

    repo.write("a.txt", b"one\n");
    stage::stage(&runner, &["a.txt".into()]).unwrap();
    let message = "--amend looks like an option\n\nBody with \"quotes\".";
    let (first, first_entry) = commit(message, false);
    assert_eq!(repo.git(1, &["rev-parse", "HEAD"]), &*first);
    assert_eq!(repo.git(1, &["log", "-1", "--format=%B"]), message);

    repo.write("a.txt", b"two\n");
    stage::stage(&runner, &["a.txt".into()]).unwrap();
    let (second, _) = commit("Second", false);
    repo.write("a.txt", b"three\n");
    stage::stage(&runner, &["a.txt".into()]).unwrap();
    let (amended, amend_entry) = commit("Second, amended", true);
    assert_ne!(second, amended);
    assert_eq!(repo.git(1, &["rev-parse", "HEAD^"]), &*first);
    assert_eq!(
        repo.git(1, &["log", "-1", "--format=%s"]),
        "Second, amended"
    );

    // The first commit is no longer where HEAD is.
    assert_eq!(
        journal.can_undo(&runner, &first_entry),
        Err(Blocked::RepositoryChanged)
    );
    journal.undo(&runner, &amend_entry).unwrap();
    assert_eq!(repo.git(1, &["rev-parse", "HEAD"]), &*second);
    // A soft reset: the amended content is still staged.
    assert_eq!(repo.git_bytes(&["show", ":a.txt"]), b"three\n");
}

#[test]
fn the_first_commit_can_be_undone() {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    let runner = repo.runner(true);
    let journal = Journal::new(&repo.path().join(".git"));
    repo.write("a.txt", b"one\n");
    stage::stage(&runner, &["a.txt".into()]).unwrap();
    let action = Action::Commit {
        message: "First".into(),
        amend: false,
    };
    let pending = journal.begin(&runner, action, "Commit".into()).unwrap();
    stage::commit(&runner, "First", false).unwrap();
    let entry = journal.finish(&runner, pending).unwrap();
    journal.undo(&runner, &entry).unwrap();
    assert_eq!(repo.git(1, &["status", "--porcelain"]), "A  a.txt");
}

#[test]
fn an_untrusted_repository_refuses_every_change() {
    let repo = TestRepo::new();
    repo.write("a.txt", b"one\n");
    repo.commit_all("base");
    repo.write("a.txt", b"two\n");
    repo.write("new.txt", b"new\n");
    let runner = repo.runner(false);
    let paths = ["a.txt".into()];
    let kind = |result: Result<(), gittree_core::GitError>| result.unwrap_err().kind;

    assert_eq!(kind(stage::stage(&runner, &paths)), GitErrorKind::Untrusted);
    assert_eq!(
        kind(stage::unstage(&runner, &paths, true)),
        GitErrorKind::Untrusted
    );
    assert_eq!(
        kind(stage::discard_files(
            &runner,
            &paths,
            &["new.txt".into()],
            true
        )),
        GitErrorKind::Untrusted
    );
    assert_eq!(
        kind(stage::commit(&runner, "no", false).map(drop)),
        GitErrorKind::Untrusted
    );
    let unstaged = diff_worktree(&runner, &entry(&repo, "a.txt"), false, false, false).unwrap();
    let all = patch::hunk_lines(&unstaged, 0);
    for target in [Target::Stage, Target::Discard] {
        let result = stage::apply_selection(&runner, &unstaged, &all, target);
        assert_eq!(kind(result), GitErrorKind::Untrusted);
    }
    // Nothing moved, and the untracked file is still there.
    assert_eq!(
        repo.git(1, &["status", "--porcelain"]),
        " M a.txt\n?? new.txt"
    );
}

#[test]
fn a_file_or_a_pattern_is_added_to_gitignore() {
    let repo = TestRepo::new();
    repo.write("build/out.log", b"log\n");
    repo.write("a[1].tmp", b"x\n");
    repo.write(".gitignore", b"target");
    let runner = repo.runner(true);

    let exact = stage::ignore_pattern(&"a[1].tmp".into());
    assert_eq!(exact, "/a\\[1].tmp");
    stage::add_ignore(&runner, &exact).unwrap();
    stage::add_ignore(&runner, "*.log").unwrap();
    assert_eq!(repo.read(".gitignore"), b"target\n/a\\[1].tmp\n*.log\n");
    // Git agrees: both files are now ignored.
    let left = repo.git(1, &["status", "--porcelain", "--untracked-files=all"]);
    assert_eq!(left, "?? .gitignore");

    assert!(stage::add_ignore(&repo.runner(false), "x").is_err());
    assert_eq!(stage::ignore_pattern(&"#notes ".into()), "/\\#notes\\ ");
}

#[test]
fn the_identity_is_read_and_set_for_this_repository_only() {
    let repo = TestRepo::new();
    let runner = repo.runner(true);
    let home_config =
        std::fs::read_to_string(std::env::var("GIT_CONFIG_GLOBAL").unwrap()).unwrap_or_default();
    stage::set_identity(&runner, "Grace", "grace@example.com").unwrap();
    assert_eq!(
        stage::identity(&runner),
        ("Grace".to_owned(), "grace@example.com".to_owned())
    );
    assert_eq!(repo.git(1, &["config", "--local", "user.name"]), "Grace");
    // The global config is as it was.
    assert_eq!(
        std::fs::read_to_string(std::env::var("GIT_CONFIG_GLOBAL").unwrap()).unwrap_or_default(),
        home_config
    );
    assert!(stage::set_identity(&runner, "--global", "x@example.com").is_err());
    assert!(stage::set_identity(&runner, "A\nB", "x@example.com").is_err());
    assert!(stage::set_identity(&repo.runner(false), "Ada", "ada@example.com").is_err());
}
