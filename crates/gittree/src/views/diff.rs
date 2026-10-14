use std::ops::Range;
use std::sync::Arc;

use gittree_core::model::{Change, DiffKind, DiffLine, FileDiff, LineKind};
use gittree_core::ops::read::LARGE_DIFF_SHOWN;
use gittree_core::ops::stage::Target;
use gittree_core::patch::{self, Lines};
use gittree_core::store::DiffView;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_kit::component::{ActiveTheme, Disableable, Rope, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, FontWeight, HighlightStyle, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Task, UniformListScrollHandle, Window, div,
    uniform_list,
};

use crate::zoom::px;
use rust_i18n::t;

use super::dialogs::confirm_destructive;
use super::ui::{empty_state, ic, kbd, status_mark};
use crate::app_state::app_state;
use crate::palette::Palette;
use crate::repo_state::{Op, RepoState};

const LINE_HEIGHT: f32 = 22.;

/// Where the shown diff comes from, which decides what can be done to it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Commit,
    Unstaged,
    Staged,
}

/// One row of the list.
#[derive(Clone, Copy)]
enum Row {
    Hunk(usize),
    /// Inline view: line `1` of hunk `0`.
    Line(usize, usize),
    /// Split view: the old line on the left, the new line on the right.
    Pair(usize, Option<usize>, Option<usize>),
}

fn rows(diff: &FileDiff, split: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for (h, hunk) in diff.hunks.iter().enumerate() {
        rows.push(Row::Hunk(h));
        if !split {
            rows.extend((0..hunk.lines.len()).map(|l| Row::Line(h, l)));
            continue;
        }
        // A run of removed lines pairs up with the added lines after it.
        let mut l = 0;
        while l < hunk.lines.len() {
            if hunk.lines[l].kind == LineKind::Context {
                rows.push(Row::Pair(h, Some(l), Some(l)));
                l += 1;
                continue;
            }
            let run = |from: usize, kind: LineKind| {
                let len = hunk.lines[from..]
                    .iter()
                    .take_while(|line| line.kind == kind)
                    .count();
                from..from + len
            };
            let removed = run(l, LineKind::Remove);
            let added = run(removed.end, LineKind::Add);
            for n in 0..removed.len().max(added.len()) {
                let side = |range: &Range<usize>| (n < range.len()).then(|| range.start + n);
                rows.push(Row::Pair(h, side(&removed), side(&added)));
            }
            l = added.end;
        }
    }
    rows
}

/// What changed inside each changed line, as a byte range of its drawn
/// text (FR-DIFF-04). Removed lines pair with the added lines after them,
/// as in the split view. `None` for a line with no partner, or when the
/// whole line changed.
// ponytail: one range per line, from the first to the last changed word.
// Two separate edits in one line show as one span. A real word diff (an
// LCS over words) would mark them apart.
fn changed_words(diff: &FileDiff) -> Words {
    let mut all: Words = diff
        .hunks
        .iter()
        .map(|hunk| vec![None; hunk.lines.len()])
        .collect();
    for (h, hunk) in diff.hunks.iter().enumerate() {
        let mut l = 0;
        while l < hunk.lines.len() {
            if hunk.lines[l].kind != LineKind::Remove {
                l += 1;
                continue;
            }
            let run = |from: usize, kind: LineKind| {
                from..from
                    + hunk.lines[from..]
                        .iter()
                        .take_while(|x| x.kind == kind)
                        .count()
            };
            let removed = run(l, LineKind::Remove);
            let added = run(removed.end, LineKind::Add);
            for (old, new) in removed.clone().zip(added.clone()) {
                let (a, b) = (display(&hunk.lines[old]), display(&hunk.lines[new]));
                if let Some((in_old, in_new)) = word_change(&a, &b) {
                    all[h][old] = Some(in_old);
                    all[h][new] = Some(in_new);
                }
            }
            l = added.end;
        }
    }
    all
}

/// The changed part of two versions of a line, widened to whole words.
fn word_change(a: &str, b: &str) -> Option<(Range<usize>, Range<usize>)> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // The bytes both share at the start, then at the end, on characters.
    let mut start = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i);
    while start > 0 && a[..start].chars().next_back().is_some_and(word) {
        start -= a[..start].chars().next_back().map_or(1, char::len_utf8);
    }
    let room = a.len().min(b.len()) - start;
    let mut end = a[start..]
        .chars()
        .rev()
        .zip(b[start..].chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum::<usize>()
        .min(room);
    // Cut back to the shared length, the end can split a character.
    while !a.is_char_boundary(a.len() - end) || !b.is_char_boundary(b.len() - end) {
        end -= 1;
    }
    while end > 0 && a[a.len() - end..].chars().next().is_some_and(word) {
        end -= a[a.len() - end..].chars().next().map_or(1, char::len_utf8);
    }
    let (old, new) = (start..a.len() - end, start..b.len() - end);
    // A line that changed all through gains nothing from a mark.
    let whole = old.len() == a.trim_end().len() && new.len() == b.trim_end().len();
    (!whole && !(old.is_empty() && new.is_empty())).then_some((old, new))
}

/// Syntax colours with `mark` laid over `range`: the spans are split at
/// its edges, so they stay in order and never overlap.
fn mark_range(spans: &Spans, range: Range<usize>, mark: HighlightStyle) -> Spans {
    let mut out = Spans::new();
    let mut at = range.start;
    for (span, style) in spans {
        let inside = span.start.max(range.start)..span.end.min(range.end);
        if inside.start >= inside.end {
            out.push((span.clone(), *style));
            continue;
        }
        if at < inside.start {
            out.push((at..inside.start, mark));
        }
        if span.start < inside.start {
            out.push((span.start..inside.start, *style));
        }
        out.push((
            inside.clone(),
            HighlightStyle {
                background_color: mark.background_color,
                ..*style
            },
        ));
        if inside.end < span.end {
            out.push((inside.end..span.end, *style));
        }
        at = at.max(inside.end);
    }
    if at < range.end {
        out.push((at..range.end, mark));
    }
    out.sort_by_key(|(span, _)| span.start);
    out
}

/// A line as it is drawn: no line ending, and tabs as four spaces.
fn display(line: &DiffLine) -> String {
    String::from_utf8_lossy(&line.text)
        .trim_end_matches(['\n', '\r'])
        .replace('\t', "    ")
}

/// The highlighter's name for a file, by its extension. None for a type
/// outside the supported list, which shows as plain text.
pub fn language_for(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "java" => "java",
        "php" => "php",
        "rb" => "ruby",
        "html" | "htm" => "html",
        "css" | "scss" => "css",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        _ => return None,
    })
}

type Spans = Vec<(Range<usize>, HighlightStyle)>;
/// Per hunk and line, the changed part of the line's text.
type Words = Vec<Vec<Option<Range<usize>>>>;

/// Syntax colours for every line of a diff, as ranges into `display(line)`.
///
/// The old and the new side are each parsed as one text. A hunk starts in
/// the middle of a file, so a colour can be off where a construct began
/// before the hunk.
pub fn highlight(diff: &FileDiff, language: &str, theme: &HighlightTheme) -> Vec<Vec<Spans>> {
    let mut all: Vec<Vec<Spans>> = diff
        .hunks
        .iter()
        .map(|hunk| vec![Spans::new(); hunk.lines.len()])
        .collect();
    for old_side in [true, false] {
        let skipped = if old_side {
            LineKind::Add
        } else {
            LineKind::Remove
        };
        let mut text = String::new();
        // Where each line of this side sits in `text`.
        let mut places = Vec::new();
        for (h, hunk) in diff.hunks.iter().enumerate() {
            for (l, line) in hunk.lines.iter().enumerate() {
                if line.kind != skipped {
                    let start = text.len();
                    text.push_str(&display(line));
                    places.push((start..text.len(), h, l));
                    text.push('\n');
                }
            }
        }
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.update(None, &Rope::from(text.as_str()), None);
        // Both lists are in text order, so one pass over the lines is enough.
        let mut at = 0;
        for (range, style) in highlighter.styles(&(0..text.len()), theme) {
            if style == HighlightStyle::default() {
                continue;
            }
            while places
                .get(at)
                .is_some_and(|(place, ..)| place.end <= range.start)
            {
                at += 1;
            }
            for (place, h, l) in places[at..]
                .iter()
                .take_while(|(place, ..)| place.start < range.end)
            {
                // Context lines take their colours from the new side.
                if old_side && diff.hunks[*h].lines[*l].kind == LineKind::Context {
                    continue;
                }
                let start = range.start.max(place.start) - place.start;
                let end = range.end.min(place.end) - place.start;
                if start < end {
                    all[*h][*l].push((start..end, style));
                }
            }
        }
    }
    all
}

/// The diff of the selected file, inline or split, with syntax colours. For
/// a working-tree diff, lines can be selected and hunks or lines staged,
/// unstaged or discarded (FR-DIFF-01 to 03, FR-DIFF-09, FR-STAGE-02 to 04).
pub struct DiffPane {
    repo: Entity<RepoState>,
    /// Shows the diff of the file view's picked commit, not the selection's.
    of_file_view: bool,
    scroll: UniformListScrollHandle,
    /// The diff and the view (split or not) the state below was made for.
    /// Holding the diff keeps its address from being reused by the next one.
    seen: Option<(Arc<FileDiff>, bool)>,
    rows: Arc<Vec<Row>>,
    /// The chosen changed lines.
    picked: Lines,
    /// The row a shift-click or drag extends from, and whether the drag
    /// selects or clears.
    anchor: Option<(usize, bool)>,
    /// The diff and theme the colours were made for.
    styled: Option<(Arc<FileDiff>, Arc<HighlightTheme>)>,
    styles: Option<Arc<Vec<Vec<Spans>>>>,
    style_task: Task<()>,
    /// What changed inside each line, made with the rows.
    words: Arc<Words>,
}

impl DiffPane {
    pub fn new(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        Self {
            repo,
            of_file_view: false,
            scroll: UniformListScrollHandle::new(),
            seen: None,
            rows: Default::default(),
            picked: Lines::new(),
            anchor: None,
            styled: None,
            styles: None,
            words: Arc::default(),
            style_task: Task::ready(()),
        }
    }

    /// A pane for the file view's history: the diff of the picked commit.
    // ponytail: "Load all" on a cut diff still reloads the selection's
    // diff, so a huge diff stays cut here.
    pub fn for_file_view(repo: Entity<RepoState>, cx: &mut Context<Self>) -> Self {
        Self {
            of_file_view: true,
            ..Self::new(repo, cx)
        }
    }

    fn origin(&self, cx: &Context<Self>) -> Origin {
        if self.of_file_view {
            return Origin::Commit;
        }
        let selection = &self.repo.read(cx).selection;
        match &selection.wip_file {
            Some((_, true)) if selection.wip => Origin::Staged,
            Some((_, false)) if selection.wip => Origin::Unstaged,
            _ => Origin::Commit,
        }
    }

    /// Rebuilds the rows and drops the line selection when another diff, or
    /// the other view, shows.
    fn sync(&mut self, diff: &Arc<FileDiff>, split: bool, cx: &mut Context<Self>) {
        let same_diff = |held: &Arc<FileDiff>| Arc::ptr_eq(held, diff);
        match &self.seen {
            Some((held, was_split)) if same_diff(held) && *was_split == split => {}
            seen => {
                if !seen.as_ref().is_some_and(|(held, _)| same_diff(held)) {
                    self.picked.clear();
                    self.anchor = None;
                }
                self.rows = Arc::new(rows(diff, split));
                self.words = Arc::new(changed_words(diff));
                self.seen = Some((diff.clone(), split));
            }
        }

        let theme = cx.theme().highlight_theme.clone();
        if self
            .styled
            .as_ref()
            .is_some_and(|(held, held_theme)| same_diff(held) && Arc::ptr_eq(held_theme, &theme))
        {
            return;
        }
        self.styled = Some((diff.clone(), theme.clone()));
        self.styles = None;
        let path = diff.new_path.as_ref().or(diff.old_path.as_ref());
        let Some(language) = path.and_then(|path| language_for(&path.to_string())) else {
            self.style_task = Task::ready(());
            return;
        };
        let diff = diff.clone();
        let parse = cx.background_spawn({
            let diff = diff.clone();
            async move { highlight(&diff, language, &theme) }
        });
        // Replacing the task drops the colours of an older diff or theme.
        self.style_task = cx.spawn(async move |this, cx| {
            let styles = parse.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .styled
                    .as_ref()
                    .is_some_and(|(held, _)| Arc::ptr_eq(held, &diff))
                {
                    this.styles = Some(Arc::new(styles));
                    cx.notify();
                }
            });
        });
    }

    /// The changed lines in a span of rows.
    fn lines_in(&self, diff: &FileDiff, from: usize, to: usize) -> Vec<(usize, usize)> {
        let changed = |h: usize, l: usize| {
            (diff.hunks[h].lines[l].kind != LineKind::Context).then_some((h, l))
        };
        self.rows[from.min(to)..=from.max(to)]
            .iter()
            .flat_map(|row| match *row {
                Row::Hunk(_) => [None, None],
                Row::Line(h, l) => [changed(h, l), None],
                Row::Pair(h, old, new) => [
                    old.and_then(|l| changed(h, l)),
                    new.and_then(|l| changed(h, l)),
                ],
            })
            .flatten()
            .collect()
    }

    /// A click selects or clears one changed line. Shift extends from the
    /// last clicked row.
    fn press(&mut self, row: usize, line: (usize, usize), shift: bool, cx: &mut Context<Self>) {
        let Some((diff, _)) = self.seen.clone() else {
            return;
        };
        match self.anchor {
            Some((anchor, _)) if shift => self.picked.extend(self.lines_in(&diff, anchor, row)),
            _ => {
                let select = !self.picked.contains(&line);
                if select {
                    self.picked.insert(line);
                } else {
                    self.picked.remove(&line);
                }
                self.anchor = Some((row, select));
            }
        }
        cx.notify();
    }

    /// Dragging over rows gives them all the state the pressed line got.
    fn drag_to(&mut self, row: usize, cx: &mut Context<Self>) {
        let (Some((anchor, select)), Some((diff, _))) = (self.anchor, self.seen.clone()) else {
            return;
        };
        for line in self.lines_in(&diff, anchor, row) {
            if select {
                self.picked.insert(line);
            } else {
                self.picked.remove(&line);
            }
        }
        cx.notify();
    }

    pub fn has_picked(&self) -> bool {
        !self.picked.is_empty()
    }

    /// Stages the selected lines of an unstaged diff, or unstages those of
    /// a staged one. Returns false when no line is selected.
    pub fn toggle_picked(&mut self, cx: &mut Context<Self>) -> bool {
        let target = match self.origin(cx) {
            Origin::Unstaged => Target::Stage,
            Origin::Staged => Target::Unstage,
            Origin::Commit => return false,
        };
        if self.picked.is_empty() {
            return false;
        }
        let lines = std::mem::take(&mut self.picked);
        self.apply(lines, target, cx);
        true
    }

    fn apply(&mut self, lines: Lines, target: Target, cx: &mut Context<Self>) {
        let Some((diff, _)) = self.seen.clone() else {
            return;
        };
        self.picked.clear();
        self.anchor = None;
        self.repo.update(cx, |repo, cx| {
            repo.run_op(
                Op::Apply {
                    diff,
                    lines,
                    target,
                },
                cx,
            )
        });
    }

    /// Asks first, then takes the lines out of the working tree.
    fn discard(&mut self, lines: Lines, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        let count = lines.len();
        confirm_destructive(
            t!("discard.title").into_owned(),
            t!("discard.lines_text", count = count).into_owned(),
            t!("discard.action").into_owned(),
            move |cx| {
                let lines = lines.clone();
                this.update(cx, |this, cx| this.apply(lines, Target::Discard, cx));
            },
            window,
            cx,
        );
    }
}

impl Render for DiffPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = crate::palette::palette(cx);
        let settings = &app_state(cx).read(cx).settings;
        let split = settings.diff_view == DiffView::Split;
        let (mono, mono_size) = (
            SharedString::from(settings.diff_font_family.clone()),
            px(settings.diff_font_size),
        );
        let repo = self.repo.read(cx);
        let (shown, loading) = match (&repo.file_view, self.of_file_view) {
            (Some(view), true) => (view.diff.clone(), view.diff_loading),
            (None, true) => (None, false),
            (_, false) => (repo.selection.diff.clone(), repo.selection.diff_loading),
        };
        let busy = repo.busy;
        let trusted = repo.repository.trusted;
        let ignore_whitespace = repo.ignore_whitespace;
        let note = |icon: &str, title: String, text: String| {
            empty_state(icon, title, text, palette).into_any_element()
        };
        let Some(diff) = shown else {
            return if loading {
                note("fetch", String::new(), t!("detail.loading").into_owned())
            } else {
                div().into_any_element()
            };
        };
        let origin = self.origin(cx);
        self.sync(&diff, split, cx);

        let path = diff.new_path.as_ref().or(diff.old_path.as_ref());
        let change = match (&diff.old_path, &diff.new_path) {
            (None, _) => Change::Added,
            (_, None) => Change::Deleted,
            (Some(old), Some(new)) if old != new => Change::Renamed,
            _ => Change::Modified,
        };
        // The inline or split choice, as two segments.
        let segment = |id: &'static str, icon: &str, label: String, on: bool| {
            h_flex()
                .id(id)
                .h(px(22.))
                .px(px(10.))
                .gap(px(6.))
                .rounded(px(4.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on {
                    palette.text
                } else {
                    palette.text_tertiary
                })
                .when(on, |segment| {
                    segment
                        .bg(palette.bg_elevated)
                        .border_1()
                        .border_color(palette.border_strong)
                })
                .when(!on, |segment| {
                    segment.cursor_pointer().on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ToggleDiffView), cx)
                    })
                })
                .child(ic(
                    icon,
                    12.,
                    if on {
                        palette.text
                    } else {
                        palette.text_tertiary
                    },
                ))
                .child(label)
        };
        let header = h_flex()
            .flex_none()
            .h(px(34.))
            .pl_3()
            .pr_2()
            .gap_2()
            .border_b_1()
            .border_color(palette.border)
            .child(status_mark(change, false, palette).font_family(mono.clone()))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(path.map(ToString::to_string).unwrap_or_default()),
            )
            .child({
                let repo = self.repo.clone();
                gpui_kit::component::checkbox::Checkbox::new("ignore-whitespace")
                    .label(t!("diff.ignore_whitespace").into_owned())
                    .checked(ignore_whitespace)
                    .on_click(move |_, _, cx| {
                        repo.update(cx, |repo, cx| repo.toggle_whitespace(cx))
                    })
            })
            .child(
                h_flex()
                    .flex_none()
                    .p(px(2.))
                    .rounded(px(6.))
                    .bg(palette.bg_sunken)
                    .border_1()
                    .border_color(palette.border)
                    .child(segment(
                        "diff-inline",
                        "inline",
                        t!("settings.diff_inline").into_owned(),
                        !split,
                    ))
                    .child(segment(
                        "diff-split",
                        "split",
                        t!("settings.diff_split").into_owned(),
                        split,
                    )),
            );

        let body = match diff.kind {
            DiffKind::Binary { old_size, new_size } => note(
                "file",
                t!("detail.binary_title").into_owned(),
                t!("detail.binary", old = old_size, new = new_size).into_owned(),
            ),
            DiffKind::ModeOnly => note(
                "check",
                t!("detail.unchanged").into_owned(),
                t!("detail.mode_only").into_owned(),
            ),
            _ if diff.hunks.is_empty() => {
                note("check", t!("detail.unchanged").into_owned(), String::new())
            }
            _ => {
                // A cut diff ends mid-hunk, so its parts cannot be applied.
                let cut = matches!(diff.kind, DiffKind::TooLarge { .. });
                // A diff without its whitespace changes no longer fits the file.
                let can_edit = origin != Origin::Commit && trusted && !cut && !ignore_whitespace;
                let rows = self.rows.clone();
                let styles = self.styles.clone();
                let words = self.words.clone();
                let list = uniform_list(
                    "diff",
                    rows.len(),
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        // The diff the rows were built from.
                        let Some((diff, _)) = this.seen.clone() else {
                            return Vec::new();
                        };
                        range
                            .filter_map(|ix| Some((ix, *rows.get(ix)?)))
                            .map(|(ix, row)| {
                                let styles = (styles.as_deref(), &*words);
                                let picked = |l: Option<usize>, h: usize| {
                                    l.is_some_and(|l| this.picked.contains(&(h, l)))
                                };
                                match row {
                                    Row::Hunk(h) => {
                                        hunk_row(&diff, h, origin, can_edit && !busy, palette, cx)
                                    }
                                    Row::Line(h, l) => line_cell(
                                        &diff,
                                        styles,
                                        (ix, h, Some(l)),
                                        [true, true],
                                        can_edit,
                                        picked(Some(l), h),
                                        palette,
                                        cx,
                                    ),
                                    Row::Pair(h, old, new) => {
                                        let (old_picked, new_picked) =
                                            (picked(old, h), picked(new, h));
                                        let left = line_cell(
                                            &diff,
                                            styles,
                                            (ix, h, old),
                                            [true, false],
                                            can_edit,
                                            old_picked,
                                            palette,
                                            cx,
                                        );
                                        let right = line_cell(
                                            &diff,
                                            styles,
                                            (ix, h, new),
                                            [false, true],
                                            can_edit,
                                            new_picked,
                                            palette,
                                            cx,
                                        );
                                        h_flex()
                                            .h(px(LINE_HEIGHT))
                                            .w_full()
                                            .child(left)
                                            .child(div().w(px(1.)).h_full().bg(palette.border))
                                            .child(right)
                                            .into_any_element()
                                    }
                                }
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .w_full()
                .font_family(mono)
                .text_size(mono_size);

                let too_large = match diff.kind {
                    DiffKind::TooLarge { lines } => {
                        let repo = self.repo.clone();
                        Some(
                            h_flex()
                                .flex_none()
                                .h(px(36.))
                                .pl_3()
                                .pr_2()
                                .gap_2()
                                .bg(palette.warning_subtle)
                                .border_b_1()
                                .border_color(palette.border)
                                .text_size(px(12.))
                                .child(ic("info", 14., palette.warning))
                                .child(
                                    div().flex_1().child(
                                        t!(
                                            "detail.too_large",
                                            lines = lines,
                                            shown = LARGE_DIFF_SHOWN
                                        )
                                        .into_owned(),
                                    ),
                                )
                                .child(
                                    Button::new("load-all")
                                        .small()
                                        .label(t!("detail.load_all").into_owned())
                                        .on_click(move |_, _, cx| {
                                            repo.update(cx, |repo, cx| repo.load_whole_diff(cx))
                                        }),
                                ),
                        )
                    }
                    _ => None,
                };

                // Shown while lines are selected.
                let picked = (!self.picked.is_empty() && can_edit).then(|| {
                    let act = |id: &'static str, label: String| {
                        Button::new(id).xsmall().label(label).disabled(busy)
                    };
                    let bar = h_flex()
                        .flex_none()
                        .h(px(36.))
                        .pl_3()
                        .pr_2()
                        .gap(px(6.))
                        .bg(palette.accent_subtle)
                        .border_b_1()
                        .border_color(palette.border)
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(
                                    t!("diff.lines_selected", count = self.picked.len())
                                        .into_owned(),
                                ),
                        )
                        .child(kbd(t!("diff.space").into_owned(), palette));
                    let bar = if origin == Origin::Unstaged {
                        bar.child(
                            act("stage-lines", t!("diff.stage_lines").into_owned())
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_picked(cx);
                                })),
                        )
                        .child(
                            act("discard-lines", t!("diff.discard_lines").into_owned())
                                .danger()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.discard(this.picked.clone(), window, cx)
                                })),
                        )
                    } else {
                        bar.child(
                            act("unstage-lines", t!("diff.unstage_lines").into_owned())
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_picked(cx);
                                })),
                        )
                    };
                    bar.child(
                        Button::new("clear-lines")
                            .xsmall()
                            .ghost()
                            .label(t!("diff.clear_lines").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.picked.clear();
                                cx.notify();
                            })),
                    )
                });

                v_flex()
                    .size_full()
                    .children(too_large)
                    .children(picked)
                    .child(list)
                    .into_any_element()
            }
        };

        v_flex()
            .size_full()
            .bg(palette.bg_app)
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
            .into_any_element()
    }
}

/// A hunk header, with its buttons when the diff is of the working tree.
fn hunk_row(
    diff: &Arc<FileDiff>,
    h: usize,
    origin: Origin,
    enabled: bool,
    palette: Palette,
    cx: &mut Context<DiffPane>,
) -> AnyElement {
    let hunk = &diff.hunks[h];
    let act = |id: &'static str, label: String| {
        Button::new((id, h))
            .xsmall()
            .label(label)
            .disabled(!enabled)
    };
    let apply = |target: Target| {
        let diff = diff.clone();
        cx.listener(move |this: &mut DiffPane, _, _, cx| {
            this.apply(patch::hunk_lines(&diff, h), target, cx)
        })
    };
    let buttons: Vec<Button> = match origin {
        Origin::Commit => Vec::new(),
        Origin::Staged => vec![
            act("unstage-hunk", t!("diff.unstage_hunk").into_owned())
                .on_click(apply(Target::Unstage)),
        ],
        Origin::Unstaged => {
            let lines = patch::hunk_lines(diff, h);
            vec![
                act("stage-hunk", t!("diff.stage_hunk").into_owned())
                    .on_click(apply(Target::Stage)),
                act("discard-hunk", t!("diff.discard_hunk").into_owned())
                    .danger()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.discard(lines.clone(), window, cx)
                    })),
            ]
        }
    };
    h_flex()
        .h(px(LINE_HEIGHT))
        .w_full()
        .pl(px(10.))
        .pr_2()
        .gap(px(6.))
        .bg(palette.diff_hunk)
        .text_size(px(11.))
        .text_color(palette.text_secondary)
        .child(div().flex_1().truncate().child(format!(
            "@@ -{},{} +{},{} @@ {}",
            hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len, hunk.section
        )))
        .children(buttons)
        .into_any_element()
}

/// One line with its gutters. `numbers` says which of the old and new line
/// numbers to show: both inline, one per side in the split view. A missing
/// line is the empty half of a split row.
#[allow(clippy::too_many_arguments)]
fn line_cell(
    diff: &FileDiff,
    (styles, words): (Option<&Vec<Vec<Spans>>>, &Words),
    (row, h, l): (usize, usize, Option<usize>),
    numbers: [bool; 2],
    can_edit: bool,
    picked: bool,
    palette: Palette,
    cx: &mut Context<DiffPane>,
) -> AnyElement {
    // Full width, so the colour of a changed line reaches the edge. In the
    // split view the two halves share the row equally.
    let cell = h_flex()
        .h(px(LINE_HEIGHT))
        .w_full()
        .flex_1()
        .overflow_hidden();
    let Some(l) = l else {
        return cell.bg(palette.bg_panel).into_any_element();
    };
    let line = &diff.hunks[h].lines[l];
    let (sign, color, gutter, sign_color) = match line.kind {
        LineKind::Add => (
            "+",
            Some(palette.diff_added),
            Some(palette.diff_added_gutter),
            palette.success,
        ),
        LineKind::Remove => (
            "-",
            Some(palette.diff_removed),
            Some(palette.diff_removed_gutter),
            palette.danger,
        ),
        LineKind::Context => (" ", None, None, palette.text_tertiary),
    };
    let gutter = if picked {
        Some(palette.diff_picked)
    } else {
        gutter
    };
    let number = |n: Option<u32>| {
        h_flex()
            .w(px(38.))
            .h_full()
            .flex_none()
            .pr(px(6.))
            .justify_end()
            .text_size(px(11.))
            .text_color(palette.text_tertiary)
            .when_some(gutter, |cell, color| cell.bg(color))
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    let text = display(line);
    let spans = styles
        .and_then(|styles| styles.get(h)?.get(l))
        // A colour range must sit on character boundaries of this text.
        .filter(|spans| {
            spans.iter().all(|(range, _)| {
                text.is_char_boundary(range.start) && text.is_char_boundary(range.end)
            })
        })
        .cloned()
        .unwrap_or_default();
    // The changed words get the stronger colour of the line's gutter.
    let changed = words.get(h).and_then(|words| words.get(l)?.clone());
    let spans = match (changed, gutter) {
        (Some(range), Some(strong))
            if !picked
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end) =>
        {
            let mark = HighlightStyle {
                background_color: Some(strong),
                ..HighlightStyle::default()
            };
            mark_range(&spans, range, mark)
        }
        _ => spans,
    };
    let selectable = can_edit && line.kind != LineKind::Context;
    cell.when_some(color, |cell, color| cell.bg(color))
        .when(picked, |cell| cell.bg(palette.diff_picked))
        .when(selectable, |cell| {
            cell.cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.press(row, (h, l), event.modifiers.shift, cx)
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.drag_to(row, cx);
                    }
                }))
        })
        .when(numbers[0], |cell| cell.child(number(line.old_no)))
        .when(numbers[1], |cell| cell.child(number(line.new_no)))
        .child(
            div()
                .w(px(18.))
                .flex_none()
                .text_center()
                .text_color(sign_color)
                .child(sign),
        )
        .child(
            div()
                .flex_1()
                .whitespace_nowrap()
                .child(StyledText::new(text).with_highlights(spans)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gittree_core::parse::diff::parse_file_diff;

    const DIFF: &[u8] = b"--- a/f.rs\n+++ b/f.rs\n@@ -1,4 +1,5 @@\n fn main() {\n\
-    let a = \"old\";\n-    // gone\n+    let a = \"new\";\n+    let b = 2;\n+    // added\n }\n";

    #[test]
    fn split_rows_pair_removed_lines_with_added_ones() {
        let diff = parse_file_diff(DIFF);
        let shape: Vec<_> = rows(&diff, true)
            .iter()
            .map(|row| match *row {
                Row::Pair(_, old, new) => (old, new),
                _ => (None, None),
            })
            .collect();
        assert_eq!(
            shape,
            [
                (None, None),
                (Some(0), Some(0)),
                (Some(1), Some(3)),
                (Some(2), Some(4)),
                (None, Some(5)),
                (Some(6), Some(6)),
            ]
        );
        assert_eq!(rows(&diff, false).len(), 8);
    }

    #[test]
    fn changed_words_are_marked_inside_paired_lines() {
        assert_eq!(
            word_change("let total = price * count;", "let total = price * amount;"),
            Some((20..25, 20..26))
        );
        // Widened to whole words, and safe on multi-byte characters.
        assert_eq!(
            word_change("naïve cafe", "naïve café"),
            Some((7..11, 7..12))
        );
        assert_eq!(word_change("abc", "xyz"), None);
        assert_eq!(word_change("same", "same"), None);
        // One line repeats the other: no character is split, and a word
        // that changed all through marks nothing.
        assert_eq!(word_change("éé", "ééé"), None);

        let diff = parse_file_diff(
            b"--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-a = 1\n-gone\n+a = 2\n context\n",
        );
        let words = changed_words(&diff);
        assert_eq!(words[0], [Some(4..5), None, Some(4..5), None]);

        let red = HighlightStyle::default();
        let bold = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..HighlightStyle::default()
        };
        let marked = mark_range(&vec![(0..3, bold), (6..9, bold)], 2..7, red);
        let edges: Vec<Range<usize>> = marked.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(edges, [0..2, 2..3, 3..6, 6..7, 7..9]);
    }

    #[test]
    fn rust_keywords_strings_and_comments_get_different_colours() {
        // FR-DIFF-03, in both themes.
        let diff = parse_file_diff(DIFF);
        for theme in [
            HighlightTheme::default_dark(),
            HighlightTheme::default_light(),
        ] {
            let styles = highlight(&diff, "rust", &theme);
            let color_at = |l: usize, needle: &str| {
                let text = display(&diff.hunks[0].lines[l]);
                let at = text.find(needle).unwrap();
                styles[0][l]
                    .iter()
                    .find(|(range, _)| range.contains(&at))
                    .and_then(|(_, style)| style.color)
            };
            let keyword = color_at(3, "let");
            let string = color_at(3, "\"new\"");
            let comment = color_at(5, "// added");
            assert!(keyword.is_some() && string.is_some() && comment.is_some());
            assert!(keyword != string && string != comment && keyword != comment);
            // The removed side is coloured from its own text.
            assert_eq!(color_at(1, "\"old\""), string);
        }
    }

    #[test]
    fn every_listed_language_has_a_grammar() {
        // The 19 languages of FR-DIFF-03, by a file name of each.
        let files = [
            "a.rs", "a.js", "a.ts", "a.tsx", "a.py", "a.go", "a.c", "a.cpp", "a.java", "a.php",
            "a.rb", "a.html", "a.css", "a.json", "a.yaml", "a.toml", "a.md", "a.sql", "a.sh",
        ];
        for file in files {
            let language = language_for(file).unwrap_or_else(|| panic!("{file} has no language"));
            // A language the kit cannot load falls back to "text".
            assert_eq!(
                &**SyntaxHighlighter::new(language).language(),
                language,
                "{file}"
            );
        }
    }

    #[test]
    fn an_unknown_file_type_is_plain_text() {
        assert_eq!(language_for("notes.xyz"), None);
        assert_eq!(language_for("Makefile"), None);
        assert_eq!(language_for("src/App.TSX"), Some("tsx"));
    }
}
