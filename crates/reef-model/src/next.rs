//! What a plan says to do next: the work ready to start, in plan order, and what the rest waits
//! on
//!
//! [`Next`] is what `mdbook-reef next` and `reef next` print, and what the JSON API answers
//! with, so a client prints the same report the book's tool does. [`Next::of`] reads one plan
//! in plan order; [`Next::queued`] reads a project's plans in the user's order first
//! ([`queue`](crate::queue)), and prints as `Next::of` does when nothing is queued.

use crate::graph::{Graph, Unmet};
use crate::name::PlanName;
use crate::queue::{EntryState, QueueEntry};
use crate::spec::Status;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The work ready to start, in the order it is to be taken, and each piece of work that waits,
/// with what it waits on
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Next {
    /// The work ready to start, in the order it is to be taken
    pub ready: Vec<Ready>,
    /// The queue's entries passed over, in the queue's order: waiting, blocked or missing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<QueueEntry>,
    /// The work not started that waits, in plan order, each with the needs not done yet
    pub waiting: Vec<Waiting>,
    /// How many problems the plan has, since it still has an order with problems in it
    #[serde(default)]
    pub problems: usize,
    /// The plans read, in the order their work is taken, or none for one plan read alone
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PlanName>,
}

/// A piece of work ready to start
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    /// The node's id
    pub id: String,
    /// The node's title
    pub title: String,
    /// The plan the node is in, when the report reads a project's plans
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanName>,
    /// Whether the user's queue put it here, ahead of plan order
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub queued: bool,
}

/// A piece of work that waits on needs not done yet
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    /// The node's id
    pub id: String,
    /// The node's title
    pub title: String,
    /// Each need not done yet: its own first, then each of its groups', innermost first
    pub needs: Vec<WaitsOn>,
    /// The plan the node is in, when the report reads a project's plans
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanName>,
}

/// One need not done yet that a piece of work waits on, its own or one of its groups'
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitsOn {
    /// The id of the node needed
    pub id: String,
    /// How far along the node needed is
    pub status: Status,
    /// The group whose need it is, or `None` for the waiting node's own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<String>,
}

impl Next {
    /// What a built plan says to do next
    ///
    /// # Arguments
    ///
    /// * `graph` - The plan, built
    /// * `problems` - How many problems were found building it
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::{Graph, Next, Node};
    ///
    /// let first = Node { id: "C1".into(), title: "C1 First".into(), ..Node::default() };
    /// let (graph, problems) = Graph::build(vec![first]);
    /// let next = Next::of(&graph, problems.len());
    /// assert_eq!(next.ready[0].id, "C1");
    /// ```
    pub fn of(graph: &Graph, problems: usize) -> Next {
        // the ready work in the order it is to be taken
        let ready = graph
            .ready_in_order()
            .into_iter()
            .map(|i| ready(graph, i, None, false))
            .collect();
        // then the work that waits, each need with how far along it is
        let waiting = graph
            .waiting_in_order()
            .into_iter()
            .map(|(i, unmet)| waiting(graph, i, &unmet, None))
            .collect();
        Next {
            ready,
            waiting,
            problems,
            ..Next::default()
        }
    }
}

/// A ready node, as the report names it
///
/// # Arguments
///
/// * `graph` - The plan the node is in
/// * `i` - The node
/// * `plan` - The plan's name, when the report reads a project's plans
/// * `queued` - Whether the user's queue put it here
pub(crate) fn ready(graph: &Graph, i: usize, plan: Option<&PlanName>, queued: bool) -> Ready {
    Ready {
        id: graph.node(i).id.clone(),
        title: graph.node(i).title.clone(),
        plan: plan.cloned(),
        queued,
    }
}

/// A waiting node, as the report names it, with each need it waits on
///
/// # Arguments
///
/// * `graph` - The plan the node is in
/// * `i` - The node
/// * `unmet` - Its needs not done yet
/// * `plan` - The plan's name, when the report reads a project's plans
pub(crate) fn waiting(
    graph: &Graph,
    i: usize,
    unmet: &[Unmet],
    plan: Option<&PlanName>,
) -> Waiting {
    Waiting {
        id: graph.node(i).id.clone(),
        title: graph.node(i).title.clone(),
        needs: unmet.iter().map(|u| waits_on(graph, u)).collect(),
        plan: plan.cloned(),
    }
}

/// One unmet need, named by ids
///
/// # Arguments
///
/// * `graph` - The plan the need is in
/// * `unmet` - The need, and the group whose need it is
pub(crate) fn waits_on(graph: &Graph, unmet: &Unmet) -> WaitsOn {
    WaitsOn {
        id: graph.node(unmet.need).id.clone(),
        status: graph.status(unmet.need),
        through: unmet.through.map(|t| graph.node(t).id.clone()),
    }
}

impl fmt::Display for Next {
    /// The report: the work ready to start, in the order it is to be taken, then the queue's
    /// entries passed over, then each piece of work that waits, with a line for its own needs
    /// and one for each of its groups' that are not done
    ///
    /// # Arguments
    ///
    /// * `f` - Where to write it
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // with several plans, each node is named in full
        let qualified = self.plans.len() > 1;
        let named = |plan: &Option<PlanName>, id: &str| match plan {
            Some(plan) if qualified => format!("{plan}/{id}"),
            _ => id.to_string(),
        };
        let ready: Vec<String> = self.ready.iter().map(|r| named(&r.plan, &r.id)).collect();
        let skipped: Vec<String> = self
            .skipped
            .iter()
            .map(|s| named(&Some(s.node.plan.clone()), &s.node.id))
            .collect();
        let waiting: Vec<String> = self.waiting.iter().map(|w| named(&w.plan, &w.id)).collect();
        // names are padded to the widest, so titles line up
        let width = ready
            .iter()
            .chain(&skipped)
            .chain(&waiting)
            .map(|name| name.chars().count())
            .max()
            .unwrap_or(0);
        // the ready work first, saying whether the queue ordered any of it
        let order = if self.ready.iter().any(|r| r.queued) {
            "queued first, then in plan order"
        } else {
            "in plan order"
        };
        writeln!(f, "Ready to start ({}), {order}:", self.ready.len())?;
        for (name, ready) in ready.iter().zip(&self.ready) {
            line(f, &format!("  {name:width$}  {}", ready.title))?;
        }
        if self.ready.is_empty() {
            writeln!(f, "  nothing")?;
        }
        // then the queue's entries passed over, each with why
        if !self.skipped.is_empty() {
            writeln!(f, "\nSkipped in the queue ({}):", self.skipped.len())?;
            for (name, entry) in skipped.iter().zip(&self.skipped) {
                line(f, &format!("  {name:width$}  {}", entry.title))?;
                for why in entry_lines(&entry.state) {
                    writeln!(f, "  {:width$}    {why}", "")?;
                }
            }
        }
        // then the work that waits, each with what holds it back
        writeln!(f, "\nWaiting ({}):", self.waiting.len())?;
        for (name, waiting) in waiting.iter().zip(&self.waiting) {
            line(f, &format!("  {name:width$}  {}", waiting.title))?;
            for needs in needs_lines(&waiting.needs) {
                writeln!(f, "  {:width$}    {needs}", "")?;
            }
        }
        if self.waiting.is_empty() {
            writeln!(f, "  nothing")?;
        }
        Ok(())
    }
}

/// Writes one line of the report, without the padding a missing title would leave at its end
///
/// # Arguments
///
/// * `f` - Where to write it
/// * `text` - The line
fn line(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    writeln!(f, "{}", text.trim_end())
}

/// Why a queue's entry is passed over or waits, a line each, in the words the report uses
///
/// # Arguments
///
/// * `state` - The entry's state
///
/// # Examples
///
/// ```
/// use reef_model::{EntryState, Status, WaitsOn};
/// use reef_model::next::entry_lines;
///
/// let needs = vec![WaitsOn { id: "C1".into(), status: Status::Todo, through: None }];
/// assert_eq!(entry_lines(&EntryState::Waiting(needs)), ["needs C1 (todo)"]);
/// assert_eq!(entry_lines(&EntryState::Missing), ["not in its plan"]);
/// ```
pub fn entry_lines(state: &EntryState) -> Vec<String> {
    match state {
        EntryState::Waiting(needs) => needs_lines(needs),
        EntryState::Blocked => vec!["blocked".to_string()],
        EntryState::Missing => vec!["not in its plan".to_string()],
        // ready, started and done entries need no word on why
        EntryState::Ready(_) | EntryState::InProgress | EntryState::Done => Vec::new(),
    }
}

/// A waiting node's unmet needs as lines: its own first, then one for each group's, each need
/// with how far along it is
///
/// # Arguments
///
/// * `needs` - A waiting node's unmet needs
///
/// # Examples
///
/// ```
/// use reef_model::{Status, WaitsOn};
/// use reef_model::next::needs_lines;
///
/// let needs = vec![
///     WaitsOn { id: "C1".into(), status: Status::Todo, through: None },
///     WaitsOn { id: "X1".into(), status: Status::InProgress, through: Some("M2".into()) },
/// ];
/// assert_eq!(needs_lines(&needs), ["needs C1 (todo)", "M2 needs X1 (in-progress)"]);
/// ```
pub fn needs_lines(needs: &[WaitsOn]) -> Vec<String> {
    by_group(needs)
        .into_iter()
        .map(|(through, needs)| {
            // each need with how far along it is
            let list: Vec<String> = needs
                .iter()
                .map(|need| format!("{} ({})", need.id, need.status.as_str()))
                .collect();
            // a group's need is named for the group
            let whose = through.map_or(String::new(), |group| format!("{group} "));
            format!("{whose}needs {}", list.join(", "))
        })
        .collect()
}

/// Unmet needs gathered by whose they are, in the order each is first met: the node's own,
/// then each of its groups'
///
/// # Arguments
///
/// * `needs` - A waiting node's unmet needs
fn by_group(needs: &[WaitsOn]) -> Vec<(Option<&str>, Vec<&WaitsOn>)> {
    let mut out: Vec<(Option<&str>, Vec<&WaitsOn>)> = Vec::new();
    for need in needs {
        // add the need to its group's line, or start a line for that group
        let through = need.through.as_deref();
        match out.iter_mut().find(|(whose, _)| *whose == through) {
            Some((_, line)) => line.push(need),
            None => out.push((through, vec![need])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Next;
    use crate::{Graph, Node, Status};

    /// A node with a title, a group, needs and a status
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `title` - Its title
    /// * `parent` - The group it is in, if any
    /// * `needs` - What it needs
    /// * `status` - Its status as written
    fn node(
        id: &str,
        title: &str,
        parent: Option<&str>,
        needs: &[&str],
        status: Option<Status>,
    ) -> Node {
        Node {
            id: id.into(),
            title: title.into(),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            status,
            ..Node::default()
        }
    }

    /// The ready work, then the waiting work with its own and its groups' needs, line by line
    #[test]
    fn ready_then_waiting() {
        let (g, d) = Graph::build(vec![
            node("X1", "X1 A spike", None, &[], Some(Status::InProgress)),
            node("C1", "C1 First", None, &[], None),
            node("M2", "M2 Store", None, &["X1"], None),
            node("C6", "C6 Schema", Some("M2"), &["C1"], None),
            node("C10", "C10 Config", None, &[], Some(Status::Done)),
        ]);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            Next::of(&g, 0).to_string(),
            "Ready to start (1), in plan order:\n  C1  C1 First\n\nWaiting (1):\n  C6  C6 Schema\n        needs C1 (todo)\n        M2 needs X1 (in-progress)\n"
        );
        // a plan with nothing to do says so
        let (g, _) = Graph::build(vec![node("a", "A", None, &[], Some(Status::Done))]);
        assert_eq!(
            Next::of(&g, 0).to_string(),
            "Ready to start (0), in plan order:\n  nothing\n\nWaiting (0):\n  nothing\n"
        );
    }

    /// The report reads back from JSON as it was, a group's need naming the group
    #[test]
    fn next_round_trips_through_json() {
        let (g, _) = Graph::build(vec![
            node("X1", "X1 A spike", None, &[], Some(Status::InProgress)),
            node("M2", "M2 Store", None, &["X1"], None),
            node("C6", "C6 Schema", Some("M2"), &[], None),
        ]);
        let next = Next::of(&g, 2);
        let json = serde_json::to_string(&next).unwrap();
        assert!(
            json.contains(r#"{"id":"X1","status":"in-progress","through":"M2"}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Next>(&json).unwrap(), next);
    }
}
