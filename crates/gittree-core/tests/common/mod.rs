//! Helpers for tests that run real Git.
#![allow(dead_code)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Once;

use tempfile::TempDir;

/// Points Git at an empty home folder, so tests never read or write the
/// developer's real config.
pub fn isolate_git() {
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
        }
    });
}

pub struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    pub fn new() -> Self {
        isolate_git();
        let repo = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git(0, &["init", "-q", "-b", "main"]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// A runner for this repository, as the app would make one.
    pub fn runner(&self, trusted: bool) -> gittree_core::GitRunner {
        gittree_core::GitRunner::new("git", self.path(), trusted)
    }

    /// Runs Git with `time` as the author and committer date. Returns stdout
    /// without the trailing newline.
    pub fn git(&self, time: i64, args: &[&str]) -> String {
        self.git_with_input(time, args, b"")
    }

    /// Writes a file in the working tree, making its folders.
    pub fn write(&self, path: &str, content: &[u8]) {
        let full = self.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }

    pub fn read(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.path().join(path)).unwrap()
    }

    /// Commits everything in the working tree.
    pub fn commit_all(&self, message: &str) {
        self.git(1, &["add", "-A"]);
        self.git(1, &["commit", "-q", "-m", message]);
    }

    /// Git's stdout as raw bytes, for content checks.
    pub fn git_bytes(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .current_dir(self.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed");
        output.stdout
    }

    pub fn git_with_input(&self, time: i64, args: &[&str], input: &[u8]) -> String {
        // The "@" marks a raw Unix timestamp, which Git needs for small values.
        let date = format!("@{time} +0000");
        let mut child = Command::new("git")
            .current_dir(self.path())
            .args(args)
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim_end().into()
    }
}
