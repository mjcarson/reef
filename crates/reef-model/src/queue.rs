//! The user's order of work: a queue of nodes across a project's plans, and what next makes of it
//!
//! - **Readiness stays derived.** A node is ready when everything it and its groups need is done.
//!   The queue only orders ready work; it never makes a waiting node ready.
//! - **The queue decides among ready nodes.** [`Next::queued`] takes each ready entry in the
//!   queue's order, then the rest of the ready work in plan order, plan by plan.
//! - **A queued group stands for its members**, its ready members taken in plan order.
//! - **A waiting entry says why.** An entry queued ahead of its needs is passed over, and the
//!   report lists it with the needs it waits on. So is a blocked entry, and one whose node or
//!   plan is gone.
//! - **Done and started entries are passed over quietly.** The queue keeps them until they are
//!   taken off, and next never takes them.

use crate::graph::Graph;
use crate::name::{NodeName, PlanName};
use crate::next::{Next, WaitsOn, ready, waiting, waits_on};
use crate::spec::Status;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One plan of a project, built, with how many problems building it found
#[derive(Debug, Clone)]
pub struct PlanGraph {
    /// The plan's full name
    pub name: PlanName,
    /// The plan, built
    pub graph: Graph,
    /// How many problems were found building it
    pub problems: usize,
}

/// One entry of the user's queue and what it is now
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// The node queued, named in full
    pub node: NodeName,
    /// The node's title, or empty for one that is missing
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Whether next takes it, and if not why
    pub state: EntryState,
}

/// What a queued entry is now, read from its plan
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryState {
    /// Ready to start: the ids next takes for it, its own or a group's ready members in plan
    /// order
    Ready(Vec<String>),
    /// Waiting on needs not done yet: its own and its groups', or for a group, its waiting
    /// members' too, each named for whose need it is
    Waiting(Vec<WaitsOn>),
    /// Started, so next doesn't hand it out again
    InProgress,
    /// Blocked, or a group whose members not done are all blocked
    Blocked,
    /// Done
    Done,
    /// Its plan isn't read, or has no such node
    Missing,
}

impl EntryState {
    /// The word a queue's listing names this state by
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::EntryState;
    ///
    /// assert_eq!(EntryState::InProgress.as_str(), "in-progress");
    /// assert_eq!(EntryState::Waiting(Vec::new()).as_str(), "waiting");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryState::Ready(_) => "ready",
            EntryState::Waiting(_) => "waiting",
            EntryState::InProgress => "in-progress",
            EntryState::Blocked => "blocked",
            EntryState::Done => "done",
            EntryState::Missing => "missing",
        }
    }
}

/// What each entry of a queue is now
///
/// # Arguments
///
/// * `plans` - The project's plans, built
/// * `queue` - The queue's entries, in the user's order
///
/// # Examples
///
/// ```
/// use reef_model::queue::entries;
/// use reef_model::{EntryState, Graph, Node, PlanGraph, PlanName};
///
/// let name = PlanName::new("reef", "central").unwrap();
/// let (graph, _) = Graph::build(vec![Node { id: "C1".into(), title: "C1 First".into(), ..Node::default() }]);
/// let plans = [PlanGraph { name: name.clone(), graph, problems: 0 }];
/// let queue = [name.node("C1").unwrap(), name.node("C9").unwrap()];
/// let states: Vec<EntryState> = entries(&plans, &queue).into_iter().map(|e| e.state).collect();
/// assert_eq!(states, [EntryState::Ready(vec!["C1".into()]), EntryState::Missing]);
/// ```
pub fn entries(plans: &[PlanGraph], queue: &[NodeName]) -> Vec<QueueEntry> {
    queue
        .iter()
        .map(|node| {
            // an entry whose plan isn't read, or whose node is gone, is missing
            let found = plans
                .iter()
                .find(|plan| plan.name == node.plan)
                .and_then(|plan| plan.graph.index_of(&node.id).map(|i| (&plan.graph, i)));
            match found {
                Some((graph, i)) => QueueEntry {
                    node: node.clone(),
                    title: graph.node(i).title.clone(),
                    state: state(graph, i),
                },
                None => QueueEntry {
                    node: node.clone(),
                    title: String::new(),
                    state: EntryState::Missing,
                },
            }
        })
        .collect()
}

/// What a node of a plan is now, as a queue's entry
///
/// # Arguments
///
/// * `graph` - The plan, built
/// * `i` - The node queued
fn state(graph: &Graph, i: usize) -> EntryState {
    // a group stands for its members
    if graph.is_group(i) {
        return group_state(graph, i);
    }
    match graph.status(i) {
        Status::Done => EntryState::Done,
        Status::InProgress => EntryState::InProgress,
        Status::Blocked => EntryState::Blocked,
        // work not started is ready, or waits on what it needs
        Status::Todo if graph.is_ready(i) => EntryState::Ready(vec![graph.node(i).id.clone()]),
        Status::Todo => {
            let needs = graph
                .unmet_needs(i)
                .iter()
                .map(|u| waits_on(graph, u))
                .collect();
            EntryState::Waiting(needs)
        }
    }
}

/// What a queued group is now: done, its ready members, what its waiting members wait on, or
/// how far along the rest are
///
/// # Arguments
///
/// * `graph` - The plan, built
/// * `group` - The group queued
fn group_state(graph: &Graph, group: usize) -> EntryState {
    // a group is done when its members are
    if graph.status(group) == Status::Done {
        return EntryState::Done;
    }
    // its pieces of work, in plan order
    let leaves: Vec<usize> = graph
        .plan_order(group)
        .into_iter()
        .filter(|&m| !graph.is_group(m))
        .collect();
    // any member ready is taken for it
    let ready: Vec<String> = leaves
        .iter()
        .filter(|&&m| graph.is_ready(m))
        .map(|&m| graph.node(m).id.clone())
        .collect();
    if !ready.is_empty() {
        return EntryState::Ready(ready);
    }
    // otherwise what its members not started wait on, each need once, named for whose it is
    let mut own = Vec::new();
    let mut others = Vec::new();
    let mut seen = HashSet::new();
    for &m in leaves.iter().filter(|&&m| graph.status(m) == Status::Todo) {
        for unmet in graph.unmet_needs(m) {
            // a need is the member's own, a group's between, or the queued group's own
            let whose = unmet.through.unwrap_or(m);
            let through = (whose != group).then(|| graph.node(whose).id.clone());
            let need = WaitsOn {
                through,
                ..waits_on(graph, &unmet)
            };
            if seen.insert((need.id.clone(), need.through.clone())) {
                // the group's own needs come first, as a node's own do
                match need.through {
                    None => own.push(need),
                    Some(_) => others.push(need),
                }
            }
        }
    }
    if !own.is_empty() || !others.is_empty() {
        own.extend(others);
        return EntryState::Waiting(own);
    }
    // the members not done are started or blocked
    let started = leaves
        .iter()
        .any(|&m| graph.status(m) == Status::InProgress);
    if started {
        EntryState::InProgress
    } else {
        EntryState::Blocked
    }
}

impl Next {
    /// What a project's plans say to do next, in the user's order first
    ///
    /// Each ready entry of the queue is taken in the queue's order, a group's ready members in
    /// plan order, each node once. Then each plan's other ready work follows in plan order, plan
    /// by plan in the order given. Entries waiting, blocked or missing are passed over and listed
    /// with why. With an empty queue and one plan this is what [`Next::of`] says, each node
    /// marked with its plan.
    ///
    /// # Arguments
    ///
    /// * `plans` - The project's plans read, built, in the order their work is taken
    /// * `queue` - The queue's entries, in the user's order
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::{Graph, Next, Node, PlanGraph, PlanName};
    ///
    /// let work = |id: &str| Node { id: id.into(), title: id.into(), ..Node::default() };
    /// let name = PlanName::new("reef", "central").unwrap();
    /// let (graph, _) = Graph::build(vec![work("C1"), work("C2")]);
    /// let plans = [PlanGraph { name: name.clone(), graph, problems: 0 }];
    /// let next = Next::queued(&plans, &[name.node("C2").unwrap()]);
    /// assert_eq!((next.ready[0].id.as_str(), next.ready[1].id.as_str()), ("C2", "C1"));
    /// ```
    pub fn queued(plans: &[PlanGraph], queue: &[NodeName]) -> Next {
        let mut ready_work = Vec::new();
        let mut skipped = Vec::new();
        let mut taken: HashSet<(PlanName, String)> = HashSet::new();
        // the queue's ready entries first, in its order
        for entry in entries(plans, queue) {
            match &entry.state {
                EntryState::Ready(ids) => {
                    // a ready entry's plan is read, so its nodes are found in it
                    let Some(plan) = plans.iter().find(|p| p.name == entry.node.plan) else {
                        continue;
                    };
                    for id in ids {
                        // a node queued twice, itself and through its group, is taken once
                        let Some(i) = plan.graph.index_of(id) else {
                            continue;
                        };
                        if taken.insert((plan.name.clone(), id.clone())) {
                            ready_work.push(ready(&plan.graph, i, Some(&plan.name), true));
                        }
                    }
                }
                // what waits or is gone is passed over, saying why
                EntryState::Waiting(_) | EntryState::Blocked | EntryState::Missing => {
                    skipped.push(entry);
                }
                // what is started or done is passed over quietly
                EntryState::InProgress | EntryState::Done => {}
            }
        }
        // then each plan's other ready work, in plan order
        for plan in plans {
            for i in plan.graph.ready_in_order() {
                if !taken.contains(&(plan.name.clone(), plan.graph.node(i).id.clone())) {
                    ready_work.push(ready(&plan.graph, i, Some(&plan.name), false));
                }
            }
        }
        // and the work that waits, plan by plan
        let waiting_work = plans
            .iter()
            .flat_map(|plan| {
                plan.graph
                    .waiting_in_order()
                    .into_iter()
                    .map(|(i, unmet)| waiting(&plan.graph, i, &unmet, Some(&plan.name)))
            })
            .collect();
        Next {
            ready: ready_work,
            skipped,
            waiting: waiting_work,
            problems: plans.iter().map(|plan| plan.problems).sum(),
            plans: plans.iter().map(|plan| plan.name.clone()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryState, PlanGraph, entries};
    use crate::{Graph, Next, Node, NodeName, PlanName, Status, WaitsOn};

    /// A node with a group, needs and a status, titled for its id
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `parent` - The group it is in, if any
    /// * `needs` - What it needs
    /// * `status` - Its status as written
    fn node(id: &str, parent: Option<&str>, needs: &[&str], status: Option<Status>) -> Node {
        Node {
            id: id.into(),
            title: format!("{id} Some work"),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            status,
            ..Node::default()
        }
    }

    /// A plan of project `reef`, built
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's name within the project
    /// * `nodes` - Its nodes
    fn plan(plan: &str, nodes: Vec<Node>) -> PlanGraph {
        let (graph, problems) = Graph::build(nodes);
        assert!(problems.is_empty(), "{problems:?}");
        PlanGraph {
            name: PlanName::new("reef", plan).unwrap(),
            graph,
            problems: 0,
        }
    }

    /// Names nodes of one plan of project `reef`
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's name within the project
    /// * `ids` - The nodes' ids
    fn names(plan: &str, ids: &[&str]) -> Vec<NodeName> {
        let plan = PlanName::new("reef", plan).unwrap();
        ids.iter().map(|id| plan.node(*id).unwrap()).collect()
    }

    /// The ids of the ready work, in the order it is to be taken
    ///
    /// # Arguments
    ///
    /// * `next` - The report
    fn ready_ids(next: &Next) -> Vec<&str> {
        next.ready.iter().map(|r| r.id.as_str()).collect()
    }

    /// An entry queued ahead of its needs is passed over, and the report names what it waits on
    #[test]
    fn entry_queued_ahead_of_its_needs_is_skipped() {
        let plans = [plan(
            "central",
            vec![
                node("C1", None, &[], None),
                node("C2", None, &["C1"], None),
                node("C3", None, &[], None),
            ],
        )];
        let next = Next::queued(&plans, &names("central", &["C2", "C3"]));
        // the queued entry that is ready comes first, then the rest in plan order
        assert_eq!(ready_ids(&next), ["C3", "C1"]);
        assert!(next.ready[0].queued && !next.ready[1].queued);
        // and the one ahead of its needs is skipped, naming them
        assert_eq!(next.skipped.len(), 1);
        assert_eq!(
            next.skipped[0].state,
            EntryState::Waiting(vec![WaitsOn {
                id: "C1".into(),
                status: Status::Todo,
                through: None,
            }])
        );
        assert_eq!(
            next.to_string(),
            "Ready to start (2), queued first, then in plan order:\n  C3  C3 Some work\n  C1  C1 Some work\n\nSkipped in the queue (1):\n  C2  C2 Some work\n        needs C1 (todo)\n\nWaiting (1):\n  C2  C2 Some work\n        needs C1 (todo)\n"
        );
    }

    /// A queued group stands for its ready members in plan order, each taken once, and waits on
    /// what its members do when none is ready
    #[test]
    fn queued_group_stands_for_its_members() {
        let plans = [plan(
            "central",
            vec![
                node("A", None, &[], None),
                node("G", None, &[], None),
                node("g2", Some("G"), &["g1"], None),
                node("g1", Some("G"), &[], None),
                node("g3", Some("G"), &[], None),
                node("H", None, &["A"], None),
                node("h1", Some("H"), &["h2"], None),
                node("h2", Some("H"), &[], None),
            ],
        )];
        // the group and one of its members queued: each member once, in plan order
        let next = Next::queued(&plans, &names("central", &["G", "g3", "H"]));
        assert_eq!(ready_ids(&next), ["g1", "g3", "A"]);
        // a group with no member ready names its own needs first, then its members'
        assert_eq!(next.skipped.len(), 1);
        let EntryState::Waiting(needs) = &next.skipped[0].state else {
            panic!("{:?}", next.skipped[0]);
        };
        let named: Vec<(&str, Option<&str>)> = needs
            .iter()
            .map(|n| (n.id.as_str(), n.through.as_deref()))
            .collect();
        assert_eq!(named, [("A", None), ("h2", Some("h1"))]);
    }

    /// Whatever the queue says, the same work is ready, and only its order moves
    #[test]
    fn queue_never_changes_readiness() {
        let nodes = || {
            vec![
                node("C1", None, &[], None),
                node("C2", None, &["C1"], None),
                node("G", None, &["C1"], None),
                node("g1", Some("G"), &[], None),
                node("C3", None, &[], Some(Status::Done)),
                node("C4", None, &["C3"], None),
            ]
        };
        let built = plan("central", nodes());
        let mut of: Vec<String> = Next::of(&built.graph, 0)
            .ready
            .into_iter()
            .map(|r| r.id)
            .collect();
        of.sort();
        for queue in [
            vec![],
            vec!["C2"],
            vec!["G", "C4"],
            vec!["C4", "C1", "C2", "g1", "C3", "nope"],
        ] {
            let plans = [plan("central", nodes())];
            let mut queued: Vec<String> = Next::queued(&plans, &names("central", &queue))
                .ready
                .into_iter()
                .map(|r| r.id)
                .collect();
            queued.sort();
            assert_eq!(queued, of, "queued {queue:?}");
        }
    }

    /// Done and started entries are passed over quietly, and missing or blocked ones with why
    #[test]
    fn done_and_missing_entries_are_passed_over() {
        let plans = [plan(
            "central",
            vec![
                node("D", None, &[], Some(Status::Done)),
                node("S", None, &[], Some(Status::InProgress)),
                node("B", None, &[], Some(Status::Blocked)),
                node("R", None, &[], None),
            ],
        )];
        let mut queue = names("central", &["D", "S", "B", "gone", "R"]);
        queue.extend(names("other", &["X"]));
        // each entry's state, the gone node's and the unread plan's missing
        let states: Vec<EntryState> = entries(&plans, &queue)
            .into_iter()
            .map(|e| e.state)
            .collect();
        assert_eq!(
            states,
            [
                EntryState::Done,
                EntryState::InProgress,
                EntryState::Blocked,
                EntryState::Missing,
                EntryState::Ready(vec!["R".into()]),
                EntryState::Missing,
            ]
        );
        // next takes the ready one, and lists the blocked and missing ones with why
        let next = Next::queued(&plans, &queue);
        assert_eq!(ready_ids(&next), ["R"]);
        assert_eq!(
            next.to_string(),
            "Ready to start (1), queued first, then in plan order:\n  R     R Some work\n\nSkipped in the queue (3):\n  B     B Some work\n          blocked\n  gone\n          not in its plan\n  X\n          not in its plan\n\nWaiting (0):\n  nothing\n"
        );
    }

    /// Across a project's plans the queue spans them, and each node is named in full
    #[test]
    fn queue_spans_plans_named_in_full() {
        let plans = [
            plan(
                "central",
                vec![node("C1", None, &[], None), node("C2", None, &["C1"], None)],
            ),
            plan("other", vec![node("O1", None, &[], None)]),
        ];
        let next = Next::queued(&plans, &names("other", &["O1"]));
        assert_eq!(
            next.to_string(),
            "Ready to start (2), queued first, then in plan order:\n  reef/other/O1    O1 Some work\n  reef/central/C1  C1 Some work\n\nWaiting (1):\n  reef/central/C2  C2 Some work\n                     needs C1 (todo)\n"
        );
        // with one plan and nothing queued, it prints as a plan alone does
        let one = [plan("central", vec![node("C1", None, &[], None)])];
        let (graph, _) = Graph::build(vec![node("C1", None, &[], None)]);
        assert_eq!(
            Next::queued(&one, &[]).to_string(),
            Next::of(&graph, 0).to_string()
        );
    }

    /// The report reads back from JSON as it was, its queue's marks and skipped entries kept
    #[test]
    fn queued_next_round_trips_through_json() {
        let plans = [plan(
            "central",
            vec![
                node("C1", None, &[], None),
                node("C2", None, &["C1"], None),
                node("C3", None, &[], Some(Status::Blocked)),
            ],
        )];
        let next = Next::queued(&plans, &names("central", &["C2", "C1", "C3", "C9"]));
        let json = serde_json::to_string(&next).unwrap();
        // an entry's state is tagged in kebab-case, and the plan rides with each node
        assert!(
            json.contains(r#""state":{"waiting":[{"id":"C1","status":"todo"}]}"#),
            "{json}"
        );
        assert!(json.contains(r#""state":"missing""#), "{json}");
        assert!(
            json.contains(r#""plan":"reef/central","queued":true"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Next>(&json).unwrap(), next);
    }
}
