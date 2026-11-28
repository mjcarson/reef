//! Each of a project's plans summed up, as the plan explorer shows them (C120)
//!
//! - **Each source read once for the project.** Its plans, work records, loops, sessions and test
//!   runs are each read once, and each plan takes its share of them, so a summary costs a read of
//!   each plan's nodes and activity rather than one per node, record or session.
//! - **Counts leave groups out.** A group's status is what its members add up to, so only the
//!   nodes no other names as its group are counted.
//! - **Who is working on what.** Each node in progress names who started its newest work record,
//!   or nobody for one set in progress with none started. Each live session names who attached
//!   it and its loop's node: the loop on its own pane, as the app's sessions panel pairs them,
//!   else the group member that pane works.
//! - **A session or a run is on one plan.** A session's is its node's, else its loop's, else its
//!   own; a run's is its own. One that names none is on its project's only plan, or on none in a
//!   project of several.
//! - **Only live work counts.** A session detached or gone, and a run that ended or went silent,
//!   make no plan active.

use reef_api::loops::member_of;
use reef_api::{
    LiveRun, LiveSession, Loop, NodeCounts, PlanSummary, Session, StoredNode, TestRun, WorkRecord,
    Working,
};
use reef_model::{NodeName, PlanName, Status};
use std::collections::{HashMap, HashSet};
use tracing::instrument;

use super::Service;
use super::sessions::now_ms;
use crate::backend::Backend;
use crate::store::NotRegistered;

/// How far back a plan's nodes done today reach, in milliseconds
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

impl<B: Backend> Service<B> {
    /// Sums up each of a project's plans, in its listing's order: who listed it, what it
    /// follows, its nodes by status, its last update and nodes done in the last 24 hours, who
    /// works on which node, its live sessions and runs, its state and whether it is active
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::plan_summaries", skip_all, err(Debug))]
    pub async fn plan_summaries(&self, project: &str) -> anyhow::Result<Vec<PlanSummary>> {
        // a project that isn't registered has no plans to sum up
        let Some(registered) = self.store.project(project).await? else {
            return Err(NotRegistered(project.to_string()).into());
        };
        // every plan with who listed it, and the project's work, loops, sessions and runs
        let plans = self.plan_listing(project).await?;
        let work = self.work(project).await?;
        let loops = self.project_loops(project).await?;
        let sessions = self.project_sessions(project).await?;
        let runs = self.project_test_runs(project).await?;
        // what names no plan is on the project's only plan, if it has one
        let only = match plans.as_slice() {
            [info] => Some(info.name.clone()),
            _ => None,
        };
        // each live session and run, by the plan it is on
        let mut sessions = live_sessions(sessions, &loops, only.as_ref());
        let mut runs = live_runs(runs, only.as_ref());
        // the window nodes done today went done in
        let now = now_ms();
        let today = now.saturating_sub(DAY_MS)..now.saturating_add(1);
        let mut summaries = Vec::with_capacity(plans.len());
        for info in plans {
            // its nodes, counted and paired with who started each in progress
            let listing = self.store.list(&info.name).await?;
            let leaves = leaves(&listing.nodes);
            let nodes = counts(&leaves);
            let in_progress = working(&leaves, &info.name, &work);
            // when it was last written, and what went done today
            let updated = self.store.plan_updated(&info.name).await?;
            let done_today = self.store.plan_done(&info.name, today.clone()).await?;
            // the commit its docs are read at, for a remote project's
            let commit = registered
                .remote
                .as_ref()
                .and_then(|remote| remote.commit_of(info.branch.as_deref()))
                .map(str::to_string);
            // its own sessions and runs
            let on_sessions = sessions.remove(&info.name).unwrap_or_default();
            let on_runs = runs.remove(&info.name).unwrap_or_default();
            summaries.push(PlanSummary::new(
                info,
                commit,
                nodes,
                updated,
                done_today,
                in_progress,
                on_sessions,
                on_runs,
            ));
        }
        Ok(summaries)
    }
}

/// A plan's nodes that aren't groups, in the order it lists them
///
/// # Arguments
///
/// * `nodes` - Every node of the plan
fn leaves(nodes: &[StoredNode]) -> Vec<&StoredNode> {
    // a node another names as its group is a group, as is one listing members inline
    let groups: HashSet<&str> = nodes
        .iter()
        .filter_map(|node| node.spec.group.as_deref())
        .collect();
    nodes
        .iter()
        .filter(|node| !groups.contains(node.spec.id.as_str()) && node.spec.nodes.is_empty())
        .collect()
}

/// How many of a plan's nodes stand at each status
///
/// # Arguments
///
/// * `leaves` - The plan's nodes that aren't groups
fn counts(leaves: &[&StoredNode]) -> NodeCounts {
    // a node that names no status is to do
    let mut counts = NodeCounts::default();
    for node in leaves {
        match node.spec.status.unwrap_or_default() {
            Status::Todo => counts.todo += 1,
            Status::InProgress => counts.in_progress += 1,
            Status::Done => counts.done += 1,
            Status::Blocked => counts.blocked += 1,
        }
    }
    counts
}

/// Each of a plan's nodes in progress, and who started its newest work record
///
/// # Arguments
///
/// * `leaves` - The plan's nodes that aren't groups
/// * `plan` - The plan's full name
/// * `work` - Every work record of its project
fn working(leaves: &[&StoredNode], plan: &PlanName, work: &[WorkRecord]) -> Vec<Working> {
    // the newest record of each of the plan's nodes
    let mut newest: HashMap<&str, &WorkRecord> = HashMap::new();
    for record in work.iter().filter(|record| &record.node.plan == plan) {
        let slot = newest.entry(record.node.id.as_str()).or_insert(record);
        if record.started_at > slot.started_at {
            *slot = record;
        }
    }
    // each node in progress, naming who started it if a record says
    leaves
        .iter()
        .filter(|node| node.spec.status == Some(Status::InProgress))
        .map(|node| {
            let record = newest.get(node.spec.id.as_str());
            Working {
                node: node.spec.id.clone(),
                started_by: record
                    .map(|record| record.started_by.clone())
                    .unwrap_or_default(),
                started_at: record.map(|record| record.started_at),
            }
        })
        .collect()
}

/// Each live session, with who attached it and its loop's node, by the plan it is on
///
/// # Arguments
///
/// * `sessions` - Every session of the project, as it reads now
/// * `loops` - Every loop feeding the project
/// * `only` - The project's only plan, if it has one
fn live_sessions(
    sessions: Vec<Session>,
    loops: &[Loop],
    only: Option<&PlanName>,
) -> HashMap<PlanName, Vec<LiveSession>> {
    let mut on: HashMap<PlanName, Vec<LiveSession>> = HashMap::new();
    for session in sessions.into_iter().filter(Session::live) {
        // the loop on its own pane, else the group member its pane works
        let own = loops.iter().find(|feeding| feeding.pane == session.pane);
        let member = member_of(loops, &session.pane);
        let node: Option<&NodeName> = own
            .and_then(|feeding| feeding.node.as_ref())
            .or_else(|| member.map(|(_, member)| &member.node));
        let feeding = own.or_else(|| member.map(|(feeding, _)| feeding));
        // its node's plan, else its loop's, else its own, else the project's only one
        let named =
            |plan: Option<&str>| plan.and_then(|plan| PlanName::new(&session.project, plan).ok());
        let plan = node
            .map(|node| node.plan.clone())
            .or_else(|| named(feeding.and_then(|feeding| feeding.plan.as_deref())))
            .or_else(|| named(session.plan.as_deref()))
            .or_else(|| only.cloned());
        // one on no plan sums up in none
        let Some(plan) = plan else {
            continue;
        };
        on.entry(plan).or_default().push(LiveSession {
            id: session.id,
            attached_by: session.attached_by,
            node: node.map(|node| node.id.clone()),
            state: session.state,
        });
    }
    on
}

/// Each test run building or running, by the plan it is on
///
/// # Arguments
///
/// * `runs` - Every run of the project, as it reads now
/// * `only` - The project's only plan, if it has one
fn live_runs(runs: Vec<TestRun>, only: Option<&PlanName>) -> HashMap<PlanName, Vec<LiveRun>> {
    let mut on: HashMap<PlanName, Vec<LiveRun>> = HashMap::new();
    for run in runs.into_iter().filter(TestRun::live) {
        // its own plan, else the project's only one
        let plan = run
            .plan
            .as_deref()
            .and_then(|plan| PlanName::new(&run.project, plan).ok())
            .or_else(|| only.cloned());
        // one on no plan sums up in none
        let Some(plan) = plan else {
            continue;
        };
        on.entry(plan).or_default().push(LiveRun {
            id: run.id,
            node: run.node,
            state: run.state,
            started_by: run.started_by,
        });
    }
    on
}

#[cfg(test)]
mod tests {
    use super::{counts, leaves};
    use reef_api::{NodeCounts, StoredNode};
    use reef_model::NodeSpec;

    /// A node read from YAML, as the store lists it
    ///
    /// # Arguments
    ///
    /// * `yaml` - The node's YAML
    fn stored(yaml: &str) -> StoredNode {
        StoredNode {
            spec: NodeSpec::from_yaml(yaml).unwrap(),
            body: String::new(),
            order: 0,
            version: 1,
        }
    }

    /// A plan's nodes are counted by status, a node with none to do and its groups left out
    #[test]
    fn counts_leave_groups_out() {
        // a group set in progress with members at every status, and one naming none
        let nodes = vec![
            stored("id: G\nstatus: in-progress\n"),
            stored("id: A\ngroup: G\nstatus: done\n"),
            stored("id: B\ngroup: G\nstatus: in-progress\n"),
            stored("id: C\ngroup: G\nstatus: blocked\n"),
            stored("id: D\ngroup: G\n"),
            stored("id: E\nstatus: done\n"),
        ];
        let leaves = leaves(&nodes);
        let ids: Vec<&str> = leaves.iter().map(|node| node.spec.id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B", "C", "D", "E"]);
        assert_eq!(
            counts(&leaves),
            NodeCounts {
                todo: 1,
                in_progress: 1,
                done: 2,
                blocked: 1,
            }
        );
    }
}
