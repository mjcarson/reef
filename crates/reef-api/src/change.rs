//! What the change stream tells of each write
//!
//! The service sends a change after each write it applied, and none for a write it refused. The
//! stream is live only: a client that connects, or connects again, reads what it needs first and
//! follows the stream after, and one that falls behind is told how many changes it missed, so it
//! reads again. Changes to one plan by two writers can arrive in either order, so each node or
//! page put carries its new version, and each work record written its own.

use reef_model::{NodeName, PlanName};
use serde::{Deserialize, Serialize};

use crate::comment::{ReviewState, ThreadState};
use crate::loops::Pane;
use crate::project::Project;

/// A node a batch of puts wrote, and the version it is now at
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutNode {
    /// The node's id
    pub id: String,
    /// Its new version
    pub version: u64,
}

/// A thread a capture followed to its node's latest revision, and the version it is now at
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowedThread {
    /// The thread's number
    pub thread: u32,
    /// Its new version
    pub version: u64,
    /// Whether its line was lost on the way, so it is now outdated
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outdated: bool,
}

/// A page an edit wrote, and the version it is now at
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutPage {
    /// The page's path
    pub path: String,
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
    /// A remote project was fetched and its branch moved, so its docs are read at a new commit
    ///
    /// Every plan of the project reads its docs there, so each plan's followers hear of it.
    ProjectFetched {
        /// The project's name
        project: String,
        /// The commit its docs are now read at
        commit: String,
    },
    /// A plan was listed in its project
    PlanAdded {
        /// The plan's full name
        plan: PlanName,
    },
    /// A batch of puts and removals was written to a plan as one change, every node it didn't
    /// touch at the version it was at
    NodesPut {
        /// The plan's full name
        plan: PlanName,
        /// Each node written, in the order the puts were given
        nodes: Vec<PutNode>,
        /// The id of each node removed, in the order the removals were given
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        removed: Vec<String>,
    },
    /// Pages of a plan were written or taken out as one change, every page it didn't touch at
    /// the version it was at
    PagesPut {
        /// The plan's full name
        plan: PlanName,
        /// Each page written
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pages: Vec<PutPage>,
        /// The path of each page taken out
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
    /// A project's queue was edited: an entry added, moved or taken off
    QueueChanged {
        /// The project whose queue it is
        project: String,
    },
    /// A node's work record was written: the node started, or a capture's SHAs added
    WorkRecorded {
        /// The node whose record it is
        node: NodeName,
        /// The version the record is now at
        version: u64,
    },
    /// A capture of a node's changes was kept as a new revision, or over its draft, which may
    /// now be sealed
    RevisionCaptured {
        /// The node whose changes it holds
        node: NodeName,
        /// The revision's number
        revision: u32,
    },
    /// A comment was kept: a new thread's first, or a reply
    CommentAdded {
        /// The node its thread is on
        node: NodeName,
        /// Its thread's number
        thread: u32,
        /// Its own number
        comment: u32,
    },
    /// A thread was marked open or resolved
    ThreadMarked {
        /// The node it is on
        node: NodeName,
        /// Its number
        thread: u32,
        /// The state it is now in
        state: ThreadState,
        /// The version it is now at
        version: u64,
    },
    /// A capture followed the threads on its node's lines to the node's latest revision
    ThreadsFollowed {
        /// The node they are on
        node: NodeName,
        /// The revision they were followed to
        revision: u32,
        /// Each thread it wrote, by number
        threads: Vec<FollowedThread>,
    },
    /// A node's review was submitted, its comments published at once, or discarded
    ReviewMarked {
        /// The node it reviews
        node: NodeName,
        /// The review's number
        review: u32,
        /// What became of it
        state: ReviewState,
        /// The version its row is at now
        version: u64,
    },
    /// A pane's loop was started, changed or stopped
    LoopChanged {
        /// The project the loop feeds
        project: String,
        /// The pane whose loop it is
        pane: Pane,
    },
    /// A node's decisions were kept: new ones, or answers to ones kept before
    DecisionsRecorded {
        /// The node
        node: NodeName,
    },
    /// A runner's session attached, reported, was acted on, or let go
    SessionChanged {
        /// The project its runner takes nodes from
        project: String,
        /// The session's id
        session: String,
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
            Change::ProjectFetched { .. } => "project-fetched",
            Change::PlanAdded { .. } => "plan-added",
            Change::NodesPut { .. } => "nodes-put",
            Change::PagesPut { .. } => "pages-put",
            Change::PlanWritten { .. } => "plan-written",
            Change::QueueChanged { .. } => "queue-changed",
            Change::WorkRecorded { .. } => "work-recorded",
            Change::RevisionCaptured { .. } => "revision-captured",
            Change::CommentAdded { .. } => "comment-added",
            Change::ThreadMarked { .. } => "thread-marked",
            Change::ThreadsFollowed { .. } => "threads-followed",
            Change::ReviewMarked { .. } => "review-marked",
            Change::LoopChanged { .. } => "loop-changed",
            Change::DecisionsRecorded { .. } => "decisions-recorded",
            Change::SessionChanged { .. } => "session-changed",
            Change::Missed { .. } => "missed",
        }
    }

    /// The project this change is to, or `None` for a gap in the stream
    pub fn project(&self) -> Option<&str> {
        match self {
            Change::ProjectAdded { project } => Some(project.name.as_str()),
            Change::PlanAdded { plan }
            | Change::NodesPut { plan, .. }
            | Change::PagesPut { plan, .. }
            | Change::PlanWritten { plan, .. } => Some(plan.project.as_str()),
            Change::QueueChanged { project }
            | Change::ProjectFetched { project, .. }
            | Change::LoopChanged { project, .. }
            | Change::SessionChanged { project, .. } => Some(project.as_str()),
            Change::WorkRecorded { node, .. }
            | Change::RevisionCaptured { node, .. }
            | Change::CommentAdded { node, .. }
            | Change::ThreadMarked { node, .. }
            | Change::ThreadsFollowed { node, .. }
            | Change::ReviewMarked { node, .. }
            | Change::DecisionsRecorded { node } => Some(node.plan.project.as_str()),
            Change::Missed { .. } => None,
        }
    }

    /// The plan this change is to, or `None` for a project's or a gap in the stream
    ///
    /// A queue spans its project's plans, so its change is the project's.
    pub fn plan(&self) -> Option<&PlanName> {
        match self {
            Change::PlanAdded { plan }
            | Change::NodesPut { plan, .. }
            | Change::PagesPut { plan, .. }
            | Change::PlanWritten { plan, .. } => Some(plan),
            Change::WorkRecorded { node, .. }
            | Change::RevisionCaptured { node, .. }
            | Change::CommentAdded { node, .. }
            | Change::ThreadMarked { node, .. }
            | Change::ThreadsFollowed { node, .. }
            | Change::ReviewMarked { node, .. }
            | Change::DecisionsRecorded { node } => Some(&node.plan),
            Change::ProjectAdded { .. }
            | Change::ProjectFetched { .. }
            | Change::QueueChanged { .. }
            | Change::LoopChanged { .. }
            | Change::SessionChanged { .. }
            | Change::Missed { .. } => None,
        }
    }

    /// Whether this change is its project's but concerns each of its plans, so a follower of
    /// any one plan of the project hears of it
    ///
    /// A fetch moves the commit every plan of the project reads its docs at.
    pub fn reaches_every_plan(&self) -> bool {
        matches!(self, Change::ProjectFetched { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, PutNode, PutPage};
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
        // a page edit names the page it wrote, or the one it took out
        let page = Change::PagesPut {
            plan: PlanName::new("reef", "central").unwrap(),
            pages: Vec::new(),
            removed: vec!["plan.md".to_string()],
        };
        let value = serde_json::to_value(&page).unwrap();
        assert_eq!(
            value,
            json!({"kind": "pages-put", "plan": "reef/central", "removed": ["plan.md"]})
        );
        assert_eq!(serde_json::from_value::<Change>(value).unwrap(), page);
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
                    remote: None,
                },
            },
            Change::ProjectFetched {
                project: "reef".to_string(),
                commit: "0123456".to_string(),
            },
            Change::PlanAdded { plan: plan.clone() },
            Change::NodesPut {
                plan: plan.clone(),
                nodes: Vec::new(),
                removed: Vec::new(),
            },
            Change::PagesPut {
                plan: plan.clone(),
                pages: vec![PutPage {
                    path: "design/app.md".to_string(),
                    version: 2,
                }],
                removed: Vec::new(),
            },
            Change::PlanWritten { plan, revision: 4 },
            Change::QueueChanged {
                project: "reef".to_string(),
            },
            Change::WorkRecorded {
                node: "reef/central/C30".parse().unwrap(),
                version: 2,
            },
            Change::RevisionCaptured {
                node: "reef/central/C32".parse().unwrap(),
                revision: 1,
            },
            Change::CommentAdded {
                node: "reef/central/C34".parse().unwrap(),
                thread: 1,
                comment: 2,
            },
            Change::ThreadMarked {
                node: "reef/central/C34".parse().unwrap(),
                thread: 1,
                state: crate::ThreadState::Resolved,
                version: 2,
            },
            Change::ThreadsFollowed {
                node: "reef/central/C36".parse().unwrap(),
                revision: 3,
                threads: vec![super::FollowedThread {
                    thread: 1,
                    version: 3,
                    outdated: true,
                }],
            },
            Change::ReviewMarked {
                node: "reef/central/C73".parse().unwrap(),
                review: 4,
                state: crate::ReviewState::Submitted,
                version: 2,
            },
            Change::Missed { count: 3 },
        ];
        for change in changes {
            assert_eq!(
                serde_json::to_value(&change).unwrap()["kind"],
                change.kind()
            );
        }
    }

    /// A fetch is its project's change, and reaches every plan of it
    #[test]
    fn a_fetch_reaches_every_plan_of_its_project() {
        // a fetch names its project and no plan
        let fetched = Change::ProjectFetched {
            project: "reef".to_string(),
            commit: "0123456".to_string(),
        };
        assert_eq!(fetched.project(), Some("reef"));
        assert_eq!(fetched.plan(), None);
        assert!(fetched.reaches_every_plan());
        // a queue's change is its project's too, but no plan's
        let queue = Change::QueueChanged {
            project: "reef".to_string(),
        };
        assert!(!queue.reaches_every_plan());
    }
}
