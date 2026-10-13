//! The app's own data: settings, session, recent repositories and the trust
//! list. One JSON file each in the app data folder.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A value kept in its own JSON file.
pub trait Persisted: Serialize + DeserializeOwned + Default {
    const FILE: &'static str;
    /// The newest file version this build can read.
    const VERSION: u32;
}

/// Reads and writes the files in one data folder.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    /// Files written by a newer build. They are never overwritten.
    frozen: HashSet<&'static str>,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            frozen: HashSet::new(),
        }
    }

    /// `GITTREE_DATA_DIR` when set, otherwise the Safi Git Tree folder in the
    /// user's application data folder.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("GITTREE_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|dir| dir.join("Safi Git Tree")))
            .unwrap_or_else(|| "Safi Git Tree".into())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Loads a file, or gives the defaults when it is missing, broken or
    /// from a newer build. A broken file is renamed aside first.
    pub fn load<T: Persisted>(&mut self) -> T {
        let path = self.dir.join(T::FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return T::default();
        };
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&bytes);
        let version = parsed
            .as_ref()
            .ok()
            .and_then(|value| value.get("version")?.as_u64());
        if version.is_some_and(|version| version > T::VERSION as u64) {
            tracing::warn!(file = T::FILE, "written by a newer build, using defaults");
            self.frozen.insert(T::FILE);
            return T::default();
        }
        // ponytail: every file is at version 1, so there is nothing to
        // upgrade yet. An upgrade step goes here when a version 2 exists.
        match parsed.and_then(serde_json::from_value) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(file = T::FILE, %error, "unreadable, using defaults");
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                T::default()
            }
        }
    }

    /// Writes a file by writing a temporary one and renaming it over the old
    /// one, so a crash never leaves half a file.
    pub fn save<T: Persisted>(&self, value: &T) -> io::Result<()> {
        if self.frozen.contains(T::FILE) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(T::FILE);
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(temp, path)
    }
}

fn version_1() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DiffView {
    #[default]
    Inline,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PullMode {
    #[default]
    FfOnly,
    Merge,
    Rebase,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub theme: ThemeChoice,
    pub ui_font_size: f32,
    pub diff_font_family: String,
    pub diff_font_size: f32,
    pub diff_view: DiffView,
    pub pull_mode: PullMode,
    /// `None` means the `git` on `PATH`.
    pub git_path: Option<PathBuf>,
    pub clone_dir: PathBuf,
    /// 0 is off.
    pub auto_fetch_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let developer = home.join("Developer");
        Self {
            version: 1,
            theme: ThemeChoice::System,
            ui_font_size: 13.0,
            diff_font_family: "Menlo".into(),
            diff_font_size: 12.0,
            diff_view: DiffView::Inline,
            pull_mode: PullMode::FfOnly,
            git_path: None,
            clone_dir: if developer.is_dir() { developer } else { home },
            auto_fetch_minutes: 0,
        }
    }
}

impl Persisted for Settings {
    const FILE: &'static str = "settings.json";
    const VERSION: u32 = 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub version: u32,
    pub window: Option<WindowState>,
    /// Reopened at launch if it still exists.
    pub open_repo: Option<PathBuf>,
    pub sidebar_width: f32,
    pub detail_size: f32,
    /// The unsent commit message of each repository.
    pub drafts: BTreeMap<PathBuf, String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: 1,
            window: None,
            open_repo: None,
            sidebar_width: 240.0,
            detail_size: 320.0,
            drafts: BTreeMap::new(),
        }
    }
}

impl Persisted for Session {
    const FILE: &'static str = "session.json";
    const VERSION: u32 = 1;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRepo {
    pub path: PathBuf,
    pub name: String,
    /// Unix seconds, UTC.
    pub last_opened: i64,
}

/// Recently opened repositories, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default = "version_1")]
    pub version: u32,
    #[serde(default)]
    pub repos: Vec<RecentRepo>,
}

impl Recent {
    pub const LIMIT: usize = 20;

    /// Puts a repository at the top of the list.
    pub fn touch(&mut self, path: &Path, name: &str, now: i64) {
        self.remove(path);
        self.repos.insert(
            0,
            RecentRepo {
                path: path.to_owned(),
                name: name.to_owned(),
                last_opened: now,
            },
        );
        self.repos.truncate(Self::LIMIT);
    }

    pub fn remove(&mut self, path: &Path) {
        self.repos.retain(|repo| repo.path != path);
    }
}

impl Default for Recent {
    fn default() -> Self {
        Self {
            version: 1,
            repos: Vec::new(),
        }
    }
}

impl Persisted for Recent {
    const FILE: &'static str = "recent.json";
    const VERSION: u32 = 1;
}

/// Canonical working tree paths of the repositories the user trusts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustList {
    #[serde(default = "version_1")]
    pub version: u32,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

impl TrustList {
    pub fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|trusted| trusted == path)
    }

    pub fn insert(&mut self, path: &Path) {
        if !self.contains(path) {
            self.paths.push(path.to_owned());
        }
    }
}

impl Default for TrustList {
    fn default() -> Self {
        Self {
            version: 1,
            paths: Vec::new(),
        }
    }
}

impl Persisted for TrustList {
    const FILE: &'static str = "trust.json";
    const VERSION: u32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        // The folder does not exist yet, as on a first run.
        let store = Store::new(dir.path().join("data"));
        (dir, store)
    }

    fn files(store: &Store) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_gives_defaults() {
        let (_dir, mut store) = store();
        assert_eq!(store.load::<Settings>(), Settings::default());
        assert_eq!(store.load::<Session>(), Session::default());
    }

    #[test]
    fn each_file_round_trips_and_leaves_no_temporary_file() {
        let (_dir, mut store) = store();
        let settings = Settings {
            theme: ThemeChoice::Dark,
            ui_font_size: 15.0,
            git_path: Some("/opt/git/bin/git".into()),
            ..Default::default()
        };
        let mut session = Session {
            open_repo: Some("/work/repo".into()),
            window: Some(WindowState {
                x: 10.0,
                y: 20.0,
                width: 1000.0,
                height: 700.0,
                maximized: false,
            }),
            ..Default::default()
        };
        session
            .drafts
            .insert("/work/repo".into(), "half a message".into());
        let mut recent = Recent::default();
        recent.touch(Path::new("/work/repo"), "repo", 100);
        let mut trust = TrustList::default();
        trust.insert(Path::new("/work/repo"));

        store.save(&settings).unwrap();
        store.save(&session).unwrap();
        store.save(&recent).unwrap();
        store.save(&trust).unwrap();

        assert_eq!(store.load::<Settings>(), settings);
        assert_eq!(store.load::<Session>(), session);
        assert_eq!(store.load::<Recent>(), recent);
        assert_eq!(store.load::<TrustList>(), trust);
        assert_eq!(
            files(&store),
            ["recent.json", "session.json", "settings.json", "trust.json"]
        );
    }

    #[test]
    fn a_missing_field_takes_its_default() {
        let (_dir, mut store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.dir().join("settings.json"),
            r#"{"version":1,"theme":"Light"}"#,
        )
        .unwrap();
        let settings: Settings = store.load();
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert_eq!(settings.ui_font_size, 13.0);
    }

    #[test]
    fn a_broken_file_is_moved_aside_and_replaced_by_defaults() {
        let (_dir, mut store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.dir().join("settings.json"), "{ not json").unwrap();
        assert_eq!(store.load::<Settings>(), Settings::default());
        assert_eq!(files(&store), ["settings.json.bad"]);

        store.save(&Settings::default()).unwrap();
        assert_eq!(files(&store), ["settings.json", "settings.json.bad"]);
    }

    #[test]
    fn a_file_from_a_newer_build_is_never_overwritten() {
        let (_dir, mut store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        let newer = r#"{"version":2,"theme":"Neon"}"#;
        std::fs::write(store.dir().join("settings.json"), newer).unwrap();

        assert_eq!(store.load::<Settings>(), Settings::default());
        store.save(&Settings::default()).unwrap();
        let on_disk = std::fs::read_to_string(store.dir().join("settings.json")).unwrap();
        assert_eq!(on_disk, newer);
    }

    #[test]
    fn recent_keeps_the_newest_twenty_without_repeats() {
        let mut recent = Recent::default();
        for n in 0..25 {
            recent.touch(Path::new(&format!("/r/{n}")), &n.to_string(), n);
        }
        assert_eq!(recent.repos.len(), Recent::LIMIT);
        assert_eq!(recent.repos[0].name, "24");

        recent.touch(Path::new("/r/10"), "10", 99);
        assert_eq!(recent.repos.len(), Recent::LIMIT);
        assert_eq!(recent.repos[0].last_opened, 99);
        assert_eq!(recent.repos.iter().filter(|r| r.name == "10").count(), 1);

        recent.remove(Path::new("/r/10"));
        assert!(recent.repos.iter().all(|r| r.name != "10"));
    }

    #[test]
    fn trust_is_by_exact_path() {
        let mut trust = TrustList::default();
        trust.insert(Path::new("/work/repo"));
        trust.insert(Path::new("/work/repo"));
        assert_eq!(trust.paths.len(), 1);
        assert!(trust.contains(Path::new("/work/repo")));
        assert!(!trust.contains(Path::new("/work/repo/sub")));
        assert!(!trust.contains(Path::new("/work")));
    }
}
