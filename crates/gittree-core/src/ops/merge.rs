//! Merge, rebase, cherry-pick, revert and reset, the operation in progress,
//! and resolving conflicted files.

use std::path::Path;

use crate::error::{GitError, GitErrorKind};
use crate::model::RepoPath;
use crate::ops::read;
use crate::runner::{Access, GitRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// A merge, rebase, cherry-pick or revert that has stopped part way, also
/// one started outside the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InProgressOp {
    pub kind: OpKind,
    /// Current and total step. Rebase only.
    pub step: Option<(u32, u32)>,
    /// What is being merged, or rebased onto, for the banner.
    pub onto: Option<String>,
    /// Files still in conflict.
    pub conflicted: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

fn check_rev(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(GitError {
            kind: GitErrorKind::Other,
            args: Vec::new(),
            exit_code: None,
            stderr: "A revision cannot be empty or start with a dash.".into(),
        });
    }
    Ok(())
}

/// Git reports a conflict on stdout and with exit code 1, which the runner
/// cannot tell from other failures. If the repository is left with an
/// operation in progress, the failure was a conflict.
fn or_conflict(
    runner: &GitRunner,
    git_dir: &Path,
    result: Result<Vec<u8>, GitError>,
) -> Result<(), GitError> {
    match result {
        Ok(_) => Ok(()),
        Err(mut error) => {
            if in_progress(runner, git_dir).is_some() {
                error.kind = GitErrorKind::Conflict;
            }
            Err(error)
        }
    }
}

pub fn merge(
    runner: &GitRunner,
    git_dir: &Path,
    branch: &str,
    no_ff: bool,
) -> Result<(), GitError> {
    check_rev(branch)?;
    let mut args = vec!["merge", "--no-edit"];
    args.extend(no_ff.then_some("--no-ff"));
    args.push(branch);
    or_conflict(runner, git_dir, runner.write(args))
}

/// The files that would conflict if `theirs` were merged into HEAD, worked
/// out without touching the working tree or the index (FR-MERGE-06).
/// Git writes the trial result's objects, so it needs a trusted repository.
// ponytail: a rebase is predicted as the same three-way merge. Replayed
// commit by commit it can conflict where the whole does not, or the other
// way round. Predicting each step needs a merge-tree per commit.
pub fn predict_conflicts(runner: &GitRunner, theirs: &str) -> Result<Vec<RepoPath>, GitError> {
    use std::io::Read;

    check_rev(theirs)?;
    let mut out = Vec::new();
    let args = [
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "-z",
        "HEAD",
        theirs,
    ];
    let result = runner.run(Access::Write, args, None, None, |stdout| {
        stdout.read_to_end(&mut out)
    });
    match result {
        Ok(_) => Ok(Vec::new()),
        // Exit code 1: the merge has conflicts. The tree's id comes
        // first, then each conflicted path, once per stage.
        Err(error) if error.exit_code == Some(1) => {
            let mut paths: Vec<RepoPath> = out
                .split(|&byte| byte == 0)
                .skip(1)
                .filter(|path| !path.is_empty())
                .map(|path| RepoPath(path.to_vec()))
                .collect();
            paths.dedup();
            Ok(paths)
        }
        Err(error) => Err(error),
    }
}

pub fn rebase(runner: &GitRunner, git_dir: &Path, onto: &str) -> Result<(), GitError> {
    check_rev(onto)?;
    or_conflict(runner, git_dir, runner.write(["rebase", onto]))
}

/// What happens to one commit in an interactive rebase (FR-REWRITE-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pick,
    /// Pick, then replace the message with the one given.
    Reword,
    /// Pick, then stop so the user can change the commit.
    Edit,
    /// Join into the commit before, keeping both messages.
    Squash,
    /// Join into the commit before, keeping only its message.
    Fixup,
    Drop,
}

/// One line of the plan: a commit, what to do with it, and the new
/// message for a reword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub oid: String,
    pub step: Step,
    pub message: String,
}

/// The commits after `base` up to HEAD, oldest first, as id and summary.
/// A range with a merge commit is refused: replaying merges is not built.
pub fn rebase_commits(runner: &GitRunner, base: &str) -> Result<Vec<(String, String)>, GitError> {
    check_rev(base)?;
    // Otherwise the rebase would move the branch onto `base`, not edit
    // the branch's own commits.
    if runner
        .read(["merge-base", "--is-ancestor", base, "HEAD"])
        .is_err()
    {
        return Err(refused(
            "This commit is not in the current branch's history.",
        ));
    }
    let range = format!("{base}..HEAD");
    let merges = runner.read(["rev-list", "--merges", "-n1", &range])?;
    if !merges.is_empty() {
        return Err(refused(
            "These commits include a merge, which this rebase cannot replay.",
        ));
    }
    let out = runner.read(["log", "--reverse", "--format=%H%x1f%s", "-z", &range])?;
    Ok(out
        .split(|&byte| byte == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (oid, summary) = entry.split_once('\x1f')?;
            Some((oid.trim().to_owned(), summary.to_owned()))
        })
        .collect())
}

fn refused(text: &str) -> GitError {
    GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: text.into(),
    }
}

/// A path as one word for `sh`, which Git uses to run the sequence editor
/// and `exec` lines, also on Windows.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// The todo list Git will follow. A reword is a pick followed by an
/// `exec` that amends the message from a file, so no editor is needed.
fn todo_text(items: &[TodoItem], message_file: &dyn Fn(usize) -> std::path::PathBuf) -> String {
    let mut todo = String::new();
    for (ix, item) in items.iter().enumerate() {
        let word = match item.step {
            Step::Pick | Step::Reword => "pick",
            Step::Edit => "edit",
            Step::Squash => "squash",
            Step::Fixup => "fixup",
            Step::Drop => "drop",
        };
        todo.push_str(&format!("{word} {}\n", item.oid));
        if item.step == Step::Reword {
            let file = sh_quote(&message_file(ix));
            todo.push_str(&format!(
                "exec git commit --amend -q --allow-empty -F {file}\n"
            ));
        }
    }
    todo
}

/// Rebases the commits after `base` by the plan in `items`, which must
/// name exactly those commits. Git's own todo editor is replaced by a copy
/// of a todo file written here. An edit step, or a conflict, leaves the
/// rebase stopped, reported as `Conflict` so it is continued like one.
pub fn rebase_interactive(
    runner: &GitRunner,
    git_dir: &Path,
    base: &str,
    items: &[TodoItem],
) -> Result<(), GitError> {
    // Only the commits that are there, each once: nothing else reaches
    // the todo list.
    let mut listed: Vec<String> = rebase_commits(runner, base)?
        .into_iter()
        .map(|c| c.0)
        .collect();
    let mut planned: Vec<String> = items.iter().map(|item| item.oid.clone()).collect();
    listed.sort();
    planned.sort();
    if listed != planned || listed.is_empty() {
        return Err(refused(
            "The plan does not match the commits after this one any more.",
        ));
    }
    if items
        .iter()
        .find(|item| item.step != Step::Drop)
        .is_some_and(|first| matches!(first.step, Step::Squash | Step::Fixup))
    {
        return Err(refused(
            "The first commit kept cannot be squashed: there is nothing before it.",
        ));
    }
    let folder = git_dir.join("gittree");
    let message_file = |ix: usize| folder.join(format!("reword-{ix}"));
    let write = |path: &Path, text: &str| {
        std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(path, text))
            .map_err(|error| refused(&format!("Could not write the rebase plan: {error}")))
    };
    for (ix, item) in items.iter().enumerate() {
        if item.step == Step::Reword {
            if item.message.trim().is_empty() {
                return Err(refused("A reworded commit needs a message."));
            }
            write(&message_file(ix), &item.message)?;
        }
    }
    let todo = folder.join("rebase-todo");
    write(&todo, &todo_text(items, &message_file))?;
    let editor = format!("sequence.editor=cp {}", sh_quote(&todo));
    let result = runner.write(["-c", &editor, "rebase", "-i", "--no-autosquash", base]);
    or_conflict(runner, git_dir, result)?;
    // An edit step stops the rebase with a clean exit.
    if in_progress(runner, git_dir).is_some() {
        return Err(GitError {
            kind: GitErrorKind::Conflict,
            ..refused("Stopped to edit a commit.")
        });
    }
    Ok(())
}

/// Skips the commit a rebase stopped on.
pub fn rebase_skip(runner: &GitRunner, git_dir: &Path) -> Result<(), GitError> {
    or_conflict(runner, git_dir, runner.write(["rebase", "--skip"]))
}

fn pick(
    runner: &GitRunner,
    git_dir: &Path,
    verb: &str,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    check_rev(oid)?;
    let parent = mainline.map(|n| n.to_string());
    let mut args = vec![verb, "--no-edit"];
    if let Some(parent) = &parent {
        args.extend(["-m", parent]);
    }
    args.push(oid);
    or_conflict(runner, git_dir, runner.write(args))
}

/// Copies a commit onto HEAD. A merge commit needs `mainline`: which parent
/// to diff against, counted from 1.
pub fn cherry_pick(
    runner: &GitRunner,
    git_dir: &Path,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    pick(runner, git_dir, "cherry-pick", oid, mainline)
}

pub fn revert(
    runner: &GitRunner,
    git_dir: &Path,
    oid: &str,
    mainline: Option<u32>,
) -> Result<(), GitError> {
    pick(runner, git_dir, "revert", oid, mainline)
}

pub fn reset(runner: &GitRunner, target: &str, mode: ResetMode) -> Result<(), GitError> {
    check_rev(target)?;
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    runner.write(["reset", "-q", flag, target]).map(drop)
}

/// The operation that is stopped part way, read from the Git folder.
pub fn in_progress(runner: &GitRunner, git_dir: &Path) -> Option<InProgressOp> {
    let read = |name: &str| std::fs::read_to_string(git_dir.join(name)).ok();
    let number = |name: &str| read(name)?.trim().parse::<u32>().ok();
    let short = |text: String| {
        let text = text.trim();
        text.strip_prefix("refs/heads/").unwrap_or(text).to_owned()
    };
    let (kind, step, onto) = if git_dir.join("rebase-merge").is_dir() {
        (
            OpKind::Rebase,
            number("rebase-merge/msgnum").zip(number("rebase-merge/end")),
            read("rebase-merge/onto").map(|oid| oid.trim().chars().take(7).collect()),
        )
    } else if git_dir.join("rebase-apply").is_dir() {
        (
            OpKind::Rebase,
            number("rebase-apply/next").zip(number("rebase-apply/last")),
            None,
        )
    } else if git_dir.join("MERGE_HEAD").exists() {
        // "Merge branch 'x'" names what is coming in.
        let name = read("MERGE_MSG").and_then(|message| {
            let line = message.lines().next()?.to_owned();
            Some(line.split('\'').nth(1)?.to_owned())
        });
        (OpKind::Merge, None, name.map(short))
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        (OpKind::CherryPick, None, None)
    } else if git_dir.join("REVERT_HEAD").exists() {
        (OpKind::Revert, None, None)
    } else {
        return None;
    };
    let conflicted = read::status(runner)
        .map(|entries| entries.iter().filter(|e| e.conflict.is_some()).count() as u32)
        .unwrap_or(0);
    Some(InProgressOp {
        kind,
        step,
        onto,
        conflicted,
    })
}

/// Finishes the operation once every conflict is resolved.
pub fn continue_op(runner: &GitRunner, git_dir: &Path, kind: OpKind) -> Result<(), GitError> {
    let args: &[&str] = match kind {
        OpKind::Merge => &["commit", "-q", "--no-edit"],
        OpKind::Rebase => &["rebase", "--continue"],
        OpKind::CherryPick => &["cherry-pick", "--continue"],
        OpKind::Revert => &["revert", "--continue"],
    };
    or_conflict(runner, git_dir, runner.write(args))
}

/// Gives the operation up and puts the repository back as it was.
pub fn abort_op(runner: &GitRunner, kind: OpKind) -> Result<(), GitError> {
    let verb = match kind {
        OpKind::Merge => "merge",
        OpKind::Rebase => "rebase",
        OpKind::CherryPick => "cherry-pick",
        OpKind::Revert => "revert",
    };
    runner.write([verb, "--abort"]).map(drop)
}

fn add(runner: &GitRunner, path: &RepoPath) -> Result<(), GitError> {
    runner
        .write(["add".as_ref(), "--".as_ref(), &*path.as_os_str()])
        .map(drop)
}

/// Writes the resolved content of a conflicted file and stages it.
pub fn resolve_file(runner: &GitRunner, path: &RepoPath, content: &[u8]) -> Result<(), GitError> {
    if !runner.trusted() {
        return Err(GitError::untrusted());
    }
    std::fs::write(runner.dir().join(path.as_os_str()), content).map_err(|error| GitError {
        kind: GitErrorKind::Other,
        args: Vec::new(),
        exit_code: None,
        stderr: error.to_string(),
    })?;
    add(runner, path)
}

/// Takes one whole side of a conflicted file and stages it. For binary
/// files and for files one side deleted.
pub fn resolve_side(runner: &GitRunner, path: &RepoPath, side: Side) -> Result<(), GitError> {
    let flag = match side {
        Side::Ours => "--ours",
        Side::Theirs => "--theirs",
    };
    let taken = runner.write([
        "checkout".as_ref(),
        flag.as_ref(),
        "--".as_ref(),
        &*path.as_os_str(),
    ]);
    match taken {
        Ok(_) => add(runner, path),
        // That side deleted the file, so resolving to it removes the file.
        Err(_) => runner
            .write([
                "rm".as_ref(),
                "-q".as_ref(),
                "--".as_ref(),
                &*path.as_os_str(),
            ])
            .map(drop),
    }
}
