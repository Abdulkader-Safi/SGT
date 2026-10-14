//! Headless tests of the M1 shell: Git check, open, init, trust and what is
//! saved between launches.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use gittree::app_state::{AppState, AppStateHandle, GitCheck, Loaded};
use gittree::views::root::RootView;
use gittree_core::model::Head;
use gittree_core::store::{Settings, Store, ThemeChoice};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};

/// Points Git at an empty home folder, so tests never touch the real config.
fn isolate_git() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("home");
        std::fs::create_dir_all(&home).unwrap();
        let config = home.join(".gitconfig");
        std::fs::write(&config, "").unwrap();
        // SAFETY: runs once, before the first Git call of this test binary.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("GIT_CONFIG_GLOBAL", &config);
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            std::env::remove_var("GITTREE_GIT");
        }
    });
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(status.status.success(), "git {args:?}");
}

struct Shell {
    app: Entity<AppState>,
    window: AnyWindowHandle,
}

/// Starts the app on a data folder, as `main` does.
fn launch(data: &Path, cli_path: Option<PathBuf>, cx: &mut TestAppContext) -> Shell {
    isolate_git();
    gittree::repo_state::WATCH_FILES.store(false, std::sync::atomic::Ordering::Relaxed);
    gittree::repo_state::PROMPTS.store(false, std::sync::atomic::Ordering::Relaxed);
    let loaded = Loaded::read(Store::new(data));
    let (app, window) = cx.update(|cx| {
        gpui_kit::init(cx);
        let app = cx.new(|cx| AppState::new(loaded, cli_path, cx));
        cx.set_global(AppStateHandle(app.clone()));
        gittree::actions::init(cx);
        let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| RootView::new(app.clone(), window, cx))
        })
        .unwrap();
        (app, window)
    });
    cx.run_until_parked();
    Shell { app, window }
}

impl Shell {
    fn in_window<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        let result = cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                act(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    /// Writes everything to disk, as quitting does.
    fn quit(self, cx: &mut TestAppContext) {
        // The writer task finishes the queue once the executor runs dry.
        drop(cx.update(|cx| self.app.update(cx, |app, _| app.flush())));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn opens_a_repository_untrusted_then_remembers_trust(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    isolate_git();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "first"],
    );
    let root = gittree_core::ops::setup::canonical(repo.path());

    let shell = launch(data.path(), Some(repo.path().to_owned()), cx);
    cx.update(|cx| {
        let open = shell
            .app
            .read(cx)
            .repo
            .as_ref()
            .expect("repository is open")
            .read(cx);
        assert_eq!(open.repository.path, root);
        assert!(!open.repository.trusted);
        assert!(!open.runner.trusted());
    });

    // The banner's button trusts it.
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    cx.update(|cx| {
        let open = shell.app.read(cx).repo.as_ref().unwrap().read(cx);
        assert!(open.repository.trusted && open.runner.trusted());
    });
    assert!(shell.in_window(cx, |window, _| window.try_find("trust").is_none()));
    shell.quit(cx);

    // A second launch with no path reopens it, still trusted, and lists it.
    let shell = launch(data.path(), None, cx);
    cx.update(|cx| {
        let app = shell.app.read(cx);
        let open = app
            .repo
            .as_ref()
            .expect("the session's repository reopens")
            .read(cx);
        assert_eq!(open.repository.path, root);
        assert!(open.repository.trusted);
        assert_eq!(app.recent.repos[0].path, root);
    });
}

#[gpui_kit::test]
fn a_plain_folder_gets_an_offer_to_create_a_repository(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let shell = launch(data.path(), Some(folder.path().to_owned()), cx);
    cx.update(|cx| assert!(shell.app.read(cx).repo.is_none()));

    // The dialog is up. Enter confirms it.
    assert!(shell.in_window(cx, |window, _| window.try_find("dialog").is_some()));
    shell.in_window(cx, |window, cx| window.press("enter", cx));

    assert!(folder.path().join(".git").is_dir());
    cx.update(|cx| {
        let open = shell
            .app
            .read(cx)
            .repo
            .as_ref()
            .expect("the new repository is open")
            .read(cx);
        assert!(
            open.repository.trusted,
            "a repository the app created is trusted"
        );
        assert!(matches!(open.repository.head, Head::Unborn(_)));
    });
}

#[gpui_kit::test]
fn a_missing_git_blocks_the_app_until_the_path_is_fixed(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    Store::new(data.path())
        .save(&Settings {
            git_path: Some("/nonexistent/git".into()),
            ..Default::default()
        })
        .unwrap();

    let shell = launch(data.path(), None, cx);
    cx.update(|cx| assert!(matches!(shell.app.read(cx).git, GitCheck::Failed(_))));
    assert!(shell.in_window(cx, |window, _| window.find("retry").visible()));

    cx.update(|cx| {
        shell
            .app
            .update(cx, |app, cx| app.update_settings(|s| s.git_path = None, cx))
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(matches!(shell.app.read(cx).git, GitCheck::Ready(_))));
    assert!(shell.in_window(cx, |window, _| window.try_find("retry").is_none()));
}

#[gpui_kit::test]
fn settings_and_the_session_survive_a_restart(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shell = launch(data.path(), None, cx);
    cx.update(|cx| {
        shell.app.update(cx, |app, cx| {
            app.update_settings(
                |settings| {
                    settings.theme = ThemeChoice::Dark;
                    settings.ui_font_size = 15.0;
                },
                cx,
            );
            app.update_session(|session| session.sidebar_width = 300.0);
        })
    });
    shell.quit(cx);

    let shell = launch(data.path(), None, cx);
    cx.update(|cx| {
        let app = shell.app.read(cx);
        assert_eq!(app.settings.theme, ThemeChoice::Dark);
        assert_eq!(app.settings.ui_font_size, 15.0);
        assert_eq!(app.session.sidebar_width, 300.0);
    });
}

#[gpui_kit::test]
fn browses_history_details_diffs_and_search(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write("main.rs", "one\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "Start the project"]);
    git(dir.path(), &["tag", "v0.1"]);
    write("main.rs", "two\n");
    write("notes.txt", "hello\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "Add notes"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert!(!repo.loading);
        let summaries: Vec<&str> = repo.history.commits.iter().map(|c| &*c.summary).collect();
        assert_eq!(summaries, ["Add notes", "Start the project"]);
        let names: Vec<&str> = repo.refs.iter().map(|r| r.short_name()).collect();
        assert_eq!(names, ["main", "v0.1"]);
    });

    // Selecting a commit loads its details, files and the first diff.
    cx.update(|cx| repo.update(cx, |repo, cx| repo.select_row(0, cx)));
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let selection = &repo.read(cx).selection;
        assert_eq!(selection.details.as_ref().unwrap().message, "Add notes");
        let files: Vec<String> = selection.files.iter().map(|f| f.path.to_string()).collect();
        assert_eq!(files, ["main.rs", "notes.txt"]);
        let diff = selection
            .diff
            .as_ref()
            .expect("the first file's diff is loaded");
        assert_eq!(diff.hunks[0].lines.len(), 2);
    });

    // The down arrow moves the selection, and it stops at the last row.
    cx.update(|cx| repo.update(cx, |repo, cx| repo.move_selection(5, cx)));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(repo.read(cx).selected_row(), Some(1)));

    // Search by text, then by file through Git.
    cx.update(|cx| repo.update(cx, |repo, cx| repo.set_search("notes".into(), cx)));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(repo.read(cx).search.matches, [0]));
    cx.update(|cx| repo.update(cx, |repo, cx| repo.set_search("file:main.rs".into(), cx)));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(repo.read(cx).search.matches, [0, 1]));

    // A commit made outside the app shows after a refs refresh, which is
    // what the file watcher asks for.
    write("notes.txt", "more\n");
    git(dir.path(), &["commit", "-q", "-am", "Made in a terminal"]);
    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.refresh(gittree::repo_state::Scope::Refs, cx)
        })
    });
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert_eq!(&*repo.history.commits[0].summary, "Made in a terminal");
        assert_eq!(repo.history.commits.len(), 3);
        // The search left "Add notes" selected. It stays selected, now one
        // row lower.
        assert_eq!(repo.selected_row(), Some(1));
        assert_eq!(repo.search.matches, [1, 2]);
    });
}

#[gpui_kit::test]
fn stages_commits_amends_discards_and_undoes(cx: &mut TestAppContext) {
    use gittree::repo_state::{Op, Scope};

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
    let git_out = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    };
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    // The app's own commits need an author.
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    write("a.rs", "fn main() {}\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    let refresh = |cx: &mut TestAppContext| {
        cx.update(|cx| repo.update(cx, |repo, cx| repo.refresh(Scope::Status, cx)));
        cx.run_until_parked();
        shell.in_window(cx, |_, _| ());
    };
    let click = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
        shell.in_window(cx, |window, cx| window.click(id, cx));
        // Once more, so the next click sees what this one changed.
        shell.in_window(cx, |_, _| ());
    };
    shell.in_window(cx, |window, cx| window.click("trust", cx));

    // No changes, no work-in-progress row.
    assert!(shell.in_window(cx, |window, _| window.try_find("wip").is_none()));
    write("a.rs", "fn main() {\n    run();\n}\n");
    write("new.txt", "new\n");
    refresh(cx);
    click(cx, "wip".into());
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert_eq!(repo.status.len(), 2);
        assert!(repo.selection.wip);
        let (path, staged) = repo.selection.wip_file.clone().unwrap();
        assert_eq!((path.to_string().as_str(), staged), ("a.rs", false));
        assert!(repo.selection.diff.is_some());
    });

    // Commit does nothing without a summary, or with nothing staged.
    click(cx, "commit".into());
    click(cx, ("stage", 0usize).into());
    cx.update(|cx| assert_eq!(repo.read(cx).changes(true).count(), 1));
    click(cx, "commit".into());
    assert_eq!(git_out(&["log", "--format=%s"]), "base");

    click(cx, "commit-summary".into());
    shell.in_window(cx, |window, cx| window.input("Call run", cx));
    click(cx, "commit".into());
    assert_eq!(git_out(&["log", "-1", "--format=%B"]), "Call run");
    assert_eq!(git_out(&["status", "--porcelain"]), "?? new.txt");
    cx.update(|cx| {
        let repo = repo.read(cx);
        // The graph has a new first row, and one file is still left.
        assert_eq!(&*repo.history.commits[0].summary, "Call run");
        assert!(repo.selection.wip && repo.status.len() == 1);
    });

    // Amend: the message comes back, and the rest goes into the same commit.
    click(cx, "stage-all".into());
    click(cx, "amend".into());
    click(cx, "commit".into());
    assert_eq!(git_out(&["log", "--format=%s"]), "Call run\nbase");
    assert_eq!(git_out(&["status", "--porcelain"]), "");
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert_eq!(repo.history.commits.len(), 2);
        // Nothing is left to commit, so the new commit is selected.
        assert!(!repo.selection.wip);
        assert_eq!(
            repo.selection.oid.as_ref(),
            Some(&repo.history.commits[0].oid)
        );
    });
    assert!(shell.in_window(cx, |window, _| window.try_find("wip").is_none()));

    // Discard asks first, and Enter means Cancel.
    let edited = "fn main() {\n    run();\n    more();\n}\n";
    write("a.rs", edited);
    refresh(cx);
    click(cx, "wip".into());
    click(cx, ("discard", 0usize).into());
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    shell.in_window(cx, |_, _| ());
    assert_eq!(read("a.rs"), edited);

    click(cx, ("discard", 0usize).into());
    // The dialog slides in over a quarter of a second of real time, and
    // takes no clicks until it has arrived.
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |_, _| ());
    click(cx, "confirm".into());
    assert_eq!(read("a.rs"), "fn main() {\n    run();\n}\n");

    // Undo brings the discarded change back, then says there is no more.
    cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(Op::Undo, cx)));
    cx.run_until_parked();
    assert_eq!(read("a.rs"), edited);
}

#[gpui_kit::test]
fn branches_tags_and_stashes_through_the_app(cx: &mut TestAppContext) {
    use gittree::repo_state::Op;
    use gittree_core::ops::branch::Target;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    let git_out = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    };
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    write("a.txt", "one\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    write("a.txt", "two\n");
    git(dir.path(), &["commit", "-q", "-am", "second"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let run = |cx: &mut TestAppContext, op: Op| {
        cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(op, cx)));
        cx.run_until_parked();
        shell.in_window(cx, |_, _| ());
    };

    // New Branch (Cmd+B) opens the dialog. Typing a name and
    // pressing Enter creates the branch and checks it out.
    shell.in_window(cx, |window, cx| {
        window.dispatch_action(Box::new(gittree::actions::NewBranch), cx)
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |window, cx| window.input("feature/x", cx));
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    assert_eq!(git_out(&["rev-parse", "--abbrev-ref", "HEAD"]), "feature/x");
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert!(
            repo.refs
                .iter()
                .any(|r| r.is_current && r.short_name() == "feature/x")
        );
    });

    // Undo goes back to main and removes the branch.
    run(cx, Op::Undo);
    assert_eq!(git_out(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(git_out(&["branch", "--list", "feature/x"]), "");

    // A checkout that would overwrite a change asks, and stashing first
    // gets through.
    git(dir.path(), &["branch", "old", "HEAD~1"]);
    write("a.txt", "mine\n");
    run(cx, Op::Checkout(Target::Branch("old".into())));
    assert_eq!(git_out(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert!(shell.in_window(cx, |window, _| window.try_find("dialog").is_some()));
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    assert_eq!(git_out(&["rev-parse", "--abbrev-ref", "HEAD"]), "old");
    cx.update(|cx| assert_eq!(repo.read(cx).stashes.len(), 1));

    // Pop the stash back on main, tag, then delete the merged branch.
    run(cx, Op::Checkout(Target::Branch("main".into())));
    run(cx, Op::StashPop(0));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "mine\n"
    );
    cx.update(|cx| assert!(repo.read(cx).stashes.is_empty()));
    run(
        cx,
        Op::CreateTag {
            name: "v1".into(),
            at: "HEAD".into(),
            message: Some("First release".into()),
        },
    );
    assert_eq!(git_out(&["cat-file", "-t", "v1"]), "tag");
    cx.update(|cx| repo.update(cx, |repo, cx| repo.delete_branch("old".into(), cx)));
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |_, _| ());
    shell.in_window(cx, |window, cx| window.click("confirm", cx));
    assert_eq!(git_out(&["branch", "--list", "old"]), "");
}

#[gpui_kit::test]
fn a_conflicting_merge_is_resolved_in_the_app(cx: &mut TestAppContext) {
    use gittree::repo_state::Op;
    use gittree_core::ops::merge::OpKind;
    use gittree_core::parse::conflict::Choice;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
    let git_out = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    };
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    write("f.txt", "one\ntwo\nthree\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    git(dir.path(), &["switch", "-q", "-c", "side"]);
    write("f.txt", "one\nSIDE\nthree\n");
    git(dir.path(), &["commit", "-q", "-am", "side"]);
    git(dir.path(), &["switch", "-q", "main"]);
    write("f.txt", "one\nMAIN\nthree\n");
    git(dir.path(), &["commit", "-q", "-am", "main"]);
    let before = git_out(&["rev-parse", "HEAD"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let run = |cx: &mut TestAppContext, op: Op| {
        cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(op, cx)));
        cx.run_until_parked();
        shell.in_window(cx, |_, _| ());
        cx.run_until_parked();
    };

    run(
        cx,
        Op::Merge {
            branch: "side".into(),
            no_ff: false,
        },
    );
    cx.update(|cx| {
        let repo = repo.read(cx);
        let op = repo.in_progress.as_ref().expect("the merge stopped");
        assert_eq!((op.kind, op.conflicted), (OpKind::Merge, 1));
        // The conflict view opened on the file by itself.
        assert!(repo.resolving && repo.selection.wip);
        let file = repo.conflict.as_ref().expect("the conflicted file is open");
        assert_eq!(file.regions.len(), 1);
    });
    // Continue is not possible yet.
    assert!(shell.in_window(cx, |window, _| window.try_find("continue-op").is_none()));

    cx.update(|cx| repo.update(cx, |repo, cx| repo.choose(0, Choice::Both, cx)));
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    shell.in_window(cx, |window, cx| window.click("mark-resolved", cx));
    shell.in_window(cx, |_, _| ());
    assert_eq!(read("f.txt"), "one\nMAIN\nSIDE\nthree\n");
    cx.update(|cx| assert_eq!(repo.read(cx).in_progress.as_ref().unwrap().conflicted, 0));

    shell.in_window(cx, |window, cx| window.click("continue-op", cx));
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| assert!(repo.read(cx).in_progress.is_none()));
    assert_eq!(
        git_out(&["rev-list", "--parents", "-1", "HEAD"])
            .split(' ')
            .count(),
        3
    );

    // One undo takes the whole merge back.
    run(cx, Op::Undo);
    assert_eq!(git_out(&["rev-parse", "HEAD"]), before);
    assert_eq!(read("f.txt"), "one\nMAIN\nthree\n");
}

/// The askpass spike (M6): Git asks through this app's binary, the question
/// reaches the listener, and the answer goes back to Git.
#[test]
fn git_gets_credentials_through_the_askpass_helper() {
    use gittree_core::askpass::{Server, is_secret};
    use std::io::Write as _;

    isolate_git();
    let server = Server::start(|question| {
        Some(if is_secret(question) { "s3cret" } else { "ada" }.to_owned())
    })
    .unwrap();
    let helper = std::path::Path::new(env!("CARGO_BIN_EXE_gittree"));
    let mut child = Command::new("git")
        .args(["-c", "credential.helper=", "credential", "fill"])
        .envs(server.env(helper))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=example.com\n\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("username=ada") && text.contains("password=s3cret"),
        "{text}"
    );

    // Once the listener is gone nothing hangs: Git just fails.
    let env = server.env(helper);
    drop(server);
    let mut child = Command::new("git")
        .args(["-c", "credential.helper=", "credential", "fill"])
        .envs(env)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=example.com\n\n")
        .unwrap();
    assert!(!child.wait().unwrap().success());
}

#[gpui_kit::test]
fn pushes_fetches_pulls_and_clones_through_the_app(cx: &mut TestAppContext) {
    use gittree::repo_state::Op;
    use gittree_core::store::PullMode;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let origin = tempfile::tempdir().unwrap();
    let url = origin.path().to_str().unwrap().to_owned();
    let git_in = |folder: &Path, args: &[&str]| {
        let out = Command::new("git")
            .current_dir(folder)
            .args(["-c", "user.name=Bo", "-c", "user.email=bo@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    };
    isolate_git();
    git_in(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    git_in(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git_in(dir.path(), &["add", "-A"]);
    git_in(dir.path(), &["commit", "-q", "-m", "first"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    // Network commands run on real threads: give them time, then let the
    // app take in the result.
    let run = |cx: &mut TestAppContext, op: Op| {
        cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(op, cx)));
        for _ in 0..200 {
            cx.run_until_parked();
            if !cx.update(|cx| repo.read(cx).busy) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        cx.run_until_parked();
    };

    run(
        cx,
        Op::AddRemote {
            name: "origin".into(),
            url: url.clone(),
        },
    );
    cx.update(|cx| assert_eq!(repo.read(cx).remotes[0].fetch_url, url));

    // The first push publishes the branch and sets its upstream.
    run(cx, Op::Push { force: false });
    assert_eq!(
        git_in(
            dir.path(),
            &["rev-parse", "--abbrev-ref", "main@{upstream}"]
        ),
        "origin/main"
    );
    assert_eq!(
        git_in(origin.path(), &["rev-parse", "main"]),
        git_in(dir.path(), &["rev-parse", "main"])
    );

    // Someone else pushes. Fetch shows it as behind, Pull brings it in.
    let other = tempfile::tempdir().unwrap();
    git_in(other.path(), &["clone", "-q", &url, "."]);
    std::fs::write(other.path().join("b.txt"), "theirs\n").unwrap();
    git_in(other.path(), &["add", "-A"]);
    git_in(other.path(), &["commit", "-q", "-m", "theirs"]);
    git_in(other.path(), &["push", "-q", "origin", "main"]);
    run(cx, Op::Fetch);
    cx.update(|cx| {
        let repo = repo.read(cx);
        let main = repo.refs.iter().find(|r| r.is_current).unwrap();
        assert_eq!((main.ahead, main.behind), (0, 1));
    });
    run(cx, Op::Pull(PullMode::FfOnly));
    assert!(dir.path().join("b.txt").exists());

    // Clone opens the new repository, trusted.
    let dest = tempfile::tempdir().unwrap();
    let target = dest.path().join("copy");
    cx.update(|cx| {
        shell.app.update(cx, |app, cx| {
            app.clone_repo(url.clone(), target.clone(), cx)
        })
    });
    for _ in 0..200 {
        cx.run_until_parked();
        if cx.update(|cx| shell.app.read(cx).clone.is_none()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    for _ in 0..50 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.update(|cx| {
        let open = shell.app.read(cx).repo.as_ref().unwrap().read(cx);
        assert_eq!(
            open.repository.path,
            gittree_core::ops::setup::canonical(&target)
        );
        assert!(open.repository.trusted);
    });
    assert!(target.join("b.txt").exists());
}

#[gpui_kit::test]
fn undo_redo_snapshots_and_the_palette(cx: &mut TestAppContext) {
    use gittree::repo_state::Op;
    use gittree_core::ops::merge::ResetMode;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    let git_out = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    };
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    write("a.txt", "one\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    write("a.txt", "two\n");
    git(dir.path(), &["commit", "-q", "-am", "second"]);
    let second = git_out(&["rev-parse", "HEAD"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let run = |cx: &mut TestAppContext, op: Op| {
        cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(op, cx)));
        cx.run_until_parked();
        shell.in_window(cx, |_, _| ());
        cx.run_until_parked();
    };
    let info = |cx: &mut TestAppContext| cx.update(|cx| repo.read(cx).undo_info.clone());
    assert_eq!(info(cx).undo, None);

    // A hard reset with an uncommitted change saves a snapshot.
    write("a.txt", "work in progress\n");
    run(
        cx,
        Op::Reset {
            target: "HEAD~1".into(),
            mode: ResetMode::Hard,
        },
    );
    assert_ne!(git_out(&["rev-parse", "HEAD"]), second);
    let undo = info(cx);
    assert!(undo.undo.is_some() && !undo.undo_blocked && undo.redo.is_none());
    cx.update(|cx| assert_eq!(repo.read(cx).snapshots.len(), 1));

    // Undo puts the commit and the change back, and offers redo.
    run(cx, Op::Undo);
    assert_eq!(git_out(&["rev-parse", "HEAD"]), second);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "work in progress\n"
    );
    assert!(info(cx).redo.is_some());
    run(cx, Op::Redo);
    assert_ne!(git_out(&["rev-parse", "HEAD"]), second);
    assert!(info(cx).redo.is_none());

    // The snapshot can still come back as a branch.
    let id = cx.update(|cx| repo.read(cx).snapshots[0].id.clone());
    run(
        cx,
        Op::RestoreSnapshot {
            id,
            branch: Some("rescued".into()),
        },
    );
    assert_eq!(git_out(&["show", "rescued:a.txt"]), "work in progress");

    // A commit made outside the app blocks undo, and the app knows.
    write("b.txt", "b\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "made in a terminal"]);
    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.refresh(gittree::repo_state::Scope::Refs, cx)
        })
    });
    cx.run_until_parked();
    assert!(info(cx).undo_blocked);

    // The palette finds a command by name and runs it.
    shell.in_window(cx, |window, cx| {
        window.dispatch_action(Box::new(gittree::actions::CommandPalette), cx)
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |window, cx| window.input("new branch", cx));
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |window, cx| window.input("from-palette", cx));
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    shell.in_window(cx, |_, _| ());
    assert_eq!(
        git_out(&["rev-parse", "--abbrev-ref", "HEAD"]),
        "from-palette"
    );
}

/// FR-AGENT-03: blame and history of the selected commit's file, each drawn
/// once, and gone when another commit is selected.
#[gpui_kit::test]
fn shows_blame_and_history_of_a_file(cx: &mut TestAppContext) {
    use gittree::repo_state::FileMode;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write("old.rs", "one\ntwo\nthree\nfour\nfive\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "Start"]);
    git(dir.path(), &["mv", "old.rs", "new.rs"]);
    git(dir.path(), &["commit", "-q", "-m", "Move it"]);
    write("new.rs", "one\nTWO\nthree\nfour\nfive\n");
    git(dir.path(), &["commit", "-q", "-am", "Shout"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    cx.update(|cx| repo.update(cx, |repo, cx| repo.select_row(0, cx)));
    cx.run_until_parked();

    cx.update(|cx| repo.update(cx, |repo, cx| repo.show_file(FileMode::Blame, cx)));
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let view = repo.read(cx).file_view.as_ref().unwrap();
        let blame = view.blame.as_ref().unwrap();
        let summaries: Vec<&str> = blame.iter().map(|line| &*line.summary).collect();
        assert_eq!(summaries, ["Start", "Shout", "Start", "Start", "Start"]);
    });

    cx.update(|cx| repo.update(cx, |repo, cx| repo.show_file(FileMode::History, cx)));
    cx.run_until_parked();
    cx.update(|cx| repo.update(cx, |repo, cx| repo.pick_file_commit(1, cx)));
    cx.run_until_parked();
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let view = repo.read(cx).file_view.as_ref().unwrap();
        let history = view.history.as_ref().unwrap();
        let summaries: Vec<&str> = history.iter().map(|c| &*c.commit.summary).collect();
        assert_eq!(summaries, ["Shout", "Move it", "Start"]);
        // The blame read earlier is kept.
        assert!(view.blame.is_some());
        let diff = view.diff.as_ref().expect("the picked commit's diff");
        assert_eq!(diff.old_path.as_ref().unwrap().to_string(), "old.rs");
    });

    cx.update(|cx| repo.update(cx, |repo, cx| repo.select_row(2, cx)));
    cx.run_until_parked();
    cx.update(|cx| assert!(repo.read(cx).file_view.is_none()));
}

/// FR-AGENT-04: a session gets a worktree on a new branch, the agent runs
/// in it with the task as its last argument, and a clean session can be
/// removed. A stand-in script plays the agent.
#[cfg(unix)]
#[gpui_kit::test]
fn an_agent_session_runs_in_its_own_worktree(cx: &mut TestAppContext) {
    use gittree_core::agent::{AgentKind, SessionState};
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    let script = outside.path().join("agent.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"$1\" > task.txt\necho all done\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let worktree = gittree_core::ops::setup::canonical(outside.path()).join("wt");

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&gittree::repo_state::RepoState) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| done(repo.read(cx))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out");
    };

    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.start_session(
                AgentKind::Other,
                script.to_str().unwrap(),
                "write the file",
                "agent/write".into(),
                worktree.clone(),
                cx,
            )
        })
    });
    wait(cx, &|repo| {
        repo.sessions
            .first()
            .is_some_and(|s| s.state != SessionState::Working)
    });
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let repo = repo.read(cx);
        let session = &repo.sessions[0];
        assert_eq!(
            (session.state, session.note.as_str(), session.base.as_str()),
            (SessionState::Done, "all done", "main")
        );
        assert_eq!(repo.centre, gittree::repo_state::Centre::Agents);
        // The app made the worktree, so it is on the trust list.
        assert!(shell.app.read(cx).trust.contains(&worktree));
    });
    assert_eq!(
        std::fs::read_to_string(worktree.join("task.txt")).unwrap(),
        "write the file\n"
    );
    // The main working tree was left alone.
    assert!(!dir.path().join("task.txt").exists());

    // Its new file keeps the worktree from being removed.
    cx.update(|cx| repo.update(cx, |repo, cx| repo.remove_session("agent/write", cx)));
    wait(cx, &|repo| !repo.busy);
    assert!(worktree.join("task.txt").exists());
    cx.update(|cx| assert_eq!(repo.read(cx).sessions.len(), 1));
    std::fs::remove_file(worktree.join("task.txt")).unwrap();
    cx.update(|cx| repo.update(cx, |repo, cx| repo.remove_session("agent/write", cx)));
    wait(cx, &|repo| repo.sessions.is_empty());
    assert!(!worktree.exists());
}

/// FR-AGENT-05: the chosen agent writes a commit message from the staged
/// diff and explains a commit. A stand-in script plays the agent and saves
/// what it was asked.
#[cfg(unix)]
#[gpui_kit::test]
fn an_agent_writes_a_message_and_explains_a_commit(cx: &mut TestAppContext) {
    use gittree::repo_state::{RepoEvent, Scope};
    use gittree_core::agent::AgentKind;
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() { run() }\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    let asked = outside.path().join("asked.txt");
    let script = outside.path().join("agent.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > '{}'\nprintf '```\\nCall run\\n\\nFrom main.\\n```\\n'\n",
            asked.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = script.to_str().unwrap().to_owned();

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    cx.update(|cx| repo.update(cx, |repo, cx| repo.refresh(Scope::Status, cx)));
    cx.run_until_parked();
    let written = std::rc::Rc::new(std::cell::RefCell::new(None));
    let _watch = cx.update(|cx| {
        let written = written.clone();
        cx.subscribe(&repo, move |_, event: &RepoEvent, _| {
            if let RepoEvent::MessageWritten(message) = event {
                *written.borrow_mut() = Some(message.clone());
            }
        })
    });
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&gittree::repo_state::RepoState) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| done(repo.read(cx))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out");
    };

    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.write_message(AgentKind::Other, &command, cx)
        })
    });
    cx.update(|cx| assert!(repo.read(cx).ai.writing));
    wait(cx, &|repo| !repo.ai.writing);
    // The code fence the agent wrapped it in is gone.
    assert_eq!(written.borrow().as_deref(), Some("Call run\n\nFrom main."));
    let question = std::fs::read_to_string(&asked).unwrap();
    assert!(question.starts_with("Write a Git commit message"));
    assert!(question.contains("+fn main() { run() }"));

    cx.update(|cx| repo.update(cx, |repo, cx| repo.select_row(0, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.explain_commit(AgentKind::Other, &command, cx)
        })
    });
    wait(cx, &|repo| repo.ai.explained.is_some());
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let repo = repo.read(cx);
        let (oid, text) = repo.ai.explained.as_ref().unwrap();
        assert_eq!(Some(oid), repo.selection.oid.as_ref());
        assert_eq!(text, "Call run\n\nFrom main.");
    });
    assert!(
        std::fs::read_to_string(&asked)
            .unwrap()
            .starts_with("Explain what the commit")
    );
}

/// FR-AGENT-05: the chosen agent sorts uncommitted work into commits, and
/// applying the plan makes them, each with its own files and undo entry.
/// A stand-in script plays the agent.
#[cfg(unix)]
#[gpui_kit::test]
fn an_agent_plans_commits_and_the_plan_is_applied(cx: &mut TestAppContext) {
    use gittree::repo_state::Scope;
    use gittree_core::agent::AgentKind;
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    // The app commits with the repository's own identity. Linux has no
    // fallback from the host name, as macOS does.
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() { run() }\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn run() {}\n").unwrap();
    // Staged, and left out of the plan: it must end up in no commit.
    std::fs::write(dir.path().join("c.txt"), "notes\n").unwrap();
    git(dir.path(), &["add", "c.txt"]);
    let asked = outside.path().join("asked.txt");
    let script = outside.path().join("agent.sh");
    let plan = r#"[{"message":"Call run","files":["a.rs","--force"]},{"message":"Add run","files":["b.rs","a.rs"]}]"#;
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > '{}'\ncat <<'END'\n```json\n{plan}\n```\nEND\n",
            asked.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = script.to_str().unwrap().to_owned();

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    cx.update(|cx| repo.update(cx, |repo, cx| repo.refresh(Scope::Status, cx)));
    cx.run_until_parked();
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&gittree::repo_state::RepoState) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| done(repo.read(cx))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out");
    };
    let log = |format: &str| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(["log", format, "--name-only"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.plan_commits(AgentKind::Other, &command, cx)
        })
    });
    wait(cx, &|repo| !repo.ai.planning);
    let question = std::fs::read_to_string(&asked).unwrap();
    assert!(
        question.contains(r#"["a.rs","c.txt","b.rs"]"#)
            && question.contains("+fn main() { run() }")
    );
    // Planning alone commits nothing.
    assert_eq!(log("--format=%s"), "base\n\na.rs\n");
    let plan = cx.update(|cx| repo.read(cx).ai.plan.clone().unwrap());
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].files, ["a.rs".into()]);
    assert_eq!(plan[1].files, ["b.rs".into()]);
    // The composer opened with the plan, and Cancel commits nothing.
    shell.in_window(cx, |window, cx| window.click("cancel", cx));

    // Any Git error while the plan is applied fails the test by name.
    let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _watch = cx.update(|cx| {
        let errors = errors.clone();
        cx.subscribe(
            &repo,
            move |_, event: &gittree::repo_state::RepoEvent, _| {
                if let gittree::repo_state::RepoEvent::Error(error) = event {
                    errors.borrow_mut().push(format!("{error:?}"));
                }
            },
        )
    });
    cx.update(|cx| repo.update(cx, |repo, cx| repo.apply_plan(plan, cx)));
    wait(cx, &|repo| !repo.busy);
    assert_eq!(*errors.borrow(), Vec::<String>::new());
    assert_eq!(
        log("--format=%s"),
        "Add run\n\nb.rs\nCall run\n\na.rs\nbase\n\na.rs\n"
    );
    let status = Command::new("git")
        .current_dir(dir.path())
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "?? c.txt\n");
    let journal = gittree_core::journal::Journal::new(&dir.path().join(".git"));
    let labels: Vec<String> = journal.entries().into_iter().map(|e| e.label).collect();
    assert_eq!(labels, ["Commit \"Call run\"", "Commit \"Add run\""]);
}

/// FR-AGENT-02: a change an agent asks for through the MCP server waits
/// for the banner's Allow, and the call is listed afterwards.
#[gpui_kit::test]
fn an_agents_change_waits_for_allow_and_is_listed(cx: &mut TestAppContext) {
    use gittree_core::mcp::{self, Outcome, Server};
    use gittree_core::store::McpAccess;
    use serde_json::json;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() { run() }\n").unwrap();

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    let (runner, common) = cx.update(|cx| {
        let repo = repo.read(cx);
        (repo.runner.clone(), repo.repository.common_dir.clone())
    });
    // The server is its own process in real use. Here it is a thread.
    let server = Server::new(runner, &common, McpAccess::ReadWrite)
        .asking(std::time::Duration::from_secs(20));
    let agent = std::thread::spawn(move || {
        let reply = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "git_add_or_commit",
                    "arguments": { "action": "add", "paths": ["a.rs"] } } }))
            .unwrap();
        reply["result"]["isError"].as_bool().unwrap()
    });
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&gittree::repo_state::RepoState) -> bool| {
        for _ in 0..400 {
            cx.update(|cx| repo.update(cx, |repo, cx| repo.load_agent_calls(cx)));
            cx.run_until_parked();
            if cx.update(|cx| done(repo.read(cx))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out");
    };

    wait(cx, &|repo| !repo.agent_requests.is_empty());
    cx.update(|cx| assert_eq!(repo.read(cx).agent_requests[0].detail, "add a.rs"));
    shell.in_window(cx, |window, cx| window.click("agent-allow", cx));
    assert!(!agent.join().unwrap(), "the change went through");
    wait(cx, &|repo| !repo.agent_calls.is_empty());
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert!(repo.agent_requests.is_empty());
        assert_eq!(repo.agent_calls[0].tool, "git_add_or_commit");
        assert_eq!(repo.agent_calls[0].outcome, Outcome::Done);
    });
    assert!(mcp::requests(&common).is_empty());
    let status = Command::new("git")
        .current_dir(dir.path())
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "M  a.rs\n");
    // The list draws in the agents view.
    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.show_centre(gittree::repo_state::Centre::Agents, cx)
        })
    });
    shell.in_window(cx, |_, _| ());
}

/// FR-MERGE-06: the merge dialog draws with its conflict prediction and
/// merges nothing on its own. `predict_conflicts` has its own test.
#[gpui_kit::test]
fn the_merge_dialog_predicts_conflicts(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("f.txt"), "one\ntwo\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    git(dir.path(), &["switch", "-q", "-c", "side"]);
    std::fs::write(dir.path().join("f.txt"), "one\nSIDE\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "side"]);
    git(dir.path(), &["switch", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "one\nMAIN\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "main"]);

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| {
        gittree::views::dialogs::merge(repo, "side".into(), "main".into(), window, cx)
    });
    // The prediction runs on the background executor, then the dialog
    // draws with it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    shell.in_window(cx, |_, _| ());
    assert!(shell.in_window(cx, |window, _| window.try_find("cancel").is_some()));
    // Nothing was merged.
    let head = Command::new("git")
        .current_dir(dir.path())
        .args(["log", "-1", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(head.stdout).unwrap(), "main\n");
}

/// FR-UNDO-03: the reflog lists where HEAD has been, and any step can
/// become a branch.
#[gpui_kit::test]
fn the_reflog_lists_heads_steps(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("f.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "second"],
    );

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    shell.in_window(cx, |window, cx| {
        window.dispatch_action(Box::new(gittree::actions::ShowReflog), cx)
    });
    let mut found = false;
    for _ in 0..200 {
        found = shell.in_window(cx, |window, _| {
            window.try_find(("reflog-branch", 1usize)).is_some()
        });
        if found {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(found, "both steps are listed");
    // The oldest step opens "Create branch" at its commit.
    // Let the dialog's opening animation end, so the click lands.
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |_, _| ());
    shell.in_window(cx, |window, cx| window.click(("reflog-branch", 1usize), cx));
    std::thread::sleep(std::time::Duration::from_millis(400));
    shell.in_window(cx, |window, cx| window.input("from-reflog", cx));
    shell.in_window(cx, |window, cx| window.press("enter", cx));
    let at = |rev: &str| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", rev])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(at("from-reflog"), at("main~1"));
}

/// FR-AGENT-06: the Launchpad reads pull requests through `gh`, and a new
/// pull request is opened after the branch is pushed. A stand-in script
/// plays `gh`.
#[cfg(unix)]
#[gpui_kit::test]
fn the_launchpad_reads_and_creates_pull_requests_through_gh(cx: &mut TestAppContext) {
    use gittree::repo_state::Centre;
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    isolate_git();
    let origin = outside.path().join("origin.git");
    git(
        outside.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    git(
        dir.path(),
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "main"]);
    git(dir.path(), &["switch", "-q", "-c", "keys"]);
    std::fs::write(dir.path().join("a.rs"), "fn main() { keys() }\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "Add keys"]);
    let asked = outside.path().join("asked.txt");
    let script = outside.path().join("gh");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
case "$1 $2" in
  "api user") echo ada ;;
  "repo view") echo main ;;
  "pr list") echo '[{{"number":3,"title":"One","author":{{"login":"grace"}},"isDraft":true,"statusCheckRollup":[]}}]' ;;
  "issue list") echo '[]' ;;
  "pr create") printf '%s\n' "$@" > '{}'; echo "https://example.com/pull/4" ;;
  *) exit 1 ;;
esac
"#,
            asked.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: only this test reads the variable.
    unsafe { std::env::set_var("GITTREE_GH", &script) };

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&gittree::repo_state::RepoState) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| done(repo.read(cx))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out");
    };

    cx.update(|cx| repo.update(cx, |repo, cx| repo.show_centre(Centre::Launchpad, cx)));
    wait(cx, &|repo| !repo.launchpad.loading);
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let repo = repo.read(cx);
        assert_eq!(repo.launchpad.error, None);
        let found = repo.launchpad.data.as_ref().unwrap();
        assert_eq!((found.login.as_str(), found.mine[0].number), ("ada", 3));
        assert_eq!(
            repo.pull_request_branches(),
            Some(("keys".to_owned(), "main".to_owned()))
        );
    });

    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.create_pull_request("Add keys".into(), "Why.".into(), false, cx)
        })
    });
    wait(cx, &|repo| !repo.busy && asked.exists());
    assert_eq!(
        std::fs::read_to_string(&asked).unwrap(),
        "pr\ncreate\n--head=keys\n--base=main\n--title=Add keys\n--body=Why.\n"
    );
    // The branch reached the remote before gh was asked.
    let pushed = Command::new("git")
        .current_dir(&origin)
        .args(["rev-parse", "--verify", "-q", "refs/heads/keys"])
        .output()
        .unwrap();
    assert!(pushed.status.success());
}

/// FR-AGENT-05: an agent's conflict result reaches the result box and
/// changes nothing on disk. A stand-in script plays the agent.
#[cfg(unix)]
#[gpui_kit::test]
fn an_agent_suggests_a_conflict_result_without_staging_it(cx: &mut TestAppContext) {
    use gittree::repo_state::Op;
    use gittree_core::agent::AgentKind;
    use std::os::unix::fs::PermissionsExt;

    let data = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    isolate_git();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Ada"]);
    git(dir.path(), &["config", "user.email", "ada@example.com"]);
    write("f.txt", "one\ntwo\nthree\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    git(dir.path(), &["switch", "-q", "-c", "side"]);
    write("f.txt", "one\nSIDE\nthree\n");
    git(dir.path(), &["commit", "-q", "-am", "side"]);
    git(dir.path(), &["switch", "-q", "main"]);
    write("f.txt", "one\nMAIN\nthree\n");
    git(dir.path(), &["commit", "-q", "-am", "main"]);
    let asked = outside.path().join("asked.txt");
    let script = outside.path().join("agent.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > '{}'\nprintf 'one\\nBOTH\\nthree\\n'\n",
            asked.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = script.to_str().unwrap().to_owned();

    let shell = launch(data.path(), Some(dir.path().to_owned()), cx);
    let repo = cx.update(|cx| shell.app.read(cx).repo.clone().unwrap());
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    let merge = Op::Merge {
        branch: "side".into(),
        no_ff: false,
    };
    cx.update(|cx| repo.update(cx, |repo, cx| repo.run_op(merge, cx)));
    for _ in 0..4 {
        cx.run_until_parked();
        shell.in_window(cx, |_, _| ());
    }
    cx.update(|cx| {
        assert!(
            repo.read(cx).conflict.is_some(),
            "the conflicted file is open"
        )
    });

    cx.update(|cx| {
        repo.update(cx, |repo, cx| {
            repo.suggest_conflict(AgentKind::Other, &command, cx)
        })
    });
    for _ in 0..200 {
        cx.run_until_parked();
        if cx.update(|cx| !repo.read(cx).ai.suggesting) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // The view has already taken the suggestion into its result box.
    shell.in_window(cx, |_, _| ());
    cx.update(|cx| {
        let ai = &repo.read(cx).ai;
        assert!(ai.suggestion.is_none());
        let (path, agent) = ai.suggested_by.as_ref().expect("a suggestion arrived");
        assert_eq!(
            (path.to_string(), agent.as_str()),
            ("f.txt".to_owned(), command.as_str())
        );
    });
    // The agent saw the markers, and the file on disk still has them.
    assert!(std::fs::read_to_string(&asked).unwrap().contains("<<<<<<<"));
    let on_disk = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(on_disk.contains("<<<<<<<") && on_disk.contains("SIDE"));
}

/// The settings window opens and draws without a panic, and a tool switch
/// reaches the saved settings. Only the page that shows first is drawn.
#[gpui_kit::test]
fn the_settings_window_draws_and_a_tool_can_be_switched_off(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shell = launch(data.path(), None, cx);
    cx.update(gittree::views::settings::open);
    cx.run_until_parked();
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 2);
    for window in windows {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    cx.update(|cx| {
        shell.app.update(cx, |app, cx| {
            app.update_settings(|s| s.mcp_tools_off.push("git_stash".into()), cx)
        })
    });
    shell.quit(cx);
    let saved = std::fs::read_to_string(data.path().join("settings.json")).unwrap();
    assert!(saved.contains("git_stash"), "{saved}");
}
