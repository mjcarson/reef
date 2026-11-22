//! The user's queue: where an edit puts an entry, the edits, and what came of one
//!
//! A project has one queue, spanning its plans, so every entry names its node in full. The
//! service keeps each entry under a rank that sorts it, which only the service makes; a client
//! says where an entry goes by its neighbours.

use reef_model::{NodeName, PlanName, QueueEntry};
use serde::{Deserialize, Serialize};

/// Where in the queue an edit puts its node
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    /// First, ahead of every other entry
    Top,
    /// Last, after every other entry
    Bottom,
    /// Just before a queued node
    Before(NodeName),
    /// Just after a queued node
    After(NodeName),
}

impl Place {
    /// The queued node this place is beside, if it is beside one
    pub fn anchor(&self) -> Option<&NodeName> {
        match self {
            Place::Before(anchor) | Place::After(anchor) => Some(anchor),
            Place::Top | Place::Bottom => None,
        }
    }
}

/// One change to a project's queue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueEdit {
    /// Puts a node in the queue, or moves it there if it is queued already
    Add {
        /// The node to queue
        node: NodeName,
        /// Where it goes
        at: Place,
    },
    /// Moves a queued node
    Move {
        /// The node to move
        node: NodeName,
        /// Where it goes
        at: Place,
    },
    /// Takes a node off the queue
    Remove {
        /// The node to take off
        node: NodeName,
    },
}

impl QueueEdit {
    /// The node this edit queues, moves or takes off
    pub fn node(&self) -> &NodeName {
        match self {
            QueueEdit::Add { node, .. } | QueueEdit::Move { node, .. } => node,
            QueueEdit::Remove { node } => node,
        }
    }

    /// Where this edit puts its node, or `None` for one that takes it off
    pub fn place(&self) -> Option<&Place> {
        match self {
            QueueEdit::Add { at, .. } | QueueEdit::Move { at, .. } => Some(at),
            QueueEdit::Remove { .. } => None,
        }
    }
}

/// One entry of a queue, with the rank it sorts by
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranked {
    /// The node queued
    pub node: NodeName,
    /// The rank that sorts it, which only the service makes
    pub rank: String,
}

/// What came of a queue edit
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Queued {
    /// The edit was made, or there was nothing to change, and the queue now reads so
    Applied {
        /// The queue's entries, in its order
        queue: Vec<Ranked>,
    },
    /// The node moved or taken off, or the one placed beside, isn't queued, so nothing changed
    NotQueued {
        /// The node that isn't queued
        node: NodeName,
    },
    /// The node added is not in its plan, so nothing changed
    NotInPlan {
        /// The node that isn't in its plan
        node: NodeName,
    },
}

/// A project's queue as it stands: each entry, its rank, and what it is now
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueListing {
    /// The plans the project lists, so a client knows whether to name nodes in full
    pub plans: Vec<PlanName>,
    /// The queue's entries, in its order
    pub entries: Vec<Listed>,
}

/// One entry of a queue's listing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    /// The rank that sorts it
    pub rank: String,
    /// The node queued, and whether next takes it, and if not why
    pub entry: QueueEntry,
}

#[cfg(test)]
mod tests {
    use super::{Listed, Place, QueueEdit, QueueListing, Queued, Ranked};
    use reef_model::{EntryState, NodeName, QueueEntry};

    /// Edits and their outcomes are tagged in kebab-case, and nodes are written in full
    #[test]
    fn queue_edits_and_outcomes_are_externally_tagged_in_kebab_case() {
        let node: NodeName = "reef/central/C21".parse().unwrap();
        let anchor: NodeName = "reef/other/O1".parse().unwrap();
        // a place beside a node names it, and one at an end is a word
        let add = QueueEdit::Add {
            node: node.clone(),
            at: Place::Before(anchor.clone()),
        };
        let json = serde_json::to_string(&add).unwrap();
        assert_eq!(
            json,
            r#"{"add":{"node":"reef/central/C21","at":{"before":"reef/other/O1"}}}"#
        );
        assert_eq!(serde_json::from_str::<QueueEdit>(&json).unwrap(), add);
        let top = QueueEdit::Move {
            node: node.clone(),
            at: Place::Top,
        };
        assert_eq!(
            serde_json::to_string(&top).unwrap(),
            r#"{"move":{"node":"reef/central/C21","at":"top"}}"#
        );
        // what came of an edit is tagged the same way
        let applied = Queued::Applied {
            queue: vec![Ranked {
                node: node.clone(),
                rank: "a0".to_string(),
            }],
        };
        assert_eq!(
            serde_json::to_string(&applied).unwrap(),
            r#"{"applied":{"queue":[{"node":"reef/central/C21","rank":"a0"}]}}"#
        );
        let not = Queued::NotQueued {
            node: anchor.clone(),
        };
        assert_eq!(
            serde_json::to_string(&not).unwrap(),
            r#"{"not-queued":{"node":"reef/other/O1"}}"#
        );
        let missing = Queued::NotInPlan { node: anchor };
        assert_eq!(
            serde_json::to_string(&missing).unwrap(),
            r#"{"not-in-plan":{"node":"reef/other/O1"}}"#
        );
    }

    /// A listing's entry carries its rank beside the entry, its state tagged in kebab-case
    #[test]
    fn listing_reads_back_as_it_was() {
        let node: NodeName = "reef/central/C21".parse().unwrap();
        let listing = QueueListing {
            plans: vec!["reef/central".parse().unwrap()],
            entries: vec![Listed {
                rank: "a0".to_string(),
                entry: QueueEntry {
                    node,
                    title: "C21 Rank strings".to_string(),
                    state: EntryState::Done,
                },
            }],
        };
        let json = serde_json::to_string(&listing).unwrap();
        assert_eq!(
            json,
            r#"{"plans":["reef/central"],"entries":[{"rank":"a0","entry":{"node":"reef/central/C21","title":"C21 Rank strings","state":"done"}}]}"#
        );
        assert_eq!(
            serde_json::from_str::<QueueListing>(&json).unwrap(),
            listing
        );
    }
}
