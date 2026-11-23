//! reef's hooks for git and Claude Code: a node's draft kept current while it is worked on, and
//! sealed once it is done
//!
//! ```text
//! reef hook post-commit          # git: draft the node this worktree is on, unless mid-rebase
//! reef hook post-rewrite <KIND>  # git: draft it once a rebase ends; an amend is post-commit's
//! reef hook post-tool-use        # Claude Code, after reef's status tool: draft a node marked done
//! reef hook stop [--pane KEY]    # Claude Code: seal the node done here, and end the loop's node
//! ```
//!
//! - **git's hooks keep the draft current.** post-commit drafts the node the worktree is on
//!   after each commit, an amend's included, in whichever worktree it runs, since git runs the
//!   repo's common hooks in every one. A rebase runs post-commit for each pick with HEAD half
//!   built, so it skips mid-rebase, and post-rewrite drafts once the rebase ends.
//! - **So does marking a node done.** Claude marks a node done before it commits, so PostToolUse
//!   on reef's status tool drafts the node it marked done, uncommitted work and all, when the
//!   worktree is on it.
//! - **Stop seals.** Only Stop sees everything after the mark, such as the plan's export and its
//!   commit, so once the node the worktree is on is done, Stop seals its draft, or takes its
//!   first revision if it has none. A round of fixes after that leaves a draft of its own, which
//!   the next Stop seals as a revision of its own.
//! - **Stop ends the loop's node once.** Once the node the pane's loop holds is done, Stop acts
//!   on its end once ([`task::ended`]), telling the user what is next, that the loop is paused,
//!   or that nothing else is ready, as a `systemMessage`, since Claude Code shows the user a
//!   Stop's output only as JSON. Mid-node Claude is waiting on the user, so Stop says nothing.
//!   Stop never blocks, since Claude Code shows a block as a problem.
//! - **Nothing to do is no failure.** A directory outside git or outside every project, a
//!   worktree no node was started in, or a node marked something other than done: the hook says
//!   nothing and succeeds. A capture that fails, or a service that doesn't answer, is an error,
//!   which git ignores and Claude Code shows without stopping.
//! - **Each line it prints starts `reef:`**, since git shows it among a commit's own output.

use reef_api::work::on_worktree;
use reef_api::{Pane, WorkRecord};
use reef_model::Status;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::capture::{Keep, Report, capture_record, seal_if_done};
use crate::client::Client;
use crate::git::Checkout;
use crate::task;
use crate::work::parse_status;

/// What Claude Code hands PostToolUse: the tool that ran, what it was called with, and where
#[derive(Debug, Deserialize)]
struct ToolUsed {
    /// The session's working directory
    #[serde(default)]
    cwd: Option<PathBuf>,
    /// The tool, such as `mcp__reef__status`
    tool_name: String,
    /// What it was called with
    #[serde(default)]
    tool_input: serde_json::Value,
}

/// What reef's status tool is called with
#[derive(Debug, Deserialize)]
struct StatusSet {
    /// The node's id
    id: String,
    /// The status it was set to
    status: String,
    /// The plan it is in, if the call named one
    #[serde(default)]
    plan: Option<String>,
}

/// What Claude Code hands Stop: where the session is
#[derive(Debug, Deserialize)]
struct Stopped {
    /// The session's working directory
    #[serde(default)]
    cwd: Option<PathBuf>,
}

/// Drafts the node the worktree is on after a commit, unless a rebase is under way
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which git runs its hooks at the worktree's root in
/// * `out` - Where to print what came of it
pub async fn post_commit<W: Write>(client: &Client, cwd: &Path, out: &mut W) -> anyhow::Result<()> {
    // a directory outside git has no node's work in it
    let Some(checkout) = open(cwd) else {
        return Ok(());
    };
    // a rebase's picks are half built, so only its end is drafted, by post-rewrite
    if checkout.rebasing() {
        return Ok(());
    }
    draft(client, &checkout, out).await
}

/// Drafts the node the worktree is on once a rebase has rewritten its commits
///
/// An amend runs post-commit too, which drafted it already, so only a rebase drafts here.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which git runs its hooks at the worktree's root in
/// * `kind` - What rewrote the commits, as git passes it: `rebase` or `amend`
/// * `out` - Where to print what came of it
pub async fn post_rewrite<W: Write>(
    client: &Client,
    cwd: &Path,
    kind: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // an amend was post-commit's
    if kind != "rebase" {
        return Ok(());
    }
    let Some(checkout) = open(cwd) else {
        return Ok(());
    };
    draft(client, &checkout, out).await
}

/// Drafts the node reef's status tool marked done, if the worktree is on it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, unless the input names the session's
/// * `input` - The event Claude Code handed the hook, as JSON
/// * `out` - Where to print what came of it
pub async fn post_tool_use<W: Write>(
    client: &Client,
    cwd: &Path,
    input: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // only a status set to done, by reef's status tool, marks a node done
    let used: ToolUsed = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("PostToolUse's input doesn't read as one: {error}"))?;
    let Some(set) = marked_done(&used) else {
        return Ok(());
    };
    let dir = used.cwd.as_deref().unwrap_or(cwd);
    let Some(checkout) = open(dir) else {
        return Ok(());
    };
    // the node marked must be the one this worktree is on, or this worktree holds none of it
    let Some(record) = on_node(client, &checkout).await? else {
        return Ok(());
    };
    let named = record.node.id == set.id
        && set
            .plan
            .as_ref()
            .is_none_or(|plan| *plan == record.node.plan.plan);
    if !named {
        return Ok(());
    }
    let mut said = Vec::new();
    capture_record(
        client,
        &checkout,
        &record,
        Keep::Draft,
        Report::Line,
        &mut said,
    )
    .await?;
    prefixed(&said, out)
}

/// Seals the node the worktree is on once it is done, its draft or its first revision if it has
/// none, then acts once on the end of the node the pane's loop holds
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, unless the input names the session's
/// * `input` - The event Claude Code handed the hook, as JSON
/// * `pane` - The pane the session runs in, if it could be found
/// * `out` - Where to print what came of it
pub async fn stop<W: Write>(
    client: &Client,
    cwd: &Path,
    input: &str,
    pane: Option<&Pane>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the session's directory, which its worktree is found from
    let stopped: Stopped = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("Stop's input doesn't read as one: {error}"))?;
    let dir = stopped.cwd.as_deref().unwrap_or(cwd);
    // sealed first, so the loop moves on only from a node whose changes are kept
    let mut said = Vec::new();
    if let Some(checkout) = open(dir)
        && let Some(record) = on_node(client, &checkout).await?
    {
        // sealed only once it is done, and only what isn't sealed yet
        let mut sealed = Vec::new();
        seal_if_done(client, &checkout, &record, Report::Line, &mut sealed).await?;
        prefixed(&sealed, &mut said)?;
    }
    // then the end of the loop's node, acted on once, wherever the session runs
    let ending = match pane {
        Some(pane) => task::ended(client, pane).await?,
        None => task::Ending::Nothing,
    };
    let Some(message) = ending.message() else {
        // nothing for the user: the seal's lines as they are
        out.write_all(&said)?;
        return Ok(());
    };
    // Claude Code shows a Stop's output to the user only when all of it is JSON
    let mut told = String::from_utf8_lossy(&said).into_owned();
    told.push_str(&format!("reef: {message}"));
    writeln!(out, "{}", serde_json::json!({ "systemMessage": told }))?;
    Ok(())
}

/// The status reef's status tool set a node to, if the tool was it and the status was done
///
/// # Arguments
///
/// * `used` - The tool that ran, and what it was called with
fn marked_done(used: &ToolUsed) -> Option<StatusSet> {
    // reef's tool, under whatever name its server was given
    if !used.tool_name.ends_with("__status") {
        return None;
    }
    // called with a node and a status, which must read as done
    let set: StatusSet = serde_json::from_value(used.tool_input.clone()).ok()?;
    (parse_status(&set.status).ok()? == Status::Done).then_some(set)
}

/// Drafts the node the worktree is on, if one was started there
///
/// # Arguments
///
/// * `client` - The service's client
/// * `checkout` - The checkout the hook runs in
/// * `out` - Where to print what came of it
async fn draft<W: Write>(client: &Client, checkout: &Checkout, out: &mut W) -> anyhow::Result<()> {
    // a worktree no node was started in has no draft to keep
    let Some(record) = on_node(client, checkout).await? else {
        return Ok(());
    };
    let mut said = Vec::new();
    capture_record(
        client,
        checkout,
        &record,
        Keep::Draft,
        Report::Line,
        &mut said,
    )
    .await?;
    prefixed(&said, out)
}

/// Opens the checkout a hook runs in, or `None` outside git or in a bare repo
///
/// # Arguments
///
/// * `dir` - The directory the hook runs in
fn open(dir: &Path) -> Option<Checkout> {
    // a hook outside a worktree has no node's work to capture
    Checkout::open(dir).ok()
}

/// The record of the node a checkout's worktree is on, or `None` where no project has its repo
/// or no node was started in it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `checkout` - The checkout the hook runs in
async fn on_node(client: &Client, checkout: &Checkout) -> anyhow::Result<Option<WorkRecord>> {
    // the project whose repo the worktree is part of, if one is registered
    let Some(project) = client.find_project(checkout.root()).await? else {
        return Ok(None);
    };
    // the node started there last, in any of the project's plans
    let records = client.work(&project.name).await?;
    Ok(on_worktree(&records, checkout.root()).cloned())
}

/// Prints what a capture said, each line marked as reef's
///
/// # Arguments
///
/// * `said` - What the capture printed
/// * `out` - Where to print it
fn prefixed<W: Write>(said: &[u8], out: &mut W) -> anyhow::Result<()> {
    // git shows a hook's lines among a commit's own, so each says whose it is
    for line in String::from_utf8_lossy(said).lines() {
        writeln!(out, "reef: {line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ToolUsed, marked_done};

    /// Reads a PostToolUse input
    ///
    /// # Arguments
    ///
    /// * `json` - The input, as Claude Code hands it
    fn used(json: serde_json::Value) -> ToolUsed {
        serde_json::from_value(json).unwrap()
    }

    /// Only reef's status tool setting a node done marks it, under any server name, with or
    /// without its plan
    #[test]
    fn only_a_status_set_done_marks_a_node() {
        // reef's status tool setting done, with a plan named and without
        let done = used(serde_json::json!({
            "session_id": "s1",
            "cwd": "/src/reef",
            "hook_event_name": "PostToolUse",
            "tool_name": "mcp__reef__status",
            "tool_input": { "id": "C56", "status": "done", "plan": "central" },
            "tool_response": [{ "type": "text", "text": "`C56` is done, at version 5\n" }],
        }));
        let set = marked_done(&done).unwrap();
        assert_eq!(
            (set.id.as_str(), set.plan.as_deref()),
            ("C56", Some("central"))
        );
        let bare = used(serde_json::json!({
            "tool_name": "mcp__plan__status",
            "tool_input": { "id": "C56", "status": "done" },
        }));
        assert_eq!(marked_done(&bare).unwrap().plan, None);
        // another status, another tool, or a call missing its node marks nothing
        let wip = used(serde_json::json!({
            "tool_name": "mcp__reef__status",
            "tool_input": { "id": "C56", "status": "in-progress" },
        }));
        assert!(marked_done(&wip).is_none());
        let accept = used(serde_json::json!({
            "tool_name": "mcp__reef__accept",
            "tool_input": { "id": "C56", "status": "done" },
        }));
        assert!(marked_done(&accept).is_none());
        let bash = used(serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "reef status C56 done" },
        }));
        assert!(marked_done(&bash).is_none());
        let nameless = used(serde_json::json!({
            "tool_name": "mcp__reef__status",
            "tool_input": { "status": "done" },
        }));
        assert!(marked_done(&nameless).is_none());
        // and a status that isn't one marks nothing either
        let odd = used(serde_json::json!({
            "tool_name": "mcp__reef__status",
            "tool_input": { "id": "C56", "status": "finished-ish" },
        }));
        assert!(marked_done(&odd).is_none());
    }
}
