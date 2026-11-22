//! reef's commands for where a node's work happens: a node started in the working directory's
//! checkout, and its work record shown
//!
//! ```text
//! reef start <ID> [--base BRANCH] [--restart]   # record the node's start here, and pin it
//! reef work [ID]                                # a node's record, or the one this worktree is on
//! ```
//!
//! - **A start is read from the checkout it runs in.** Its worktree's root and branch, the base
//!   branch, HEAD, the worktree written as a tree with anything uncommitted in it, and where HEAD
//!   left the base branch. The service reads no checkout, so the CLI sends what it read.
//! - **The base branch** is `--base`, else the repo's default: the branch `origin/HEAD` names,
//!   else `main` or `master`. On the base branch itself, only its upstream brings other work in,
//!   so the base is the upstream it tracks, or none.
//! - **The start is pinned before it is recorded,** under `refs/reef/start/<plan>/<id>`, so no gc
//!   can prune it after a rebase. A start the service refuses puts the pin back as it was.
//! - **A node is started once.** A second start is refused, saying where the first one was, and
//!   `--restart` starts it again from here, replacing its record.
//! - **A node to do goes in progress,** as `reef status` would put it; any other status stays.
//! - **A worktree is on the node started there last**, which `reef work` shows without an id.

use reef_api::work::on_worktree;
use reef_api::{StartWork, Started, WorkRecord, WorkStart};
use reef_model::{NodeName, Status};
use std::io::Write;
use std::path::Path;

use crate::cli::{self, PlanPick};
use crate::client::Client;
use crate::git::{Checkout, pin_ref};
use crate::work;

/// Starts a node in the working directory's checkout: reads its start, pins it, and records it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in the node's checkout
/// * `id` - The node's id
/// * `pick` - The project and plan named, if they were
/// * `base` - The base branch named, if one was
/// * `restart` - Whether to start a node started already again, from here
/// * `out` - Where to print what came of it
pub async fn start<W: Write>(
    client: &Client,
    cwd: &Path,
    id: &str,
    pick: &PlanPick,
    base: Option<&str>,
    restart: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    // the checkout first, so a directory outside git asks nothing of the service
    let checkout = Checkout::open(cwd)?;
    // the project, whose repo the checkout must be part of
    let project = cli::picked(client, cwd, &pick.project).await?;
    let main = checkout.main_root()?;
    if main != project.repo {
        anyhow::bail!(
            "{} is not in project `{}`'s repo, {}: start its nodes from a checkout of it",
            checkout.root().display(),
            project.name,
            project.repo.display()
        );
    }
    let plan = cli::plan_in(client, project, pick.plan.as_deref()).await?;
    let node = plan.node(id)?;
    // the node must be in its plan, and be work, since a group's members are what is worked
    let listing = client.list(&plan).await?;
    let Some(stored) = listing.nodes.iter().find(|stored| stored.spec.id == id) else {
        anyhow::bail!("`{id}` is not a node of `{plan}`");
    };
    if listing
        .nodes
        .iter()
        .any(|member| member.spec.group.as_deref() == Some(id))
    {
        anyhow::bail!("`{id}` is a group, which is worked through its members: start one of them");
    }
    let todo = stored
        .spec
        .status
        .is_none_or(|status| status == Status::Todo);
    // a node started already starts again only when asked to
    let replaces = match (client.work_of(&node).await?, restart) {
        (None, _) => None,
        (Some(record), true) => Some(record.version),
        (Some(record), false) => anyhow::bail!(
            "`{id}` was started already, {}: `--restart` starts it again from here, replacing \
             that record",
            where_started(&record)
        ),
    };
    // what the start read, pinned so no gc prunes it after a rebase
    let mut read = read_start(&checkout, &node, base)?;
    let message = format!("reef: {node} started");
    let pinned = checkout.pin(&read.pin, &read.start_tree, &read.start, &message)?;
    read.pinned = pinned.id.clone();
    let request = StartWork {
        start: read,
        replaces,
    };
    // a start refused wrote nothing, so its pin is put back; a failure may have written, so not
    let record = match client.start_work(&node, &request).await? {
        Started::Recorded { record } => record,
        Started::Stale { record } => {
            checkout.unpin(&pinned)?;
            match record {
                Some(record) => anyhow::bail!(
                    "`{id}` was started meanwhile, {}: nothing was recorded",
                    where_started(&record)
                ),
                None => anyhow::bail!("`{id}`'s record went meanwhile: nothing was recorded"),
            }
        }
        Started::NotInPlan { node } => {
            checkout.unpin(&pinned)?;
            anyhow::bail!("`{node}` went from its plan meanwhile: nothing was recorded")
        }
    };
    writeln!(out, "started `{id}` in {}", record.start.worktree.display())?;
    show(&record, out)?;
    // a node to do is in progress once it is started
    if todo {
        work::status(client, &plan, id, Status::InProgress, out).await?;
    }
    Ok(())
}

/// Shows a node's work record, or that of the node the working directory's worktree is on
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `id` - The node's id, or `None` for the node this worktree is on
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print it
pub async fn work<W: Write>(
    client: &Client,
    cwd: &Path,
    id: Option<&str>,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    let record = match id {
        // a node named is in the plan picked
        Some(id) => {
            let plan = cli::picked_plan(client, cwd, pick).await?;
            let node = plan.node(id)?;
            client.work_of(&node).await?.ok_or_else(|| {
                anyhow::anyhow!("`{id}` has not been started: `reef start {id}` starts it here")
            })?
        }
        // else the one started last in this worktree, in any of the project's plans
        None => {
            let checkout = Checkout::open(cwd)?;
            let project = cli::picked(client, cwd, &pick.project).await?;
            let records = client.work(&project.name).await?;
            on_worktree(&records, checkout.root())
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no node of `{}` has been started in {}: `reef start <ID>` starts one",
                        project.name,
                        checkout.root().display()
                    )
                })?
        }
    };
    show(&record, out)
}

/// Reads a start from a checkout, its pin named but not yet written
///
/// # Arguments
///
/// * `checkout` - The checkout the node's work happens in
/// * `node` - The node being started, which names its pin
/// * `base` - The base branch named, if one was
fn read_start(
    checkout: &Checkout,
    node: &NodeName,
    base: Option<&str>,
) -> anyhow::Result<WorkStart> {
    // what is checked out
    let branch = checkout.branch()?;
    let start = checkout.head()?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no commit to start from: commit something first",
            checkout.root().display()
        )
    })?;
    // the base branch named, which must name a commit here, else the repo's default
    let base_branch = match base {
        Some(name) => {
            if checkout.commit(name)?.is_none() {
                anyhow::bail!("`--base {name}` names nothing here: name a branch this repo has");
            }
            Some(name.to_string())
        }
        None => checkout.default_branch()?,
    };
    // working on the base branch itself, only its upstream brings anyone else's work in
    let base_branch = match base_branch {
        Some(base) if branch.as_deref() == Some(base.as_str()) => checkout.upstream(&base)?,
        other => other,
    };
    // where the start left the base branch
    let fork = match &base_branch {
        Some(base) => match checkout.commit(base)? {
            Some(tip) => checkout.merge_base(&start, &tip)?,
            None => None,
        },
        None => None,
    };
    // the worktree as it is, so anything left uncommitted is the start's, not this node's
    let start_tree = checkout.snapshot()?;
    Ok(WorkStart {
        worktree: checkout.root().to_path_buf(),
        branch,
        base_branch,
        start,
        start_tree,
        fork,
        pin: pin_ref(node),
        // the commit pinned is known once the pin is written
        pinned: String::new(),
    })
}

/// Where and when a record says its node was started, for a message
///
/// # Arguments
///
/// * `record` - The record
fn where_started(record: &WorkRecord) -> String {
    // the worktree, and the branch if one was checked out
    let branch = match &record.start.branch {
        Some(branch) => format!(" on `{branch}`"),
        None => String::new(),
    };
    format!(
        "in {}{branch} at {}",
        record.start.worktree.display(),
        when(record.started_at)
    )
}

/// Prints a work record, a field a line
///
/// # Arguments
///
/// * `record` - The record
/// * `out` - Where to print it
fn show<W: Write>(record: &WorkRecord, out: &mut W) -> anyhow::Result<()> {
    // what isn't there says so
    let or_none = |value: &Option<String>| value.clone().unwrap_or_else(|| "none".to_string());
    let start = &record.start;
    let rows = [
        ("node", record.node.to_string()),
        ("worktree", start.worktree.display().to_string()),
        ("branch", or_none(&start.branch)),
        ("base", or_none(&start.base_branch)),
        ("start", start.start.clone()),
        ("tree", start.start_tree.clone()),
        ("fork", or_none(&start.fork)),
        ("pin", format!("{} at {}", start.pin, start.pinned)),
        ("started", when(record.started_at)),
        ("captures", record.captures.len().to_string()),
    ];
    let rows = rows
        .into_iter()
        .map(|(key, value)| (key.to_string(), value));
    cli::columns(out, rows)
}

/// A time the service stamped, in milliseconds since the epoch, as a date and time in UTC
///
/// # Arguments
///
/// * `ms` - The milliseconds since the epoch
fn when(ms: u64) -> String {
    // whole seconds, as git writes a commit's time
    let seconds = i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    gix::date::Time::new(seconds, 0).format_or_unix(gix::date::time::format::ISO8601)
}

#[cfg(test)]
mod tests {
    use super::when;

    /// A time the service stamped reads as a date and time in UTC, to the second
    #[test]
    fn times_read_in_utc() {
        assert_eq!(when(0), "1970-01-01 00:00:00 +0000");
        assert_eq!(when(1_795_000_000_123), "2026-11-18 11:06:40 +0000");
    }
}
