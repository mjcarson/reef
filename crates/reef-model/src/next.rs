//! What a plan says to do next: the work ready to start, in plan order, and what the rest waits
//! on
//!
//! [`Next`] is what `mdbook-reef next` and `reef next` print, and what the JSON API answers
//! with, so a client prints the same report the book's tool does.

use crate::graph::{Graph, Unmet};
use crate::spec::Status;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The work ready to start, in plan order, and each piece of work that waits, with what it
/// waits on
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Next {
    /// The work ready to start, in the order it is to be taken
    pub ready: Vec<Ready>,
    /// The work not started that waits, in plan order, each with the needs not done yet
    pub waiting: Vec<Waiting>,
    /// How many problems the plan has, since it still has an order with problems in it
    #[serde(default)]
    pub problems: usize,
}

/// A piece of work ready to start
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    /// The node's id
    pub id: String,
    /// The node's title
    pub title: String,
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
            .map(|i| Ready {
                id: graph.node(i).id.clone(),
                title: graph.node(i).title.clone(),
            })
            .collect();
        // then the work that waits, each need with how far along it is
        let waiting = graph
            .waiting_in_order()
            .into_iter()
            .map(|(i, unmet)| Waiting {
                id: graph.node(i).id.clone(),
                title: graph.node(i).title.clone(),
                needs: unmet.iter().map(|u| waits_on(graph, u)).collect(),
            })
            .collect();
        Next {
            ready,
            waiting,
            problems,
        }
    }
}

/// One unmet need, named by ids
///
/// # Arguments
///
/// * `graph` - The plan the need is in
/// * `unmet` - The need, and the group whose need it is
fn waits_on(graph: &Graph, unmet: &Unmet) -> WaitsOn {
    WaitsOn {
        id: graph.node(unmet.need).id.clone(),
        status: graph.status(unmet.need),
        through: unmet.through.map(|t| graph.node(t).id.clone()),
    }
}

impl fmt::Display for Next {
    /// The report: the work ready to start, in plan order, then each piece of work that waits,
    /// with a line for its own needs and one for each of its groups' that are not done
    ///
    /// # Arguments
    ///
    /// * `f` - Where to write it
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ids are padded to the widest, so titles line up
        let ids = self.ready.iter().map(|r| &r.id);
        let width = ids
            .chain(self.waiting.iter().map(|w| &w.id))
            .map(|id| id.chars().count())
            .max()
            .unwrap_or(0);
        // the ready work first, in the order it is to be taken
        writeln!(f, "Ready to start ({}), in plan order:", self.ready.len())?;
        for ready in &self.ready {
            writeln!(f, "  {:width$}  {}", ready.id, ready.title)?;
        }
        if self.ready.is_empty() {
            writeln!(f, "  nothing")?;
        }
        // then the work that waits, each with what holds it back
        writeln!(f, "\nWaiting ({}):", self.waiting.len())?;
        for waiting in &self.waiting {
            writeln!(f, "  {:width$}  {}", waiting.id, waiting.title)?;
            for (through, needs) in by_group(&waiting.needs) {
                // each need with how far along it is
                let list: Vec<String> = needs
                    .iter()
                    .map(|need| format!("{} ({})", need.id, need.status.as_str()))
                    .collect();
                // a group's need is named for the group
                let whose = through.map_or(String::new(), |group| format!("{group} "));
                writeln!(f, "  {:width$}    {whose}needs {}", "", list.join(", "))?;
            }
        }
        if self.waiting.is_empty() {
            writeln!(f, "  nothing")?;
        }
        Ok(())
    }
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
