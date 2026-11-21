//! What the change stream tells of each write
//!
//! The service sends a change after each write it applied, and none for a write it refused. The
//! stream is live only: a client that connects, or connects again, reads what it needs first and
//! follows the stream after, and one that falls behind is told how many changes it missed, so it
//! reads again. Changes to one plan by two writers can arrive in either order, so each node put
//! carries its new version.

use reef_model::PlanName;
use serde::{Deserialize, Serialize};

use crate::project::Project;

/// A node a batch of puts wrote, and the version it is now at
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutNode {
    /// The node's id
    pub id: String,
    /// Its new version
    pub version: u64,
}

/// One change the service made, or a gap in the stream
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Change {
    /// A project was registered
    ProjectAdded {
        /// The project, its repo located
        project: Project,
    },
    /// A plan was listed in its project
    PlanAdded {
        /// The plan's full name
        plan: PlanName,
    },
    /// A batch of puts and removals was written into a plan's current revision
    NodesPut {
        /// The plan's full name
        plan: PlanName,
        /// Each node written, in the order the puts were given
        nodes: Vec<PutNode>,
        /// The id of each node removed, in the order the removals were given
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        removed: Vec<String>,
    },
    /// A whole plan was written as a new revision, made current
    PlanWritten {
        /// The plan's full name
        plan: PlanName,
        /// The revision readers now load
        revision: u64,
    },
    /// The stream fell behind and dropped changes, so a client should read again
    Missed {
        /// How many changes were dropped
        count: u64,
    },
}

impl Change {
    /// The name this kind of change takes, as its `kind` and as its event in the stream
    pub fn kind(&self) -> &'static str {
        match self {
            Change::ProjectAdded { .. } => "project-added",
            Change::PlanAdded { .. } => "plan-added",
            Change::NodesPut { .. } => "nodes-put",
            Change::PlanWritten { .. } => "plan-written",
            Change::Missed { .. } => "missed",
        }
    }

    /// The project this change is to, or `None` for a gap in the stream
    pub fn project(&self) -> Option<&str> {
        match self {
            Change::ProjectAdded { project } => Some(project.name.as_str()),
            Change::PlanAdded { plan }
            | Change::NodesPut { plan, .. }
            | Change::PlanWritten { plan, .. } => Some(plan.project.as_str()),
            Change::Missed { .. } => None,
        }
    }

    /// The plan this change is to, or `None` for a project's or a gap in the stream
    pub fn plan(&self) -> Option<&PlanName> {
        match self {
            Change::PlanAdded { plan }
            | Change::NodesPut { plan, .. }
            | Change::PlanWritten { plan, .. } => Some(plan),
            Change::ProjectAdded { .. } | Change::Missed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, PutNode};
    use reef_model::PlanName;
    use serde_json::json;

    /// A change names its kind beside its fields, and that kind is its event's name
    #[test]
    fn change_is_tagged_by_its_kind() {
        // a batch of one put to a plan
        let change = Change::NodesPut {
            plan: PlanName::new("reef", "central").unwrap(),
            nodes: vec![PutNode {
                id: "C53".to_string(),
                version: 2,
            }],
            removed: Vec::new(),
        };
        let value = serde_json::to_value(&change).unwrap();
        assert_eq!(
            value,
            json!({"kind": "nodes-put", "plan": "reef/central", "nodes": [{"id": "C53", "version": 2}]})
        );
        assert_eq!(value["kind"], change.kind());
        // and it reads back as itself
        assert_eq!(serde_json::from_value::<Change>(value).unwrap(), change);
        // a batch that removed nodes names them, and one that removed none leaves the key out
        let removed = Change::NodesPut {
            plan: PlanName::new("reef", "central").unwrap(),
            nodes: Vec::new(),
            removed: vec!["C62".to_string()],
        };
        let value = serde_json::to_value(&removed).unwrap();
        assert_eq!(value["removed"], json!(["C62"]));
        assert_eq!(serde_json::from_value::<Change>(value).unwrap(), removed);
    }

    /// Each change's kind is the tag serde writes for it
    #[test]
    fn kinds_match_their_tags() {
        // one change of every kind
        let plan = PlanName::new("reef", "central").unwrap();
        let changes = [
            Change::ProjectAdded {
                project: crate::Project {
                    name: "reef".to_string(),
                    repo: "/src/reef".into(),
                },
            },
            Change::PlanAdded { plan: plan.clone() },
            Change::NodesPut {
                plan: plan.clone(),
                nodes: Vec::new(),
                removed: Vec::new(),
            },
            Change::PlanWritten { plan, revision: 4 },
            Change::Missed { count: 3 },
        ];
        for change in changes {
            assert_eq!(
                serde_json::to_value(&change).unwrap()["kind"],
                change.kind()
            );
        }
    }
}
