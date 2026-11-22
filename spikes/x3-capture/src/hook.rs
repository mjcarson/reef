//! The triggers that capture a node: Claude Code's hooks and git's
//!
//! Each runs the same capture in the node's own local checkout; they differ only in when they
//! fire and how they know which node it is.
//!
//! - **PostToolUse** on reef's `status` tool, when it marks a node done: the tool's input names
//!   the node.
//! - **Stop**, at the end of Claude's turn: every node done in this worktree whose done mark
//!   isn't sealed yet, once each. It is the only trigger that seals a revision.
//! - **post-commit**, after each commit: the node this worktree is working on, the last one
//!   started there. Skipped mid-rebase, mid-merge or mid-cherry-pick, where HEAD is half-built.
//! - **post-rewrite**, after an amend or a rebase, a local squash included: the same node.
//!
//! Claude Code's WorktreeCreate and WorktreeRemove hooks are here too. With them, `claude -w
//! <node>` asks reef for the node's worktree: reef makes it and starts the node in it, then
//! takes a last draft before it goes.

use serde_json::{Value, json};
use std::path::Path;
use tracing::instrument;

use crate::capture::{self, Options};
use crate::store::{Status, Store};

/// Runs one hook, returning what it prints on stdout, if anything
///
/// # Arguments
///
/// * `store` - The state dir
/// * `event` - The hook's event: PostToolUse, Stop, post-commit or post-rewrite
/// * `args` - The hook's arguments, as git passes post-rewrite its kind
/// * `input` - What the hook was given on stdin
/// * `cwd` - The directory the hook runs in
#[instrument(name = "hook::run", skip(store, args, input, cwd), err(Debug))]
pub fn run(
    store: &Store,
    event: &str,
    args: &[String],
    input: &[u8],
    cwd: &Path,
) -> anyhow::Result<Option<String>> {
    match event {
        // Claude Code hands its hooks JSON
        "PostToolUse" => post_tool_use(store, &serde_json::from_slice(input)?).map(|_| None),
        "Stop" => stop(store, &serde_json::from_slice(input)?).map(|_| None),
        "WorktreeCreate" => worktree_create(store, &serde_json::from_slice(input)?).map(Some),
        "WorktreeRemove" => worktree_remove(store, &serde_json::from_slice(input)?).map(|_| None),
        // git hooks run in the worktree's root, with no node of their own
        "post-commit" => git_hook(store, "post-commit", cwd, true).map(|_| None),
        "post-rewrite" => {
            // amend or rebase, then one `old new` line per rewritten commit
            let kind = args.first().map(String::as_str).unwrap_or("unknown");
            let rewritten = String::from_utf8_lossy(input).lines().count();
            store.log(
                "post-rewrite",
                json!({ "kind": kind, "rewritten": rewritten }),
            )?;
            git_hook(store, &format!("post-rewrite:{kind}"), cwd, false).map(|_| None)
        }
        // anything else is only logged
        other => {
            store.log(other, json!({ "args": args }))?;
            Ok(None)
        }
    }
}

/// Makes the worktree `claude -w <node>` asked for, and starts the node in it
///
/// The worktree goes where Claude Code would put its own, `.claude/worktrees/<name>` under the
/// repo's root, on branch `reef/<name>` from the session's HEAD. Its path is the answer.
///
/// # Arguments
///
/// * `store` - The state dir
/// * `input` - The hook's input
fn worktree_create(store: &Store, input: &Value) -> anyhow::Result<String> {
    // the worktree's name is the node's id
    let name = input["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("WorktreeCreate gave no name"))?;
    let cwd = input["cwd"].as_str().unwrap_or(".");
    let git = store.git(cwd);
    let root = git.toplevel()?;
    let path = root.join(".claude/worktrees").join(name);
    let path_str = path.to_string_lossy().into_owned();
    let branch = format!("reef/{name}");
    // a new worktree on a new branch, from where the session is
    git.at(&root)
        .run(&["worktree", "add", "-q", "-b", &branch, &path_str, "HEAD"])?;
    let work = capture::start(store, &store.git(&path), name, Some("main"), &[])?;
    store.log(
        "WorktreeCreate",
        json!({ "name": name, "session": input["session_id"], "worktree": work.worktree }),
    )?;
    Ok(work.worktree.to_string_lossy().into_owned())
}

/// Takes a last draft of the node a worktree holds, then removes the worktree, keeping its branch
///
/// # Arguments
///
/// * `store` - The state dir
/// * `input` - The hook's input
fn worktree_remove(store: &Store, input: &Value) -> anyhow::Result<()> {
    let path = input["worktree_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("WorktreeRemove gave no path"))?;
    let root = capture::worktree_of(store, Path::new(path))?;
    let node = store.load()?.current(&root).map(|work| work.node.clone());
    store.log("WorktreeRemove", json!({ "worktree": root, "node": node }))?;
    // whatever it holds is captured before it goes
    if let Some(node) = &node {
        capture_logged(store, node, "WorktreeRemove")?;
    }
    store
        .git(&root)
        .run(&["worktree", "remove", "--force", path])?;
    Ok(())
}

/// Captures the node reef's status tool just marked done
///
/// # Arguments
///
/// * `store` - The state dir
/// * `input` - The hook's input
fn post_tool_use(store: &Store, input: &Value) -> anyhow::Result<()> {
    // only reef's status tool, setting done, says a node is finished
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let status = input["tool_input"]["status"].as_str().unwrap_or_default();
    let node = input["tool_input"]["id"].as_str().unwrap_or_default();
    store.log(
        "PostToolUse",
        json!({
            "tool": tool,
            "tool_input": input["tool_input"],
            "session": input["session_id"],
            "cwd": input["cwd"],
        }),
    )?;
    if !tool.ends_with("__status") || status != "done" || node.is_empty() {
        return Ok(());
    }
    capture_logged(store, node, "PostToolUse")
}

/// Captures every node done in this worktree since its last capture
///
/// # Arguments
///
/// * `store` - The state dir
/// * `input` - The hook's input
fn stop(store: &Store, input: &Value) -> anyhow::Result<()> {
    // the session's worktree, from the directory Claude Code ran the hook in
    let cwd = input["cwd"].as_str().unwrap_or(".");
    let root = capture::worktree_of(store, Path::new(cwd))?;
    let state = store.load()?;
    // done here, and its latest done mark not sealed yet
    let pending: Vec<String> = state
        .work
        .values()
        .filter(|work| work.worktree == root)
        .filter(|work| {
            state
                .nodes
                .get(&work.node)
                .is_some_and(|node| node.status == Status::Done && node.sealed_for != node.done_at)
        })
        .map(|work| work.node.clone())
        .collect();
    store.log(
        "Stop",
        json!({
            "session": input["session_id"],
            "stop_hook_active": input["stop_hook_active"],
            "worktree": root,
            "pending": pending,
        }),
    )?;
    for node in pending {
        capture_logged(store, &node, "Stop")?;
    }
    Ok(())
}

/// Captures the node a worktree is working on, from a git hook
///
/// # Arguments
///
/// * `store` - The state dir
/// * `trigger` - Which hook this is
/// * `cwd` - The worktree's root, where git runs its hooks
/// * `skip_mid_operation` - Whether to do nothing while a rebase or merge is under way
fn git_hook(
    store: &Store,
    trigger: &str,
    cwd: &Path,
    skip_mid_operation: bool,
) -> anyhow::Result<()> {
    let git = store.git(cwd);
    let root = git.toplevel()?;
    let git = git.at(&root);
    // a rebase's picks each commit, but HEAD only holds part of the branch until it ends
    let mid = git.mid_operation()?;
    let state = store.load()?;
    let current = state.current(&root).map(|work| work.node.clone());
    store.log(
        trigger,
        json!({
            "worktree": root,
            "node": current,
            "mid_operation": mid,
            "head": git.commit("HEAD")?,
        }),
    )?;
    if mid && skip_mid_operation {
        return Ok(());
    }
    match current {
        Some(node) => capture_logged(store, &node, trigger),
        None => Ok(()),
    }
}

/// Captures a node, logging a failure rather than failing the hook
///
/// A hook that fails would interrupt Claude or the user's commit over something only reef
/// cares about, so a failed capture is recorded and the hook still succeeds.
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
/// * `trigger` - What started it
fn capture_logged(store: &Store, node: &str, trigger: &str) -> anyhow::Result<()> {
    if let Err(error) = capture::capture(store, node, &Options::chosen(trigger)) {
        store.log(
            "capture-failed",
            json!({ "node": node, "trigger": trigger, "error": format!("{error:#}") }),
        )?;
    }
    Ok(())
}
