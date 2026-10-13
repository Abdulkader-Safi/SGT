//! Pull requests and issues through `gh`, the GitHub command the user has
//! installed and signed in to (FR-AGENT-06).
//!
//! The app holds no token and talks to no server itself. `gh` is started
//! with an argument vector and no shell, only in a trusted repository,
//! because it runs Git with the repository's own config.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// What the checks on a pull request say, taken together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checks {
    None,
    Running,
    Pass,
    /// How many failed.
    Failing(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub draft: bool,
    /// "APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED" or empty.
    pub review: String,
    pub additions: u64,
    pub deletions: u64,
    /// As GitHub gives it: `2026-10-13T09:30:00Z`.
    pub updated: String,
    pub url: String,
    pub branch: String,
    pub checks: Checks,
}

impl PullRequest {
    /// The author has something to fix before reviewers are needed.
    pub fn needs_work(&self) -> bool {
        self.review == "CHANGES_REQUESTED" || matches!(self.checks, Checks::Failing(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub updated: String,
    pub url: String,
}

/// Everything the Launchpad shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launchpad {
    pub login: String,
    /// The branch pull requests go into by default.
    pub default_branch: String,
    /// Open pull requests waiting on this user's review.
    pub to_review: Vec<PullRequest>,
    /// This user's open pull requests.
    pub mine: Vec<PullRequest>,
    /// Open issues assigned to this user.
    pub issues: Vec<Issue>,
}

const PR_FIELDS: &str = "number,title,author,isDraft,reviewDecision,additions,deletions,\
updatedAt,url,headRefName,statusCheckRollup";

/// `gh`, run in one repository.
#[derive(Debug, Clone)]
pub struct Gh {
    program: PathBuf,
    dir: PathBuf,
    trusted: bool,
}

impl Gh {
    /// `program` is `None` for the `gh` a terminal would find.
    pub fn new(program: Option<PathBuf>, dir: &Path, trusted: bool) -> Self {
        Self {
            program: program.unwrap_or_else(|| crate::agent::find_program("gh")),
            dir: dir.to_owned(),
            trusted,
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        if !self.trusted {
            return Err("Trust this repository to use its pull requests and issues.".into());
        }
        let out = Command::new(&self.program)
            .args(args)
            .current_dir(&self.dir)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                format!("Could not start gh: {error}. Is the GitHub CLI installed?")
            })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            let said = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            Err(if said.is_empty() {
                "gh ended with an error.".into()
            } else {
                said
            })
        }
    }

    /// Reads the user, the default branch, and the open pull requests and
    /// issues that concern the user.
    pub fn launchpad(&self) -> Result<Launchpad, String> {
        let login = self.run(&["api", "user", "--jq", ".login"])?;
        let default_branch = self.run(&[
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "--jq",
            ".defaultBranchRef.name",
        ])?;
        let prs = |filter: &[&str]| -> Result<Vec<PullRequest>, String> {
            let mut args = vec!["pr", "list", "--state", "open", "--json", PR_FIELDS];
            args.extend_from_slice(filter);
            Ok(parse_prs(&self.run(&args)?))
        };
        let issues = self.run(&[
            "issue",
            "list",
            "--state",
            "open",
            "--assignee",
            "@me",
            "--json",
            "number,title,author,updatedAt,url",
        ])?;
        Ok(Launchpad {
            login,
            default_branch,
            to_review: prs(&["--search", "review-requested:@me"])?,
            mine: prs(&["--author", "@me"])?,
            issues: parse_issues(&issues),
        })
    }

    /// Opens a pull request from `head` into `base` and returns its
    /// address. The branch must already be on the remote.
    pub fn create_pull_request(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String, String> {
        if title.trim().is_empty() {
            return Err("A pull request needs a title.".into());
        }
        // Each value rides on its flag, so none can be read as an option.
        let (head, base) = (format!("--head={head}"), format!("--base={base}"));
        let (title, body) = (
            format!("--title={}", title.trim()),
            format!("--body={body}"),
        );
        let mut args = vec!["pr", "create", &head, &base, &title, &body];
        if draft {
            args.push("--draft");
        }
        let said = self.run(&args)?;
        // gh prints the address last.
        Ok(said.lines().last().unwrap_or_default().trim().to_owned())
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn author(value: &Value) -> String {
    value["author"]["login"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Checks come as check runs (`status`, `conclusion`) and as commit
/// statuses (`state`).
fn checks(rollup: &Value) -> Checks {
    let Some(list) = rollup.as_array().filter(|list| !list.is_empty()) else {
        return Checks::None;
    };
    let word = |check: &Value, key: &str| check[key].as_str().unwrap_or_default().to_uppercase();
    let failed = list
        .iter()
        .filter(|check| {
            let (conclusion, state) = (word(check, "conclusion"), word(check, "state"));
            [
                "FAILURE",
                "TIMED_OUT",
                "CANCELLED",
                "ACTION_REQUIRED",
                "STARTUP_FAILURE",
            ]
            .contains(&conclusion.as_str())
                || ["FAILURE", "ERROR"].contains(&state.as_str())
        })
        .count();
    let running = list.iter().any(|check| {
        let (status, state) = (word(check, "status"), word(check, "state"));
        (!status.is_empty() && status != "COMPLETED")
            || ["PENDING", "EXPECTED"].contains(&state.as_str())
    });
    match (failed, running) {
        (0, false) => Checks::Pass,
        (0, true) => Checks::Running,
        (failed, _) => Checks::Failing(failed),
    }
}

fn parse_prs(json: &str) -> Vec<PullRequest> {
    let list: Vec<Value> = serde_json::from_str(json).unwrap_or_default();
    list.iter()
        .map(|pr| PullRequest {
            number: pr["number"].as_u64().unwrap_or(0),
            title: text(pr, "title"),
            author: author(pr),
            draft: pr["isDraft"].as_bool().unwrap_or(false),
            review: text(pr, "reviewDecision"),
            additions: pr["additions"].as_u64().unwrap_or(0),
            deletions: pr["deletions"].as_u64().unwrap_or(0),
            updated: text(pr, "updatedAt"),
            url: text(pr, "url"),
            branch: text(pr, "headRefName"),
            checks: checks(&pr["statusCheckRollup"]),
        })
        .collect()
}

fn parse_issues(json: &str) -> Vec<Issue> {
    let list: Vec<Value> = serde_json::from_str(json).unwrap_or_default();
    list.iter()
        .map(|issue| Issue {
            number: issue["number"].as_u64().unwrap_or(0),
            title: text(issue, "title"),
            author: author(issue),
            updated: text(issue, "updatedAt"),
            url: text(issue, "url"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pull_requests_and_sums_up_their_checks() {
        let json = r#"[
          {"number":7,"title":"Add keys","author":{"login":"ada"},"isDraft":false,
           "reviewDecision":"CHANGES_REQUESTED","additions":4,"deletions":1,
           "updatedAt":"2026-10-13T09:30:00Z","url":"https://example.com/7","headRefName":"keys",
           "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"},
                                {"status":"COMPLETED","conclusion":"FAILURE"},
                                {"state":"ERROR"}]},
          {"number":8,"title":"Draft","author":{"login":"ada"},"isDraft":true,"reviewDecision":"",
           "statusCheckRollup":[{"status":"IN_PROGRESS","conclusion":""}]},
          {"number":9,"title":"Green","author":null,
           "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"},{"state":"SUCCESS"}]},
          {"number":10,"title":"Bare","statusCheckRollup":[]}
        ]"#;
        let prs = parse_prs(json);
        let brief: Vec<_> = prs
            .iter()
            .map(|pr| (pr.number, pr.checks, pr.needs_work()))
            .collect();
        assert_eq!(
            brief,
            [
                (7, Checks::Failing(2), true),
                (8, Checks::Running, false),
                (9, Checks::Pass, false),
                (10, Checks::None, false),
            ]
        );
        assert_eq!(
            (
                prs[0].author.as_str(),
                prs[0].additions,
                prs[0].branch.as_str()
            ),
            ("ada", 4, "keys")
        );
        assert!(prs[1].draft && prs[2].author.is_empty());
        // Anything that is not the expected JSON gives an empty list.
        assert!(parse_prs("not json").is_empty() && parse_issues("{}").is_empty());
    }

    #[test]
    fn an_untrusted_repository_never_starts_gh() {
        let gh = Gh::new(Some("/no/such/gh".into()), Path::new("."), false);
        assert!(
            gh.launchpad()
                .unwrap_err()
                .starts_with("Trust this repository")
        );
        let trusted = Gh::new(Some("/no/such/gh".into()), Path::new("."), true);
        assert!(
            trusted
                .launchpad()
                .unwrap_err()
                .starts_with("Could not start gh")
        );
    }

    /// A stand-in `gh` that answers by its first two arguments and writes
    /// down what it was asked to create.
    #[cfg(unix)]
    #[test]
    fn reads_the_launchpad_and_creates_a_pull_request_through_gh() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("gh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
case "$1 $2" in
  "api user") echo ada ;;
  "repo view") echo main ;;
  "pr list") echo '[{"number":3,"title":"One","author":{"login":"grace"},"statusCheckRollup":[]}]' ;;
  "issue list") echo '[{"number":5,"title":"Bug","author":{"login":"ada"},"url":"u"}]' ;;
  "pr create") printf '%s\n' "$@" > asked.txt; echo "Creating pull request"; echo "https://example.com/pull/4" ;;
  *) echo "unexpected: $*" >&2; exit 1 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gh = Gh::new(Some(script), dir.path(), true);

        let found = gh.launchpad().unwrap();
        assert_eq!(
            (found.login.as_str(), found.default_branch.as_str()),
            ("ada", "main")
        );
        assert_eq!(
            (
                found.to_review[0].number,
                found.mine.len(),
                found.issues[0].number
            ),
            (3, 1, 5)
        );

        let url = gh
            .create_pull_request("keys", "main", " -Add keys ", "Body\ntwo", true)
            .unwrap();
        assert_eq!(url, "https://example.com/pull/4");
        let asked = std::fs::read_to_string(dir.path().join("asked.txt")).unwrap();
        assert_eq!(
            asked,
            "pr\ncreate\n--head=keys\n--base=main\n--title=-Add keys\n--body=Body\ntwo\n--draft\n"
        );
        assert!(
            gh.create_pull_request("keys", "main", "  ", "", false)
                .is_err()
        );
    }
}
