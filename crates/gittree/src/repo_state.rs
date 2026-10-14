use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gittree_core::agent::{self, AgentKind, PlannedCommit, Session, SessionState};
use gittree_core::graph::{GraphLayout, GraphRow};
use gittree_core::hosting::{self, Gh};
use gittree_core::journal::{Action, Blocked, Journal};
use gittree_core::mcp;
use gittree_core::model::{
    Change, Commit, CommitDetails, FileChange, FileDiff, Head, Oid, Ref, RepoPath, Repository,
    StatusEntry,
};
use gittree_core::ops::branch::{self, Stash, Target as Checkout};
use gittree_core::ops::file::{self, BlameLine, FileCommit};
use gittree_core::ops::merge::{self, InProgressOp, OpKind, ResetMode, Side};
use gittree_core::ops::read;
use gittree_core::ops::remote::{self, Progress, Remote};
use gittree_core::ops::stage::{self, Target};
use gittree_core::ops::worktree;
use gittree_core::parse::conflict::{Choice, Region, parse_conflicts, resolve};
use gittree_core::patch::{self, Lines};
use gittree_core::store::PullMode;
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
    /// An agent wrote this commit message for the staged changes.
    MessageWritten(String),
    /// An agent's commit plan is in `ai.plan`, ready to be shown.
    Planned,
    /// The app made this worktree, so it joins the trust list.
    WorktreeCreated(std::path::PathBuf),
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
    /// Git or ssh asks for a user name, password or passphrase.
    Credential(CredentialRequest),
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
    /// Empties the index, then makes each planned commit from its files.
    CommitPlan {
        unstage: Vec<RepoPath>,
        plan: Vec<PlannedCommit>,
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
    Merge {
        branch: String,
        no_ff: bool,
    },
    Rebase(String),
    /// Replays the commits after `base` by the user's plan.
    InteractiveRebase {
        base: String,
        items: Vec<merge::TodoItem>,
    },
    CherryPick {
        oid: String,
        mainline: Option<u32>,
    },
    Revert {
        oid: String,
        mainline: Option<u32>,
    },
    Reset {
        target: String,
        mode: ResetMode,
    },
    /// Finishes, gives up or (rebase only) skips the operation in progress.
    ContinueOp,
    AbortOp,
    SkipOp,
    /// Writes the resolved content of a conflicted file and stages it.
    ResolveFile {
        path: RepoPath,
        content: Vec<u8>,
    },
    ResolveSide {
        path: RepoPath,
        side: Side,
    },
    Fetch,
    Pull(PullMode),
    /// Pushes the current branch. `force` uses a lease on the commit the
    /// remote branch had at the last fetch.
    Push {
        force: bool,
    },
    PushTag {
        tag: String,
        delete: bool,
    },
    AddRemote {
        name: String,
        url: String,
    },
    RenameRemote {
        old: String,
        new: String,
    },
    SetRemoteUrl {
        name: String,
        url: String,
    },
    RemoveRemote(String),
    /// Makes the worktree and branch of an agent session.
    StartSession {
        path: std::path::PathBuf,
        branch: String,
    },
    RemoveWorktree(std::path::PathBuf),
    /// Opens a pull request through `gh`. The branch is pushed before it.
    CreatePullRequest {
        head: String,
        base: String,
        title: String,
        body: String,
        draft: bool,
    },
    /// Takes back the last undoable action.
    Undo,
    /// Runs the last undone action again.
    Redo,
    /// Brings back what a snapshot saved, in place or as a new branch.
    RestoreSnapshot {
        id: String,
        branch: Option<String>,
    },
}

/// What Undo and Redo would do right now, for the toolbar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoInfo {
    /// The label of the action Undo would take back.
    pub undo: Option<String>,
    /// Undo is not possible: the repository changed outside the app, or
    /// the action reached a remote.
    pub undo_blocked: bool,
    pub redo: Option<String>,
}

fn undo_info(journal: &Journal, runner: &GitRunner) -> UndoInfo {
    let undo = journal.next_undo();
    let redo = journal
        .next_redo()
        .filter(|entry| journal.can_redo(runner, entry));
    UndoInfo {
        undo_blocked: undo
            .as_ref()
            .is_some_and(|entry| journal.can_undo(runner, entry).is_err()),
        undo: undo.map(|entry| entry.label),
        redo: redo.map(|entry| entry.label),
    }
}

impl Op {
    /// The four commands that talk to a remote.
    fn uses_network(&self) -> bool {
        matches!(
            self,
            Op::Fetch | Op::Pull(_) | Op::Push { .. } | Op::PushTag { .. }
        )
    }
}

/// A question from Git or ssh that the user has to answer. Sending `None`
/// cancels it.
#[derive(Clone)]
pub struct CredentialRequest {
    pub question: String,
    pub secret: bool,
    pub reply: std::sync::mpsc::Sender<Option<String>>,
}

/// What a network op needs besides the runner.
struct Net {
    env: remote::NetEnv,
    cancel: Arc<AtomicBool>,
    progress: smol::channel::Sender<Progress>,
    pull_mode: PullMode,
    /// The current branch, its remote, and the commit its upstream is at.
    branch: Option<(String, String, Option<Oid>)>,
    /// Held so the socket lives as long as the command.
    _askpass: Option<gittree_core::askpass::Server>,
    /// Keeps the question channel open when there is no listener.
    _ask: Option<smol::channel::Sender<CredentialRequest>>,
}

/// The conflicted file open in the conflict view.
pub struct ConflictFile {
    pub path: RepoPath,
    pub content: Arc<Vec<u8>>,
    pub regions: Vec<Region>,
    pub choices: Vec<Choice>,
}

impl ConflictFile {
    /// The file with every choice so far applied.
    pub fn result(&self) -> Vec<u8> {
        resolve(&self.content, &self.regions, &self.choices)
    }
}

/// What a finished op has to tell the UI.
enum Done {
    Changed,
    /// The action stopped on conflicts and waits to be continued.
    Conflict,
    Committed(Oid),
    Notice(String),
    /// The worktree of the waiting session exists.
    SessionReady,
    /// A pull request was opened at this address.
    PullRequest(String),
    WorktreeRemoved(std::path::PathBuf),
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

/// What shows between the sidebar and the details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Centre {
    #[default]
    Graph,
    Agents,
    Launchpad,
}

/// Pull requests and issues, as last read through `gh` (FR-AGENT-06).
#[derive(Default)]
pub struct LaunchpadState {
    pub loading: bool,
    pub error: Option<String>,
    pub data: Option<hosting::Launchpad>,
    /// The Issues tab shows, not Pull requests.
    pub issues_tab: bool,
}

/// `gh` for this repository. `GITTREE_GH` names another program, which the
/// tests use.
fn gh_for(runner: &GitRunner) -> Gh {
    let program = std::env::var_os("GITTREE_GH").map(Into::into);
    Gh::new(program, runner.dir(), runner.trusted())
}

/// What the AI helpers are doing and what they last wrote (FR-AGENT-05).
#[derive(Default)]
pub struct Ai {
    /// A commit message is being written.
    pub writing: bool,
    /// The agent that wrote the message now in the commit box.
    pub written_by: Option<String>,
    /// The commit being explained.
    pub explaining: Option<Oid>,
    /// A commit and its explanation.
    pub explained: Option<(Oid, String)>,
    /// A conflict result is being worked out.
    pub suggesting: bool,
    /// A conflicted file and the result an agent suggests for it. The
    /// conflict view takes it into its result box.
    pub suggestion: Option<(RepoPath, String)>,
    /// The agent whose suggestion is in the result box, and for which file.
    pub suggested_by: Option<(RepoPath, String)>,
    /// Commits are being planned from the uncommitted work.
    pub planning: bool,
    /// The plan an agent proposed. Nothing is committed until it is applied.
    pub plan: Option<Vec<PlannedCommit>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Blame,
    History,
}

/// One file over time, shown in place of the commit details: its blame
/// or its history, as the file is in the commit `rev` (FR-AGENT-03).
pub struct FileView {
    pub rev: Oid,
    pub path: RepoPath,
    pub mode: FileMode,
    /// `None` until read. Each is read the first time its mode shows.
    pub blame: Option<Arc<Vec<BlameLine>>>,
    pub history: Option<Arc<Vec<FileCommit>>>,
    /// The commit of the clicked blame line or history row.
    pub picked: Option<Oid>,
    /// What the picked history row did to the file.
    pub diff: Option<Arc<FileDiff>>,
    pub diff_loading: bool,
}

/// How many commits a file's history lists.
// ponytail: a fixed cap with no "load more". Raise it or page it if a
// file with more history than this turns up.
const FILE_HISTORY_LIMIT: usize = 500;

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
    pub ai: Ai,
    ai_task: Task<()>,
    explain_task: Task<()>,
    /// Agent sessions, oldest first. Saved in the repository's own folder.
    pub sessions: Vec<Session>,
    /// The calls agents made through the MCP server, oldest first, and
    /// the changes waiting for a yes or no (FR-AGENT-02).
    /// Diffs leave out changes that are only whitespace. For this run of
    /// the app only.
    pub ignore_whitespace: bool,
    pub agent_calls: Vec<mcp::Call>,
    pub agent_requests: Vec<mcp::Request>,
    agent_task: Task<()>,
    pub centre: Centre,
    pub launchpad: LaunchpadState,
    launchpad_task: Task<()>,
    /// The command and session that start once their worktree exists.
    pending_session: Option<(Vec<String>, Session)>,
    /// Each running agent, by its branch, with the flag that stops it.
    agents: Vec<(String, Arc<AtomicBool>, Task<()>)>,
    pub file_view: Option<FileView>,
    file_task: Task<()>,
    pub search: Search,
    pub status: Vec<StatusEntry>,
    pub stashes: Vec<Stash>,
    pub remotes: Vec<Remote>,
    pub undo_info: UndoInfo,
    /// Journal entries that saved something, newest first.
    pub snapshots: Vec<gittree_core::journal::JournalEntry>,
    /// What a running network command last reported, and how to stop it.
    pub progress: Option<Progress>,
    pub net_running: bool,
    net_cancel: Arc<AtomicBool>,
    /// A merge, rebase, cherry-pick or revert stopped part way.
    pub in_progress: Option<InProgressOp>,
    /// The conflict view shows in place of the staging view.
    pub resolving: bool,
    pub conflict: Option<ConflictFile>,
    /// Files resolved since the conflict view opened, for its list.
    pub resolved: Vec<RepoPath>,
    conflict_loading: Option<RepoPath>,
    /// An op is running. Views grey out their buttons meanwhile.
    pub busy: bool,
    ops: VecDeque<Op>,
    journal: Journal,
    /// Set by a commit: once nothing is left to commit, the selection moves
    /// from the work-in-progress row to this commit.
    after_commit: Option<Oid>,
    op_task: Task<()>,
    net_task: Task<()>,
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
    /// Checks once a minute whether a background fetch is due.
    _auto_fetch: Task<()>,
    last_fetch: Option<std::time::Instant>,
    /// The running fetch was started by the timer, so a failure is only
    /// logged: being offline should not bring a toast every few minutes.
    quiet_fetch: bool,
}

impl EventEmitter<RepoEvent> for RepoState {}

impl RepoState {
    pub fn new(repository: Repository, runner: GitRunner, cx: &mut Context<Self>) -> Self {
        let journal = Journal::new(&repository.common_dir);
        let mut this = Self {
            _watch: watch(&repository.path, cx),
            _auto_fetch: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(60))
                        .await;
                    if this.update(cx, |this, cx| this.auto_fetch(cx)).is_err() {
                        return;
                    }
                }
            }),
            // The first background fetch waits a full interval.
            last_fetch: Some(std::time::Instant::now()),
            quiet_fetch: false,
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
            remotes: Vec::new(),
            undo_info: UndoInfo::default(),
            snapshots: Vec::new(),
            progress: None,
            net_running: false,
            net_cancel: Default::default(),
            in_progress: None,
            resolving: false,
            conflict: None,
            resolved: Vec::new(),
            conflict_loading: None,
            busy: false,
            ops: VecDeque::new(),
            journal,
            after_commit: None,
            op_task: Task::ready(()),
            net_task: Task::ready(()),
            status_task: Task::ready(()),
            message_task: Task::ready(()),
            log_generation: 0,
            log_cancel: Default::default(),
            log_task: Task::ready(()),
            refs_task: Task::ready(()),
            details_task: Task::ready(()),
            diff_task: Task::ready(()),
            search_task: Task::ready(()),
            file_view: None,
            file_task: Task::ready(()),
            ai: Ai::default(),
            ai_task: Task::ready(()),
            explain_task: Task::ready(()),
            sessions: Vec::new(),
            ignore_whitespace: false,
            agent_calls: Vec::new(),
            agent_requests: Vec::new(),
            agent_task: Task::ready(()),
            centre: Centre::Graph,
            launchpad: LaunchpadState::default(),
            launchpad_task: Task::ready(()),
            pending_session: None,
            agents: Vec::new(),
        };
        this.sessions = agent::load(&this.repository.common_dir);
        this.refresh(Scope::All, cx);
        // Old journal entries go once per open, quietly.
        let (journal, runner) = (this.journal.clone(), this.runner.clone());
        cx.background_spawn(async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs() as i64);
            if let Err(error) = journal.prune(&runner, now) {
                tracing::warn!(%error, "journal pruning failed");
            }
        })
        .detach();
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
        self.load_agent_calls(cx);
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
        // A file that is no longer in conflict leaves the conflict view.
        let conflicted = |path: &RepoPath| {
            self.status
                .iter()
                .any(|entry| entry.conflict.is_some() && &entry.path == path)
        };
        if self
            .conflict
            .as_ref()
            .is_some_and(|file| !conflicted(&file.path))
        {
            self.conflict = None;
        }
        self.resolved.retain(|path| !conflicted(path));
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
        self.file_view = None;
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
        let ignore = self.ignore_whitespace;
        let runner = self.runner.clone();
        let read = cx.background_spawn(async move {
            read::diff_worktree(&runner, &entry, staged, load_all, ignore)
        });
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
        let git_dir = self.repository.git_dir.clone();
        let net = self.net_for(&op, cx);
        if let Op::ResolveFile { path, .. } | Op::ResolveSide { path, .. } = &op {
            self.resolved.push(path.clone());
            self.conflict = None;
        }
        let run = cx.background_spawn(async move {
            execute(op, &runner, &journal, &git_dir, has_commits, label, &net)
        });
        self.op_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                this.net_running = false;
                this.progress = None;
                let quiet = std::mem::take(&mut this.quiet_fetch);
                match result {
                    Ok(Done::Changed) => {}
                    Ok(Done::Conflict) => {
                        // Straight to the files that need a decision.
                        this.select_wip(cx);
                        this.resolving = true;
                        cx.emit(RepoEvent::Notice(t!("merge.stopped").into_owned()));
                    }
                    Ok(Done::Committed(oid)) => {
                        this.ai.written_by = None;
                        this.after_commit = Some(oid);
                        cx.emit(RepoEvent::Committed);
                    }
                    Ok(Done::Notice(text)) => cx.emit(RepoEvent::Notice(text)),
                    Ok(Done::SessionReady) => this.session_ready(cx),
                    Ok(Done::PullRequest(url)) => {
                        cx.emit(RepoEvent::Notice(
                            t!("launchpad.created", url = &url).into_owned(),
                        ));
                        this.load_launchpad(cx);
                    }
                    Ok(Done::WorktreeRemoved(path)) => {
                        this.sessions.retain(|session| session.path != path);
                        this.save_sessions();
                    }
                    Err(error) if quiet => {
                        tracing::warn!(%error, "background fetch failed");
                    }
                    Err(error) => {
                        this.pending_session = None;
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

    /// The current branch with the remote it pushes to and the commit its
    /// upstream was at when last fetched.
    fn push_target(&self) -> Option<(String, String, Option<Oid>)> {
        let current = self.refs.iter().find(|r| r.is_current)?;
        let upstream = current.upstream.as_deref();
        // "refs/remotes/origin/main" names the remote.
        let remote = upstream
            .and_then(|name| name.strip_prefix("refs/remotes/"))
            .and_then(|rest| rest.split('/').next())
            .map(str::to_owned)
            .or_else(|| {
                let names = self.remotes.iter().map(|remote| remote.name.clone());
                let mut names: Vec<String> = names.collect();
                let origin = names.iter().position(|name| name == "origin");
                origin
                    .map(|ix| names.swap_remove(ix))
                    .or_else(|| names.pop())
            })?;
        let seen = upstream.and_then(|name| {
            self.refs
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.target.clone())
        });
        Some((current.short_name().to_owned(), remote, seen))
    }

    /// Sets up what a network op needs: the askpass listener, a way to
    /// cancel, and a channel for progress. Other ops get an empty one.
    fn net_for(&mut self, op: &Op, cx: &mut Context<Self>) -> Net {
        let (progress, updates) = smol::channel::unbounded::<Progress>();
        let mut net = Net {
            env: Vec::new(),
            cancel: Default::default(),
            progress,
            pull_mode: PullMode::FfOnly,
            branch: self.push_target(),
            _askpass: None,
            _ask: None,
        };
        if let Op::Pull(mode) = op {
            net.pull_mode = *mode;
        }
        if !op.uses_network() {
            return net;
        }
        self.net_cancel = net.cancel.clone();
        self.net_running = true;

        // Questions come from the listener's thread. They cross to the UI
        // through a channel, and the answer goes back through another.
        let (ask, questions) = smol::channel::unbounded::<CredentialRequest>();
        if PROMPTS.load(Ordering::Relaxed) {
            let cancel = net.cancel.clone();
            let askpass = gittree_core::askpass::Server::start(move |question| {
                let (reply, answer) = std::sync::mpsc::channel();
                let request = CredentialRequest {
                    question: question.to_owned(),
                    secret: gittree_core::askpass::is_secret(question),
                    reply,
                };
                ask.send_blocking(request).ok()?;
                let answer = answer.recv().ok().flatten();
                if answer.is_none() {
                    // Cancelling the prompt cancels the command.
                    cancel.store(true, Ordering::Relaxed);
                }
                answer
            });
            match (askpass, std::env::current_exe()) {
                (Ok(server), Ok(exe)) => {
                    net.env = server.env(&exe);
                    net._askpass = Some(server);
                }
                _ => tracing::warn!("credential prompts are unavailable"),
            }
        } else {
            net._ask = Some(ask);
        }
        enum Message {
            Progress(Progress),
            Question(CredentialRequest),
        }
        self.net_task = cx.spawn(async move |this, cx| {
            loop {
                let message = smol::future::or(
                    async { updates.recv().await.ok().map(Message::Progress) },
                    async { questions.recv().await.ok().map(Message::Question) },
                )
                .await;
                // Both channels close when the command ends.
                let Some(message) = message else { return };
                let live = this.update(cx, |this, cx| match message {
                    Message::Progress(progress) => {
                        this.progress = Some(progress);
                        cx.notify();
                    }
                    Message::Question(request) => cx.emit(RepoEvent::Credential(request)),
                });
                if live.is_err() {
                    return;
                }
            }
        });
        net
    }

    /// Stops the network command that is running.
    pub fn cancel_network(&mut self) {
        self.net_cancel.store(true, Ordering::Relaxed);
    }

    /// Opens a conflicted file in the conflict view. The file is read off
    /// the UI thread.
    pub fn open_conflict(&mut self, path: RepoPath, cx: &mut Context<Self>) {
        self.resolving = true;
        // Already being read: asking again would only start over.
        if self.conflict_loading.as_ref() == Some(&path) {
            return;
        }
        self.conflict_loading = Some(path.clone());
        let full = self.repository.path.join(path.as_os_str());
        let read = cx.background_spawn(async move { std::fs::read(full).unwrap_or_default() });
        self.message_task = cx.spawn(async move |this, cx| {
            let content = read.await;
            let _ = this.update(cx, |this, cx| {
                let regions = parse_conflicts(&content);
                this.conflict_loading = None;
                this.conflict = Some(ConflictFile {
                    path,
                    choices: vec![Choice::Unresolved; regions.len()],
                    regions,
                    content: Arc::new(content),
                });
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Picks what to keep of one region of the open conflict.
    pub fn choose(&mut self, region: usize, choice: Choice, cx: &mut Context<Self>) {
        if let Some(slot) = self
            .conflict
            .as_mut()
            .and_then(|file| file.choices.get_mut(region))
        {
            *slot = choice;
            cx.notify();
        }
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
        let git_dir = self.repository.git_dir.clone();
        let journal = self.journal.clone();
        let read = cx.background_spawn(async move {
            Ok::<_, GitError>((
                read::list_refs(&runner)?,
                read::head(&runner)?,
                (
                    branch::list_stashes(&runner).unwrap_or_default(),
                    merge::in_progress(&runner, &git_dir),
                    remote::list_remotes(&runner).unwrap_or_default(),
                    undo_info(&journal, &runner),
                    journal.snapshots(),
                ),
            ))
        });
        // Replacing the task drops an older read that is still running.
        self.refs_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((refs, head, (stashes, in_progress, remotes, undo, snapshots))) => {
                    if stashes != this.stashes
                        || in_progress != this.in_progress
                        || remotes != this.remotes
                        || undo != this.undo_info
                        || snapshots != this.snapshots
                    {
                        this.undo_info = undo;
                        this.snapshots = snapshots;
                        this.stashes = stashes;
                        this.remotes = remotes;
                        if in_progress.is_none() {
                            this.resolving = false;
                            this.conflict = None;
                            this.resolved.clear();
                        }
                        this.in_progress = in_progress;
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
        self.file_view = None;
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
        let ignore = self.ignore_whitespace;
        let runner = self.runner.clone();
        let read = cx.background_spawn({
            let oid = oid.clone();
            async move { read::diff_commit_file(&runner, &oid, &file, load_all, ignore) }
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

    /// Has the chosen agent write a commit message for the staged changes.
    /// It arrives as `RepoEvent::MessageWritten`.
    pub fn write_message(&mut self, kind: AgentKind, custom: &str, cx: &mut Context<Self>) {
        let argv = match agent::ask_command(kind, custom) {
            Ok(argv) => argv,
            Err(problem) => return cx.emit(RepoEvent::Notice(problem)),
        };
        let by = match kind {
            AgentKind::Other => argv[0].clone(),
            known => known.label().to_owned(),
        };
        self.ai.writing = true;
        cx.notify();
        let runner = self.runner.clone();
        let asked = cx.background_spawn(async move {
            let patch = read::staged_patch(&runner).map_err(|error| error.stderr)?;
            if patch.is_empty() {
                return Err(t!("ai.nothing_staged").into_owned());
            }
            agent::ask(
                &argv,
                &agent::message_prompt(&patch),
                &AtomicBool::new(false),
            )
        });
        self.ai_task = cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.ai.writing = false;
                match answer {
                    Ok(message) => {
                        this.ai.written_by = Some(by);
                        cx.emit(RepoEvent::MessageWritten(message));
                    }
                    Err(problem) => cx.emit(RepoEvent::Notice(problem)),
                }
                cx.notify();
            });
        });
    }

    /// Has the chosen agent sort the uncommitted work into commits. The
    /// plan lands in `ai.plan` with `RepoEvent::Planned`. Nothing changes
    /// in the repository until `apply_plan`.
    pub fn plan_commits(&mut self, kind: AgentKind, custom: &str, cx: &mut Context<Self>) {
        let argv = match agent::ask_command(kind, custom) {
            Ok(argv) => argv,
            Err(problem) => return cx.emit(RepoEvent::Notice(problem)),
        };
        let files: Vec<RepoPath> = self.status.iter().map(|e| e.path.clone()).collect();
        self.ai.planning = true;
        cx.notify();
        let (runner, has_commits) = (self.runner.clone(), self.has_commits());
        let asked = cx.background_spawn(async move {
            let patch = read::uncommitted_patch(&runner, has_commits).map_err(|e| e.stderr)?;
            let answer = agent::ask(
                &argv,
                &agent::plan_prompt(&files, &patch),
                &AtomicBool::new(false),
            )?;
            agent::parse_plan(&answer, &files)
        });
        self.ai_task = cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.ai.planning = false;
                match answer {
                    Ok(plan) => {
                        this.ai.plan = Some(plan);
                        cx.emit(RepoEvent::Planned);
                    }
                    Err(problem) => cx.emit(RepoEvent::Notice(problem)),
                }
                cx.notify();
            });
        });
    }

    /// Makes the planned commits, in order. What is staged now is unstaged
    /// first, so each commit holds its own files and no others.
    pub fn apply_plan(&mut self, mut plan: Vec<PlannedCommit>, cx: &mut Context<Self>) {
        self.ai.plan = None;
        // A commit here would end a merge or rebase with part of the work.
        if self.in_progress.is_some() {
            return;
        }
        // A renamed file takes its old name along, or the commit would
        // add the new file and leave the old one behind.
        for commit in &mut plan {
            let old_names = self.status.iter().filter_map(|entry| {
                let renamed = commit.files.contains(&entry.path);
                entry.orig_path.clone().filter(|_| renamed)
            });
            let old_names: Vec<RepoPath> = old_names.collect();
            commit.files.extend(old_names);
        }
        let unstage = entry_paths(self.changes(true));
        self.run_op(Op::CommitPlan { unstage, plan }, cx);
    }

    /// Has the chosen agent suggest a result for the open conflicted file.
    /// Nothing is staged: the suggestion only fills the result box.
    pub fn suggest_conflict(&mut self, kind: AgentKind, custom: &str, cx: &mut Context<Self>) {
        let Some(file) = &self.conflict else {
            return;
        };
        let argv = match agent::ask_command(kind, custom) {
            Ok(argv) => argv,
            Err(problem) => return cx.emit(RepoEvent::Notice(problem)),
        };
        let by = match kind {
            AgentKind::Other => argv[0].clone(),
            known => known.label().to_owned(),
        };
        let (path, content) = (file.path.clone(), file.content.clone());
        if content.len() > agent::ASK_LIMIT {
            return cx.emit(RepoEvent::Notice(t!("ai.too_long").into_owned()));
        }
        self.ai.suggesting = true;
        cx.notify();
        let asked = cx.background_spawn(async move {
            let answer = agent::ask(
                &argv,
                &agent::conflict_prompt(&content),
                &AtomicBool::new(false),
            )?;
            // A text file ends with a line ending, which the answer lost.
            Ok::<_, String>(format!("{answer}\n"))
        });
        self.ai_task = cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.ai.suggesting = false;
                match answer {
                    Ok(text) => {
                        this.ai.suggested_by = Some((path.clone(), by));
                        this.ai.suggestion = Some((path, text));
                    }
                    Err(problem) => cx.emit(RepoEvent::Notice(problem)),
                }
                cx.notify();
            });
        });
    }

    /// Has the chosen agent explain the selected commit.
    pub fn explain_commit(&mut self, kind: AgentKind, custom: &str, cx: &mut Context<Self>) {
        let Some(oid) = self.selection.oid.clone() else {
            return;
        };
        let argv = match agent::ask_command(kind, custom) {
            Ok(argv) => argv,
            Err(problem) => return cx.emit(RepoEvent::Notice(problem)),
        };
        self.ai.explaining = Some(oid.clone());
        cx.notify();
        let runner = self.runner.clone();
        let asked = cx.background_spawn({
            let oid = oid.clone();
            async move {
                let patch = read::commit_patch(&runner, &oid).map_err(|error| error.stderr)?;
                agent::ask(
                    &argv,
                    &agent::explain_prompt(&patch),
                    &AtomicBool::new(false),
                )
            }
        });
        self.explain_task = cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if this.ai.explaining.as_ref() != Some(&oid) {
                    return;
                }
                this.ai.explaining = None;
                match answer {
                    Ok(text) => this.ai.explained = Some((oid, text)),
                    Err(problem) => cx.emit(RepoEvent::Notice(problem)),
                }
                cx.notify();
            });
        });
    }

    /// Shows a view in the centre, or the graph again when it already
    /// shows. The Launchpad is read the first time it opens.
    pub fn show_centre(&mut self, centre: Centre, cx: &mut Context<Self>) {
        self.centre = if self.centre == centre {
            Centre::Graph
        } else {
            centre
        };
        cx.notify();
        if self.centre == Centre::Launchpad && self.launchpad.data.is_none() {
            self.load_launchpad(cx);
        }
    }

    /// Reads the pull requests and issues again.
    pub fn load_launchpad(&mut self, cx: &mut Context<Self>) {
        self.launchpad.loading = true;
        self.launchpad.error = None;
        cx.notify();
        let gh = gh_for(&self.runner);
        let read = cx.background_spawn(async move { gh.launchpad() });
        self.launchpad_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.launchpad.loading = false;
                match result {
                    Ok(data) => this.launchpad.data = Some(data),
                    Err(problem) => this.launchpad.error = Some(problem),
                }
                cx.notify();
            });
        });
    }

    /// The current branch and the branch a pull request from it goes into.
    pub fn pull_request_branches(&self) -> Option<(String, String)> {
        let Head::Branch(head) = &self.repository.head else {
            return None;
        };
        let base = self
            .launchpad
            .data
            .as_ref()
            .map(|data| data.default_branch.clone())
            .filter(|base| !base.is_empty())
            .unwrap_or_else(|| "main".into());
        (*head != base).then(|| (head.clone(), base))
    }

    /// Pushes the current branch, then opens a pull request from it.
    pub fn create_pull_request(
        &mut self,
        title: String,
        body: String,
        draft: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((head, base)) = self.pull_request_branches() else {
            return;
        };
        // The remote needs the branch and its newest commits first.
        self.run_op(Op::Push { force: false }, cx);
        self.run_op(
            Op::CreatePullRequest {
                head,
                base,
                title,
                body,
                draft,
            },
            cx,
        );
    }

    /// Reads what the MCP server left for the app: the calls agents made
    /// and the changes waiting for an answer. The server is another
    /// process, so the two meet in files in the repository's Git folder.
    // ponytail: read with every status refresh, which the file watcher
    // starts. A worktree's shared Git folder is outside the watched
    // folder, so there it shows on the next refresh.
    pub fn load_agent_calls(&mut self, cx: &mut Context<Self>) {
        let common_dir = self.repository.common_dir.clone();
        let read = cx
            .background_spawn(async move { (mcp::calls(&common_dir), mcp::requests(&common_dir)) });
        self.agent_task = cx.spawn(async move |this, cx| {
            let (calls, requests) = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.agent_calls != calls || this.agent_requests != requests {
                    this.agent_calls = calls;
                    this.agent_requests = requests;
                    cx.notify();
                }
            });
        });
    }

    /// Allows or denies a change an agent is waiting to make.
    pub fn answer_agent(&mut self, id: &str, allow: bool, cx: &mut Context<Self>) {
        self.agent_requests.retain(|request| request.id != id);
        cx.notify();
        let (common_dir, id) = (self.repository.common_dir.clone(), id.to_owned());
        cx.background_spawn(async move {
            if let Err(error) = mcp::answer(&common_dir, &id, allow) {
                tracing::warn!(%error, "the agent was not waiting for the answer any more");
            }
        })
        .detach();
    }

    fn save_sessions(&self) {
        if let Err(error) = agent::save(&self.repository.common_dir, &self.sessions) {
            tracing::error!(%error, "could not save the agent sessions");
        }
    }

    /// Starts an agent on `task` in a new worktree at `path`, on a new
    /// branch off the current commit. The agent runs once the worktree is
    /// there.
    pub fn start_session(
        &mut self,
        kind: AgentKind,
        custom: &str,
        task: &str,
        branch: String,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let argv = match agent::command(kind, custom, task) {
            Ok(argv) => argv,
            Err(problem) => return cx.emit(RepoEvent::Notice(problem)),
        };
        let base = match &self.repository.head {
            Head::Branch(name) | Head::Unborn(name) => name.clone(),
            Head::Detached(oid) => oid.to_string(),
        };
        let agent = match kind {
            AgentKind::Other => argv[0].clone(),
            known => known.label().to_owned(),
        };
        let session = Session {
            agent,
            task: task.trim().to_owned(),
            branch: branch.clone(),
            base,
            path: path.clone(),
            state: SessionState::Working,
            note: String::new(),
        };
        self.pending_session = Some((argv, session));
        self.centre = Centre::Agents;
        self.run_op(Op::StartSession { path, branch }, cx);
    }

    /// The worktree exists: list the session and run its agent.
    fn session_ready(&mut self, cx: &mut Context<Self>) {
        let Some((argv, session)) = self.pending_session.take() else {
            return;
        };
        let (branch, path) = (session.branch.clone(), session.path.clone());
        self.sessions.push(session);
        self.save_sessions();
        // The app made this folder, so it is trusted when opened.
        cx.emit(RepoEvent::WorktreeCreated(path.clone()));
        let cancel = Arc::new(AtomicBool::new(false));
        let run = cx.background_spawn({
            let cancel = cancel.clone();
            async move {
                let mut last = String::new();
                let ended = agent::run(&argv, &path, &cancel, |line| {
                    if !line.trim().is_empty() {
                        last = line.trim().to_owned();
                    }
                });
                (ended, last, cancel.load(Ordering::Relaxed))
            }
        });
        let name = branch.clone();
        let task = cx.spawn(async move |this, cx| {
            let (ended, last, stopped) = run.await;
            let _ = this.update(cx, |this, cx| {
                this.agents.retain(|(branch, ..)| *branch != name);
                let Some(session) = this.sessions.iter_mut().find(|s| s.branch == name) else {
                    return;
                };
                (session.state, session.note) = match ended {
                    _ if stopped => (SessionState::Stopped, last),
                    Ok(true) => (SessionState::Done, last),
                    Ok(false) => (SessionState::Failed, last),
                    Err(problem) => (SessionState::Failed, problem),
                };
                let notice = match session.state {
                    SessionState::Done => t!("agents.done", branch = &name),
                    SessionState::Stopped => t!("agents.stopped", branch = &name),
                    _ => t!("agents.failed", branch = &name),
                };
                this.save_sessions();
                cx.emit(RepoEvent::Notice(notice.into_owned()));
                cx.notify();
            });
        });
        self.agents.push((branch, cancel, task));
        cx.notify();
    }

    /// Stops a running agent. Its worktree and what it wrote stay.
    pub fn stop_session(&mut self, branch: &str) {
        if let Some((_, cancel, _)) = self.agents.iter().find(|(name, ..)| name == branch) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Removes a session's worktree and drops it from the list. Git
    /// refuses while the worktree has changes, so nothing is lost. A
    /// session whose folder is already gone is only dropped.
    pub fn remove_session(&mut self, branch: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.branch == branch) else {
            return;
        };
        if session.state == SessionState::Working {
            return cx.emit(RepoEvent::Notice(t!("agents.stop_first").into_owned()));
        }
        if session.path.exists() {
            self.run_op(Op::RemoveWorktree(session.path.clone()), cx);
        } else {
            self.sessions.retain(|s| s.branch != branch);
            self.save_sessions();
            cx.notify();
        }
    }

    /// Shows the blame or the history of the selected commit's selected
    /// file, as that commit has it.
    pub fn show_file(&mut self, mode: FileMode, cx: &mut Context<Self>) {
        let file = self
            .selection
            .file
            .and_then(|ix| self.selection.files.get(ix));
        let (Some(rev), Some(file)) = (self.selection.oid.clone(), file) else {
            return;
        };
        match &mut self.file_view {
            Some(view) => view.mode = mode,
            None => {
                self.file_view = Some(FileView {
                    rev,
                    path: file.path.clone(),
                    mode,
                    blame: None,
                    history: None,
                    picked: None,
                    diff: None,
                    diff_loading: false,
                })
            }
        }
        cx.notify();
        let view = self.file_view.as_ref().expect("set above");
        let loaded = match mode {
            FileMode::Blame => view.blame.is_some(),
            FileMode::History => view.history.is_some(),
        };
        if loaded {
            return;
        }
        let (runner, rev, path) = (self.runner.clone(), view.rev.clone(), view.path.clone());
        let read = cx.background_spawn({
            let (rev, path) = (rev.clone(), path.clone());
            async move {
                Ok::<_, GitError>(match mode {
                    FileMode::Blame => (Some(file::blame(&runner, &rev, &path)?), None),
                    FileMode::History => (
                        None,
                        Some(file::history(&runner, &rev, &path, FILE_HISTORY_LIMIT)?),
                    ),
                })
            }
        });
        self.file_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                let Some(view) = this.file_view.as_mut() else {
                    return;
                };
                if view.rev != rev || view.path != path {
                    return;
                }
                // A failed read shows as an empty list, with the error.
                let (blame, history) = result.unwrap_or_else(|error| {
                    cx.emit(RepoEvent::Error(error));
                    match mode {
                        FileMode::Blame => (Some(Vec::new()), None),
                        FileMode::History => (None, Some(Vec::new())),
                    }
                });
                if let Some(blame) = blame {
                    view.blame = Some(Arc::new(blame));
                }
                if let Some(history) = history {
                    view.history = Some(Arc::new(history));
                }
                cx.notify();
            });
        });
    }

    /// Back to the commit details and the diff.
    pub fn close_file_view(&mut self, cx: &mut Context<Self>) {
        self.file_view = None;
        self.file_task = Task::ready(());
        cx.notify();
    }

    /// Marks the commit of a clicked blame line.
    pub fn pick_blame_commit(&mut self, oid: Oid, cx: &mut Context<Self>) {
        if let Some(view) = self.file_view.as_mut() {
            view.picked = Some(oid);
            cx.notify();
        }
    }

    /// Fetches when the interval set in Settings has passed since the last
    /// background fetch, and nothing else is running (FR-SYNC-05).
    // ponytail: a remote that asks for a password asks each time. Leave
    // auto-fetch off for those, or use a credential helper.
    fn auto_fetch(&mut self, cx: &mut Context<Self>) {
        let minutes = crate::app_state::app_state(cx)
            .read(cx)
            .settings
            .auto_fetch_minutes;
        let idle = self.runner.trusted() && !self.busy && !self.remotes.is_empty();
        if idle && fetch_due(minutes, self.last_fetch.map(|at| at.elapsed())) {
            self.last_fetch = Some(std::time::Instant::now());
            self.quiet_fetch = true;
            self.run_op(Op::Fetch, cx);
        }
    }

    /// Turns ignoring whitespace on or off, and reads the shown diff again.
    pub fn toggle_whitespace(&mut self, cx: &mut Context<Self>) {
        self.ignore_whitespace = !self.ignore_whitespace;
        if let Some(view) = &self.file_view
            && let Some(row) = view.history.as_ref().and_then(|list| {
                list.iter()
                    .position(|entry| Some(&entry.commit.oid) == view.picked.as_ref())
            })
        {
            self.pick_file_commit(row, cx);
        }
        if self.selection.wip {
            if let Some((path, staged)) = self.selection.wip_file.clone() {
                self.select_wip_file(path, staged, false, cx);
            }
        } else if let Some(ix) = self.selection.file {
            self.select_file(ix, false, cx);
        }
        cx.notify();
    }

    /// Marks a history row and reads what that commit did to the file.
    pub fn pick_file_commit(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(view) = self.file_view.as_mut() else {
            return;
        };
        let Some(entry) = view
            .history
            .as_ref()
            .and_then(|list| list.get(row))
            .cloned()
        else {
            return;
        };
        let oid = entry.commit.oid.clone();
        view.picked = Some(oid.clone());
        view.diff = None;
        view.diff_loading = true;
        cx.notify();
        let ignore = self.ignore_whitespace;
        let runner = self.runner.clone();
        let read = cx.background_spawn({
            let oid = oid.clone();
            async move { read::diff_commit_file(&runner, &oid, &entry.file, false, ignore) }
        });
        self.diff_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                let Some(view) = this.file_view.as_mut() else {
                    return;
                };
                if view.picked.as_ref() != Some(&oid) {
                    return;
                }
                view.diff_loading = false;
                match result {
                    Ok(diff) => view.diff = Some(Arc::new(diff)),
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
        Op::Merge { branch, .. } => t!("journal.merge", branch = branch).into_owned(),
        Op::Rebase(onto) => t!("journal.rebase", onto = onto).into_owned(),
        Op::InteractiveRebase { base, .. } => {
            t!("journal.interactive_rebase", base = &base[..7.min(base.len())]).into_owned()
        }
        Op::CherryPick { oid, .. } => {
            t!("journal.cherry_pick", oid = &oid[..7.min(oid.len())]).into_owned()
        }
        Op::Revert { oid, .. } => t!("journal.revert", oid = &oid[..7.min(oid.len())]).into_owned(),
        Op::Reset { target, .. } => {
            t!("journal.reset", target = &target[..7.min(target.len())]).into_owned()
        }
        Op::Stage(_)
        | Op::Unstage(_)
        // Each of its commits gets a label of its own.
        | Op::CommitPlan { .. }
        | Op::Undo
        | Op::StashApply(_)
        | Op::StashPop(_)
        | Op::ContinueOp
        | Op::AbortOp
        | Op::SkipOp
        | Op::ResolveFile { .. }
        | Op::ResolveSide { .. }
        | Op::Fetch
        | Op::Pull(_)
        | Op::Push { .. }
        | Op::PushTag { .. }
        | Op::AddRemote { .. }
        | Op::RenameRemote { .. }
        | Op::SetRemoteUrl { .. }
        | Op::RemoveRemote(_)
        | Op::StartSession { .. }
        | Op::RemoveWorktree(_)
        | Op::CreatePullRequest { .. }
        | Op::Redo
        | Op::RestoreSnapshot { .. } => String::new(),
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
    git_dir: &Path,
    has_commits: bool,
    label: String,
    net: &Net,
) -> Result<Done, GitError> {
    let report = |progress: Progress| {
        let _ = net.progress.send_blocking(progress);
    };
    // For an action that can stop on a conflict: the entry is parked until
    // the operation is continued to its end.
    let stoppable = |action: Action, run: &dyn Fn() -> Result<(), GitError>| {
        let pending = journal.begin(runner, action, label.clone())?;
        match run() {
            Ok(()) => journal.finish(runner, pending).map(|_| Done::Changed),
            Err(error) if error.kind == GitErrorKind::Conflict => {
                journal.park(pending)?;
                Ok(Done::Conflict)
            }
            Err(error) => {
                journal.abandon(runner, pending);
                Err(error)
            }
        }
    };
    let kind = || merge::in_progress(runner, git_dir).map(|op| op.kind);
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
        Op::CommitPlan { unstage, plan } => {
            stage::unstage(runner, &unstage, has_commits)?;
            let mut done = Done::Changed;
            // Stops at the first commit that fails. Those before it stay,
            // each with its own undo entry.
            for commit in plan {
                let summary = commit.message.lines().next().unwrap_or_default();
                let label = t!("journal.commit", summary = summary).into_owned();
                let action = Action::Commit {
                    message: commit.message.clone(),
                    amend: false,
                };
                let pending = journal.begin(runner, action, label)?;
                let made = stage::stage(runner, &commit.files)
                    .and_then(|()| stage::commit(runner, &commit.message, false));
                match made {
                    Ok(oid) => {
                        journal.finish(runner, pending)?;
                        done = Done::Committed(oid);
                    }
                    Err(error) => {
                        journal.abandon(runner, pending);
                        return Err(error);
                    }
                }
            }
            Ok(done)
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
        Op::Merge { branch, no_ff } => {
            let action = Action::Merge {
                branch: branch.clone(),
                no_ff,
            };
            stoppable(action, &|| merge::merge(runner, git_dir, &branch, no_ff))
        }
        // Journaled as a rebase onto `base`: undo puts the snapshot back,
        // and redo resets to the result, so the plan is not needed again.
        Op::InteractiveRebase { base, items } => {
            stoppable(Action::Rebase { onto: base.clone() }, &|| {
                merge::rebase_interactive(runner, git_dir, &base, &items)
            })
        }
        Op::Rebase(onto) => stoppable(Action::Rebase { onto: onto.clone() }, &|| {
            merge::rebase(runner, git_dir, &onto)
        }),
        Op::CherryPick { oid, mainline } => {
            let action = Action::CherryPick {
                oid: oid.clone(),
                mainline,
            };
            stoppable(action, &|| {
                merge::cherry_pick(runner, git_dir, &oid, mainline)
            })
        }
        Op::Revert { oid, mainline } => {
            let action = Action::Revert {
                oid: oid.clone(),
                mainline,
            };
            stoppable(action, &|| merge::revert(runner, git_dir, &oid, mainline))
        }
        Op::Reset { target, mode } => {
            let action = Action::Reset {
                target: target.clone(),
                mode,
            };
            journaled(action, &|| {
                merge::reset(runner, &target, mode).map(|()| Done::Changed)
            })
        }
        Op::ContinueOp | Op::SkipOp => {
            let Some(kind) = kind() else {
                return Ok(Done::Changed);
            };
            let step = if matches!(op, Op::SkipOp) && kind == OpKind::Rebase {
                merge::rebase_skip(runner, git_dir)
            } else {
                merge::continue_op(runner, git_dir, kind)
            };
            match step {
                Ok(()) => {
                    // The whole operation is through: its entry can go in.
                    if let Some(pending) = journal.take_parked() {
                        journal.finish(runner, pending)?;
                    }
                    Ok(Done::Changed)
                }
                // A rebase stopped again, on its next commit.
                Err(error) if error.kind == GitErrorKind::Conflict => Ok(Done::Conflict),
                Err(error) => Err(error),
            }
        }
        Op::AbortOp => {
            if let Some(kind) = kind() {
                merge::abort_op(runner, kind)?;
            }
            if let Some(pending) = journal.take_parked() {
                journal.abandon(runner, pending);
            }
            Ok(Done::Changed)
        }
        Op::ResolveFile { path, content } => {
            merge::resolve_file(runner, &path, &content).map(|()| Done::Changed)
        }
        Op::ResolveSide { path, side } => {
            merge::resolve_side(runner, &path, side).map(|()| Done::Changed)
        }
        Op::Fetch => remote::fetch(runner, &net.env, &net.cancel, report).map(|()| Done::Changed),
        Op::Pull(_) => {
            // A pull can stop on a conflict like a merge or a rebase.
            let pulled = remote::pull(runner, net.pull_mode, &net.env, &net.cancel, report);
            match pulled {
                Err(_) if merge::in_progress(runner, git_dir).is_some() => Ok(Done::Conflict),
                other => other.map(|()| Done::Changed),
            }
        }
        Op::Push { force } => {
            let Some((branch, remote_name, seen)) = &net.branch else {
                return Ok(Done::Notice(t!("sync.no_remote").into_owned()));
            };
            let lease = if force { seen.as_deref() } else { None };
            if force && lease.is_none() {
                return Ok(Done::Notice(t!("sync.nothing_to_force").into_owned()));
            }
            // The commit about to be replaced on the remote is kept.
            let pending = match (force, seen) {
                (true, Some(replaced)) => {
                    let action = Action::ForcePush {
                        remote: remote_name.clone(),
                        branch: branch.clone(),
                        replaced: replaced.clone(),
                    };
                    let label = t!("journal.force_push", branch = branch).into_owned();
                    Some(journal.begin(runner, action, label)?)
                }
                _ => None,
            };
            let pushed = remote::push(
                runner,
                remote_name,
                branch,
                // A branch with no upstream is published in the same step.
                seen.is_none(),
                lease,
                &net.env,
                &net.cancel,
                report,
            );
            match (&pushed, pending) {
                (Ok(()), Some(pending)) => drop(journal.finish(runner, pending)?),
                (Err(_), Some(pending)) => journal.abandon(runner, pending),
                _ => {}
            }
            pushed?;
            Ok(Done::Notice(
                t!("sync.pushed", branch = branch, remote = remote_name).into_owned(),
            ))
        }
        Op::PushTag { tag, delete } => {
            let Some((_, remote_name, _)) = &net.branch else {
                return Ok(Done::Notice(t!("sync.no_remote").into_owned()));
            };
            remote::push_tag(runner, remote_name, &tag, delete, &net.env, &net.cancel)
                .map(|()| Done::Changed)
        }
        Op::AddRemote { name, url } => {
            remote::add_remote(runner, &name, &url).map(|()| Done::Changed)
        }
        Op::RenameRemote { old, new } => {
            remote::rename_remote(runner, &old, &new).map(|()| Done::Changed)
        }
        Op::SetRemoteUrl { name, url } => {
            remote::set_remote_url(runner, &name, &url).map(|()| Done::Changed)
        }
        Op::RemoveRemote(name) => remote::remove_remote(runner, &name).map(|()| Done::Changed),
        Op::StartSession { path, branch } => {
            worktree::add(runner, &path, &branch, "HEAD").map(|()| Done::SessionReady)
        }
        Op::RemoveWorktree(path) => {
            worktree::remove(runner, &path).map(|()| Done::WorktreeRemoved(path))
        }
        Op::CreatePullRequest {
            head,
            base,
            title,
            body,
            draft,
        } => gh_for(runner)
            .create_pull_request(&head, &base, &title, &body, draft)
            .map(Done::PullRequest)
            .map_err(|problem| GitError {
                kind: GitErrorKind::Other,
                args: vec!["gh".into(), "pr".into(), "create".into()],
                exit_code: None,
                stderr: problem,
            }),
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
                Err(Blocked::NotUndoable) => Ok(Done::Notice(
                    t!("undo.remote", label = entry.label).into_owned(),
                )),
                Err(Blocked::RepositoryChanged | Blocked::AlreadyUndone) => Ok(Done::Notice(
                    t!("undo.blocked", label = entry.label).into_owned(),
                )),
            }
        }
        Op::Redo => {
            let Some(entry) = journal.next_redo() else {
                return Ok(Done::Notice(t!("undo.nothing_redo").into_owned()));
            };
            if !journal.can_redo(runner, &entry) {
                return Ok(Done::Notice(
                    t!("undo.redo_blocked", label = entry.label).into_owned(),
                ));
            }
            journal.redo(runner, git_dir, &entry)?;
            Ok(Done::Notice(
                t!("undo.redone", label = entry.label).into_owned(),
            ))
        }
        Op::RestoreSnapshot { id, branch } => {
            let Some(entry) = journal.snapshots().into_iter().find(|entry| entry.id == id) else {
                return Ok(Done::Changed);
            };
            journal.restore(runner, &entry, branch.as_deref())?;
            Ok(Done::Notice(
                t!("snapshots.restored", label = entry.label).into_owned(),
            ))
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

/// Whether a background fetch is due. 0 minutes is off.
fn fetch_due(minutes: u32, since_last: Option<Duration>) -> bool {
    let every = Duration::from_secs(u64::from(minutes) * 60);
    minutes > 0 && since_last.is_none_or(|since| since >= every)
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

/// Headless tests also switch the credential listener off, for the same
/// reason: it answers from its own thread.
pub static PROMPTS: AtomicBool = AtomicBool::new(true);

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

    #[test]
    fn a_background_fetch_is_due_once_the_interval_has_passed() {
        let minutes = |n: u64| Some(Duration::from_secs(n * 60));
        assert!(!fetch_due(0, minutes(1000)));
        assert!(!fetch_due(5, minutes(4)));
        assert!(fetch_due(5, minutes(5)));
        assert!(fetch_due(15, None));
    }

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
