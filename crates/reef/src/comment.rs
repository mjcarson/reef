//! reef's commands for a plan's comment threads
//!
//! ```text
//! reef comment resolve <N> [--node ID] [--revision N]   # resolve thread N, linking a revision
//! ```
//!
//! - **Resolved by a change.** A thread is resolved by the revision that answered it, which is
//!   linked to it, so the user sees which change answered which comment. A commit's
//!   `Reef-Resolves: <N>` trailer does the same when the commit is captured; `resolve` is for a
//!   change made without one.
//! - **The node this worktree is on, by default.** The revision linked is the latest of the node
//!   started here last, unless `--node` names another, a bare id in the thread's plan or a full
//!   name in any of the project's, and `--revision` one of its revisions.
//! - **Over the version read.** The thread is read, then resolved over its version, so a mark
//!   made between the two is never undone unseen.

use reef_api::work::on_worktree;
use reef_api::{ResolveThread, Resolved, Revision};
use reef_model::{NodeName, PlanName};
use std::io::Write;
use std::path::Path;

use crate::cli::{self, PlanPick};
use crate::client::Client;
use crate::git::Checkout;

/// Resolves a thread by hand, linking a revision of a node's changes to it as the one that
/// answered it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, which picks the node when none is named
/// * `thread` - The thread's number in its plan
/// * `node` - The node whose revision answered it, else the node this worktree is on
/// * `revision` - The revision that answered it, else the node's latest
/// * `pick` - The project and plan the thread is in, if they were named
/// * `out` - Where to print what came of it
pub async fn resolve<W: Write>(
    client: &Client,
    cwd: &Path,
    thread: u32,
    node: Option<&str>,
    revision: Option<u32>,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the thread, read for the version the resolve is made over
    let plan = cli::picked_plan(client, cwd, pick).await?;
    let Some(read) = client.thread(&plan, thread).await? else {
        anyhow::bail!("`{plan}` has no comment thread {thread}: no thread starts at #{thread}");
    };
    // the node whose revision answered it, and that revision
    let node = answering(client, cwd, &plan, node).await?;
    let revision = match revision {
        Some(revision) => revision,
        None => latest(client, &node).await?.revision,
    };
    let resolve = ResolveThread {
        node: node.clone(),
        revision,
        read: read.version,
    };
    match client.resolve_thread(&plan, thread, &resolve).await? {
        Resolved::Resolved { .. } => {
            writeln!(
                out,
                "resolved #{thread} of `{plan}`, answered by revision {revision} of `{node}`"
            )?;
            Ok(())
        }
        Resolved::Stale { thread: now } => anyhow::bail!(
            "#{thread} changed while it was being resolved, and is {} now: nothing was written, \
             so resolve it again",
            now.state.as_str()
        ),
        Resolved::NoThread { .. } => {
            anyhow::bail!("#{thread} went from `{plan}` meanwhile: nothing was written")
        }
        Resolved::NoRevision { node, revision } => anyhow::bail!(
            "`{node}` has no revision {revision}: `reef node capture` keeps one, and `reef work \
             {}` shows where it is worked",
            node.id
        ),
    }
}

/// The node whose revision answered a thread: the one named, else the one this worktree is on
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `plan` - The thread's plan, which a bare id is in
/// * `named` - The node as typed, if it was: a bare id in the plan, or a full name
async fn answering(
    client: &Client,
    cwd: &Path,
    plan: &PlanName,
    named: Option<&str>,
) -> anyhow::Result<NodeName> {
    match named {
        // a full name is any of the project's plans', which the service checks
        Some(text) if text.contains('/') => Ok(text.parse()?),
        Some(id) => Ok(plan.node(id)?),
        // else the node started last in this worktree, in any of the project's plans
        None => {
            let checkout = Checkout::open(cwd).map_err(|error| {
                anyhow::anyhow!(
                    "{error:#}: name the node whose revision answered the thread with `--node`"
                )
            })?;
            let records = client.work(&plan.project).await?;
            on_worktree(&records, checkout.root())
                .map(|record| record.node.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no node of `{}` has been started in {}: name the node whose revision \
                         answered the thread with `--node`",
                        plan.project,
                        checkout.root().display()
                    )
                })
        }
    }
}

/// A node's latest revision, which it must have
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node's full name
async fn latest(client: &Client, node: &NodeName) -> anyhow::Result<Revision> {
    // the last of its revisions, oldest first
    let mut revisions = client.revisions(node).await?;
    revisions.pop().ok_or_else(|| {
        anyhow::anyhow!(
            "`{node}` has no captured revision to link: `reef node capture` keeps one, or name \
             another with `--node`"
        )
    })
}
