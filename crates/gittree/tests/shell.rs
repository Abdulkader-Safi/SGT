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
    let root = repo.path().canonicalize().unwrap();

    let shell = launch(data.path(), Some(repo.path().to_owned()), cx);
    cx.update(|cx| {
        let open = shell
            .app
            .read(cx)
            .repo
            .as_ref()
            .expect("repository is open");
        assert_eq!(open.repository.path, root);
        assert!(!open.repository.trusted);
        assert!(!open.runner.trusted());
    });

    // The banner's button trusts it.
    shell.in_window(cx, |window, cx| window.click("trust", cx));
    cx.update(|cx| {
        let open = shell.app.read(cx).repo.as_ref().unwrap();
        assert!(open.repository.trusted && open.runner.trusted());
    });
    assert!(shell.in_window(cx, |window, _| window.try_find("trust").is_none()));
    shell.quit(cx);

    // A second launch with no path reopens it, still trusted, and lists it.
    let shell = launch(data.path(), None, cx);
    cx.update(|cx| {
        let app = shell.app.read(cx);
        let open = app.repo.as_ref().expect("the session's repository reopens");
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
            .expect("the new repository is open");
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
