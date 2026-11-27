//! The user's queue from the terminal: what to take first, added, moved, taken off and listed
//!
//! ```text
//! reef queue add <ID> [--top | --bottom | --before ID | --after ID] [--project NAME] [--plan NAME]
//! reef queue move <ID> (--top | --bottom | --before ID | --after ID) [--project NAME] [--plan NAME]
//! reef queue rm <ID>... [--project NAME] [--plan NAME]
//! reef queue ls [--project NAME] [--plan NAME | --all]
//! ```
//!
//! - **A queue a plan.** Each plan's entries are a queue of their own, placed among themselves
//!   alone. An entry is a bare id in the picked plan, or a full name, `project/plan/id`, in any
//!   of the project's plans. A node beside which one goes is a bare id in that one's plan, or a
//!   full name, of the same plan.
//! - **`ls` lists the plan picked**, by `--plan`, else the checkout's branch or the node its
//!   worktree is on, and every plan's entries in the project's order where nothing picks one,
//!   or with `--all`.
//! - **Adding a queued node moves it.** It goes to the bottom unless a place is given; a move
//!   needs one.
//! - **`ls` says what next makes of each entry**: ready, waiting with the needs it waits on, in
//!   progress, blocked, done, or missing once its node or plan is gone. Names are bare in a
//!   project of one plan, and full across several.

use reef_api::{Place, QueueEdit, QueueListing, Queued};
use reef_model::next::needs_lines;
use reef_model::{EntryState, NodeName, PlanName};
use std::io::Write;
use std::path::Path;

use crate::cli::{At, PlanPick, picked, picked_plan};
use crate::client::Client;

/// The node a queue command names, and the project whose queue it edits
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `pick` - The project and plan named, if they were
/// * `text` - The node as typed: a bare id in the picked plan, or a full name
async fn target(
    client: &Client,
    cwd: &Path,
    pick: &PlanPick,
    text: &str,
) -> anyhow::Result<(String, NodeName)> {
    // a full name is any of the picked project's plans', which the service checks
    if text.contains('/') {
        let node: NodeName = text.parse()?;
        let project = picked(client, cwd, &pick.project).await?;
        return Ok((project.name, node));
    }
    // a bare id is the picked plan's
    let plan = picked_plan(client, cwd, pick).await?;
    let node = plan.node(text)?;
    Ok((plan.project.clone(), node))
}

/// Where the command line says to put an entry, a node beside it named within the entry's plan,
/// or `None` if it says nowhere
///
/// # Arguments
///
/// * `at` - The place as typed
/// * `within` - The entry's plan, which a bare id beside it is in
fn place(at: &At, within: &PlanName) -> anyhow::Result<Option<Place>> {
    // clap lets only one be given
    Ok(if at.top {
        Some(Place::Top)
    } else if at.bottom {
        Some(Place::Bottom)
    } else if let Some(before) = &at.before {
        Some(Place::Before(NodeName::resolve(before, within)?))
    } else if let Some(after) = &at.after {
        Some(Place::After(NodeName::resolve(after, within)?))
    } else {
        None
    })
}

/// The text a node not queued was typed as: the entry's own, or the node it was to go beside
///
/// # Arguments
///
/// * `node` - The node the service said isn't queued
/// * `entry` - The entry edited, and how it was typed
/// * `at` - The place as typed
fn typed(node: &NodeName, entry: (&NodeName, &str), at: &At) -> String {
    // the entry itself, or else the one node a place names
    if node == entry.0 {
        return entry.1.to_string();
    }
    at.before
        .clone()
        .or_else(|| at.after.clone())
        .unwrap_or_else(|| node.to_string())
}

/// Says what came of putting an entry somewhere in the queue, or why nothing changed
///
/// # Arguments
///
/// * `queued` - What came of the edit
/// * `node` - The entry, as it was named
/// * `text` - The entry, as it was typed
/// * `at` - The place, as it was typed
/// * `out` - Where to print it
fn placed<W: Write>(
    queued: Queued,
    node: &NodeName,
    text: &str,
    at: &At,
    out: &mut W,
) -> anyhow::Result<()> {
    match queued {
        // where it is now, of how many
        Queued::Applied { queue } => {
            let at = queue
                .iter()
                .position(|entry| entry.node == *node)
                .map_or(0, |i| i + 1);
            writeln!(out, "`{text}` is queued {at} of {}", queue.len())?;
            Ok(())
        }
        Queued::NotQueued { node: missing } => anyhow::bail!(
            "`{}` isn't queued: `reef queue ls` lists what is",
            typed(&missing, (node, text), at)
        ),
        Queued::NotInPlan { node } => anyhow::bail!("node `{node}` is not in its plan"),
    }
}

/// Puts a node in its project's queue, where the command line says or at the bottom, moving it
/// there if it is queued already
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `text` - The node, as typed
/// * `at` - Where it goes, as typed
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print where it went
pub async fn add<W: Write>(
    client: &Client,
    cwd: &Path,
    text: &str,
    at: &At,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the node and where it goes, the bottom if nowhere was said
    let (project, node) = target(client, cwd, pick, text).await?;
    let to = place(at, &node.plan)?.unwrap_or(Place::Bottom);
    let edit = QueueEdit::Add {
        node: node.clone(),
        at: to,
    };
    let queued = client.edit_queue(&project, &edit).await?;
    placed(queued, &node, text, at, out)
}

/// Moves a queued node to where the command line says
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `text` - The node, as typed
/// * `at` - Where it goes, as typed
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print where it went
pub async fn mv<W: Write>(
    client: &Client,
    cwd: &Path,
    text: &str,
    at: &At,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // a move says where to
    let (project, node) = target(client, cwd, pick, text).await?;
    let Some(to) = place(at, &node.plan)? else {
        anyhow::bail!(
            "`reef queue move` needs where to: --top, --bottom, --before ID or --after ID"
        );
    };
    let edit = QueueEdit::Move {
        node: node.clone(),
        at: to,
    };
    let queued = client.edit_queue(&project, &edit).await?;
    placed(queued, &node, text, at, out)
}

/// Takes nodes off their project's queue, one at a time, stopping at one that isn't queued
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory
/// * `texts` - The nodes, as typed
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print each taken off
pub async fn rm<W: Write>(
    client: &Client,
    cwd: &Path,
    texts: &[String],
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    for text in texts {
        // each its own edit, which deletes its one row
        let (project, node) = target(client, cwd, pick, text).await?;
        let edit = QueueEdit::Remove { node };
        match client.edit_queue(&project, &edit).await? {
            Queued::Applied { .. } => writeln!(out, "took `{text}` off the queue")?,
            Queued::NotQueued { .. } => {
                anyhow::bail!("`{text}` isn't queued: `reef queue ls` lists what is")
            }
            Queued::NotInPlan { node } => anyhow::bail!("node `{node}` is not in its plan"),
        }
    }
    Ok(())
}

/// Lists a plan's queue, or every plan's entries of its project's: each entry, whether next
/// takes it, and if not why
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project's name
/// * `plan` - The plan whose queue to list, or `None` for every plan's
/// * `out` - Where to print it
pub async fn ls<W: Write>(
    client: &Client,
    project: &str,
    plan: Option<&PlanName>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan's own entries, else the project's
    let (listing, whose) = match plan {
        Some(plan) => (client.plan_queue(plan).await?, plan.to_string()),
        None => (client.queue(project).await?, project.to_string()),
    };
    // an empty queue says how to fill it
    if listing.entries.is_empty() {
        writeln!(
            out,
            "the queue of `{whose}` is empty: `reef queue add <ID>` queues a node"
        )?;
        return Ok(());
    }
    write!(out, "{}", listed(&listing))?;
    Ok(())
}

/// A queue's listing as `reef queue ls` prints it: a line an entry, its place, its name, its
/// state and its title, and under it why it waits, or which members next takes for it, and who
/// queued or moved it there, if that is known
///
/// # Arguments
///
/// * `listing` - The queue's listing
///
/// # Examples
///
/// ```
/// use reef_api::{Listed, QueueListing};
/// use reef_model::{EntryState, QueueEntry};
///
/// let listing = QueueListing {
///     plans: vec!["reef/central".parse().unwrap()],
///     entries: vec![Listed {
///         rank: "a0".into(),
///         entry: QueueEntry {
///             node: "reef/central/C1".parse().unwrap(),
///             title: "C1 First".into(),
///             state: EntryState::Ready(vec!["C1".into()]),
///         },
///         placed: None,
///     }],
/// };
/// assert_eq!(reef::queue::listed(&listing), "1. C1  ready  C1 First\n");
/// ```
pub fn listed(listing: &QueueListing) -> String {
    // with several plans, each entry is named in full
    let qualified = listing.plans.len() > 1;
    let rows: Vec<(String, &str, String, Vec<String>)> = listing
        .entries
        .iter()
        .map(|listed| {
            let entry = &listed.entry;
            let name = match qualified {
                true => entry.node.to_string(),
                false => entry.node.id.clone(),
            };
            // why it waits, then who put it there, if anyone is known to have
            let mut details = details(&entry.node, &entry.state);
            if let Some(placed) = listed.placed.as_ref().and_then(|placed| placed.describe()) {
                details.push(placed);
            }
            (name, entry.state.as_str(), entry.title.clone(), details)
        })
        .collect();
    // each column as wide as its widest cell
    let place = rows.len().to_string().len();
    let name = rows
        .iter()
        .map(|row| row.0.chars().count())
        .max()
        .unwrap_or(0);
    let state = rows.iter().map(|row| row.1.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (i, (named, word, title, details)) in rows.iter().enumerate() {
        // its place, its name, its state and its title
        let line = format!("{:>place$}. {named:name$}  {word:state$}  {title}", i + 1);
        out.push_str(line.trim_end());
        out.push('\n');
        // and under its state, why it waits, or what next takes for it
        for detail in details {
            out.push_str(&format!(
                "{:indent$}{detail}\n",
                "",
                indent = place + 2 + name + 2
            ));
        }
    }
    out
}

/// The lines listed under an entry: the needs it waits on, or the members next takes for a
/// group
///
/// # Arguments
///
/// * `node` - The entry's node
/// * `state` - The entry's state
fn details(node: &NodeName, state: &EntryState) -> Vec<String> {
    match state {
        EntryState::Waiting(needs) => needs_lines(needs),
        // a group stands for its ready members, which are named
        EntryState::Ready(ids) if !(ids.len() == 1 && ids[0] == node.id) => {
            vec![format!("members ready: {}", ids.join(", "))]
        }
        _ => Vec::new(),
    }
}
