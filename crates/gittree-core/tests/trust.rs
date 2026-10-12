//! FR-SAFE-04: an untrusted repository cannot run code and cannot be changed.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use common::TestRepo;
use gittree_core::GitErrorKind;
use gittree_core::ops::setup::open;
use gittree_core::store::TrustList;

/// A repository whose hooks and fsmonitor command each leave a marker file
/// when Git runs them.
struct Planted {
    repo: TestRepo,
    markers: tempfile::TempDir,
}

impl Planted {
    fn new() -> Self {
        let repo = TestRepo::new();
        let markers = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("file.txt"), "one\n").unwrap();
        repo.git(1, &["add", "file.txt"]);
        repo.git(1, &["commit", "-q", "-m", "first"]);

        let script = |name: &str| {
            format!(
                "#!/bin/sh\ntouch '{}'\n",
                markers.path().join(name).display()
            )
        };
        let write_script = |path: PathBuf, name: &str| {
            std::fs::write(&path, script(name)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let hooks = repo.path().join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        for hook in ["pre-commit", "post-index-change", "post-checkout"] {
            write_script(hooks.join(hook), hook);
        }
        let fsmonitor = repo.path().join(".git/fsmonitor.sh");
        write_script(fsmonitor.clone(), "fsmonitor");
        repo.git(
            0,
            &["config", "core.fsmonitor", fsmonitor.to_str().unwrap()],
        );

        // Make the working tree differ from the index, so status has work.
        std::fs::write(repo.path().join("file.txt"), "two\n").unwrap();
        Self { repo, markers }
    }

    fn markers(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.markers.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

const STATUS: [&str; 3] = ["status", "--porcelain=v2", "-z"];

#[test]
fn plain_git_runs_the_planted_programs() {
    // The control: without the app's overrides the markers do appear, so the
    // next test proves something.
    let planted = Planted::new();
    planted.repo.git(0, &STATUS);
    assert!(planted.markers().contains(&"fsmonitor".to_owned()));
}

#[test]
fn browsing_untrusted_runs_nothing() {
    let planted = Planted::new();
    let repository = open("git".as_ref(), planted.repo.path(), &TrustList::default()).unwrap();
    assert!(!repository.trusted);

    let runner = planted.repo.runner(false);
    runner.read(STATUS).unwrap();
    runner.read(["log", "--all", "--format=%H"]).unwrap();
    runner.read(["diff"]).unwrap();
    assert_eq!(planted.markers(), [] as [&str; 0]);
}

#[test]
fn untrusted_refuses_every_write_before_git_starts() {
    let planted = Planted::new();
    let runner = planted.repo.runner(false);
    let error = runner
        .write(["commit", "-a", "-m", "should not happen"])
        .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Untrusted);
    assert_eq!(planted.markers(), [] as [&str; 0]);
    assert_eq!(planted.repo.git(0, &["rev-list", "--count", "HEAD"]), "1");
    // Nothing ran, so nothing was recorded.
    assert!(runner.activity().is_empty());
}

#[test]
fn trusted_runs_hooks_like_the_command_line() {
    let planted = Planted::new();
    let runner = planted.repo.runner(true);
    runner
        .run(
            gittree_core::Access::Write,
            [
                "-c",
                "user.name=Ada",
                "-c",
                "user.email=ada@example.com",
                "commit",
                "-a",
                "-m",
                "second",
            ],
            None,
            None,
            |_| Ok(()),
        )
        .unwrap();
    assert!(planted.markers().contains(&"pre-commit".to_owned()));
    assert_eq!(planted.repo.git(0, &["rev-list", "--count", "HEAD"]), "2");
}

#[test]
fn calls_are_recorded_with_their_result() {
    let planted = Planted::new();
    let runner = planted.repo.runner(false);
    runner.read(["rev-parse", "HEAD"]).unwrap();
    runner.read(["rev-parse", "--verify", "nope"]).unwrap_err();

    let activity = runner.activity();
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0].args, ["rev-parse", "HEAD"]);
    assert_eq!(activity[0].exit_code, 0);
    assert_ne!(activity[1].exit_code, 0);
    assert!(!activity[1].stderr.is_empty());
}
