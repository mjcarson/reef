//! A node's work record: where its work happens, what it started from, and what each capture of
//! its changes read
//!
//! `reef start` reads the node's own checkout and sends what it read as a [`WorkStart`]: the
//! worktree, its branch, the base branch other work comes in on, the start as a commit and as a
//! tree, where the start left the base branch, and the ref that pins it. The service keeps one
//! record a node, which a start names the version of to replace it. Each capture of the node's
//! changes then sends its files, which the service keeps as a [`Revision`], and adds the SHAs it
//! read, as provenance only: they may stop resolving once squashed. A capture also sends each
//! thread a `Reef-Resolves` trailer on the node's commits names, which the service resolves,
//! linking the revision. A capture is a draft or a seal: git's hooks keep the node's latest
//! revision current as its draft, and a seal keeps it for good. The service never reads a
//! checkout, so everything here is what a client read in one.

use reef_model::NodeName;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::comment::{Resolves, ResolvingCommit, TrailerRead};
use crate::revision::{FileDiff, Revision};

/// Where a node's work happens and what it started from, as read from its own checkout
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStart {
    /// The root of the worktree the work happens in, absolute, as the checkout's host sees it
    pub worktree: PathBuf,
    /// The branch checked out there when it started, or `None` on a detached HEAD
    pub branch: Option<String>,
    /// The ref other work comes in on: the base branch, or the upstream it tracks when the node
    /// works on the base branch itself, or `None` when nothing brings other work in
    pub base_branch: Option<String>,
    /// The commit HEAD was at when it started
    pub start: String,
    /// The worktree as one tree when it started, uncommitted changes included, so work an
    /// earlier node left uncommitted is the start's and not this node's
    pub start_tree: String,
    /// Where the start left the base branch, `merge-base(start, base_branch)`, if they meet
    pub fork: Option<String>,
    /// The ref that keeps the start from being pruned, such as `refs/reef/start/central/C30`
    pub pin: String,
    /// The commit the pin points at: the start's tree, its parent the start
    pub pinned: String,
}

/// What one capture of a node's changes read from its checkout
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureShas {
    /// The commit HEAD was at
    pub head: String,
    /// The worktree as one tree, uncommitted changes included
    pub tree: String,
    /// Where HEAD met the base branch, if they meet
    pub merge_base: Option<String>,
    /// The tree the diff started from: the start's tree, carried over what the base branch
    /// brought in since
    pub base: String,
}

/// One capture's SHAs as a work record keeps them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRead {
    /// When the service added it, in milliseconds since the epoch
    pub at: u64,
    /// The revision it stored, or the one it found unchanged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    /// What the capture read
    #[serde(flatten)]
    pub shas: CaptureShas,
}

/// A node's work record as the service keeps it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    /// The node whose work this is
    pub node: NodeName,
    /// Bumped by every write of this record, and named by a start that replaces it
    pub version: u64,
    /// When the service recorded the start, in milliseconds since the epoch, later than every
    /// start its project recorded before
    pub started_at: u64,
    /// Where the work happens and what it started from
    #[serde(flatten)]
    pub start: WorkStart,
    /// What each capture since the start read, oldest first
    #[serde(default)]
    pub captures: Vec<CaptureRead>,
}

/// A start of a node's work, sent by `reef start`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartWork {
    /// What the start read from the node's checkout
    pub start: WorkStart,
    /// The version of the node's record this start replaces, or `None` for a node not started
    #[serde(default)]
    pub replaces: Option<u64>,
}

/// What came of a start
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Started {
    /// The start was recorded, replacing the record it named if it named one
    Recorded {
        /// The node's record as it now stands
        record: WorkRecord,
        /// The draft the record replaced left, sealed as it stood so no capture of the new start
        /// writes over it, if it left one
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed: Option<u32>,
    },
    /// The node's record is not the one the start named, so nothing was written: it was started
    /// already, or started again since, or its record named is gone
    Stale {
        /// The node's record as it stands, if it has one
        record: Option<WorkRecord>,
    },
    /// The node is not in its plan, so nothing was written
    NotInPlan {
        /// The node that isn't in its plan
        node: NodeName,
    },
}

/// One capture of a node's changes, sent to be kept as a revision, its SHAs added to its record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddCapture {
    /// The commit the start the capture was taken from is pinned at, which names that start
    pub pinned: String,
    /// What the capture read
    pub shas: CaptureShas,
    /// The files that conflicted carrying the start forward
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// Every changed file from the base to the worktree's tree, its hunks and all
    #[serde(default)]
    pub files: Vec<FileDiff>,
    /// Each thread a `Reef-Resolves` trailer names, on the node's commits since its start that
    /// its base branch doesn't hold
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves: Vec<Resolves>,
    /// The node's own commits since its start that its base branch doesn't hold, oldest first,
    /// none of them a merge
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<ResolvingCommit>,
    /// Whether it only keeps the node's draft current, rather than sealing what it took, as a
    /// capture that doesn't say does
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
}

/// What came of a capture
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Captured {
    /// The capture was kept as a new revision, a draft or sealed as it asked, and its SHAs added
    Added {
        /// The node's record as it now stands
        record: WorkRecord,
        /// The revision it was kept as
        revision: Revision,
        /// What each thread its trailers named came to, the revision linked to those resolved
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        trailers: Vec<TrailerRead>,
    },
    /// The capture was kept over the node's draft, under the draft's number: its files written
    /// over the draft's if they changed, and sealed if it seals, and its SHAs added
    Replaced {
        /// The node's record as it now stands
        record: WorkRecord,
        /// The revision as it now stands, still a draft unless the capture sealed it
        revision: Revision,
        /// What each thread its trailers named came to, the revision linked to those resolved
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        trailers: Vec<TrailerRead>,
    },
    /// The capture's base and tree are its node's latest revision's, and it sealed nothing that
    /// wasn't sealed, so no revision was written, and its SHAs were added naming that one
    Unchanged {
        /// The node's record as it now stands
        record: WorkRecord,
        /// The latest revision, which it matched
        revision: Revision,
        /// What each thread its trailers named came to, the latest revision linked to those
        /// resolved
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        trailers: Vec<TrailerRead>,
    },
    /// The node has no record, so nothing was added
    NotStarted {
        /// The node that hasn't been started
        node: NodeName,
    },
    /// The node was started again since the capture's start, so its SHAs are none of this
    /// start's, and nothing was added
    Restarted {
        /// The node's record as it stands
        record: WorkRecord,
    },
}

/// The record of the node a worktree is working on: the one started there last
///
/// A commit made after a node is done, and before the next one starts, still belongs to it, so
/// the last to start is the one, whatever its status.
///
/// # Arguments
///
/// * `records` - The records of the worktree's project
/// * `worktree` - The worktree's root, as the records name it
///
/// # Examples
///
/// ```
/// use reef_api::work::on_worktree;
///
/// assert!(on_worktree(&[], std::path::Path::new("/src/reef")).is_none());
/// ```
pub fn on_worktree<'a>(records: &'a [WorkRecord], worktree: &Path) -> Option<&'a WorkRecord> {
    // the latest start among the worktree's own
    records
        .iter()
        .filter(|record| record.start.worktree == worktree)
        .max_by_key(|record| record.started_at)
}

#[cfg(test)]
mod tests {
    use super::{
        AddCapture, CaptureRead, CaptureShas, Captured, StartWork, Started, WorkRecord, WorkStart,
        on_worktree,
    };
    use std::path::{Path, PathBuf};

    /// A start in a worktree, its SHAs made up
    ///
    /// # Arguments
    ///
    /// * `worktree` - The worktree's root
    fn start(worktree: &str) -> WorkStart {
        WorkStart {
            worktree: PathBuf::from(worktree),
            branch: Some("reef/C30".to_string()),
            base_branch: Some("main".to_string()),
            start: "a".repeat(40),
            start_tree: "b".repeat(40),
            fork: None,
            pin: "refs/reef/start/central/C30".to_string(),
            pinned: "c".repeat(40),
        }
    }

    /// A record of a node, started in a worktree at a time
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id in `reef/central`
    /// * `worktree` - The worktree's root
    /// * `started_at` - When it started
    fn record(id: &str, worktree: &str, started_at: u64) -> WorkRecord {
        WorkRecord {
            node: format!("reef/central/{id}").parse().unwrap(),
            version: 1,
            started_at,
            start: start(worktree),
            captures: Vec::new(),
        }
    }

    /// A record's start sits beside its name, and a capture's SHAs beside its time
    #[test]
    fn records_flatten_what_was_read() {
        // a record with one capture
        let mut kept = record("C30", "/src/reef", 7);
        kept.captures.push(CaptureRead {
            at: 9,
            revision: Some(1),
            shas: CaptureShas {
                head: "d".repeat(40),
                tree: "e".repeat(40),
                merge_base: None,
                base: "b".repeat(40),
            },
        });
        let json = serde_json::to_value(&kept).unwrap();
        // the start's fields are the record's own, and so are a capture's SHAs
        assert_eq!(json["node"], "reef/central/C30");
        assert_eq!(json["worktree"], "/src/reef");
        assert_eq!(json["pin"], "refs/reef/start/central/C30");
        assert_eq!(json["captures"][0]["at"], 9);
        assert_eq!(json["captures"][0]["revision"], 1);
        assert_eq!(json["captures"][0]["head"], "d".repeat(40));
        // and it reads back as it was
        assert_eq!(serde_json::from_value::<WorkRecord>(json).unwrap(), kept);
    }

    /// Requests and their outcomes are tagged in kebab-case, a start replacing nothing by default
    #[test]
    fn starts_and_captures_are_tagged_in_kebab_case() {
        // a start with no version named replaces nothing
        let json = serde_json::json!({ "start": start("/src/reef") });
        let read: StartWork = serde_json::from_value(json).unwrap();
        assert_eq!(read.replaces, None);
        // each outcome is tagged by its kind
        let stale = Started::Stale { record: None };
        assert_eq!(
            serde_json::to_string(&stale).unwrap(),
            r#"{"stale":{"record":null}}"#
        );
        let missing = Started::NotInPlan {
            node: "reef/central/C99".parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&missing).unwrap(),
            r#"{"not-in-plan":{"node":"reef/central/C99"}}"#
        );
        let none = Captured::NotStarted {
            node: "reef/central/C99".parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"not-started":{"node":"reef/central/C99"}}"#
        );
        // and a capture names the start it was taken from beside what it read
        let add = AddCapture {
            pinned: "c".repeat(40),
            shas: CaptureShas {
                head: "d".repeat(40),
                tree: "e".repeat(40),
                merge_base: Some("f".repeat(40)),
                base: "b".repeat(40),
            },
            conflicts: Vec::new(),
            files: Vec::new(),
            resolves: Vec::new(),
            commits: Vec::new(),
            draft: false,
        };
        let json = serde_json::to_string(&add).unwrap();
        assert_eq!(serde_json::from_str::<AddCapture>(&json).unwrap(), add);
        // one sent with no files or conflicts, as C30's were, reads as a sealing capture of
        // nothing, which is what every capture was before drafts
        let bare = serde_json::json!({ "pinned": add.pinned, "shas": add.shas });
        assert_eq!(serde_json::from_value::<AddCapture>(bare).unwrap(), add);
        assert!(!json.contains("draft"), "{json}");
        // and a draft says so
        let draft = AddCapture { draft: true, ..add };
        let json = serde_json::to_value(&draft).unwrap();
        assert_eq!(json["draft"], true);
        assert_eq!(serde_json::from_value::<AddCapture>(json).unwrap(), draft);
    }

    /// A worktree is on the node started there last, and on none where nothing started
    #[test]
    fn a_worktree_is_on_its_latest_start() {
        // two nodes in one worktree, and one in another started later than both
        let records = [
            record("C30", "/src/reef", 10),
            record("C31", "/src/reef", 20),
            record("C32", "/src/reef-b", 30),
        ];
        let on = |dir: &str| on_worktree(&records, Path::new(dir)).map(|r| r.node.id.as_str());
        assert_eq!(on("/src/reef"), Some("C31"));
        assert_eq!(on("/src/reef-b"), Some("C32"));
        assert_eq!(on("/src/elsewhere"), None);
    }
}
