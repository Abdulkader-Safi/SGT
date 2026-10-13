use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gittree_core::graph::{GraphLayout, GraphRow};
use gittree_core::model::{
    Commit, CommitDetails, FileChange, FileDiff, Head, Oid, Ref, Repository,
};
use gittree_core::ops::read;
use gittree_core::{GitError, GitRunner};
use gpui_kit::{AppContext, Context, EventEmitter, Task};
use notify::Watcher;

/// What to read again from Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Refs and HEAD. The log follows if they moved.
    Refs,
    Log,
    All,
}

pub enum RepoEvent {
    /// A Git call failed and the user should hear about it.
    Error(GitError),
    /// The graph should bring this row into view.
    Reveal(usize),
}

/// The commit list with its graph rows.
#[derive(Default)]
pub struct History {
    pub commits: Vec<Commit>,
    pub rows: Vec<GraphRow>,
    /// Widest row, in lanes.
    pub lanes: usize,
    index: HashMap<Oid, u32>,
}

impl History {
    fn extend(&mut self, commits: Vec<Commit>, rows: Vec<GraphRow>) {
        let start = self.commits.len();
        for (offset, (commit, row)) in commits.iter().zip(&rows).enumerate() {
            self.index
                .insert(commit.oid.clone(), (start + offset) as u32);
            let widest = row
                .edges
                .iter()
                .map(|edge| edge.from_lane.max(edge.to_lane))
                .fold(row.lane, u16::max);
            self.lanes = self.lanes.max(widest as usize + 1);
        }
        self.commits.extend(commits);
        self.rows.extend(rows);
    }

    pub fn row_of(&self, oid: &str) -> Option<usize> {
        self.index.get(oid).map(|&row| row as usize)
    }
}

/// The commit shown in the detail panel.
#[derive(Default)]
pub struct Selection {
    pub oid: Option<Oid>,
    pub details: Option<CommitDetails>,
    pub files: Vec<FileChange>,
    pub file: Option<usize>,
    pub diff: Option<FileDiff>,
    pub diff_loading: bool,
}

#[derive(Default)]
pub struct Search {
    pub query: String,
    /// Matching rows, top to bottom.
    pub matches: Vec<usize>,
    /// Which match is current.
    pub at: usize,
    pub running: bool,
}

type Batch = (Vec<Commit>, Vec<GraphRow>);

/// The single source of truth for one open repository. Views read it and
/// call its methods. They hold no repository data and never run Git.
pub struct RepoState {
    pub repository: Repository,
    pub runner: GitRunner,
    pub history: History,
    /// True until the first load of the history has finished.
    pub loading: bool,
    pub log_error: Option<GitError>,
    pub refs: Vec<Ref>,
    /// Indexes into `refs`, by the commit each points at.
    pub refs_at: HashMap<Oid, Vec<usize>>,
    pub selection: Selection,
    pub search: Search,
    // A result is dropped unless its generation is still the current one.
    log_generation: u64,
    log_cancel: Arc<AtomicBool>,
    log_task: Task<()>,
    refs_task: Task<()>,
    details_task: Task<()>,
    diff_task: Task<()>,
    search_task: Task<()>,
    _watch: Task<()>,
}

impl EventEmitter<RepoEvent> for RepoState {}

impl RepoState {
    pub fn new(repository: Repository, runner: GitRunner, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            _watch: watch(&repository.path, cx),
            repository,
            runner,
            history: History::default(),
            loading: true,
            log_error: None,
            refs: Vec::new(),
            refs_at: HashMap::new(),
            selection: Selection::default(),
            search: Search::default(),
            log_generation: 0,
            log_cancel: Default::default(),
            log_task: Task::ready(()),
            refs_task: Task::ready(()),
            details_task: Task::ready(()),
            diff_task: Task::ready(()),
            search_task: Task::ready(()),
        };
        this.refresh(Scope::All, cx);
        this
    }

    /// Swaps in a runner after the repository's trust changed.
    pub fn set_trusted(&mut self, runner: GitRunner, cx: &mut Context<Self>) {
        self.repository.trusted = runner.trusted();
        self.runner = runner;
        cx.notify();
    }

    pub fn has_commits(&self) -> bool {
        !matches!(self.repository.head, Head::Unborn(_))
    }

    pub fn refresh(&mut self, scope: Scope, cx: &mut Context<Self>) {
        match scope {
            Scope::Refs => self.refresh_refs(false, cx),
            Scope::Log => self.reload_log(cx),
            Scope::All => {
                self.refresh_refs(true, cx);
                self.reload_log(cx);
            }
        }
    }

    /// Reads refs and HEAD. If they moved, the log is read again too.
    fn refresh_refs(&mut self, quiet_log: bool, cx: &mut Context<Self>) {
        let runner = self.runner.clone();
        let read = cx.background_spawn(async move {
            Ok::<_, GitError>((read::list_refs(&runner)?, read::head(&runner)?))
        });
        // Replacing the task drops an older read that is still running.
        self.refs_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((refs, head)) => {
                    if refs == this.refs && head == this.repository.head {
                        return;
                    }
                    this.refs_at.clear();
                    for (ix, r) in refs.iter().enumerate() {
                        this.refs_at.entry(r.target.clone()).or_default().push(ix);
                    }
                    let first = this.refs.is_empty();
                    this.refs = refs;
                    this.repository.head = head;
                    if !quiet_log && !first {
                        this.reload_log(cx);
                    }
                    cx.notify();
                }
                Err(error) => cx.emit(RepoEvent::Error(error)),
            });
        });
    }

    /// Reads the history again. The first load shows rows as they arrive. A
    /// later one builds the new list aside and swaps it in whole, so the
    /// graph never flickers through a half-loaded state.
    fn reload_log(&mut self, cx: &mut Context<Self>) {
        self.log_cancel.store(true, Ordering::Relaxed);
        self.log_cancel = Default::default();
        self.log_generation += 1;
        let generation = self.log_generation;
        let streaming = self.history.commits.is_empty();
        self.log_error = None;

        let (tx, batches) = smol::channel::unbounded::<Batch>();
        let runner = self.runner.clone();
        let cancel = self.log_cancel.clone();
        let git = cx.background_spawn(async move {
            let mut layout = GraphLayout::default();
            read::stream_log(
                &runner,
                |commits| {
                    let rows = commits
                        .iter()
                        .map(|commit| layout.push(&commit.oid, &commit.parents))
                        .collect();
                    let _ = tx.send_blocking((commits, rows));
                },
                &cancel,
            )
        });
        self.log_task = cx.spawn(async move |this, cx| {
            let mut fresh = History::default();
            while let Ok((commits, rows)) = batches.recv().await {
                if streaming {
                    let live = this.update(cx, |this, cx| {
                        if this.log_generation == generation {
                            this.history.extend(commits, rows);
                            cx.notify();
                        }
                    });
                    if live.is_err() {
                        return;
                    }
                } else {
                    fresh.extend(commits, rows);
                }
            }
            let result = git.await;
            let _ = this.update(cx, |this, cx| {
                if this.log_generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(()) => {
                        if !streaming {
                            this.history = fresh;
                        }
                        // Recount the matches, but leave the selection alone.
                        this.run_search(false, cx);
                    }
                    Err(error) => this.log_error = Some(error),
                }
                cx.notify();
            });
        });
    }

    /// Selects a commit and loads its details and files.
    pub fn select(&mut self, oid: Option<Oid>, cx: &mut Context<Self>) {
        if self.selection.oid == oid {
            return;
        }
        self.selection = Selection {
            oid: oid.clone(),
            ..Default::default()
        };
        self.diff_task = Task::ready(());
        cx.notify();
        let Some(oid) = oid else {
            self.details_task = Task::ready(());
            return;
        };
        let runner = self.runner.clone();
        let read = cx.background_spawn({
            let oid = oid.clone();
            async move {
                Ok::<_, GitError>((
                    read::commit_details(&runner, &oid)?,
                    read::commit_files(&runner, &oid)?,
                ))
            }
        });
        self.details_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.selection.oid.as_ref() != Some(&oid) {
                    return;
                }
                match result {
                    Ok((details, files)) => {
                        this.selection.details = Some(details);
                        this.selection.files = files;
                        if !this.selection.files.is_empty() {
                            this.select_file(0, false, cx);
                        }
                        cx.notify();
                    }
                    Err(error) => cx.emit(RepoEvent::Error(error)),
                }
            });
        });
    }

    /// Selects the commit in a row and asks the graph to show it.
    pub fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(commit) = self.history.commits.get(row) {
            self.select(Some(commit.oid.clone()), cx);
            cx.emit(RepoEvent::Reveal(row));
        }
    }

    pub fn select_oid(&mut self, oid: &str, cx: &mut Context<Self>) {
        if let Some(row) = self.history.row_of(oid) {
            self.select_row(row, cx);
        }
    }

    pub fn selected_row(&self) -> Option<usize> {
        self.history.row_of(self.selection.oid.as_deref()?)
    }

    /// Moves the selection by `delta` rows, stopping at the ends.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let last = self.history.commits.len().saturating_sub(1) as isize;
        let from = self.selected_row().map_or(-1, |row| row as isize);
        self.select_row((from + delta).clamp(0, last) as usize, cx);
    }

    /// Loads the diff of one of the selected commit's files. `load_all`
    /// lifts the cut on a very large diff.
    pub fn select_file(&mut self, ix: usize, load_all: bool, cx: &mut Context<Self>) {
        let (Some(oid), Some(file)) = (self.selection.oid.clone(), self.selection.files.get(ix))
        else {
            return;
        };
        let file = file.clone();
        self.selection.file = Some(ix);
        self.selection.diff_loading = true;
        cx.notify();
        let runner = self.runner.clone();
        let read = cx.background_spawn({
            let oid = oid.clone();
            async move { read::diff_commit_file(&runner, &oid, &file, load_all) }
        });
        self.diff_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.selection.oid.as_ref() != Some(&oid) || this.selection.file != Some(ix) {
                    return;
                }
                this.selection.diff_loading = false;
                match result {
                    Ok(diff) => this.selection.diff = Some(diff),
                    Err(error) => cx.emit(RepoEvent::Error(error)),
                }
                cx.notify();
            });
        });
    }

    /// Searches the loaded commits. Plain words match the summary, author
    /// or hash. `author:` limits to the author, and `file:` asks Git which
    /// commits touched a path.
    pub fn set_search(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search.query != query {
            self.search.query = query;
            self.search.at = 0;
            self.run_search(true, cx);
        }
    }

    fn run_search(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let query = self.search.query.trim().to_owned();
        self.search_task = Task::ready(());
        self.search.running = false;
        if let Some(path) = query.strip_prefix("file:").map(str::trim) {
            self.search.matches.clear();
            if path.is_empty() {
                cx.notify();
                return;
            }
            self.search.running = true;
            let runner = self.runner.clone();
            let path = path.to_owned();
            let read = cx.background_spawn(async move {
                runner.read([
                    "log",
                    "--exclude=refs/gittree/*",
                    "--all",
                    "--format=%H",
                    "--",
                    &path,
                ])
            });
            self.search_task = cx.spawn(async move |this, cx| {
                let result = read.await;
                let _ = this.update(cx, |this, cx| {
                    this.search.running = false;
                    match result {
                        Ok(out) => {
                            let mut rows: Vec<usize> = String::from_utf8_lossy(&out)
                                .lines()
                                .filter_map(|oid| this.history.row_of(oid))
                                .collect();
                            rows.sort_unstable();
                            this.search.matches = rows;
                            if reveal {
                                this.reveal_match(cx);
                            }
                        }
                        Err(error) => cx.emit(RepoEvent::Error(error)),
                    }
                    cx.notify();
                });
            });
            cx.notify();
            return;
        }

        self.search.matches = search_commits(&self.history.commits, &query);
        self.search.at = self
            .search
            .at
            .min(self.search.matches.len().saturating_sub(1));
        if reveal {
            self.reveal_match(cx);
        }
        cx.notify();
    }

    /// Moves to the next match, or the previous one, wrapping around.
    pub fn step_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.search.matches.len();
        if count == 0 {
            return;
        }
        self.search.at = (self.search.at + if forward { 1 } else { count - 1 }) % count;
        self.reveal_match(cx);
        cx.notify();
    }

    fn reveal_match(&mut self, cx: &mut Context<Self>) {
        if let Some(&row) = self.search.matches.get(self.search.at) {
            self.select_row(row, cx);
        }
    }
}

/// Rows whose commit matches a query that is not a `file:` search.
// ponytail: a linear scan with a lower-cased copy per commit. About 10 ms
// for 100,000 commits. Build an index if search ever feels slow.
pub fn search_commits(commits: &[Commit], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let (author_only, needle) = match query.strip_prefix("author:") {
        Some(name) => (true, name.trim().to_lowercase()),
        None => (false, query.to_lowercase()),
    };
    if needle.is_empty() {
        return Vec::new();
    }
    let has = |text: &str| text.to_lowercase().contains(&needle);
    commits
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let by_author = has(&c.author_name) || has(&c.author_email);
            if author_only {
                by_author
            } else {
                by_author || has(&c.summary) || c.oid.starts_with(&needle)
            }
        })
        .map(|(row, _)| row)
        .collect()
}

/// Headless tests switch the file watcher off. It reports from its own
/// thread, which GPUI's deterministic test scheduler does not allow.
pub static WATCH_FILES: AtomicBool = AtomicBool::new(true);

/// Watches the repository folder and refreshes after things settle for
/// 300 ms. If the watcher cannot start, the app still refreshes on demand.
fn watch(path: &Path, cx: &mut Context<RepoState>) -> Task<()> {
    if !WATCH_FILES.load(Ordering::Relaxed) {
        return Task::ready(());
    }
    let (tx, events) = smol::channel::unbounded::<()>();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.try_send(());
        }
    })
    .and_then(|mut watcher| {
        watcher.watch(path, notify::RecursiveMode::Recursive)?;
        Ok(watcher)
    });
    let watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "file watcher did not start");
            return Task::ready(());
        }
    };
    cx.spawn(async move |this, cx| {
        let _watcher = watcher;
        while events.recv().await.is_ok() {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            while events.try_recv().is_ok() {}
            // M2 has no status view, so every change only needs the refs.
            if this
                .update(cx, |this, cx| this.refresh(Scope::Refs, cx))
                .is_err()
            {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(oid: &str, author: &str, summary: &str) -> Commit {
        Commit {
            oid: oid.into(),
            parents: Vec::new(),
            author_name: author.into(),
            author_email: format!("{}@example.com", author.to_lowercase())
                .as_str()
                .into(),
            author_time: 0,
            summary: summary.into(),
        }
    }

    #[test]
    fn search_matches_summary_author_and_hash() {
        let commits = [
            commit("abc123", "Ada", "Fix the parser"),
            commit("def456", "Grace", "Add a test for Ada's fix"),
            commit("0abc99", "Linus", "Unrelated"),
        ];
        assert_eq!(search_commits(&commits, "fix"), [0, 1]);
        assert_eq!(search_commits(&commits, "ADA"), [0, 1]);
        assert_eq!(search_commits(&commits, "author:ada"), [0]);
        assert_eq!(search_commits(&commits, "author: grace@example"), [1]);
        // A hash matches from its start only.
        assert_eq!(search_commits(&commits, "abc"), [0]);
        assert_eq!(search_commits(&commits, "  "), [] as [usize; 0]);
        assert_eq!(search_commits(&commits, "author:"), [] as [usize; 0]);
    }
}
