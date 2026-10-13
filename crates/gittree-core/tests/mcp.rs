//! The MCP server against real Git.

mod common;

use common::TestRepo;
use gittree_core::journal::Journal;
use gittree_core::mcp::Server;
use gittree_core::store::McpAccess;
use serde_json::{Value, json};

fn server(repo: &TestRepo, trusted: bool, access: McpAccess) -> Server {
    Server::new(repo.runner(trusted), &repo.path().join(".git"), access)
}

/// Calls a tool and returns its text and whether it failed.
fn call(server: &Server, name: &str, arguments: Value) -> (String, bool) {
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments } });
    let reply = server.handle(&request).unwrap();
    let result = &reply["result"];
    (
        result["content"][0]["text"].as_str().unwrap().to_owned(),
        result["isError"].as_bool().unwrap(),
    )
}

fn repo_with_changes() -> TestRepo {
    let repo = TestRepo::new();
    repo.git(1, &["config", "user.name", "Ada"]);
    repo.git(1, &["config", "user.email", "ada@example.com"]);
    repo.write("a.txt", b"one\ntwo\n");
    repo.commit_all("base");
    repo.write("a.txt", b"one\nTWO\n");
    repo.write("new.txt", b"new\n");
    repo
}

#[test]
fn speaks_the_protocol() {
    let repo = repo_with_changes();
    let server = server(&repo, true, McpAccess::ReadOnly);
    let hello = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26" } }))
        .unwrap();
    assert_eq!(hello["result"]["protocolVersion"], "2025-03-26");
    assert!(hello["result"]["capabilities"]["tools"].is_object());

    // A notification gets no answer.
    assert!(
        server
            .handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none()
    );
    let unknown = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }))
        .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    let list = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }))
        .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "git_status",
            "git_log_or_diff",
            "git_blame",
            "git_branch",
            "git_checkout",
            "git_stash",
            "git_worktree",
            "git_add_or_commit"
        ]
    );

    // Over stdio: one line in, one line out.
    let mut out = Vec::new();
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\nnot json\n";
    server.serve(&input[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["id"], 7);
    assert_eq!(lines[1]["error"]["code"], -32700);
}

#[test]
fn reads_status_log_diff_blame_and_branches() {
    let repo = repo_with_changes();
    // Reading works in an untrusted repository too.
    let server = server(&repo, false, McpAccess::ReadOnly);

    let (status, failed) = call(&server, "git_status", json!({}));
    assert!(!failed);
    assert!(status.starts_with("On branch main"));
    assert!(status.contains("modified: a.txt") && status.contains("untracked: new.txt"));

    let (log, _) = call(&server, "git_log_or_diff", json!({ "action": "log" }));
    assert!(log.contains("\tAda\t") && log.trim_end().ends_with("base"));
    let (diff, _) = call(
        &server,
        "git_log_or_diff",
        json!({ "action": "diff", "path": "a.txt" }),
    );
    assert!(diff.contains("-two") && diff.contains("+TWO"));
    let (blame, _) = call(
        &server,
        "git_blame",
        json!({ "path": "a.txt", "start_line": 1, "end_line": 1 }),
    );
    assert!(blame.contains("one") && !blame.contains("TWO"));
    let (branches, _) = call(&server, "git_branch", json!({}));
    assert!(branches.starts_with("* main "));
}

#[test]
fn an_argument_cannot_become_a_git_option() {
    let repo = repo_with_changes();
    let server = server(&repo, true, McpAccess::ReadWrite);
    let (text, failed) = call(
        &server,
        "git_log_or_diff",
        json!({ "action": "log", "revision": "--output=/tmp/sgt-mcp-test" }),
    );
    assert!(failed && text.contains("dash"));
    // A path that looks like an option is only ever a path.
    let (_, failed) = call(
        &server,
        "git_log_or_diff",
        json!({ "action": "diff", "path": "--output=/tmp/sgt-mcp-test" }),
    );
    assert!(!failed);
    assert!(!std::path::Path::new("/tmp/sgt-mcp-test").exists());
}

#[test]
fn changes_need_trust_and_the_access_setting() {
    let repo = repo_with_changes();
    let add = json!({ "action": "add", "paths": ["a.txt"] });
    for (trusted, access) in [(false, McpAccess::ReadWrite), (true, McpAccess::ReadOnly)] {
        let (_, failed) = call(
            &server(&repo, trusted, access),
            "git_add_or_commit",
            add.clone(),
        );
        assert!(failed, "trusted {trusted}, {access:?}");
    }
    assert_eq!(
        repo.git(1, &["status", "--porcelain"]),
        " M a.txt\n?? new.txt"
    );

    let server = server(&repo, true, McpAccess::ReadWrite);
    assert!(!call(&server, "git_add_or_commit", add).1);
    let (text, failed) = call(
        &server,
        "git_add_or_commit",
        json!({ "action": "commit", "message": "Shout the second line" }),
    );
    assert!(!failed && text.starts_with("Committed "));
    assert_eq!(
        repo.git(1, &["log", "-1", "--format=%s"]),
        "Shout the second line"
    );

    // The app can undo what the agent did.
    let journal = Journal::new(&repo.path().join(".git"));
    let entries = journal.entries();
    assert_eq!(entries.len(), 1);
    journal.undo(&repo.runner(true), &entries[0]).unwrap();
    assert_eq!(repo.git(1, &["log", "-1", "--format=%s"]), "base");
    assert_eq!(
        repo.git(1, &["status", "--porcelain"]),
        "M  a.txt\n?? new.txt"
    );
}

#[test]
fn branches_checkout_stash_and_worktrees_are_journaled_and_never_forced() {
    let repo = repo_with_changes();
    let journal = Journal::new(&repo.path().join(".git"));
    let server = server(&repo, true, McpAccess::ReadWrite);
    let ok = |name: &str, arguments: Value| {
        let (text, failed) = call(&server, name, arguments);
        assert!(!failed, "{name}: {text}");
        text
    };
    let refused = |name: &str, arguments: Value| {
        let (text, failed) = call(&server, name, arguments);
        assert!(failed, "{name} should be refused: {text}");
        text
    };

    // The changes are stashed, untracked file included, then brought back.
    ok("git_stash", json!({ "action": "push", "message": "wip" }));
    assert_eq!(repo.git(1, &["status", "--porcelain"]), "");
    assert!(ok("git_stash", json!({})).contains("wip"));
    refused("git_stash", json!({ "action": "drop" }));

    ok("git_branch", json!({ "action": "create", "name": "topic" }));
    ok("git_checkout", json!({ "branch": "topic" }));
    assert_eq!(
        repo.git(1, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "topic"
    );
    refused("git_checkout", json!({ "branch": "--detach" }));
    refused("git_branch", json!({ "action": "create", "name": "-x" }));

    // A branch with commits of its own is not deleted.
    repo.write("b.txt", b"b\n");
    repo.commit_all("on topic");
    ok("git_checkout", json!({ "branch": "main" }));
    refused("git_branch", json!({ "action": "delete", "name": "topic" }));
    assert!(ok("git_branch", json!({ "action": "list" })).contains("topic"));

    ok("git_stash", json!({ "action": "pop" }));
    assert!(repo.git(1, &["status", "--porcelain"]).contains("new.txt"));

    // Each change the app can undo has a journal entry.
    let labels: Vec<String> = journal.entries().iter().map(|e| e.label.clone()).collect();
    assert_eq!(
        labels,
        [
            "Stash changes (by an agent)",
            "Create branch topic (by an agent)",
            "Checkout topic (by an agent)",
            "Checkout main (by an agent)",
        ]
    );

    let outside = tempfile::tempdir().unwrap();
    let path = gittree_core::ops::setup::canonical(outside.path()).join("wt");
    let path_text = path.to_str().unwrap();
    ok(
        "git_worktree",
        json!({ "action": "add", "path": path_text, "branch": "agent/x" }),
    );
    assert!(ok("git_worktree", json!({})).contains("agent/x"));
    std::fs::write(path.join("work.txt"), "work").unwrap();
    refused(
        "git_worktree",
        json!({ "action": "remove", "path": path_text }),
    );
    assert!(path.join("work.txt").exists());
    refused(
        "git_worktree",
        json!({ "action": "add", "path": "relative", "branch": "y" }),
    );

    // Read only: listing works, every change is refused.
    let reader = self::server(&repo, true, McpAccess::ReadOnly);
    for (name, arguments) in [
        ("git_branch", json!({ "action": "create", "name": "no" })),
        ("git_checkout", json!({ "branch": "topic" })),
        ("git_stash", json!({ "action": "push" })),
        (
            "git_worktree",
            json!({ "action": "remove", "path": path_text }),
        ),
    ] {
        assert!(call(&reader, name, arguments).1, "{name} must be refused");
    }
    assert!(!call(&reader, "git_stash", json!({})).1);
}
