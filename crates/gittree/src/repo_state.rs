use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gittree_core::graph::{GraphLayout, GraphRow};
use gittree_core::journal::{Action, Blocked, Journal};
use gittree_core::model::{
    Change, Commit, CommitDetails, FileChange, FileDiff, Head, Oid, Ref, RepoPath, Repository,
    StatusEntry,
};
use gittree_core::ops::branch::{self, Stash, Target as Checkout};
use gittree_core::ops::read;
use gittree_core::ops::stage::{self, Target};
use gittree_core::patch::{self, Lines};
use gittree_core::{GitError, GitErrorKind, GitRunner};
use gpui_kit::{AppContext, Context, EventEmitter, Task};
use notify::Watcher;
use rust_i18n::t;

/// What to read again from Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Changed, staged and untracked files.
    Status,
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
    /// A commit was made, so the commit box can clear.
    Committed,
    /// The message of the commit that Amend would replace.
    HeadMessage(String),
    /// Something the user should read, already in plain words.
    Notice(String),
    /// Deleting this branch needs a yes: these commits are only on it.
    ConfirmDeleteBranch {
        name: String,
        lost: Vec<(Oid, String)>,
    },
    /// Local changes are in the way of this checkout.
    CheckoutBlocked(Checkout),
}

/// A change to the repository. `RepoState::run_op` is the only way to make
/// one.
#[derive(Debug, Clone)]
pub enum Op {
    Stage(Vec<RepoPath>),
    Unstage(Vec<RepoPath>),
    /// Stages, unstages or discards the chosen lines of a diff.
    Apply {
        diff: Arc<FileDiff>,
        lines: Lines,
        target: Target,
    },
    DiscardFiles {
        tracked: Vec<RepoPath>,
        untracked: Vec<RepoPath>,
    },
    Commit {
        message: String,
        amend: bool,
    },
    Checkout(Checkout),
    /// Stashes everything, then checks out. For when local changes are in
    /// the way.
    StashAndSwitch(Checkout),
    CreateBranch {
        name: String,
        at: String,
        checkout: bool,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    DeleteBranch {
        name: String,
        force: bool,
    },
    CreateTag {
        name: String,
        at: String,
        message: Option<String>,
    },
    DeleteTag(String),
    StashPush {
        message: String,
        include_untracked: bool,
    },
    StashApply(u32),
    StashPop(u32),
    StashDrop(u32),
    /// Takes back the last undoable action.
    Undo,
}

/// What a finished op has to tell the UI.
enum Done {
    Changed,
    Committed(Oid),
    Notice(String),
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

/// What the detail panel shows: a commit, or the work in progress.
#[derive(Default)]
pub struct Selection {
    pub oid: Option<Oid>,
    pub details: Option<CommitDetails>,
    pub files: Vec<FileChange>,
    pub file: Option<usize>,
    /// The work-in-progress row is selected, so the staging view shows.
    pub wip: bool,
    /// The changed file whose diff shows, and whether its staged side does.
    pub wip_file: Option<(RepoPath, bool)>,
    /// The diff of the selected file, of a commit or of the work in
    /// progress. It is swapped for a new `Arc` only when it changes.
    pub diff: Option<Arc<FileDiff>>,
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
    pub status: Vec<StatusEntry>,
    pub stashes: Vec<Stash>,
    /// An op is running. Views grey out their buttons meanwhile.
    pub busy: bool,
    ops: VecDeque<Op>,
    journal: Journal,
    /// Set by a commit: once nothing is left to commit, the selection moves
    /// from the work-in-progress row to this commit.
    after_commit: Option<Oid>,
    op_task: Task<()>,
    status_task: Task<()>,
    message_task: Task<()>,
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
        let journal = Journal::new(&repository.common_dir);
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
            status: Vec::new(),
            stashes: Vec::new(),
            busy: false,
            ops: VecDeque::new(),
            journal,
            after_commit: None,
            op_task: Task::ready(()),
            status_task: Task::ready(()),
            message_task: Task::ready(()),
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
            Scope::Status => self.refresh_status(cx),
            Scope::Refs => self.refresh_refs(false, cx),
            Scope::Log => self.reload_log(cx),
            Scope::All => {
                self.refresh_status(cx);
                self.refresh_refs(true, cx);
                self.reload_log(cx);
            }
        }
    }

    /// Reads the changed files. A failure is logged and otherwise silent:
    /// the next trigger tries again.
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let runner = self.runner.clone();
        self.status_task = cx.spawn(async move |this, cx| {
            let mut result = Err(None);
            for attempt in 0..2 {
                let runner = runner.clone();
                result = cx
                    .background_spawn(async move { read::status(&runner) })
                    .await
                    .map_err(Some);
                let locked = matches!(&result, Err(Some(e)) if e.kind == GitErrorKind::IndexLocked);
                if !locked || attempt == 1 {
                    break;
                }
                // Another Git process holds the index. One more try.
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
            }
            let _ = this.update(cx, |this, cx| match result {
                Ok(status) => this.set_status(status, cx),
                Err(error) => tracing::warn!(?error, "status failed"),
            });
        });
    }

    fn set_status(&mut self, mut status: Vec<StatusEntry>, cx: &mut Context<Self>) {
        // Conflicted files first, then by path as Git lists them.
        status.sort_by_key(|entry| entry.conflict.is_none());
        self.status = status;
        let after_commit = self.after_commit.take();
        if self.selection.wip {
            match after_commit {
                Some(oid) if self.status.is_empty() => {
                    self.select(Some(oid), cx);
                    return;
                }
                _ => self.reselect_wip_file(cx),
            }
        }
        cx.notify();
    }

    /// The changed files of one list.
    pub fn changes(&self, staged: bool) -> impl Iterator<Item = &StatusEntry> {
        self.status.iter().filter(move |entry| {
            if staged {
                entry.staged()
            } else {
                entry.unstaged()
            }
        })
    }

    /// The work-in-progress row shows while there is something to commit,
    /// and stays while it is selected.
    pub fn wip_visible(&self) -> bool {
        !self.status.is_empty() || self.selection.wip
    }

    /// Selects the work-in-progress row, which opens the staging view.
    pub fn select_wip(&mut self, cx: &mut Context<Self>) {
        if self.selection.wip {
            return;
        }
        self.selection = Selection {
            wip: true,
            ..Default::default()
        };
        self.details_task = Task::ready(());
        self.diff_task = Task::ready(());
        self.after_commit = None;
        self.reselect_wip_file(cx);
        cx.notify();
    }

    /// Keeps the selected changed file if it is still in its list, else
    /// moves to the first file of that list, else of the other one. Its
    /// diff is read again either way, since the file may have changed.
    fn reselect_wip_file(&mut self, cx: &mut Context<Self>) {
        let wanted = self.selection.wip_file.take();
        let side = wanted.as_ref().is_some_and(|(_, staged)| *staged);
        let still =
            wanted.filter(|(path, staged)| self.changes(*staged).any(|entry| &entry.path == path));
        let first = |staged: bool| {
            self.changes(staged)
                .next()
                .map(|e| (e.path.clone(), staged))
        };
        match still.or_else(|| first(side)).or_else(|| first(!side)) {
            Some((path, staged)) => self.select_wip_file(path, staged, false, cx),
            None => {
                self.selection.diff = None;
                self.selection.diff_loading = false;
                self.diff_task = Task::ready(());
            }
        }
    }

    /// Shows the diff of a changed file: its staged side, or what is not
    /// staged yet. `load_all` lifts the cut on a very large diff.
    pub fn select_wip_file(
        &mut self,
        path: RepoPath,
        staged: bool,
        load_all: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self
            .changes(staged)
            .find(|entry| entry.path == path)
            .cloned()
        else {
            return;
        };
        let file = Some((path, staged));
        if self.selection.wip_file != file {
            self.selection.diff = None;
        }
        self.selection.wip_file = file.clone();
        self.selection.diff_loading = true;
        // A reload keeps a cut diff cut, and a fully loaded one whole.
        let load_all = load_all
            || self.selection.diff.as_ref().is_some_and(|diff| {
                !matches!(diff.kind, gittree_core::model::DiffKind::TooLarge { .. })
                    && diff.hunks.iter().map(|h| h.lines.len()).sum::<usize>()
                        > read::LARGE_DIFF_LINES
            });
        cx.notify();
        let runner = self.runner.clone();
        let read =
            cx.background_spawn(
                async move { read::diff_worktree(&runner, &entry, staged, load_all) },
            );
        self.diff_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                if !this.selection.wip || this.selection.wip_file != file {
                    return;
                }
                this.selection.diff_loading = false;
                match result {
                    Ok(diff) => this.set_diff(diff),
                    Err(error) => cx.emit(RepoEvent::Error(error)),
                }
                cx.notify();
            });
        });
    }

    /// Loads all of a diff that was cut for its size.
    pub fn load_whole_diff(&mut self, cx: &mut Context<Self>) {
        if let Some((path, staged)) = self
            .selection
            .wip_file
            .clone()
            .filter(|_| self.selection.wip)
        {
            self.select_wip_file(path, staged, true, cx);
        } else if let Some(file) = self.selection.file {
            self.select_file(file, true, cx);
        }
    }

    /// Keeps the old `Arc` when the diff did not change, so views can tell
    /// a new diff from a reload by its address.
    fn set_diff(&mut self, diff: FileDiff) {
        if self.selection.diff.as_deref() != Some(&diff) {
            self.selection.diff = Some(Arc::new(diff));
        }
    }

    /// Queues a change. Ops run one at a time. An op that can destroy work
    /// or move HEAD gets a journal entry, and the status and refs are read
    /// again when it ends.
    pub fn run_op(&mut self, op: Op, cx: &mut Context<Self>) {
        if !self.runner.trusted() {
            cx.emit(RepoEvent::Error(GitError::untrusted()));
            return;
        }
        self.ops.push_back(op);
        if !self.busy {
            self.next_op(cx);
        }
    }

    fn next_op(&mut self, cx: &mut Context<Self>) {
        let Some(op) = self.ops.pop_front() else {
            self.busy = false;
            // The watcher was muted while ops ran. One refresh covers them.
            self.refresh_status(cx);
            self.refresh_refs(false, cx);
            cx.notify();
            return;
        };
        self.busy = true;
        cx.notify();
        let (runner, journal) = (self.runner.clone(), self.journal.clone());
        let has_commits = self.has_commits();
        let label = op_label(&op);
        let checkout = match &op {
            Op::Checkout(target) => Some(target.clone()),
            _ => None,
        };
        let run =
            cx.background_spawn(async move { execute(op, &runner, &journal, has_commits, label) });
        self.op_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Done::Changed) => {}
                    Ok(Done::Committed(oid)) => {
                        this.after_commit = Some(oid);
                        cx.emit(RepoEvent::Committed);
                    }
                    Ok(Done::Notice(text)) => cx.emit(RepoEvent::Notice(text)),
                    Err(error) => {
                        // What was queued behind a failed op was chosen
                        // against a state that did not come about.
                        this.ops.clear();
                        match checkout {
                            Some(target) if error.kind == GitErrorKind::DirtyWorkTree => {
                                cx.emit(RepoEvent::CheckoutBlocked(target))
                            }
                            _ => cx.emit(RepoEvent::Error(error)),
                        }
                    }
                }
                this.next_op(cx);
            });
        });
    }

    /// Deletes a branch. One with commits of its own is not deleted here:
    /// the view is asked to confirm first, naming those commits.
    pub fn delete_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let runner = self.runner.clone();
        let branch_name = name.clone();
        let check =
            cx.background_spawn(async move { branch::unmerged_commits(&runner, &branch_name) });
        self.message_task = cx.spawn(async move |this, cx| {
            let lost = check.await;
            let _ = this.update(cx, |_, cx| match lost {
                Ok(lost) => cx.emit(RepoEvent::ConfirmDeleteBranch { name, lost }),
                Err(error) => cx.emit(RepoEvent::Error(error)),
            });
        });
    }

    /// Reads the last commit's message, for the Amend toggle.
    pub fn load_head_message(&mut self, cx: &mut Context<Self>) {
        let runner = self.runner.clone();
        let read = cx.background_spawn(async move { runner.read(["log", "-1", "--format=%B"]) });
        self.message_task = cx.spawn(async move |this, cx| {
            if let Ok(out) = read.await {
                let message = String::from_utf8_lossy(&out).trim_end().to_owned();
                let _ = this.update(cx, |_, cx| cx.emit(RepoEvent::HeadMessage(message)));
            }
        });
    }

    /// True when the last commit is already on its upstream, so amending it
    /// will need a force-push.
    pub fn head_is_pushed(&self) -> bool {
        self.refs
            .iter()
            .any(|r| r.is_current && r.upstream.is_some() && r.ahead == 0)
    }

    /// Reads refs and HEAD. If they moved, the log is read again too.
    fn refresh_refs(&mut self, quiet_log: bool, cx: &mut Context<Self>) {
        let runner = self.runner.clone();
        let read = cx.background_spawn(async move {
            Ok::<_, GitError>((
                read::list_refs(&runner)?,
                read::head(&runner)?,
                branch::list_stashes(&runner).unwrap_or_default(),
            ))
        });
        // Replacing the task drops an older read that is still running.
        self.refs_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((refs, head, stashes)) => {
                    if stashes != this.stashes {
                        this.stashes = stashes;
                        cx.notify();
                    }
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
        if self.selection.oid == oid && !self.selection.wip {
            return;
        }
        self.selection = Selection {
            oid: oid.clone(),
            ..Default::default()
        };
        self.after_commit = None;
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
    /// The work-in-progress row, when it shows, sits above the first commit.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let last = self.history.commits.len().saturating_sub(1) as isize;
        let from = self.selected_row().map_or(-1, |row| row as isize);
        let to = from + delta;
        if self.selection.wip && delta < 0 {
            return;
        }
        if to < 0 && from >= 0 && self.wip_visible() {
            self.select_wip(cx);
            return;
        }
        self.select_row(to.clamp(0, last) as usize, cx);
    }

    /// Loads the diff of one of the selected commit's files. `load_all`
    /// lifts the cut on a very large diff.
    pub fn select_file(&mut self, ix: usize, load_all: bool, cx: &mut Context<Self>) {
        let (Some(oid), Some(file)) = (self.selection.oid.clone(), self.selection.files.get(ix))
        else {
            return;
        };
        let file = file.clone();
        if self.selection.file != Some(ix) {
            self.selection.diff = None;
        }
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
                    Ok(diff) => this.set_diff(diff),
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

/// The undo tooltip text of an op that gets a journal entry.
fn op_label(op: &Op) -> String {
    match op {
        Op::Commit { message, .. } => {
            let summary = message.lines().next().unwrap_or_default();
            t!("journal.commit", summary = summary).into_owned()
        }
        Op::DiscardFiles { tracked, untracked } => t!(
            "journal.discard_files",
            count = tracked.len() + untracked.len()
        )
        .into_owned(),
        Op::Apply { diff, .. } => {
            let path = diff.new_path.as_ref().or(diff.old_path.as_ref());
            let path = path.map(ToString::to_string).unwrap_or_default();
            t!("journal.discard_lines", path = path).into_owned()
        }
        Op::Checkout(target) | Op::StashAndSwitch(target) => {
            t!("journal.checkout", target = target_name(target)).into_owned()
        }
        Op::CreateBranch { name, .. } => t!("journal.create_branch", name = name).into_owned(),
        Op::RenameBranch { old, new } => {
            t!("journal.rename_branch", old = old, new = new).into_owned()
        }
        Op::DeleteBranch { name, .. } => t!("journal.delete_branch", name = name).into_owned(),
        Op::CreateTag { name, .. } => t!("journal.create_tag", name = name).into_owned(),
        Op::DeleteTag(name) => t!("journal.delete_tag", name = name).into_owned(),
        Op::StashPush { .. } => t!("journal.stash_push").into_owned(),
        Op::StashDrop(_) => t!("journal.stash_drop").into_owned(),
        Op::Stage(_) | Op::Unstage(_) | Op::Undo | Op::StashApply(_) | Op::StashPop(_) => {
            String::new()
        }
    }
}

/// A checkout target in words.
pub fn target_name(target: &Checkout) -> String {
    match target {
        Checkout::Branch(name) | Checkout::Remote(name) => name.clone(),
        Checkout::Commit(oid) => oid[..7.min(oid.len())].to_owned(),
    }
}

/// Runs one op on the background executor.
fn execute(
    op: Op,
    runner: &GitRunner,
    journal: &Journal,
    has_commits: bool,
    label: String,
) -> Result<Done, GitError> {
    // Snapshot, run, then record. A failed action leaves no entry behind.
    let journaled = |action: Action, run: &dyn Fn() -> Result<Done, GitError>| {
        let pending = journal.begin(runner, action, label.clone())?;
        match run() {
            Ok(done) => journal.finish(runner, pending).map(|_| done),
            Err(error) => {
                journal.abandon(runner, pending);
                Err(error)
            }
        }
    };
    match op {
        Op::Stage(paths) => stage::stage(runner, &paths).map(|()| Done::Changed),
        Op::Unstage(paths) => stage::unstage(runner, &paths, has_commits).map(|()| Done::Changed),
        Op::Apply {
            diff,
            lines,
            target: Target::Discard,
        } => {
            let (Some(patch), Some(path)) = (patch::build(&diff, &lines, true), &diff.new_path)
            else {
                return Ok(Done::Changed);
            };
            let action = Action::DiscardLines {
                path: path.clone(),
                patch: patch.clone(),
            };
            journaled(action, &|| {
                stage::apply_patch(runner, &patch, Target::Discard).map(|()| Done::Changed)
            })
        }
        Op::Apply {
            diff,
            lines,
            target,
        } => stage::apply_selection(runner, &diff, &lines, target).map(|()| Done::Changed),
        Op::DiscardFiles { tracked, untracked } => {
            let action = Action::DiscardFiles {
                tracked: tracked.clone(),
                untracked: untracked.clone(),
            };
            journaled(action, &|| {
                stage::discard_files(runner, &tracked, &untracked, has_commits)
                    .map(|()| Done::Changed)
            })
        }
        Op::Commit { message, amend } => {
            let action = Action::Commit {
                message: message.clone(),
                amend,
            };
            journaled(action, &|| {
                stage::commit(runner, &message, amend).map(Done::Committed)
            })
        }
        Op::Checkout(target) => {
            let action = Action::Checkout {
                target: target_name(&target),
            };
            journaled(action, &|| {
                branch::checkout(runner, &target).map(|()| Done::Changed)
            })
        }
        Op::StashAndSwitch(target) => {
            let stash = Action::StashPush {
                message: label.clone(),
                include_untracked: true,
            };
            journaled(stash, &|| {
                branch::stash_push(runner, &label, true).map(|()| Done::Changed)
            })?;
            let action = Action::Checkout {
                target: target_name(&target),
            };
            journaled(action, &|| {
                branch::checkout(runner, &target).map(|()| Done::Changed)
            })
        }
        Op::CreateBranch { name, at, checkout } => {
            let action = Action::CreateBranch {
                name: name.clone(),
                at: at.clone(),
                checkout,
            };
            journaled(action, &|| {
                branch::create_branch(runner, &name, &at, checkout).map(|()| Done::Changed)
            })
        }
        Op::RenameBranch { old, new } => {
            let action = Action::RenameBranch {
                old: old.clone(),
                new: new.clone(),
            };
            journaled(action, &|| {
                branch::rename_branch(runner, &old, &new).map(|()| Done::Changed)
            })
        }
        Op::DeleteBranch { name, force } => {
            let action = Action::DeleteBranch {
                name: name.clone(),
                force,
            };
            journaled(action, &|| {
                branch::delete_branch(runner, &name, force).map(|()| Done::Changed)
            })
        }
        Op::CreateTag { name, at, message } => {
            let action = Action::CreateTag {
                name: name.clone(),
                at: at.clone(),
                message: message.clone(),
            };
            journaled(action, &|| {
                branch::create_tag(runner, &name, &at, message.as_deref()).map(|()| Done::Changed)
            })
        }
        Op::DeleteTag(name) => {
            let action = Action::DeleteTag { name: name.clone() };
            journaled(action, &|| {
                branch::delete_tag(runner, &name).map(|()| Done::Changed)
            })
        }
        Op::StashPush {
            message,
            include_untracked,
        } => {
            let action = Action::StashPush {
                message: message.clone(),
                include_untracked,
            };
            journaled(action, &|| {
                branch::stash_push(runner, &message, include_untracked).map(|()| Done::Changed)
            })
        }
        Op::StashApply(index) => branch::stash_apply(runner, index).map(|()| Done::Changed),
        Op::StashPop(index) => branch::stash_pop(runner, index).map(|()| Done::Changed),
        Op::StashDrop(index) => {
            let stashes = branch::list_stashes(runner)?;
            let Some(stash) = stashes.into_iter().find(|stash| stash.index == index) else {
                return Ok(Done::Changed);
            };
            let action = Action::StashDrop {
                oid: stash.oid,
                message: stash.message,
            };
            journaled(action, &|| {
                branch::stash_drop(runner, index).map(|()| Done::Changed)
            })
        }
        Op::Undo => {
            let entries = journal.entries();
            let Some(entry) = entries.iter().rev().find(|entry| !entry.undone) else {
                return Ok(Done::Notice(t!("undo.nothing").into_owned()));
            };
            match journal.can_undo(runner, entry) {
                Ok(()) => {
                    journal.undo(runner, entry)?;
                    Ok(Done::Notice(
                        t!("undo.done", label = entry.label).into_owned(),
                    ))
                }
                Err(Blocked::RepositoryChanged | Blocked::AlreadyUndone) => Ok(Done::Notice(
                    t!("undo.blocked", label = entry.label).into_owned(),
                )),
            }
        }
    }
}

/// The paths to pass Git for whole-file actions on these entries. A renamed
/// file needs both of its names.
pub fn entry_paths<'a>(entries: impl IntoIterator<Item = &'a StatusEntry>) -> Vec<RepoPath> {
    entries
        .into_iter()
        .flat_map(|entry| entry.orig_path.iter().chain([&entry.path]).cloned())
        .collect()
}

/// Splits entries into tracked and untracked paths, for a discard.
pub fn discard_paths<'a>(
    entries: impl IntoIterator<Item = &'a StatusEntry>,
) -> (Vec<RepoPath>, Vec<RepoPath>) {
    let (untracked, tracked): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|entry| entry.worktree == Change::Untracked);
    (entry_paths(tracked), entry_paths(untracked))
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
            // ponytail: every event reads the status and the refs. Both are
            // cheap. Split by path (Git folder or working tree) if a huge
            // repository makes this show in a profile.
            let live = this.update(cx, |this, cx| {
                // An op refreshes when it ends, and its own writes should
                // not start a second refresh.
                if !this.busy {
                    this.refresh(Scope::Status, cx);
                    this.refresh(Scope::Refs, cx);
                }
            });
            if live.is_err() {
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
