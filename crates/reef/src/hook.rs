//! reef's hooks for git and Claude Code: a node's draft kept current while it is worked on, and
//! sealed once it is done
//!
//! ```text
//! reef hook post-commit          # git: draft the node this worktree is on, unless mid-rebase
//! reef hook post-rewrite <KIND>  # git: draft it once a rebase ends; an amend is post-commit's
//! reef hook post-tool-use        # Claude Code, after reef's status tool: draft a node marked done
//! reef hook stop [--pane KEY]    # Claude Code: seal the node done here, and end the loop's node
//! reef hook clear ...            # Stop's, in the background: clear the session, ask for the next
//! reef hook worktree-create      # Claude Code, for `claude -w <id>`: the node's worktree, started
//! reef hook worktree-remove      # Claude Code: a last draft, then the worktree, its branch kept
//! ```
//!
//! - **git's hooks keep the draft current.** post-commit drafts the node the worktree is on
//!   after each commit, an amend's included, in whichever worktree it runs, since git runs the
//!   repo's common hooks in every one. A rebase runs post-commit for each pick with HEAD half
//!   built, so it skips mid-rebase, and post-rewrite drafts once the rebase ends.
//! - **A merge commit is no node's work.** It brings in someone else's: what the base branch
//!   brought in is carried at the next capture, and a group's member merged back after its group
//!   is that member's. So post-commit drafts nothing for one, which would otherwise show a done
//!   node's next revision holding its group's other work.
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
//! - **Then it clears between nodes.** With another node next in a zellij or tmux pane, Stop
//!   starts the injector in the background ([`crate::clear`]), which types `/clear` and the
//!   next node's kickoff into the pane once Claude is idle and its input box empty. Anywhere
//!   else, or if the injector can't start, the user types `/clear` and `next`.
//! - **A group's last Stop clears the loop's pane.** For a loop wider than one node, each
//!   node's Stop, the loop's own or a member's, says what the group still waits on, and the one
//!   that finds it all done ends the group. It clears the loop's own pane, with the session and
//!   Claude Code process the loop keeps, wherever that Stop ran.
//! - **reef makes a node's worktree.** WorktreeCreate replaces Claude Code's own `git worktree
//!   add`, so `claude -w <id>` gets `.claude/worktrees/<id>` on `reef/<id>`, with the node
//!   started there and the session's pane joined to it where it is a loop's member
//!   ([`crate::worktree`]). Its standard output is the worktree's path alone, as Claude Code
//!   reads it, so everything else it says goes to standard error. A name that is no node gets
//!   a worktree as Claude Code would make it, on `worktree-<name>`. WorktreeRemove takes a last
//!   draft of the node, then removes the worktree, never forced, keeping its branch.
//! - **Nothing to do is no failure.** A directory outside git or outside every project, a
//!   worktree no node was started in, or a node marked something other than done: the hook says
//!   nothing and succeeds. A capture that fails, or a service that doesn't answer, is an error,
//!   which git ignores and Claude Code shows without stopping.
//! - **Each line it prints starts `reef:`**, since git shows it among a commit's own output.

use reef_api::loops::member_of;
use reef_api::{Pane, RecordDecisions, Via, WorkRecord};
use reef_model::{NodeName, Status};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::capture::{Keep, Report, capture_record, seal_if_done};
use crate::clear::{self, Clear};
use crate::client::Client;
use crate::decisions;
use crate::git::Checkout;
use crate::work::parse_status;
use crate::{task, worktree};

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

/// What Claude Code hands WorktreeCreate: the worktree's name, and where the session is
#[derive(Debug, Deserialize)]
struct WorktreeAsked {
    /// The name `claude -w` or EnterWorktree was given
    name: String,
    /// The session's working directory, which the worktree is made from
    #[serde(default)]
    cwd: Option<PathBuf>,
}

/// What Claude Code hands WorktreeRemove: the worktree it is done with
#[derive(Debug, Deserialize)]
struct WorktreeDone {
    /// The worktree's root
    worktree_path: PathBuf,
}

/// What Claude Code hands Stop: the session, and where it is
#[derive(Debug, Deserialize)]
struct Stopped {
    /// The session that stopped, which the injector clears
    #[serde(default)]
    session_id: Option<String>,
    /// The session's working directory
    #[serde(default)]
    cwd: Option<PathBuf>,
    /// The session's transcript, which its decisions are read from
    #[serde(default)]
    transcript_path: Option<PathBuf>,
}

/// What Claude Code hands SessionEnd: the session's transcript, and where it was
#[derive(Debug, Deserialize)]
struct Ended {
    /// The session's working directory
    #[serde(default)]
    cwd: Option<PathBuf>,
    /// The session's transcript, which its decisions are read from
    #[serde(default)]
    transcript_path: Option<PathBuf>,
}

/// Drafts the node the worktree is on after a commit, unless a rebase is under way or the commit
/// is a merge
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
    // and a merge brings in work that isn't the node's
    if checkout.head_merges()? {
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
    // a plan named alone, or in full as the tools take it too
    let named = record.node.id == set.id
        && set.plan.as_ref().is_none_or(|plan| {
            *plan == record.node.plan.plan || *plan == record.node.plan.to_string()
        });
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
/// none, then acts once on the end of the node the pane's loop holds, starting the injector
/// that clears the session when another node is next and the pane can be typed into
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, unless the input names the session's
/// * `input` - The event Claude Code handed the hook, as JSON
/// * `pane` - The pane the session runs in, if it could be found
/// * `pid` - The Claude Code process the session runs in, `CLAUDE_PID`, if it was named
/// * `spawn` - Starts the injector in the background, as [`crate::clear::spawn`] does
/// * `out` - Where to print what came of it
pub async fn stop<W, S>(
    client: &Client,
    cwd: &Path,
    input: &str,
    pane: Option<&Pane>,
    pid: Option<u32>,
    spawn: S,
    out: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
    S: FnOnce(&Clear) -> anyhow::Result<()>,
{
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
    // the plans and questions of the turn, kept with their nodes, a failure said but no stop
    let transcript = stopped.transcript_path.as_deref();
    if let Err(error) = keep_decisions(client, dir, transcript, pane, Place::Session).await {
        writeln!(
            said,
            "reef: keeping the session's decisions failed: {error:#}"
        )?;
    }
    // then the end of the loop's node, or of its group, acted on once, wherever the session runs
    let ended = match pane {
        Some(pane) => task::ended(client, pane).await?,
        None => task::Ended {
            ending: task::Ending::Nothing,
            feeding: None,
            member: None,
        },
    };
    // the loop's own pane is the one cleared, with its session and process
    let (target, session, pid) = match &ended.feeding {
        Some(feeding) if ended.member.is_some() => {
            (Some(&feeding.pane), feeding.session.clone(), feeding.pid)
        }
        Some(feeding) => (pane, stopped.session_id, pid.or(feeding.pid)),
        None => (pane, stopped.session_id, pid),
    };
    // another node next clears the session, wherever reef can type into its pane
    let Some(message) = clear::hand_off(&ended.ending, target, session, pid, spawn) else {
        // nothing for the user: the seal's lines as they are
        out.write_all(&said)?;
        return Ok(());
    };
    // a member's Stop that ended its group says it was the last
    let message = match (&ended.member, &ended.ending) {
        (
            Some(member),
            task::Ending::Next { .. } | task::Ending::Paused(_) | task::Ending::Finished(_),
        ) => format!(
            "`{member}` was the last of its group to be done, so the loop's pane moves on. \
             {message}"
        ),
        _ => message,
    };
    // Claude Code shows a Stop's output to the user only when all of it is JSON
    let mut told = String::from_utf8_lossy(&said).into_owned();
    told.push_str(&format!("reef: {message}"));
    writeln!(out, "{}", serde_json::json!({ "systemMessage": told }))?;
    Ok(())
}

/// Makes the worktree `claude -w <name>` asks for, and prints its path alone: a node's own,
/// started there with the pane joined to it where it is a loop's member, or a plain one for a
/// name that is no node
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, unless the input names the session's
/// * `input` - The event Claude Code handed the hook, as JSON
/// * `pane` - The pane the session runs in, if it could be found
/// * `said` - Where to say what came of it, standard error, since Claude Code reads the path
///   from standard output
/// * `out` - Where to print the worktree's path
pub async fn worktree_create<S: Write, W: Write>(
    client: &Client,
    cwd: &Path,
    input: &str,
    pane: Option<&Pane>,
    said: &mut S,
    out: &mut W,
) -> anyhow::Result<()> {
    // the name asked for, from the session's directory
    let asked: WorktreeAsked = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("WorktreeCreate's input doesn't read as one: {error}"))?;
    let dir = asked.cwd.as_deref().unwrap_or(cwd);
    let checkout = Checkout::open(dir)?;
    // a node of the project the repo is registered as, else a worktree like Claude Code's own
    let node = match client.find_project(checkout.root()).await? {
        Some(project) => worktree::resolve(client, &project.name, &asked.name).await?,
        None => None,
    };
    let path = match node {
        Some(node) => {
            let mut lines = Vec::new();
            let made = worktree::for_node(client, dir, &node, pane, &mut lines).await;
            prefixed(&lines, said)?;
            let made = made?;
            // whose member the pane now works, if it joined one
            if let Some(joined) = &made.joined {
                writeln!(
                    said,
                    "reef: this session works `{node}` for the loop in {}",
                    joined.pane.describe()
                )?;
            }
            made.path
        }
        None => worktree::plain(dir, &asked.name)?,
    };
    // the path alone, as Claude Code reads it
    writeln!(out, "{}", path.display())?;
    Ok(())
}

/// Takes a last draft of the node a worktree is on, then removes the worktree, keeping its
/// branch
///
/// # Arguments
///
/// * `client` - The service's client
/// * `input` - The event Claude Code handed the hook, as JSON
/// * `out` - Where to print what came of it
pub async fn worktree_remove<W: Write>(
    client: &Client,
    input: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    let done: WorktreeDone = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("WorktreeRemove's input doesn't read as one: {error}"))?;
    let checkout = Checkout::open(&done.worktree_path)?;
    // whatever it holds is drafted before it goes
    if let Some(record) = on_node(client, &checkout).await? {
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
        prefixed(&said, out)?;
    }
    // removed from the main worktree, so git never removes the one it runs in
    let main = Checkout::open(&checkout.main_root()?)?;
    main.remove_worktree(checkout.root()).map_err(|error| {
        error.context(format!(
            "{} was left as it is: commit or discard its changes, then `git worktree remove` it",
            checkout.root().display()
        ))
    })?;
    writeln!(out, "reef: removed {}", checkout.root().display())?;
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

/// Where a hook keeps the decisions its transcript names no node for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// On the node the session is on now, as Stop finds it before the loop moves on
    Session,
    /// Nowhere: only those the transcript's own next_task placed are kept, since by a
    /// session's end its pane's loop may hold the next node already
    Named,
}

/// Claude Code's SessionEnd: keeps the plans and questions of a session that ends, a clear's or
/// a plan approval's included, with the nodes its next_task named
///
/// Stop reads them at each turn's end, but a clear ends the session before its turn does. Its
/// last lines may not be written yet when this runs, so an approval that clears is read again
/// from the session the clear starts, by that session's Stop.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The hook's working directory
/// * `input` - What Claude Code handed the hook on standard input
/// * `pane` - The pane the session runs in, if it runs in one
/// * `out` - Where the hook's output goes
pub async fn session_end<W: Write>(
    client: &Client,
    cwd: &Path,
    input: &str,
    pane: Option<&Pane>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the session's directory and transcript
    let ended: Ended = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("SessionEnd's input doesn't read as one: {error}"))?;
    let dir = ended.cwd.as_deref().unwrap_or(cwd);
    let transcript = ended.transcript_path.as_deref();
    keep_decisions(client, dir, transcript, pane, Place::Named).await?;
    out.flush()?;
    Ok(())
}

/// Keeps a session's decisions, read from its transcript, with the nodes they were made on,
/// returning how many were new or changed
///
/// Each is kept on the node the session's last next_task named before it. One asked before any
/// is kept, if `place` says so, on the node its pane's loop holds, else on the node its checkout
/// was started on. One with no node to go on is left out.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `dir` - The session's working directory
/// * `transcript` - The session's transcript, if Claude Code named one
/// * `pane` - The pane the session runs in, if it runs in one
/// * `place` - Where those its transcript names no node for go
pub async fn keep_decisions(
    client: &Client,
    dir: &Path,
    transcript: Option<&Path>,
    pane: Option<&Pane>,
    place: Place,
) -> anyhow::Result<u32> {
    // a transcript not named, or not written yet, holds nothing
    let Some(text) = transcript.and_then(|path| std::fs::read_to_string(path).ok()) else {
        return Ok(0);
    };
    // answered in the app for a runner's session, in the terminal for any other
    let via = match pane.is_some_and(Pane::runs) {
        true => Via::App,
        false => Via::Terminal,
    };
    let read = decisions::read(&text, via);
    if read.is_empty() {
        return Ok(0);
    }
    // the node for those asked before any next_task, found only if one was and may be placed
    let unplaced = read.iter().any(|decision| decision.node.is_none());
    let fallback = match (unplaced, place) {
        (true, Place::Session) => session_node(client, dir, pane).await?,
        _ => None,
    };
    let mut by_node: BTreeMap<NodeName, RecordDecisions> = BTreeMap::new();
    for decision in read {
        let Some(node) = decision.node.or_else(|| fallback.clone()) else {
            continue;
        };
        by_node
            .entry(node)
            .or_insert_with(|| RecordDecisions {
                decisions: Vec::new(),
            })
            .decisions
            .push(decision.decided);
    }
    // each node's kept at once, counting what was new or changed
    let mut kept = 0;
    for (node, record) in by_node {
        if let reef_api::DecisionsRecorded::Recorded { added, updated, .. } =
            client.record_decisions(&node, &record).await?
        {
            kept += added + updated;
        }
    }
    Ok(kept)
}

/// The node a session is on: the one its pane's loop holds, as its own or a member's, else the
/// one its checkout was started on
///
/// # Arguments
///
/// * `client` - The service's client
/// * `dir` - The session's working directory
/// * `pane` - The pane the session runs in, if it runs in one
async fn session_node(
    client: &Client,
    dir: &Path,
    pane: Option<&Pane>,
) -> anyhow::Result<Option<NodeName>> {
    // the pane's loop's node, or the member it works
    if let Some(pane) = pane {
        let loops = client.loops().await?;
        if let Some(node) = loops
            .iter()
            .find(|feeding| feeding.pane == *pane)
            .and_then(|feeding| feeding.node.clone())
        {
            return Ok(Some(node));
        }
        if let Some((_, member)) = member_of(&loops, pane) {
            return Ok(Some(member.node.clone()));
        }
    }
    // else the node the checkout was started on
    let Some(checkout) = open(dir) else {
        return Ok(None);
    };
    Ok(on_node(client, &checkout).await?.map(|record| record.node))
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
    // the node started there last, in the plan the checkout's branch picks, else in any
    crate::cli::worktree_record(client, &project, checkout.root()).await
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
