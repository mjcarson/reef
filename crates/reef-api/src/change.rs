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
    /// A remote project was fetched and a branch of it moved, so the plans that follow that
    /// branch read their docs at a new commit
    ///
    /// Only those plans' followers hear of it. One from a service before plans followed
    /// branches names no plans, and reaches every plan of the project.
    ProjectFetched {
        /// The project's name
        project: String,
        /// The commit the branch is now at
        commit: String,
        /// The branch that moved, or `None` from a service that fetched one branch alone
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// The plans whose docs are read at that branch, or `None` for every plan
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plans: Option<Vec<String>>,
    },
    /// A plan was listed in its project
    PlanAdded {
        /// The plan's full name
        plan: PlanName,
    },
    /// The branch a plan follows was set, changed or dropped, so its docs are read elsewhere
    PlanBranched {
        /// The plan's full name
        plan: PlanName,
        /// The branch it follows now, or `None` for its project's own
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
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
    /// A plan's queue was edited: an entry added, moved or taken off
    ///
    /// Only that plan's followers hear of it, and its project's. One from a service before each
    /// plan had a queue of its own names no plan, and is its project's alone.
    QueueChanged {
        /// The project whose queue it is
        project: String,
        /// The plan whose entries it edited, or `None` from a service that named none
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<PlanName>,
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
    /// A test run was opened, or opened again, or its client reported
    TestRunChanged {
        /// The project whose tests it runs
        project: String,
        /// The run's id
        run: String,
        /// The plan of the node its checkout is on, or `None` for a run on no node, which every
        /// plan of the project hears of
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<PlanName>,
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
            Change::PlanBranched { .. } => "plan-branched",
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
            Change::TestRunChanged { .. } => "test-run-changed",
            Change::Missed { .. } => "missed",
        }
    }

    /// The project this change is to, or `None` for a gap in the stream
    pub fn project(&self) -> Option<&str> {
        match self {
            Change::ProjectAdded { project } => Some(project.name.as_str()),
            Change::PlanAdded { plan }
            | Change::PlanBranched { plan, .. }
            | Change::NodesPut { plan, .. }
            | Change::PagesPut { plan, .. }
            | Change::PlanWritten { plan, .. } => Some(plan.project.as_str()),
            Change::QueueChanged { project, .. }
            | Change::ProjectFetched { project, .. }
            | Change::LoopChanged { project, .. }
            | Change::SessionChanged { project, .. }
            | Change::TestRunChanged { project, .. } => Some(project.as_str()),
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
    /// Each plan has a queue of its own, so its queue's change is the plan's, unless it came
    /// from a service that named no plan.
    pub fn plan(&self) -> Option<&PlanName> {
        match self {
            Change::QueueChanged { plan, .. } | Change::TestRunChanged { plan, .. } => {
                plan.as_ref()
            }
            Change::PlanAdded { plan }
            | Change::PlanBranched { plan, .. }
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
            | Change::LoopChanged { .. }
            | Change::SessionChanged { .. }
            | Change::Missed { .. } => None,
        }
    }

    /// Whether this change is its project's but concerns one of its plans, so a follower of
    /// that plan alone hears of it
    ///
    /// A fetch moves the commit the plans that follow its branch read their docs at, or every
    /// plan's, from a service that names none.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's name within the change's project
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::Change;
    ///
    /// let fetched = Change::ProjectFetched {
    ///     project: "reef".to_string(),
    ///     commit: "0123456".to_string(),
    ///     branch: Some("dev".to_string()),
    ///     plans: Some(vec!["app".to_string()]),
    /// };
    /// assert!(fetched.reaches("app"));
    /// assert!(!fetched.reaches("central"));
    /// ```
    pub fn reaches(&self, plan: &str) -> bool {
        match self {
            // the plans the fetch names, or every plan where it names none
            Change::ProjectFetched { plans, .. } => plans
                .as_ref()
                .is_none_or(|plans| plans.iter().any(|name| name == plan)),
            // a run on no node is every plan's to show
            Change::TestRunChanged { plan: None, .. } => true,
            // no other project's change concerns a plan
            _ => false,
        }
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
                    registered: None,
                    name: "reef".to_string(),
                    repo: "/src/reef".into(),
                    remote: None,
                },
            },
            Change::ProjectFetched {
                project: "reef".to_string(),
                commit: "0123456".to_string(),
                branch: None,
                plans: None,
            },
            Change::PlanAdded { plan: plan.clone() },
            Change::PlanBranched {
                plan: plan.clone(),
                branch: Some("dev".to_string()),
            },
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
                plan: Some(PlanName::new("reef", "central").unwrap()),
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
            Change::TestRunChanged {
                project: "reef".to_string(),
                run: "r1".to_string(),
                plan: Some(PlanName::new("reef", "central").unwrap()),
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

    /// A fetch is its project's change, and reaches the plans it names, or every plan of its
    /// project from a service that names none
    #[test]
    fn a_fetch_reaches_the_plans_that_follow_its_branch() {
        // a fetch from a service that fetched one branch names no plan, and reaches every one
        let fetched = Change::ProjectFetched {
            project: "reef".to_string(),
            commit: "0123456".to_string(),
            branch: None,
            plans: None,
        };
        assert_eq!(fetched.project(), Some("reef"));
        assert_eq!(fetched.plan(), None);
        assert!(fetched.reaches("central"));
        // and it reads back from that service's body, which has neither key
        let old = json!({"kind": "project-fetched", "project": "reef", "commit": "0123456"});
        assert_eq!(serde_json::from_value::<Change>(old).unwrap(), fetched);
        // one that names its branch's plans reaches only those
        let narrowed = Change::ProjectFetched {
            project: "reef".to_string(),
            commit: "0123456".to_string(),
            branch: Some("dev".to_string()),
            plans: Some(vec!["app".to_string()]),
        };
        assert!(narrowed.reaches("app"));
        assert!(!narrowed.reaches("central"));
        // and one that names none reaches none
        let none = Change::ProjectFetched {
            project: "reef".to_string(),
            commit: "0123456".to_string(),
            branch: Some("gone".to_string()),
            plans: Some(Vec::new()),
        };
        assert!(!none.reaches("central"));
        // a queue's change from a service that named no plan is its project's alone
        let queue = Change::QueueChanged {
            project: "reef".to_string(),
            plan: None,
        };
        assert_eq!(queue.plan(), None);
        assert!(!queue.reaches("central"));
        let old = json!({"kind": "queue-changed", "project": "reef"});
        assert_eq!(serde_json::from_value::<Change>(old).unwrap(), queue);
        // and one naming its plan is that plan's
        let queue = Change::QueueChanged {
            project: "reef".to_string(),
            plan: Some(PlanName::new("reef", "app").unwrap()),
        };
        assert_eq!(queue.plan().map(|plan| plan.plan.as_str()), Some("app"));
        // and a plan's branch changing is that plan's
        let branched = Change::PlanBranched {
            plan: PlanName::new("reef", "app").unwrap(),
            branch: None,
        };
        assert_eq!(branched.plan().map(|plan| plan.plan.as_str()), Some("app"));
    }

    /// A test run on a node is its plan's, and one on no node reaches every plan of its project
    #[test]
    fn a_test_run_reaches_its_plan_or_every_plan() {
        // a run on a node of `app` is that plan's alone
        let on_node = Change::TestRunChanged {
            project: "reef".to_string(),
            run: "r1".to_string(),
            plan: Some(PlanName::new("reef", "app").unwrap()),
        };
        assert_eq!(on_node.project(), Some("reef"));
        assert_eq!(on_node.plan().map(|plan| plan.plan.as_str()), Some("app"));
        assert!(!on_node.reaches("central"));
        // and a run on no node is no plan's, but every plan hears of it
        let on_none = Change::TestRunChanged {
            project: "reef".to_string(),
            run: "r2".to_string(),
            plan: None,
        };
        assert_eq!(on_none.plan(), None);
        assert!(on_none.reaches("central") && on_none.reaches("app"));
        let wire = json!({"kind": "test-run-changed", "project": "reef", "run": "r2"});
        assert_eq!(serde_json::to_value(&on_none).unwrap(), wire);
    }
}
