//! reef's commands for starting and finishing work: a node's status and criteria set, the plan
//! checked, and what is ready said
//!
//! ```text
//! reef status <ID> <STATUS>             # todo, in-progress, done or blocked
//! reef accept <ID>                      # list a node's acceptance criteria, numbered
//! reef accept <ID> <N> <STATUS>         # met, unmet or todo, with --test naming its test
//! reef check                            # the plan's problems, failing if it has any
//! reef next                             # the work ready to start, and what the rest waits on
//! ```
//!
//! - **A status or a criterion is a put of its node.** The node is read, changed and put back at
//!   the version it was read at, so one changed by another writer since is refused as stale,
//!   and a change that breaks the plan is refused with check's messages.
//! - **A group's status is derived.** A group is done when its members are, so its status is
//!   never set; its members' are.
//! - **Criteria are numbered from 1**, in the order written, as check's messages number them. A
//!   performance criterion with a target is met or not by its metric's figures, so its status
//!   is never set either.

use reef_api::{Listing, NodePut, NodeWrite, Puts, StoredNode};
use reef_model::{Met, PlanName, Status};
use std::io::Write;

use crate::client::Client;
use crate::node;

/// Reads a node's status as the frontmatter takes it, aliases and all
///
/// # Arguments
///
/// * `text` - The status, such as `done` or `in-progress`
///
/// # Examples
///
/// ```
/// use reef::work::parse_status;
/// use reef_model::Status;
///
/// assert_eq!(parse_status("in-progress").unwrap(), Status::InProgress);
/// assert_eq!(parse_status("wip").unwrap(), Status::InProgress);
/// assert!(parse_status("finished-ish").is_err());
/// ```
pub fn parse_status(text: &str) -> Result<Status, String> {
    // the model's own names for it, so the command takes what frontmatter does
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|_| format!("`{text}` is not a status: todo, in-progress, done or blocked"))
}

/// Reads whether a criterion is met as the frontmatter takes it, aliases and all
///
/// # Arguments
///
/// * `text` - Whether it is met, such as `met` or `unmet`
///
/// # Examples
///
/// ```
/// use reef::work::parse_met;
/// use reef_model::Met;
///
/// assert_eq!(parse_met("met").unwrap(), Met::Met);
/// assert_eq!(parse_met("failed").unwrap(), Met::Unmet);
/// assert!(parse_met("maybe").is_err());
/// ```
pub fn parse_met(text: &str) -> Result<Met, String> {
    // the model's own names for it, so the command takes what frontmatter does
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|_| format!("`{text}` is not a criterion's status: met, unmet or todo"))
}

/// Sets a node's status, putting it back at the version it was read at
///
/// A group's status is derived from its members, so a group's is refused.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan the node is in
/// * `id` - The node's id
/// * `status` - Where its work now stands
/// * `out` - Where to print what came of it
pub async fn status<W: Write>(
    client: &Client,
    plan: &PlanName,
    id: &str,
    status: Status,
    out: &mut W,
) -> anyhow::Result<()> {
    // the plan as it stands, since whether a node is a group is whether others name it
    let listing = client.list(plan).await?;
    let node = stored(&listing, plan, id)?;
    if listing
        .nodes
        .iter()
        .any(|member| member.spec.group.as_deref() == Some(id))
    {
        anyhow::bail!(
            "`{id}` is a group, which is done when its members are: set their status instead"
        );
    }
    // to do is the default, which the canonical YAML leaves out
    let wanted = (status != Status::Todo).then_some(status);
    if node.spec.status == wanted {
        writeln!(out, "`{id}` is already {}", status.as_str())?;
        return Ok(());
    }
    // put back over the version read, checked against the whole plan
    let mut write = NodeWrite::from(node.clone());
    write.spec.status = wanted;
    let version = put_back(client, plan, write, node.version, "status change").await?;
    writeln!(out, "`{id}` is {}, at version {version}", status.as_str())?;
    Ok(())
}

/// Lists a node's acceptance criteria, numbered from 1, or sets whether one is met and the test
/// that checks it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan the node is in
/// * `id` - The node's id
/// * `set` - The criterion's number and whether it is met, or `None` to list them
/// * `test` - The test that checks the criterion, to set with it
/// * `out` - Where to print the criteria, or what came of it
pub async fn accept<W: Write>(
    client: &Client,
    plan: &PlanName,
    id: &str,
    set: Option<(usize, Met)>,
    test: Option<&str>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the node as it stands
    let listing = client.list(plan).await?;
    let node = stored(&listing, plan, id)?;
    let criteria = &node.spec.acceptance;
    // with no criterion named, each one numbered, with whether it is met and its test
    let Some((number, met)) = set else {
        if criteria.is_empty() {
            writeln!(out, "`{id}` has no acceptance criteria")?;
        }
        for (k, criterion) in criteria.iter().enumerate() {
            // a target's criterion is met or not by its metric, unless its author said
            let status = match criterion.status {
                Some(met) => met.as_str(),
                None if criterion.has_target() => "by its metric",
                None => Met::Todo.as_str(),
            };
            let kind = criterion.kind.as_str();
            writeln!(out, "{}. {status}  {kind}: {}", k + 1, criterion.text)?;
            if let Some(test) = &criterion.test {
                writeln!(out, "   test: {test}")?;
            }
        }
        return Ok(());
    };
    // the criterion named must be one of its, and one whose status is set by hand
    let count = criteria.len();
    let Some(criterion) = number.checked_sub(1).and_then(|k| criteria.get(k)) else {
        anyhow::bail!(
            "`{id}` has {count} acceptance {}, numbered from 1: `reef accept {id}` lists them",
            if count == 1 { "criterion" } else { "criteria" }
        );
    };
    if criterion.has_target() {
        anyhow::bail!(
            "criterion {number} of `{id}` is met or not by its metric's `after` figure against \
             its target: record the figures in the node's `performance` instead"
        );
    }
    // to do is the default, which the canonical YAML leaves out
    let wanted = (met != Met::Todo).then_some(met);
    let test = test.map(str::to_string).or_else(|| criterion.test.clone());
    if criterion.status == wanted && criterion.test == test {
        writeln!(
            out,
            "criterion {number} of `{id}` is already {}",
            met.as_str()
        )?;
        return Ok(());
    }
    // put back over the version read, checked against the whole plan
    let mut write = NodeWrite::from(node.clone());
    let changed = &mut write.spec.acceptance[number - 1];
    changed.status = wanted;
    changed.test = test;
    let version = put_back(client, plan, write, node.version, "criterion change").await?;
    writeln!(
        out,
        "criterion {number} of `{id}` is {}, at version {version}",
        met.as_str()
    )?;
    Ok(())
}

/// Prints each of a plan's problems, as `mdbook-reef check` prints a book's, returning how many
/// there are
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to check
/// * `out` - Where to print its problems
pub async fn check<W: Write>(
    client: &Client,
    plan: &PlanName,
    out: &mut W,
) -> anyhow::Result<usize> {
    // check's own messages, the graph's first
    let problems = client.check(plan).await?;
    for problem in &problems {
        writeln!(out, "error: {problem}")?;
    }
    Ok(problems.len())
}

/// Says how many problems a plan has, as a line of its own
///
/// # Arguments
///
/// * `plan` - The plan checked
/// * `count` - How many problems check found
///
/// # Examples
///
/// ```
/// use reef::work::problems;
/// use reef_model::PlanName;
///
/// let plan = PlanName::new("reef", "central").unwrap();
/// assert_eq!(problems(&plan, 0), "`reef/central` has no problems");
/// assert_eq!(problems(&plan, 1), "`reef/central` has 1 problem");
/// ```
pub fn problems(plan: &PlanName, count: usize) -> String {
    match count {
        0 => format!("`{plan}` has no problems"),
        1 => format!("`{plan}` has 1 problem"),
        n => format!("`{plan}` has {n} problems"),
    }
}

/// Prints the work ready to start in a plan, in plan order, then each piece of work that waits
/// and what it waits on, as `mdbook-reef next` prints a book's
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to look in
/// * `out` - Where to print it
pub async fn next<W: Write>(client: &Client, plan: &PlanName, out: &mut W) -> anyhow::Result<()> {
    // the plan still has an order with problems in it, but say that they are there
    let next = client.next(plan).await?;
    if next.problems > 0 {
        writeln!(
            out,
            "warning: {}; `reef check` lists them",
            problems(plan, next.problems)
        )?;
    }
    write!(out, "{next}")?;
    Ok(())
}

/// The node of a listing with an id, or an error naming it in its plan
///
/// # Arguments
///
/// * `listing` - The plan as it stands
/// * `plan` - The plan's full name
/// * `id` - The node's id
fn stored<'a>(listing: &'a Listing, plan: &PlanName, id: &str) -> anyhow::Result<&'a StoredNode> {
    // an id that breaks the rule names says so before it is looked for
    let name = plan.node(id)?;
    listing
        .nodes
        .iter()
        .find(|node| node.spec.id == id)
        .ok_or_else(|| anyhow::anyhow!("node `{name}` is not in its plan"))
}

/// Puts a changed node back over the version it was read at, returning its new version
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan the node is in
/// * `node` - The node as changed
/// * `read` - The version it was read at
/// * `action` - What the change was, such as `status change`, for the message if it is refused
async fn put_back(
    client: &Client,
    plan: &PlanName,
    node: NodeWrite,
    read: u64,
    action: &str,
) -> anyhow::Result<u64> {
    // one put, checked against the whole plan
    let batch = Puts::new(vec![NodePut::new(node, Some(read))]);
    let checked = client.batch(plan, &batch).await?;
    let versions = node::settled(checked, plan, action)?;
    versions
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("the service wrote no version for the {action}"))
}
