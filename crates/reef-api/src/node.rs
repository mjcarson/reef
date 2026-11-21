//! Nodes as the service reads and writes them, and what came of a write
//!
//! Each put names the version its writer read, or `None` for a new node, and each removal the
//! version its writer read, so a write made against a node that has moved on is refused as stale,
//! an answer and not a failure. A write that would add a problem to the plan is refused with
//! check's messages. Either refusal writes nothing.
//!
//! Enums take serde's external tagging, their variants in kebab-case, so an answer reads as
//! `{"applied":{"versions":[1]}}` or `{"refused":[...]}`.

use reef_model::{Diagnostic, NameError, NodeSpec, Origin, PlanName, Record};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// One removal of a batch: a node's id, and the version its writer read it at
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRemove {
    /// The id of the node to remove
    pub id: String,
    /// The version it was read at, which it must still be at to be removed
    pub read: u64,
}

impl NodeRemove {
    /// A removal of a node, at the version it was read at
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the node to remove
    /// * `read` - The version it was read at
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::NodeRemove;
    ///
    /// let remove = NodeRemove::new("C62", 3);
    /// assert_eq!(remove.id, "C62");
    /// ```
    pub fn new<T: Into<String>>(id: T, read: u64) -> NodeRemove {
        NodeRemove {
            id: id.into(),
            read,
        }
    }
}

/// A batch of puts and removals to one plan, checked as one change
///
/// A group and its members go in together, and come out together, since either alone breaks
/// the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Puts {
    /// The nodes to write, each with the version its writer read
    pub puts: Vec<NodePut>,
    /// The nodes to remove, each with the version its writer read
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removes: Vec<NodeRemove>,
}

impl Puts {
    /// A batch of puts alone
    ///
    /// # Arguments
    ///
    /// * `puts` - The nodes to write, each with the version its writer read
    pub fn new(puts: Vec<NodePut>) -> Puts {
        Puts {
            puts,
            removes: Vec::new(),
        }
    }

    /// A batch of removals alone
    ///
    /// # Arguments
    ///
    /// * `removes` - The nodes to remove, each with the version its writer read
    pub fn removing(removes: Vec<NodeRemove>) -> Puts {
        Puts {
            puts: Vec::new(),
            removes,
        }
    }
}

/// Lifts a spec's inline members out of it, as the store keeps them: the spec first, then each
/// member after the node that lists it, depth first
///
/// A member belongs to the node that lists it unless it names a group of its own, so each is
/// given that node's id as its `group`. Its `body` stays in its spec, as it was written.
///
/// # Arguments
///
/// * `spec` - The spec, its members inline
///
/// # Examples
///
/// ```
/// use reef_api::lift;
/// use reef_model::NodeSpec;
///
/// let member = NodeSpec { id: "C11".into(), ..NodeSpec::default() };
/// let group = NodeSpec { id: "M3".into(), nodes: vec![member], ..NodeSpec::default() };
/// let lifted = lift(group);
/// assert_eq!(lifted[1].group.as_deref(), Some("M3"));
/// ```
pub fn lift(spec: NodeSpec) -> Vec<NodeSpec> {
    let mut out = Vec::new();
    lift_into(spec, &mut out);
    out
}

/// The puts a node's YAML makes: the node, at the version its writer read, then each member
/// lifted out of it, each a new node
///
/// A node the plan has keeps its place in the written order and its body. A new node goes after
/// the plan's last, each member after the node before it, with an empty body.
///
/// # Arguments
///
/// * `listing` - The plan as it stands
/// * `specs` - The node's spec, then each member lifted out of it
/// * `read` - The version the node was got at, or `None` for a new node
///
/// # Examples
///
/// ```
/// use reef_api::{Listing, puts_for};
/// use reef_model::NodeSpec;
///
/// let listing = Listing { revision: 1, nodes: Vec::new() };
/// let spec = NodeSpec { id: "C17".into(), ..NodeSpec::default() };
/// let puts = puts_for(&listing, vec![spec], None);
/// assert_eq!((puts[0].node.order, puts[0].read), (0, None));
/// ```
pub fn puts_for(listing: &Listing, specs: Vec<NodeSpec>, read: Option<u64>) -> Vec<NodePut> {
    // the nodes the plan has, by id, and the first place after its last
    let stored: HashMap<&str, &StoredNode> = listing
        .nodes
        .iter()
        .map(|node| (node.spec.id.as_str(), node))
        .collect();
    let mut next = listing
        .nodes
        .iter()
        .map(|node| node.order + 1)
        .max()
        .unwrap_or(0);
    specs
        .into_iter()
        .enumerate()
        .map(|(i, spec)| {
            // the node the YAML is of names the version it was got at, and its members are new
            let read = if i == 0 { read } else { None };
            // a stored node keeps its place and body, and a new one goes after the last
            let (order, body) = match stored.get(spec.id.as_str()) {
                Some(node) => (node.order, node.body.clone()),
                None => {
                    next += 1;
                    (next - 1, String::new())
                }
            };
            NodePut::new(NodeWrite { spec, body, order }, read)
        })
        .collect()
}

/// Pushes a spec, then each of its members after it, depth first
///
/// # Arguments
///
/// * `spec` - The spec to push, members and all
/// * `out` - The lifted specs so far
fn lift_into(mut spec: NodeSpec, out: &mut Vec<NodeSpec>) {
    // take its members out of it, so the store holds it alone
    let members = std::mem::take(&mut spec.nodes);
    let id = spec.id.clone();
    out.push(spec);
    for mut member in members {
        // a member belongs to the node that lists it unless it names a group
        if member.group.is_none() {
            member.group = Some(id.clone());
        }
        lift_into(member, out);
    }
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
    use super::{
        Checked, NodePut, NodeRemove, NodeWrite, Puts, Refusal, Rewrite, Rewritten, StalePut, lift,
    };
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

    /// A batch with removals reads back the same, and one sent without them has none
    #[test]
    fn removals_ride_in_the_batch() {
        // a put and a removal, as the API sends them
        let batch = Puts {
            puts: vec![NodePut::new(NodeWrite::new("a", 0), None)],
            removes: vec![NodeRemove::new("b", 2)],
        };
        let value = serde_json::to_value(&batch).expect("the batch serializes");
        assert_eq!(value["removes"], json!([{"id": "b", "read": 2}]));
        assert_eq!(serde_json::from_value::<Puts>(value).unwrap(), batch);
        // a batch of puts alone leaves the key out, and one written without it reads as none
        let puts = Puts::new(vec![NodePut::new(NodeWrite::new("a", 0), None)]);
        let value = serde_json::to_value(&puts).expect("the batch serializes");
        assert!(value.get("removes").is_none(), "{value}");
        let read: Puts = serde_json::from_value(json!({"puts": []})).unwrap();
        assert!(read.removes.is_empty());
    }

    /// Inline members lift into nodes of their own after the node that lists them, their group
    /// filled in unless they name one
    #[test]
    fn inline_members_lift_with_their_group() {
        // a group listing a member with members of its own, and one that names another group
        let spec = |id: &str, nodes: Vec<NodeSpec>| NodeSpec {
            id: id.to_string(),
            nodes,
            ..NodeSpec::default()
        };
        let mut elsewhere = spec("m2", Vec::new());
        elsewhere.group = Some("G2".to_string());
        let group = spec(
            "G",
            vec![spec("m1", vec![spec("m1a", Vec::new())]), elsewhere],
        );
        // each comes out alone, depth first, after the node that lists it
        let lifted = lift(group);
        let ids: Vec<&str> = lifted.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["G", "m1", "m1a", "m2"]);
        assert!(lifted.iter().all(|node| node.nodes.is_empty()));
        // each in the group that lists it, unless it named its own
        let groups: Vec<Option<&str>> = lifted.iter().map(|node| node.group.as_deref()).collect();
        assert_eq!(groups, [None, Some("G"), Some("m1"), Some("G2")]);
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
