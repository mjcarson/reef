//! Nodes from what a plan keeps: each node's spec, body and place in the order it was written
//! in, whether read from a book's files or from a store.

use crate::graph::{Diagnostic, Node};
use crate::name::NodeName;
use crate::spec::NodeSpec;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Where a node was written
///
/// In the viewer's payload a file node carries `source` and `line`, and a stored node carries
/// `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Origin {
    /// A markdown file of a book
    File {
        /// The file, relative to the book's `src`
        source: String,
        /// The line of that file the node starts on
        #[serde(default)]
        line: usize,
    },
    /// A node kept in a plan's store
    Stored {
        /// The node's full name
        name: NodeName,
    },
}

impl Default for Origin {
    /// A file with no path, for nodes built by hand
    fn default() -> Self {
        Origin::File { source: String::new(), line: 0 }
    }
}

impl Origin {
    /// The path a node's links are written relative to: its file, or for a stored node the
    /// root its plan's docs are read from
    pub fn base(&self) -> &str {
        match self {
            Origin::File { source, .. } => source,
            Origin::Stored { .. } => "",
        }
    }

    /// The file a node was written in, if it was written in one
    pub fn file(&self) -> Option<&str> {
        match self {
            Origin::File { source, .. } => Some(source),
            Origin::Stored { .. } => None,
        }
    }

    /// A problem found in a node written here
    ///
    /// # Arguments
    ///
    /// * `message` - What the problem is
    pub fn diagnostic(&self, message: String) -> Diagnostic {
        match self {
            // a file's problem is at its line
            Origin::File { source, line } => Diagnostic { source: source.clone(), line: Some(*line), message },
            // a stored node's problem is at its name
            Origin::Stored { name } => Diagnostic { source: name.to_string(), line: None, message },
        }
    }
}

impl fmt::Display for Origin {
    /// Write where a node was written: `file:line`, or its full name
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::File { source, line } => write!(f, "{source}:{line}"),
            Origin::Stored { name } => write!(f, "{name}"),
        }
    }
}

/// A node as a plan keeps it: its spec, its body, and its place in the order the plan was
/// written in
#[derive(Debug, Clone)]
pub struct Record {
    /// The node as written, with any members listed inline
    pub spec: NodeSpec,
    /// Its body as markdown: for a file node, the file after its frontmatter
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
    /// Where it was written: its file, or its full name, whose id is the spec's
    pub origin: Origin,
}

/// Turn the records a plan keeps into the nodes [`Graph::build`](crate::Graph::build) takes,
/// in the order the plan was written in
///
/// Records are put in `order`, ties kept as given, and each one's inline members follow it.
/// A stored node's members are named in its plan. A file node's members share its file and
/// its line, which the caller can move to the line each member's id is written on.
///
/// # Arguments
///
/// * `records` - The plan's records, in any order
///
/// # Examples
///
/// ```
/// use reef_model::{NodeSpec, Origin, PlanName, Record, nodes_from_records};
///
/// let plan = PlanName::new("reef", "central").unwrap();
/// let record = |id: &str, order| Record {
///     spec: NodeSpec { id: id.into(), ..NodeSpec::default() },
///     body: String::new(),
///     order,
///     origin: Origin::Stored { name: plan.node(id).unwrap() },
/// };
/// let nodes = nodes_from_records(vec![record("C2", 2), record("C1", 1)]);
/// assert_eq!(nodes[0].origin.to_string(), "reef/central/C1");
/// ```
pub fn nodes_from_records(mut records: Vec<Record>) -> Vec<Node> {
    // put the records in the order the plan was written in, ties as given
    records.sort_by_key(|r| r.order);
    // flatten each into its node and its members
    let mut nodes = Vec::with_capacity(records.len());
    for r in records {
        Node::from_spec(r.spec, r.origin, r.body, &mut nodes);
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::PlanName;
    use crate::{Graph, Payload};

    /// A stored record of `id` in `reef/central`, with members
    fn stored(id: &str, order: u64, members: &[&str]) -> Record {
        let plan = PlanName::new("reef", "central").unwrap();
        let nodes = members.iter().map(|m| NodeSpec { id: m.to_string(), ..NodeSpec::default() }).collect();
        Record {
            spec: NodeSpec { id: id.into(), needs: vec!["missing".into()], nodes, ..NodeSpec::default() },
            body: format!("# {id} title\n"),
            order,
            origin: Origin::Stored { name: plan.node(id).unwrap() },
        }
    }

    #[test]
    fn stored_records_in_written_order() {
        let nodes = nodes_from_records(vec![stored("b", 5, &[]), stored("g", 1, &["m1", "m2"]), stored("a", 5, &[])]);
        // records come out in order, ties as given, members after their group
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["g", "m1", "m2", "b", "a"]);
        // each is named in its plan, members too
        let names: Vec<String> = nodes.iter().map(|n| n.origin.to_string()).collect();
        assert_eq!(names[..3], ["reef/central/g", "reef/central/m1", "reef/central/m2"]);
        assert_eq!(nodes[1].parent.as_deref(), Some("g"));
        assert!(nodes[1].inline && !nodes[0].inline);
        // a title is still taken from the body
        assert_eq!(nodes[0].title, "g title");
        // a stored node's links are written from the root of its plan's docs
        assert_eq!(nodes[0].origin.base(), "");
        assert_eq!(nodes[0].origin.file(), None);
    }

    #[test]
    fn stored_problems_name_the_node() {
        let (_, diags) = Graph::build(nodes_from_records(vec![stored("a", 0, &[])]));
        // a problem in a stored node is at its name, with no line
        let msgs: Vec<String> = diags.iter().map(|d| d.to_string()).collect();
        assert_eq!(msgs, ["reef/central/a: needs `missing`, which is not a node"]);
    }

    #[test]
    fn file_records() {
        let spec = NodeSpec { id: "g".into(), nodes: vec![NodeSpec { id: "m".into(), ..NodeSpec::default() }], ..NodeSpec::default() };
        let origin = Origin::File { source: "plan/g.md".into(), line: 1 };
        let nodes = nodes_from_records(vec![Record { spec, body: String::new(), order: 0, origin: origin.clone() }]);
        // a file's members share its file and line
        assert_eq!(nodes[1].origin, origin);
        assert_eq!(origin.base(), "plan/g.md");
        assert_eq!(origin.to_string(), "plan/g.md:1");
        assert_eq!(origin.diagnostic("x".into()).to_string(), "plan/g.md:1: x");
    }

    #[test]
    fn origins_in_the_payload() {
        let file = Node { id: "a".into(), title: "A".into(), origin: Origin::File { source: "a.md".into(), line: 3 }, inline: true, ..Node::default() };
        let name = PlanName::new("reef", "central").unwrap().node("b").unwrap();
        let stored = Node { id: "b".into(), title: "B".into(), origin: Origin::Stored { name }, ..Node::default() };
        let json = serde_json::to_string(&Payload { nodes: vec![file, stored] }).unwrap();
        // a file node's source and line sit where they always have, and a stored node has its name
        assert_eq!(
            json,
            r#"{"nodes":[{"id":"a","title":"A","source":"a.md","line":3,"inline":true},{"id":"b","title":"B","name":"reef/central/b"}]}"#
        );
        // both read back as they were written
        let back: Payload = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        assert!(matches!(&back.nodes[1].origin, Origin::Stored { name } if name.id == "b"));
    }
}
