use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gittree_core::model::{GitVersion, Repository};
use gittree_core::ops::setup;
use gittree_core::store::{Persisted, Recent, Session, Settings, Store, TrustList};
use gittree_core::{GitError, GitErrorKind, GitRunner};
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Task};

use crate::repo_state::{RepoEvent, RepoState};

/// The app's files as read at launch.
pub struct Loaded {
    pub store: Store,
    pub settings: Settings,
    pub session: Session,
    pub recent: Recent,
    pub trust: TrustList,
}

impl Loaded {
    /// Reads every file. Call it off the UI thread.
    pub fn read(mut store: Store) -> Self {
        Self {
            settings: store.load(),
            session: store.load(),
            recent: store.load(),
            trust: store.load(),
            store,
        }
    }
}

pub enum GitCheck {
    Checking,
    Ready(GitVersion),
    /// `GitNotFound` or `GitTooOld`.
    Failed(GitError),
}

pub enum AppEvent {
    /// A Git call failed and the user should hear about it.
    Error(GitError),
    /// The folder the user chose is not a repository.
    NotARepository(PathBuf),
    /// Something to tell the user, already in plain words.
    Notice(String),
    /// Git or ssh asks for a user name, password or passphrase.
    Credential(crate::repo_state::CredentialRequest),
}

/// A clone that is running, or that failed and waits for another try.
pub struct CloneJob {
    pub url: String,
    pub dest: PathBuf,
    pub progress: Option<gittree_core::ops::remote::Progress>,
    pub error: Option<GitError>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CloneJob {
    pub fn running(&self) -> bool {
        self.error.is_none()
    }
}

type SaveJob = Box<dyn FnOnce(&Store) + Send>;

/// Everything that outlives a repository: settings, session, the recent
/// list, the trust list and the Git that was found.
pub struct AppState {
    pub settings: Settings,
    pub session: Session,
    pub recent: Recent,
    pub trust: TrustList,
    pub git: GitCheck,
    pub repo: Option<Entity<RepoState>>,
    pub clone: Option<CloneJob>,
    /// Opened as soon as Git is ready: the command-line path, or the last
    /// session's repository.
    pending_open: Option<(PathBuf, Quiet)>,
    session_changed: bool,
    saves: smol::channel::Sender<SaveJob>,
    _tasks: Vec<Task<()>>,
    _on_quit: gpui_kit::Subscription,
}

/// Whether a failed open is reported. Restoring the last session is quiet:
/// a repository that has gone away is not an error worth a message.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quiet {
    Yes,
    No,
}

impl EventEmitter<AppEvent> for AppState {}

impl AppState {
    pub fn new(loaded: Loaded, cli_path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let Loaded {
            store,
            settings,
            session,
            recent,
            trust,
        } = loaded;

        // One task writes every file, in order, off the UI thread.
        let (saves, jobs) = smol::channel::unbounded::<SaveJob>();
        let writer = cx.background_spawn(async move {
            while let Ok(job) = jobs.recv().await {
                job(&store);
            }
        });
        let autosave = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                if this.update(cx, |this, _| this.save_session()).is_err() {
                    return;
                }
            }
        });

        let pending_open = cli_path
            .map(|path| (path, Quiet::No))
            .or_else(|| session.open_repo.clone().map(|path| (path, Quiet::Yes)));
        let on_quit = cx.on_app_quit(|this, _| this.flush());
        let mut this = Self {
            settings,
            session,
            recent,
            trust,
            git: GitCheck::Checking,
            repo: None,
            clone: None,
            pending_open,
            session_changed: false,
            saves,
            _tasks: vec![writer, autosave],
            _on_quit: on_quit,
        };
        this.check_git(cx);
        this
    }

    /// Looks for a usable Git. Runs at launch and when the Git path setting
    /// changes.
    pub fn check_git(&mut self, cx: &mut Context<Self>) {
        let custom = self.settings.git_path.clone();
        let found = cx.background_spawn(async move { setup::find_git(custom.as_deref()) });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |this, cx| {
                match found {
                    Ok(version) => {
                        tracing::info!(git = %version.text, path = %version.path.display(), "found Git");
                        this.git = GitCheck::Ready(version);
                        if let Some((path, quiet)) = this.pending_open.take() {
                            this.open_with(path, quiet, cx);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "no usable Git");
                        this.git = GitCheck::Failed(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_path(&self) -> Option<PathBuf> {
        match &self.git {
            GitCheck::Ready(version) => Some(version.path.clone()),
            _ => None,
        }
    }

    /// Opens the repository that contains `path`.
    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_with(path, Quiet::No, cx);
    }

    fn open_with(&mut self, path: PathBuf, quiet: Quiet, cx: &mut Context<Self>) {
        let Some(git) = self.git_path() else {
            return;
        };
        let trust = self.trust.clone();
        let opened = cx.background_spawn({
            let path = path.clone();
            async move { setup::open(&git, &path, &trust) }
        });
        cx.spawn(async move |this, cx| {
            let opened = opened.await;
            let _ = this.update(cx, |this, cx| match opened {
                Ok(repository) => this.show(repository, cx),
                Err(_) if quiet == Quiet::Yes => {
                    this.session.open_repo = None;
                    this.session_changed = true;
                }
                Err(error) if error.kind == GitErrorKind::NotARepository => {
                    cx.emit(AppEvent::NotARepository(path));
                }
                Err(error) => cx.emit(AppEvent::Error(error)),
            });
        })
        .detach();
    }

    /// Makes `path` a repository and opens it.
    pub fn init(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(git) = self.git_path() else {
            return;
        };
        let mut trust = self.trust.clone();
        let created = cx.background_spawn(async move {
            let repository = setup::init(&git, &path, &mut trust)?;
            Ok::<_, GitError>((repository, trust))
        });
        cx.spawn(async move |this, cx| {
            let created = created.await;
            let _ = this.update(cx, |this, cx| match created {
                Ok((repository, trust)) => {
                    this.trust = trust;
                    this.save(&this.trust);
                    this.show(repository, cx);
                }
                Err(error) => cx.emit(AppEvent::Error(error)),
            });
        })
        .detach();
    }

    /// Clones `url` into `dest`, then opens it as a trusted repository.
    /// Progress and a failure show in `self.clone`.
    pub fn clone_repo(&mut self, url: String, dest: PathBuf, cx: &mut Context<Self>) {
        use gittree_core::askpass;
        use gittree_core::ops::remote;
        use std::sync::atomic::Ordering;

        let Some(git) = self.git_path() else {
            return;
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.clone = Some(CloneJob {
            url: url.clone(),
            dest: dest.clone(),
            progress: None,
            error: None,
            cancel: cancel.clone(),
        });
        cx.notify();

        let (progress, updates) = smol::channel::unbounded();
        let (ask, questions) = smol::channel::unbounded();
        let prompts = crate::repo_state::PROMPTS.load(Ordering::Relaxed);
        // Without a listener the sender is kept, so the channel stays open.
        let (server, keep) = if prompts {
            let stop = cancel.clone();
            let server = askpass::Server::start(move |question| {
                let (reply, answer) = std::sync::mpsc::channel();
                let request = crate::repo_state::CredentialRequest {
                    question: question.to_owned(),
                    secret: askpass::is_secret(question),
                    reply,
                };
                ask.send_blocking(request).ok()?;
                let answer = answer.recv().ok().flatten();
                if answer.is_none() {
                    stop.store(true, Ordering::Relaxed);
                }
                answer
            });
            (server.ok(), None)
        } else {
            (None, Some(ask))
        };
        let cloned = cx.background_spawn({
            let dest = dest.clone();
            async move {
                let _keep = keep;
                let env = match (&server, std::env::current_exe()) {
                    (Some(server), Ok(exe)) => server.env(&exe),
                    _ => Vec::new(),
                };
                let parent = dest.parent().unwrap_or(&dest).to_owned();
                std::fs::create_dir_all(&parent).ok();
                // Cloning makes a new repository, so there is no trust to check yet.
                let runner = GitRunner::new(&git, &parent, true);
                remote::clone(&runner, &url, &dest, &env, &cancel, |update| {
                    let _ = progress.send_blocking(update);
                })
            }
        });
        cx.spawn(async move |this, cx| {
            enum Message {
                Progress(remote::Progress),
                Question(crate::repo_state::CredentialRequest),
            }
            let listen = async {
                loop {
                    let message = smol::future::or(
                        async { updates.recv().await.ok().map(Message::Progress) },
                        async { questions.recv().await.ok().map(Message::Question) },
                    )
                    .await;
                    let Some(message) = message else { return };
                    let _ = this.update(cx, |this, cx| match message {
                        Message::Progress(update) => {
                            if let Some(job) = &mut this.clone {
                                job.progress = Some(update);
                                cx.notify();
                            }
                        }
                        Message::Question(request) => cx.emit(AppEvent::Credential(request)),
                    });
                }
            };
            let (result, ()) = smol::future::zip(cloned, listen).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.clone = None;
                        // The app made this repository, so it is trusted.
                        let path = gittree_core::ops::setup::canonical(&dest);
                        this.trust.insert(&path);
                        this.save(&this.trust);
                        this.open(path, cx);
                    }
                    Err(error) if error.kind == GitErrorKind::Cancelled => this.clone = None,
                    Err(error) => {
                        if let Some(job) = &mut this.clone {
                            job.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops a running clone, or dismisses one that failed.
    pub fn cancel_clone(&mut self, cx: &mut Context<Self>) {
        match &self.clone {
            Some(job) if job.running() => {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed)
            }
            _ => self.clone = None,
        }
        cx.notify();
    }

    fn show(&mut self, repository: Repository, cx: &mut Context<Self>) {
        let Some(git) = self.git_path() else {
            return;
        };
        self.recent
            .touch(&repository.path, &repository.name(), now());
        self.save(&self.recent);
        self.session.open_repo = Some(repository.path.clone());
        self.save(&self.session);
        let runner = GitRunner::new(git, &repository.path, repository.trusted);
        let repo = cx.new(|cx| RepoState::new(repository, runner, cx));
        // A view shows these. AppState only passes them on.
        cx.subscribe(&repo, |_, _, event: &RepoEvent, cx| match event {
            RepoEvent::Error(error) => cx.emit(AppEvent::Error(error.clone())),
            RepoEvent::Notice(text) => cx.emit(AppEvent::Notice(text.clone())),
            RepoEvent::Credential(request) => cx.emit(AppEvent::Credential(request.clone())),
            _ => {}
        })
        .detach();
        self.repo = Some(repo);
        cx.notify();
    }

    pub fn close_repo(&mut self, cx: &mut Context<Self>) {
        self.repo = None;
        self.session.open_repo = None;
        self.save(&self.session);
        cx.notify();
    }

    /// Trusts the open repository, which lets it be changed and lets its
    /// hooks run. The choice is kept.
    pub fn trust_repo(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some(git)) = (self.repo.clone(), self.git_path()) else {
            return;
        };
        let path = repo.read(cx).repository.path.clone();
        self.trust.insert(&path);
        repo.update(cx, |repo, cx| {
            repo.set_trusted(GitRunner::new(git, &path, true), cx)
        });
        self.save(&self.trust);
        cx.notify();
    }

    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let git_path = self.settings.git_path.clone();
        change(&mut self.settings);
        self.save(&self.settings);
        if self.settings.git_path != git_path {
            self.check_git(cx);
        }
        cx.notify();
    }

    pub fn remove_recent(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.recent.remove(path);
        self.save(&self.recent);
        cx.notify();
    }

    /// Changes the session. It is written within 30 seconds and at quit.
    pub fn update_session(&mut self, change: impl FnOnce(&mut Session)) {
        let before = self.session.clone();
        change(&mut self.session);
        self.session_changed |= self.session != before;
    }

    fn save_session(&mut self) {
        if std::mem::take(&mut self.session_changed) {
            self.save(&self.session);
        }
    }

    /// Writes the session and waits until every queued file is on disk.
    pub fn flush(&mut self) -> impl Future<Output = ()> + use<> {
        self.session_changed = true;
        self.save_session();
        let (done, written) = smol::channel::bounded::<()>(1);
        let _ = self.saves.try_send(Box::new(move |_| {
            let _ = done.try_send(());
        }));
        async move {
            let _ = written.recv().await;
        }
    }

    fn save<T: Persisted + Clone + Send + 'static>(&self, value: &T) {
        let value = value.clone();
        let _ = self.saves.try_send(Box::new(move |store| {
            if let Err(error) = store.save(&value) {
                tracing::error!(file = T::FILE, %error, "could not save");
            }
        }));
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// The app state, readable from anywhere on the UI thread.
pub struct AppStateHandle(pub gpui_kit::Entity<AppState>);

impl gpui_kit::Global for AppStateHandle {}

pub fn app_state(cx: &App) -> gpui_kit::Entity<AppState> {
    cx.global::<AppStateHandle>().0.clone()
}
