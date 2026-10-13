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
