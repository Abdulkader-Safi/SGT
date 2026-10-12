mod common;

use std::path::Path;

use common::{TestRepo, isolate_git};
use gittree_core::GitErrorKind;
use gittree_core::model::{Head, ObjectFormat};
use gittree_core::ops::setup::{find_git, init, open};
use gittree_core::store::TrustList;

const GIT: &str = "git";

#[test]
fn finds_the_installed_git() {
    isolate_git();
    let version = find_git(None).unwrap();
    assert!((version.major, version.minor) >= (2, 39));
    assert!(version.text.starts_with("git version "));
}

#[test]
fn a_missing_git_is_reported_as_not_found() {
    isolate_git();
    let error = find_git(Some(Path::new("/nonexistent/git"))).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::GitNotFound);
}

#[test]
fn a_program_that_is_not_git_is_reported_as_not_found() {
    isolate_git();
    let error = find_git(Some(Path::new("/bin/echo"))).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::GitNotFound);
}

#[test]
fn an_old_git_is_reported_with_its_version() {
    isolate_git();
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("git");
    std::fs::write(&fake, "#!/bin/sh\necho 'git version 2.30.1'\n").unwrap();
    make_executable(&fake);
    let error = find_git(Some(&fake)).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::GitTooOld);
    assert_eq!(error.stderr, "git version 2.30.1");
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn opens_a_repository_from_a_subfolder() {
    let repo = TestRepo::new();
    repo.git(1, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let sub = repo.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();

    let opened = open(GIT.as_ref(), &sub, &TrustList::default()).unwrap();
    let root = repo.path().canonicalize().unwrap();
    assert_eq!(opened.path, root);
    assert_eq!(opened.git_dir, root.join(".git"));
    assert_eq!(opened.common_dir, root.join(".git"));
    assert_eq!(opened.object_format, ObjectFormat::Sha1);
    assert_eq!(opened.head, Head::Branch("main".into()));
    assert!(!opened.trusted);
}

#[test]
fn a_repository_on_the_trust_list_opens_trusted() {
    let repo = TestRepo::new();
    let mut trust = TrustList::default();
    trust.insert(&repo.path().canonicalize().unwrap());
    assert!(open(GIT.as_ref(), repo.path(), &trust).unwrap().trusted);
}

#[test]
fn head_is_unborn_then_a_branch_then_detached() {
    let repo = TestRepo::new();
    let head = || {
        open(GIT.as_ref(), repo.path(), &TrustList::default())
            .unwrap()
            .head
    };
    assert_eq!(head(), Head::Unborn("main".into()));

    repo.git(1, &["commit", "-q", "--allow-empty", "-m", "first"]);
    assert_eq!(head(), Head::Branch("main".into()));

    repo.git(0, &["switch", "-q", "--detach"]);
    assert_eq!(
        head(),
        Head::Detached(repo.git(0, &["rev-parse", "HEAD"]).into())
    );
}

#[test]
fn a_plain_folder_is_not_a_repository() {
    isolate_git();
    let dir = tempfile::tempdir().unwrap();
    let error = open(GIT.as_ref(), dir.path(), &TrustList::default()).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::NotARepository);
}

#[test]
fn a_missing_folder_is_an_error_but_not_a_missing_git() {
    isolate_git();
    let error = open(
        GIT.as_ref(),
        Path::new("/nonexistent/folder"),
        &TrustList::default(),
    )
    .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Other);
}

#[test]
fn init_creates_a_trusted_empty_repository() {
    isolate_git();
    let dir = tempfile::tempdir().unwrap();
    let mut trust = TrustList::default();
    let repo = init(GIT.as_ref(), dir.path(), &mut trust).unwrap();

    assert!(dir.path().join(".git").is_dir());
    assert!(repo.trusted);
    assert!(matches!(repo.head, Head::Unborn(_)));
    assert_eq!(trust.paths, [dir.path().canonicalize().unwrap()]);
}

#[test]
fn init_on_an_existing_repository_does_not_trust_it() {
    let repo = TestRepo::new();
    let mut trust = TrustList::default();
    let opened = init(GIT.as_ref(), repo.path(), &mut trust).unwrap();
    assert!(!opened.trusted);
    assert!(trust.paths.is_empty());
}
