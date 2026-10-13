use std::collections::HashSet;

use gittree_core::model::{RepoPath, StatusEntry};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, ClickEvent, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use rust_i18n::t;

use super::dialogs::confirm_destructive;
use super::diff::DiffPane;
use super::graph::mono;
use super::ui::{caption, empty_state, ic, kbd, status_mark};
use crate::actions::StageSelection;
use crate::app_state::AppState;
use crate::palette::palette;
use crate::repo_state::{Op, RepoEvent, RepoState, discard_paths, entry_paths};

/// Past this many characters the summary counter warns.
pub const SUMMARY_LIMIT: usize = 72;

/// The work in progress: the unstaged and staged lists, the diff of one
/// file, and the commit box (FR-STAGE-01 to FR-STAGE-06).
pub struct StagingView {
    app: Entity<AppState>,
    repo: Entity<RepoState>,
    diff: Entity<DiffPane>,
    pub summary: Entity<InputState>,
    pub description: Entity<TextareaState>,
    pub amend: bool,
    /// What the commit box held before Amend replaced it.
    before_amend: Option<(String, String)>,
    /// Files chosen with a Cmd-click or a Shift-click, with their list.
    marked: HashSet<(RepoPath, bool)>,
    /// Focus for the lists and the diff. Space works on what is selected
    /// while it is here, and types a space while a text field has it.
    pub focus: FocusHandle,
}

/// Splits a commit message into its summary and the rest.
fn split_message(message: &str) -> (String, String) {
    let (summary, description) = message.split_once('\n').unwrap_or((message, ""));
    (summary.trim().to_owned(), description.trim().to_owned())
}

fn join_message(summary: &str, description: &str) -> String {
    let (summary, description) = (summary.trim(), description.trim());
    if description.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary}\n\n{description}")
    }
}

impl StagingView {
    pub fn new(
        app: Entity<AppState>,
        repo: Entity<RepoState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path = repo.read(cx).repository.path.clone();
        let draft = app.read(cx).session.drafts.get(&path).cloned();
        let (summary, description) = split_message(&draft.unwrap_or_default());
        let summary = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("commit.summary").into_owned())
                .default_value(summary)
        });
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder(t!("commit.description").into_owned())
                .default_value(description)
        });
        // Typing saves the draft, and Cmd+Enter in either field commits.
        let on_input =
            |this: &mut Self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
                match event {
                    InputEvent::Change => this.save_draft(cx),
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.commit(window, cx),
                    _ => {}
                }
            };
        cx.subscribe_in(&summary, window, move |this, _, event, window, cx| {
            on_input(this, event, window, cx)
        })
        .detach();
        cx.subscribe_in(&description, window, move |this, _, event, window, cx| {
            on_input(this, event, window, cx)
        })
        .detach();
        cx.observe(&repo, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            &repo,
            window,
            |this, _, event: &RepoEvent, window, cx| match event {
                RepoEvent::Committed => {
                    this.amend = false;
                    this.before_amend = None;
                    this.set_message("", "", window, cx);
                }
                RepoEvent::HeadMessage(message) if this.amend => {
                    let (summary, description) = split_message(message);
                    this.set_message(&summary, &description, window, cx);
                }
                RepoEvent::MessageWritten(message) => {
                    let (summary, description) = split_message(message);
                    this.set_message(&summary, &description, window, cx);
                }
                RepoEvent::Planned => super::dialogs::commit_plan(this.repo.clone(), window, cx),
                _ => {}
            },
        )
        .detach();
        Self {
            diff: cx.new(|cx| DiffPane::new(repo.clone(), cx)),
            app,
            repo,
            summary,
            description,
            amend: false,
            before_amend: None,
            marked: HashSet::new(),
            focus: cx.focus_handle(),
        }
    }

    fn message(&self, cx: &Context<Self>) -> (String, String) {
        (
            self.summary.read(cx).value().to_string(),
            self.description.read(cx).value().to_string(),
        )
    }

    pub fn set_message(
        &mut self,
        summary: &str,
        description: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (summary, description) = (summary.to_owned(), description.to_owned());
        self.summary
            .update(cx, |input, cx| input.set_value(summary, window, cx));
        self.description
            .update(cx, |input, cx| input.set_value(description, window, cx));
        self.save_draft(cx);
        cx.notify();
    }

    /// Keeps the unsent message with the session, per repository. An
    /// amended commit's message is not a draft.
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        if self.amend {
            return;
        }
        let path = self.repo.read(cx).repository.path.clone();
        let (summary, description) = self.message(cx);
        let draft = join_message(&summary, &description);
        self.app.update(cx, |app, _| {
            app.update_session(|session| {
                if draft.is_empty() {
                    session.drafts.remove(&path);
                } else {
                    session.drafts.insert(path, draft);
                }
            })
        });
        cx.notify();
    }

    /// Amend fills the box with the last commit's message. Switching it off
    /// brings back what was there.
    pub fn set_amend(&mut self, amend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.amend == amend {
            return;
        }
        if amend {
            self.before_amend = Some(self.message(cx));
            self.amend = true;
            self.repo.update(cx, |repo, cx| repo.load_head_message(cx));
        } else {
            self.amend = false;
            let (summary, description) = self.before_amend.take().unwrap_or_default();
            self.set_message(&summary, &description, window, cx);
        }
        cx.notify();
    }

    pub fn can_commit(&self, cx: &Context<Self>) -> bool {
        let repo = self.repo.read(cx);
        let staged = repo.changes(true).next().is_some();
        let conflicts = repo.status.iter().any(|entry| entry.conflict.is_some());
        repo.repository.trusted
            && !repo.busy
            && !conflicts
            && !self.summary.read(cx).value().trim().is_empty()
            // Amending only the message needs nothing staged.
            && (staged || self.amend)
            && (!self.amend || repo.has_commits())
    }

    pub fn commit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_commit(cx) {
            return;
        }
        let (summary, description) = self.message(cx);
        let op = Op::Commit {
            message: join_message(&summary, &description),
            amend: self.amend,
        };
        self.repo.update(cx, |repo, cx| repo.run_op(op, cx));
    }

    /// The entries an action on `clicked` covers: every marked file of its
    /// list when it is one of them, else only itself.
    fn acted_on(&self, clicked: &(RepoPath, bool), cx: &Context<Self>) -> Vec<StatusEntry> {
        let staged = clicked.1;
        let repo = self.repo.read(cx);
        let together = self.marked.contains(clicked);
        repo.changes(staged)
            .filter(|entry| {
                let key = (entry.path.clone(), staged);
                if together {
                    self.marked.contains(&key)
                } else {
                    &key == clicked
                }
            })
            .cloned()
            .collect()
    }

    fn move_files(&mut self, entries: &[StatusEntry], staged: bool, cx: &mut Context<Self>) {
        if entries.is_empty() {
            return;
        }
        let paths = entry_paths(entries);
        let op = if staged {
            Op::Unstage(paths)
        } else {
            Op::Stage(paths)
        };
        self.marked.clear();
        self.repo.update(cx, |repo, cx| repo.run_op(op, cx));
    }

    fn discard_files(
        &mut self,
        entries: Vec<StatusEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() {
            return;
        }
        let repo = self.repo.clone();
        let text = match &entries[..] {
            [one] => t!("discard.file_text", path = one.path.to_string()),
            many => t!("discard.files_text", count = many.len()),
        };
        confirm_destructive(
            t!("discard.title").into_owned(),
            text.into_owned(),
            t!("discard.action").into_owned(),
            move |cx| {
                let (tracked, untracked) = discard_paths(&entries);
                repo.update(cx, |repo, cx| {
                    repo.run_op(Op::DiscardFiles { tracked, untracked }, cx)
                });
            },
            window,
            cx,
        );
        self.marked.clear();
    }

    /// Space: the selected lines if there are any, else the selected files.
    fn toggle_selection(&mut self, cx: &mut Context<Self>) {
        if self.diff.update(cx, |diff, cx| diff.toggle_picked(cx)) {
            return;
        }
        let Some(file) = self.repo.read(cx).selection.wip_file.clone() else {
            return;
        };
        let entries = self.acted_on(&file, cx);
        self.move_files(&entries, file.1, cx);
    }

    /// A click shows the file. Cmd adds it to the marked files, and Shift
    /// marks everything from the shown file to it.
    fn click_file(
        &mut self,
        path: RepoPath,
        staged: bool,
        event: &ClickEvent,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        let key = (path.clone(), staged);
        let shown = self.repo.read(cx).selection.wip_file.clone();
        if modifiers.secondary() {
            // The shown file is part of the selection it started.
            self.marked.extend(shown.filter(|shown| shown.1 == staged));
            if !self.marked.remove(&key) {
                self.marked.insert(key);
            }
            cx.notify();
            return;
        }
        if modifiers.shift
            && let Some((from, true)) = shown.map(|shown| (shown.0, shown.1 == staged))
        {
            let paths: Vec<RepoPath> = self
                .repo
                .read(cx)
                .changes(staged)
                .map(|entry| entry.path.clone())
                .collect();
            let at = |wanted: &RepoPath| paths.iter().position(|path| path == wanted);
            if let (Some(a), Some(b)) = (at(&from), at(&path)) {
                self.marked = paths[a.min(b)..=a.max(b)]
                    .iter()
                    .map(|path| (path.clone(), staged))
                    .collect();
            }
            cx.notify();
            return;
        }
        self.marked.clear();
        self.repo
            .update(cx, |repo, cx| repo.select_wip_file(path, staged, false, cx));
    }

    fn file_list(&self, staged: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let mono = mono(cx);
        let repo = self.repo.read(cx);
        let can_change = repo.repository.trusted && !repo.busy;
        let shown = repo.selection.wip_file.clone();
        let entries: Vec<StatusEntry> = repo.changes(staged).cloned().collect();
        let marked_here = self.marked.iter().filter(|key| key.1 == staged).count();
        let small = |id: &'static str, label: String| {
            Button::new(id).xsmall().label(label).disabled(!can_change)
        };

        let title = if staged {
            t!("staging.staged", count = entries.len())
        } else {
            t!("staging.unstaged", count = entries.len())
        };
        let all = entries.clone();
        let header = h_flex()
            .flex_none()
            .h(px(30.))
            .pl_3()
            .pr_2()
            .gap_2()
            .border_b_1()
            .border_color(p.border)
            .child(div().flex_1().child(caption(title.to_uppercase(), p)))
            .when(marked_here > 1, |header| {
                let label = if staged {
                    t!("staging.unstage_selected", count = marked_here)
                } else {
                    t!("staging.stage_selected", count = marked_here)
                };
                header.child(
                    small(
                        if staged {
                            "unstage-marked"
                        } else {
                            "stage-marked"
                        },
                        label.into_owned(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let entries: Vec<StatusEntry> = this
                            .repo
                            .read(cx)
                            .changes(staged)
                            .filter(|e| this.marked.contains(&(e.path.clone(), staged)))
                            .cloned()
                            .collect();
                        this.move_files(&entries, staged, cx);
                    })),
                )
            })
            .child(
                small(
                    if staged { "unstage-all" } else { "stage-all" },
                    if staged {
                        t!("staging.unstage_all").into_owned()
                    } else {
                        t!("staging.stage_all").into_owned()
                    },
                )
                .disabled(!can_change || entries.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| this.move_files(&all, staged, cx))),
            );

        // ponytail: every row is built each frame. Fine for the 500 changed
        // files the spec asks for. Use a virtual list if that grows.
        let rows = entries.into_iter().enumerate().map(|(ix, entry)| {
            let key = (entry.path.clone(), staged);
            let is_shown = shown.as_ref() == Some(&key);
            let is_marked = self.marked.contains(&key);
            let name = match &entry.orig_path {
                Some(from) if staged => format!("{from} → {}", entry.path),
                _ => entry.path.to_string(),
            };
            let conflicted = entry.conflict.is_some();
            let (click_path, act_key, discard_key) = (entry.path.clone(), key.clone(), key.clone());
            let lit = is_shown || is_marked;
            // Row actions show on the selected rows and under the pointer.
            let actions = |row: gpui_kit::Div| {
                if lit {
                    row
                } else {
                    row.invisible()
                        .group_hover("file-row", |style| style.visible())
                }
            };
            h_flex()
                .id((if staged { "staged" } else { "unstaged" }, ix))
                .group("file-row")
                .h(px(28.))
                .pl_3()
                .pr_2()
                .gap_2()
                .cursor_pointer()
                .when(is_shown, |row| row.bg(p.bg_selected))
                .when(is_marked && !is_shown, |row| row.bg(p.accent_subtle))
                .when(!lit, |row| row.hover(|style| style.bg(p.bg_hover)))
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.click_file(click_path.clone(), staged, event, cx)
                }))
                .child(
                    status_mark(
                        if staged { entry.index } else { entry.worktree },
                        conflicted,
                        p,
                    )
                    .font_family(mono.clone()),
                )
                .child(div().flex_1().truncate().child(name))
                .when(conflicted, |row| {
                    row.child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.danger)
                            .child(t!("staging.conflicted").into_owned()),
                    )
                })
                .when(!conflicted, |row| {
                    row.child(
                        actions(h_flex().flex_none().gap_1()).child(
                            Button::new((if staged { "unstage" } else { "stage" }, ix))
                                .xsmall()
                                .ghost()
                                .disabled(!can_change)
                                .label(if staged {
                                    t!("staging.unstage").into_owned()
                                } else {
                                    t!("staging.stage").into_owned()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let entries = this.acted_on(&act_key, cx);
                                    this.move_files(&entries, staged, cx);
                                })),
                        ),
                    )
                })
                .when(!staged && !conflicted, |row| {
                    row.child(
                        actions(h_flex().flex_none()).child(
                            Button::new(("discard", ix))
                                .xsmall()
                                .ghost()
                                .disabled(!can_change)
                                .label(t!("staging.discard").into_owned())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    let entries = this.acted_on(&discard_key, cx);
                                    this.discard_files(entries, window, cx);
                                })),
                        ),
                    )
                })
        });

        v_flex().flex_none().child(header).child(
            v_flex()
                .id(if staged {
                    "staged-list"
                } else {
                    "unstaged-list"
                })
                .max_h(px(150.))
                .children(rows)
                .overflow_y_scrollbar(),
        )
    }
}

impl Render for StagingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let repo = self.repo.read(cx);
        let (busy, trusted, has_commits) = (repo.busy, repo.repository.trusted, repo.has_commits());
        let no_changes = repo.status.is_empty();
        let nothing_staged = repo.changes(true).next().is_none();
        let conflicts = repo.status.iter().any(|entry| entry.conflict.is_some());
        let pushed = self.amend && repo.head_is_pushed();
        let can_commit = self.can_commit(cx);
        let amend = self.amend;
        let (writing, written_by) = (repo.ai.writing, repo.ai.written_by.clone());
        let rewrite = written_by.is_some();
        // Asks the agent chosen in Settings for a message.
        let write_message = cx.listener(|this, _: &gpui_kit::ClickEvent, _, cx| {
            let settings = &this.app.read(cx).settings;
            let (kind, command) = (settings.ai_agent, settings.ai_command.clone());
            this.repo
                .update(cx, |repo, cx| repo.write_message(kind, &command, cx));
        });
        let planning = repo.ai.planning;
        let mid_operation = repo.in_progress.is_some();
        let plan_commits = cx.listener(|this, _: &gpui_kit::ClickEvent, _, cx| {
            let settings = &this.app.read(cx).settings;
            let (kind, command) = (settings.ai_agent, settings.ai_command.clone());
            this.repo
                .update(cx, |repo, cx| repo.plan_commits(kind, &command, cx));
        });
        let summary_len = self.summary.read(cx).value().trim().chars().count();
        let too_long = summary_len > SUMMARY_LIMIT;

        let changes = if no_changes {
            empty_state(
                "check",
                t!("staging.none_title").into_owned(),
                t!("staging.none_text").into_owned(),
                p,
            )
            .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .overflow_hidden()
                .child(self.file_list(false, cx))
                .child(self.file_list(true, cx))
                .when(nothing_staged, |lists| {
                    lists.child(
                        h_flex()
                            .flex_none()
                            .h(px(28.))
                            .px_3()
                            .text_size(px(12.))
                            .text_color(p.text_tertiary)
                            .child(t!("staging.nothing_staged").into_owned()),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .border_t_1()
                        .border_color(p.border)
                        .child(self.diff.clone()),
                )
                .into_any_element()
        };

        let note = |icon: &str, text: String, color| {
            h_flex()
                .gap(px(6.))
                .items_start()
                .text_size(px(12.))
                .text_color(color)
                .child(ic(icon, 14., color))
                .child(div().flex_1().child(text))
        };
        let commit_box = v_flex()
            .flex_none()
            .p_3()
            .gap_2()
            .bg(p.bg_panel)
            .border_t_1()
            .border_color(p.border)
            .child(
                Input::new(&self.summary)
                    .id("commit-summary")
                    .small()
                    .suffix(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(if too_long { p.warning } else { p.text_tertiary })
                            .child(format!("{summary_len}/{SUMMARY_LIMIT}")),
                    ),
            )
            .child(Textarea::new(&self.description).small())
            .when(too_long, |commit| {
                commit.child(note("info", t!("commit.too_long").into_owned(), p.warning))
            })
            .when(conflicts, |commit| {
                let repo = self.repo.clone();
                commit
                    .child(note(
                        "warning",
                        t!("staging.resolve_first").into_owned(),
                        p.danger,
                    ))
                    .child(
                        Button::new("resolve-conflicts")
                            .primary()
                            .small()
                            .label(t!("conflict.open").into_owned())
                            .on_click(move |_, _, cx| {
                                repo.update(cx, |repo, cx| {
                                    repo.resolving = true;
                                    cx.notify();
                                })
                            }),
                    )
            })
            .when(pushed, |commit| {
                commit.child(note(
                    "warning",
                    t!("commit.amend_pushed").into_owned(),
                    p.warning,
                ))
            })
            // Its own row: the commit row has no room in a narrow panel.
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().children(written_by.map(|agent| {
                        note(
                            "commit",
                            t!("ai.written", agent = agent).into_owned(),
                            p.text_tertiary,
                        )
                    })))
                    .child(
                        Button::new("plan-commits")
                            .small()
                            .loading(planning)
                            .disabled(
                                planning
                                    || writing
                                    || busy
                                    || no_changes
                                    || conflicts
                                    || mid_operation
                                    || !trusted,
                            )
                            .label(if planning {
                                t!("ai.planning").into_owned()
                            } else {
                                t!("ai.plan").into_owned()
                            })
                            .on_click(plan_commits),
                    )
                    .child(
                        Button::new("write-message")
                            .small()
                            .loading(writing)
                            .disabled(writing || planning || nothing_staged || !trusted)
                            .label(if writing {
                                t!("ai.writing").into_owned()
                            } else if rewrite {
                                t!("ai.again").into_owned()
                            } else {
                                t!("ai.write").into_owned()
                            })
                            .on_click(write_message),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("amend")
                            .label(t!("commit.amend").into_owned())
                            .checked(amend)
                            .disabled(!trusted || !has_commits)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                this.set_amend(*checked, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(kbd("⌘↵", p))
                    .child(
                        Button::new("commit")
                            .primary()
                            .loading(busy)
                            .disabled(!can_commit)
                            .label(if busy {
                                t!("commit.committing").into_owned()
                            } else if amend {
                                t!("commit.amend_action").into_owned()
                            } else {
                                t!("commit.action").into_owned()
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                    ),
            );

        v_flex()
            .size_full()
            .bg(p.bg_panel)
            .border_l_1()
            .border_color(p.border)
            .child(
                v_flex()
                    .id("changes")
                    .flex_1()
                    .overflow_hidden()
                    .key_context("Changes")
                    .track_focus(&self.focus)
                    .on_action(
                        cx.listener(|this, _: &StageSelection, _, cx| this.toggle_selection(cx)),
                    )
                    .child(changes),
            )
            .child(commit_box)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_splits_into_summary_and_description_and_back() {
        assert_eq!(split_message("Fix it"), ("Fix it".into(), String::new()));
        let (summary, description) = split_message("Fix it\n\nBecause.\n\nMore.\n");
        assert_eq!(
            (summary.as_str(), description.as_str()),
            ("Fix it", "Because.\n\nMore.")
        );
        assert_eq!(
            join_message(&summary, &description),
            "Fix it\n\nBecause.\n\nMore."
        );
        assert_eq!(join_message(" Fix it ", "  "), "Fix it");
        assert_eq!(join_message("", ""), "");
    }
}
