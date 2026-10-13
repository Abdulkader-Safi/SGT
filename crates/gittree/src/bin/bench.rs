//! Performance measurements for NFR-01. Opens the graph on a repository,
//! waits for the full history, runs a scripted scroll and prints the numbers.
//!
//! Usage: cargo run --release -p gittree --bin bench -- <path to a repository>

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gittree::repo_state::RepoState;
use gittree::views::graph::GraphView;
use gittree::window_options;
use gittree_core::GitRunner;
use gittree_core::ops::setup;
use gittree_core::store::TrustList;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    WindowOptions, canvas, div, point, px,
};

const FRAMES_PER_PHASE: usize = 600;

/// Scroll phases: a name and how far each frame moves, in pixels. The first
/// is a steady scroll of three rows per frame. The second moves a whole
/// window per frame, so no row is ever seen twice.
const PHASES: [(&str, f32); 2] = [("smooth scroll", 72.), ("page scroll", 776.)];

#[derive(Default)]
struct Stats {
    first_rows: Option<Duration>,
    frame: usize,
    offset: f32,
    last_frame: Option<Instant>,
    /// Time from the start of one frame to the start of the next.
    intervals: [Vec<Duration>; PHASES.len()],
    /// Time the app spends on one frame: render, layout and paint.
    builds: [Vec<Duration>; PHASES.len()],
}

struct Bench {
    repo: PathBuf,
    start: Instant,
    graph: Entity<GraphView>,
    history: Entity<RepoState>,
    loaded: Option<Duration>,
    stats: Rc<RefCell<Stats>>,
    _observe: Subscription,
}

impl Render for Bench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame_start = Instant::now();
        let history = self.history.read(cx);
        let rows = history.history.commits.len();
        if !history.loading {
            self.loaded.get_or_insert(self.start.elapsed());
        }
        let graph = self.graph.read(cx);
        let mut stats = self.stats.borrow_mut();

        // Scroll only starts once the whole history is in.
        let mut phase = None;
        if self.loaded.is_some() {
            if stats.frame < FRAMES_PER_PHASE * PHASES.len() {
                let current = stats.frame / FRAMES_PER_PHASE;
                stats.offset += PHASES[current].1;
                let handle = &graph.scroll.0.borrow().base_handle;
                handle.set_offset(point(px(0.), px(-stats.offset)));
                if let Some(last) = stats.last_frame.replace(frame_start) {
                    stats.intervals[current].push(frame_start - last);
                }
                stats.frame += 1;
                phase = Some(current);
                window.request_animation_frame();
            } else {
                self.report(&stats, window, cx);
                cx.quit();
            }
        }

        // Painted last, so it marks the end of the app's work on this frame.
        let start = self.start;
        let shared = self.stats.clone();
        let frame_end = canvas(
            |_, _, _| (),
            move |_, _, _, _| {
                let mut stats = shared.borrow_mut();
                if rows > 0 {
                    stats.first_rows.get_or_insert(start.elapsed());
                }
                if let Some(phase) = phase {
                    stats.builds[phase].push(frame_start.elapsed());
                }
            },
        )
        .absolute()
        .size(px(1.));

        div().size_full().child(self.graph.clone()).child(frame_end)
    }
}

impl Bench {
    fn report(&self, stats: &Stats, window: &Window, cx: &Context<Self>) {
        let ms = |time: Option<Duration>| time.map_or(f64::NAN, |t| t.as_secs_f64() * 1e3);

        println!("repository        {}", self.repo.display());
        println!(
            "commits           {}",
            self.history.read(cx).history.commits.len()
        );
        println!(
            "display refresh   {:.1} ms",
            ms(window.refresh_interval(cx))
        );
        println!("first rows drawn  {:.0} ms", ms(stats.first_rows));
        println!("full load         {:.0} ms", ms(self.loaded));
        for (phase, (name, step)) in PHASES.iter().enumerate() {
            println!("{name}, {step} px per frame, {FRAMES_PER_PHASE} frames");
            summarise("  app work per frame", &stats.builds[phase]);
            summarise("  frame to frame    ", &stats.intervals[phase]);
        }
        println!(
            "peak memory       {:.0} MB",
            peak_memory_bytes() as f64 / 1e6
        );
    }
}

fn summarise(label: &str, times: &[Duration]) {
    // GPUI draws no frames while another window covers this one. A gap that
    // long is a pause, not a slow frame.
    let (mut sorted, pauses): (Vec<Duration>, Vec<Duration>) =
        times.iter().partition(|t| t.as_secs() < 1);
    sorted.sort();
    let at = |q: f64| {
        sorted
            .get(((sorted.len() as f64 * q) as usize).min(sorted.len().saturating_sub(1)))
            .map_or(f64::NAN, |t| t.as_secs_f64() * 1e3)
    };
    let slow = sorted.iter().filter(|t| t.as_secs_f64() > 0.016).count();
    print!(
        "{label}  median {:.2} ms, 95th percentile {:.2} ms, worst {:.2} ms, over 16 ms: {slow}",
        at(0.5),
        at(0.95),
        at(1.0),
    );
    if pauses.is_empty() {
        println!();
    } else {
        println!(", pauses left out: {}", pauses.len());
    }
}

/// The largest resident size the process reached. macOS reports bytes.
fn peak_memory_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given.
    unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init().ru_maxrss as u64
    }
}

fn main() {
    let start = Instant::now();
    let Some(repo) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: bench <path to a repository>");
        std::process::exit(2);
    };
    // The bench is not the app: it opens the repository before the UI starts.
    let repository =
        setup::open("git".as_ref(), &repo, &TrustList::default()).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        });
    gpui_kit::application().run(move |cx| {
        let runner = GitRunner::new("git", &repository.path, false);
        let history = cx.new(|cx| RepoState::new(repository, runner, cx));
        gpui_kit::init(cx);
        let loaded = gittree::app_state::Loaded::read(gittree_core::store::Store::new(
            std::env::temp_dir().join("gittree-bench"),
        ));
        let app = cx.new(|cx| gittree::app_state::AppState::new(loaded, None, cx));
        cx.set_global(gittree::app_state::AppStateHandle(app));
        let options = WindowOptions {
            // GPUI slows an unfocused window to 30 frames per second, which
            // would hide the real frame times when the bench is not in front.
            inactive_frame_interval: None,
            ..window_options(None, cx)
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| {
                let graph = cx.new(|cx| GraphView::new(history.clone(), cx));
                Bench {
                    repo,
                    start,
                    loaded: None,
                    stats: Default::default(),
                    _observe: cx.observe(&history, |_, _, cx| cx.notify()),
                    graph,
                    history,
                }
            })
        })
        .expect("failed to open window");
    });
}
