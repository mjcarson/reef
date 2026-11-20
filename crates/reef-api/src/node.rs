//! Nodes as the service reads and writes them, and what came of a write
//!
//! Each put names the version its writer read, or `None` for a new node, so a write made against
//! a node that has moved on is refused as stale, an answer and not a failure. A write that would
//! add a problem to the plan is refused with check's messages. Either refusal writes nothing.
//!
//! Enums take serde's external tagging, their variants in kebab-case, so an answer reads as
//! `{"applied":{"versions":[1]}}` or `{"refused":[...]}`.

use reef_model::{Diagnostic, NameError, NodeSpec, Origin, PlanName, Record};
use serde::{Deserialize, Serialize};

/// A node as a reader got it from the store
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredNode {
    /// Its spec, with no inline members
    pub spec: NodeSpec,
    /// Its body as markdown: a file node's page, empty for one that was listed inline
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
    /// The version it was read at, which a put of it names
    pub version: u64,
}

/// A node as a writer gives it to the store
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeWrite {
    /// Its spec, with no inline members: each member is a node of its own
    pub spec: NodeSpec,
    /// Its body as markdown
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
}

impl NodeWrite {
    /// A node with only an id, an empty body and an order
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `order` - Where it falls in the order its plan was written in
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::NodeWrite;
    ///
    /// let node = NodeWrite::new("C53", 2);
    /// assert_eq!(node.spec.id, "C53");
    /// ```
    pub fn new<T: Into<String>>(id: T, order: u64) -> NodeWrite {
        NodeWrite {
            spec: NodeSpec {
                id: id.into(),
                ..NodeSpec::default()
            },
            body: String::new(),
            order,
        }
    }
}

impl From<StoredNode> for NodeWrite {
    /// The node as a writer would put it back, without the version it was read at
    ///
    /// # Arguments
    ///
    /// * `node` - The node as it was read
    fn from(node: StoredNode) -> NodeWrite {
        NodeWrite {
            spec: node.spec,
            body: node.body,
            order: node.order,
        }
    }
}

/// One put of a batch: a node, and the version its writer read it at
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePut {
    /// The node as it should be written
    pub node: NodeWrite,
    /// The version it was read at, or `None` for a node that must not exist yet
    #[serde(default)]
    pub read: Option<u64>,
}

impl NodePut {
    /// A put of a node, over the version it was read at
    ///
    /// # Arguments
    ///
    /// * `node` - The node as it should be written
    /// * `read` - The version it was read at, or `None` for a new node
    pub fn new(node: NodeWrite, read: Option<u64>) -> NodePut {
        NodePut { node, read }
    }
}

/// A batch of puts to one plan, checked as one change
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Puts {
    /// The nodes to write, each with the version its writer read
    pub puts: Vec<NodePut>,
}

/// A whole plan, written as a new revision
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanWrite {
    /// Every node of the plan, each a node of its own
    pub nodes: Vec<NodeWrite>,
}

/// Why a conditional write was refused, as shoal says it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refusal {
    /// The write expected no row, and one is stored
    Exists,
    /// The write expected a row, and none is stored
    Missing,
    /// The write expected a row at a version, and the stored row is at another
    Moved,
}

impl Refusal {
    /// Name this refusal the way a person reads it
    pub fn describe(self) -> &'static str {
        match self {
            Refusal::Exists => "it already exists",
            Refusal::Missing => "it does not exist",
            Refusal::Moved => "it changed since it was read",
        }
    }
}

impl std::fmt::Display for Refusal {
    /// Shows the refusal as the phrase that describes it
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

/// A put of a batch that found its node other than its writer read it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StalePut {
    /// The node's id
    pub id: String,
    /// How the node differs from what its writer read
    pub refusal: Refusal,
}

/// What came of a checked batch of puts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Checked {
    /// Every put was written
    Applied {
        /// Each put's new version, in the order the puts were given
        versions: Vec<u64>,
    },
    /// Some puts found their nodes other than their writers read them, so nothing was written
    Stale(Vec<StalePut>),
    /// The check found problems the puts would add, so nothing was written
    Refused(Vec<Diagnostic>),
}

/// What came of a checked whole-plan write
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rewrite {
    /// The plan was written as a new revision, made current
    Applied(Rewritten),
    /// The check found problems the write would add, so nothing was written
    Refused(Vec<Diagnostic>),
}

/// What a whole-plan write did
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rewritten {
    /// The revision it made current
    pub revision: u64,
    /// How many rows a killed or failed write had left behind, which it pruned first
    pub recovered: usize,
    /// Whether pruning the revision it replaced failed, leaving that to the next write
    pub left_retired: bool,
}

/// A plan's current revision, as a reader got it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    /// The revision that was current
    pub revision: u64,
    /// Its nodes, in the order the plan was written in
    pub nodes: Vec<StoredNode>,
}

impl Listing {
    /// The records these nodes make, each named in its plan, for `Graph::build`
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan these nodes were read from
    pub fn records(self, plan: &PlanName) -> Result<Vec<Record>, NameError> {
        // each node becomes a stored record, named by its plan and id
        records(plan, self.nodes.into_iter().map(NodeWrite::from).collect())
    }
}

/// The records nodes make, each named in its plan, for `Graph::build`
///
/// # Arguments
///
/// * `plan` - The plan the nodes are part of
/// * `nodes` - The nodes, in written order
pub fn records(plan: &PlanName, nodes: Vec<NodeWrite>) -> Result<Vec<Record>, NameError> {
    // each node becomes a stored record, named by its plan and id
    nodes
        .into_iter()
        .map(|node| {
            let name = plan.node(node.spec.id.as_str())?;
            Ok(Record {
                spec: node.spec,
                body: node.body,
                order: node.order,
                origin: Origin::Stored { name },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Checked, NodePut, NodeWrite, Refusal, Rewrite, Rewritten, StalePut};
    use reef_model::{Diagnostic, NodeSpec};
    use serde_json::json;

    /// A spec with every kind of key a node is written with, as JSON
    fn full_spec() -> serde_json::Value {
        json!({
            "id": "C53",
            "title": "C53 The JSON API on axum",
            "status": "in-progress",
            "optional": true,
            "group": "M3",
            "needs": ["C11"],
            "helps": ["C40"],
            "docs": ["../design/app.md#the-shell"],
            "description": {
                "why": "Nothing outside the process can reach the service.",
                "what": "Plain JSON routes, typed in `reef-api`.",
                "details": "A small binary serves them.",
            },
            "acceptance": [
                {"integration": "A client puts a node", "test": "tests/api.rs::a", "status": "met"},
                {"kind": "performance", "text": "A payload is timed", "metric": "plan-payload-ms"},
            ],
            "performance": {
                "labels": ["In process", "Over HTTP"],
                "note": "Ten rounds.",
                "metrics": [{
                    "name": "A plan's payload, ms",
                    "key": "plan-payload-ms",
                    "unit": "ms",
                    "before": [1.0, 0.9, 1.1],
                    "after": 1.5,
                }],
            },
        })
    }

    /// A node with every kind of key reads back the same after a trip through the API's JSON
    #[test]
    fn spec_round_trips_through_json() {
        // the spec as a client would send it
        let spec: NodeSpec = serde_json::from_value(full_spec()).expect("the spec deserializes");
        // sent as the API sends it, and read back as a client reads it
        let put = NodePut::new(
            NodeWrite {
                spec: spec.clone(),
                body: "# C53\n".to_string(),
                order: 3,
            },
            Some(2),
        );
        let text = serde_json::to_string(&put).expect("the put serializes");
        let back: NodePut = serde_json::from_str(&text).expect("the put deserializes");
        assert_eq!(back, put);
        // and nothing the author wrote was lost on the way
        assert_eq!(back.node.spec.acceptance.len(), 2);
        assert!(back.node.spec.performance.is_some());
        assert!(back.node.spec.description.is_some());
    }

    /// A put that names no version reads as a new node's
    #[test]
    fn put_without_read_is_new() {
        // a client may leave `read` out for a node that must not exist yet
        let put: NodePut =
            serde_json::from_value(json!({"node": {"spec": {"id": "a"}, "body": "", "order": 0}}))
                .expect("the put deserializes");
        assert_eq!(put, NodePut::new(NodeWrite::new("a", 0), None));
    }

    /// Each outcome's JSON is pinned, since clients match on it
    #[test]
    fn outcomes_are_externally_tagged_in_kebab_case() {
        // an applied batch names each put's version
        let applied = Checked::Applied { versions: vec![1] };
        assert_eq!(
            serde_json::to_value(&applied).unwrap(),
            json!({"applied": {"versions": [1]}})
        );
        // a stale batch names each stale put and why
        let stale = Checked::Stale(vec![StalePut {
            id: "a".to_string(),
            refusal: Refusal::Moved,
        }]);
        assert_eq!(
            serde_json::to_value(&stale).unwrap(),
            json!({"stale": [{"id": "a", "refusal": "moved"}]})
        );
        // a refused write carries check's problems
        let refused = Rewrite::Refused(vec![Diagnostic {
            source: "reef/central/a".to_string(),
            line: None,
            message: "needs `x`, which is not a node".to_string(),
        }]);
        assert_eq!(
            serde_json::to_value(&refused).unwrap(),
            json!({"refused": [{
                "source": "reef/central/a",
                "line": null,
                "message": "needs `x`, which is not a node",
            }]})
        );
        // and an applied rewrite names the revision it made current
        let rewritten = Rewrite::Applied(Rewritten {
            revision: 7,
            recovered: 0,
            left_retired: false,
        });
        assert_eq!(
            serde_json::to_value(&rewritten).unwrap(),
            json!({"applied": {"revision": 7, "recovered": 0, "left_retired": false}})
        );
        // each reads back as itself
        for outcome in [applied, stale] {
            let text = serde_json::to_string(&outcome).unwrap();
            assert_eq!(serde_json::from_str::<Checked>(&text).unwrap(), outcome);
        }
    }
}
