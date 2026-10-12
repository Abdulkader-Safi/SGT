use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use gittree_core::graph::{GraphLayout, GraphRow};
use gittree_core::model::Commit;
use gittree_core::ops::read::stream_log;
use gittree_core::{GitError, GitRunner};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, UniformListScrollHandle, Window, div, px, uniform_list,
};

use super::graph_element::{ROW_HEIGHT, graph_cell};

/// A history being read: batches of commits with their graph rows, then the
/// result of the Git call.
pub struct History {
    batches: smol::channel::Receiver<(Vec<Commit>, Vec<GraphRow>)>,
    git: Task<Result<(), GitError>>,
}

/// Starts reading a repository's history on the background executor. Call it
/// before the window exists, so Git runs while the UI starts up.
pub fn load_history(repo: PathBuf, cx: &App) -> History {
    let (tx, batches) = smol::channel::unbounded();
    let git = cx.background_spawn(async move {
        let mut layout = GraphLayout::default();
        let cancel = AtomicBool::new(false);
        stream_log(
            &GitRunner::new("git", repo, false),
            |commits| {
                let rows: Vec<GraphRow> = commits
                    .iter()
                    .map(|commit| layout.push(&commit.oid, &commit.parents))
                    .collect();
                // Nobody is listening once the receiver is gone.
                if tx.send_blocking((commits, rows)).is_err() {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
        )
    });
    History { batches, git }
}

/// The commit graph of one repository. This is the M0 spike: it holds the
/// history itself. From M2 the data lives in `RepoState`.
pub struct GraphView {
    commits: Vec<Commit>,
    rows: Vec<GraphRow>,
    /// Widest row seen so far, in lanes.
    lanes: usize,
    error: Option<SharedString>,
    pub scroll: UniformListScrollHandle,
    pub first_batch_at: Option<Instant>,
    pub loaded_at: Option<Instant>,
    _load: Task<()>,
}

impl GraphView {
    pub fn new(history: History, cx: &mut Context<Self>) -> Self {
        let load = cx.spawn(async move |this, cx| {
            while let Ok((commits, rows)) = history.batches.recv().await {
                let added = this.update(cx, |this, cx| {
                    this.first_batch_at.get_or_insert_with(Instant::now);
                    this.lanes = rows.iter().map(lane_count).fold(this.lanes, usize::max);
                    this.commits.extend(commits);
                    this.rows.extend(rows);
                    cx.notify();
                });
                if added.is_err() {
                    return;
                }
            }
            let result = history.git.await;
            let _ = this.update(cx, |this, cx| {
                this.loaded_at = Some(Instant::now());
                this.error = result.err().map(|error| error.to_string().into());
                cx.notify();
            });
        });
        Self {
            commits: Vec::new(),
            rows: Vec::new(),
            lanes: 1,
            error: None,
            scroll: UniformListScrollHandle::new(),
            first_batch_at: None,
            loaded_at: None,
            _load: load,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

fn lane_count(row: &GraphRow) -> usize {
    let widest = row
        .edges
        .iter()
        .map(|edge| edge.from_lane.max(edge.to_lane))
        .fold(row.lane, u16::max);
    widest as usize + 1
}

impl Render for GraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, muted) = (theme.background, theme.muted_foreground);
        let lanes = self.lanes;

        let list = uniform_list(
            "graph",
            self.rows.len(),
            cx.processor(move |this, range: Range<usize>, _, _| {
                range
                    .map(|ix| {
                        let commit = &this.commits[ix];
                        div()
                            .id(ix)
                            .h(ROW_HEIGHT)
                            .w_full()
                            .px_2()
                            .gap_2()
                            .flex()
                            .items_center()
                            .child(graph_cell(this.rows[ix].clone(), lanes, background))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .child(SharedString::from(commit.summary.clone())),
                            )
                            .child(
                                div()
                                    .w(px(140.))
                                    .truncate()
                                    .text_color(muted)
                                    .child(SharedString::from(commit.author_name.clone())),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .text_color(muted)
                                    .child(SharedString::from(commit.oid[..7].to_string())),
                            )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();

        div()
            .size_full()
            .bg(background)
            .text_color(theme.foreground)
            .text_sm()
            .children(self.error.clone())
            .child(list)
    }
}
