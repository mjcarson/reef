//! Comments on a node's work: threads on a line of one of its revisions, or on the node as a
//! whole
//!
//! A thread is started on its [`Target`]: one line of a file in one of the node's revisions,
//! named by its [`Anchor`], or the node itself, as a general comment on its changes. Replies
//! follow in the thread, and the thread as a whole is open or resolved. Every comment, a reply
//! included, takes the next number in its plan, and a thread is named by its first comment's, so
//! one number names a thread wherever in the plan it is read from.
//!
//! A comment's body is markdown, kept as written. The service renders it on read, raw HTML
//! escaped, so a client shows the HTML as it is.
//!
//! A thread on a line follows its line through the node's later revisions. Each capture compares
//! the revision the line was last in with the next, and the thread keeps where the line went in
//! each, its target staying where it was started. Once a revision no longer holds the line, the
//! thread is outdated from that revision on, and is followed no further.

use reef_model::NodeName;
use serde::{Deserialize, Serialize};

use crate::revision::Anchor;

/// The most bytes a comment's body may hold
pub const COMMENT_BODY_MAX: usize = 64 << 10;

/// What a thread is on
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "kebab-case")]
pub enum Target {
    /// The node as a whole, as a general comment on its work
    Node,
    /// One line of a file in one of the node's revisions
    Line(Anchor),
}

impl Target {
    /// The anchor of the line this target is on, if it is on one
    pub fn anchor(&self) -> Option<&Anchor> {
        match self {
            Target::Node => None,
            Target::Line(anchor) => Some(anchor),
        }
    }
}

/// Whether a thread still wants an answer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadState {
    /// It still wants an answer
    Open,
    /// It was answered
    Resolved,
}

impl ThreadState {
    /// The name this state takes in JSON and on the command line
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadState::Open => "open",
            ThreadState::Resolved => "resolved",
        }
    }
}

/// One comment in a thread
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// Its number in its plan, which no restart resets
    pub number: u32,
    /// When it was written, in milliseconds since the epoch
    pub at: u64,
    /// Its body, as markdown, as it was written
    pub body: String,
    /// Its body rendered, raw HTML escaped, filled in as it is read
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub html: String,
}

/// A thread of comments on a node's work
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The node it is on
    pub node: NodeName,
    /// Its number: its first comment's
    pub number: u32,
    /// The version its row is at, which a mark names to change it: a mark moves it on, and so
    /// does a capture that follows its line
    pub version: u64,
    /// What it is on, as it was started
    pub target: Target,
    /// Where its line is in each revision after its target's that it was followed to, oldest
    /// first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub followed: Vec<Anchor>,
    /// The revision that no longer held its line, from which on it is outdated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outdated: Option<u32>,
    /// Whether it is open or resolved
    pub state: ThreadState,
    /// Its comments, the first one first
    pub comments: Vec<Comment>,
}

impl Thread {
    /// Whether it still wants an answer
    pub fn is_open(&self) -> bool {
        self.state == ThreadState::Open
    }

    /// Whether a later revision no longer held its line
    pub fn is_outdated(&self) -> bool {
        self.outdated.is_some()
    }

    /// Where its line is in the latest revision it was followed to, or `None` for a thread on
    /// the node as a whole
    ///
    /// An outdated thread's is where its line was last.
    pub fn latest_anchor(&self) -> Option<&Anchor> {
        // the last place it was followed to, else where it was started
        self.followed.last().or(self.target.anchor())
    }

    /// Where its line is in one revision of its node, if it was followed there
    ///
    /// # Arguments
    ///
    /// * `revision` - The revision's number
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{Anchor, Side, Target, Thread, ThreadState};
    ///
    /// let at = |revision, line| Anchor {
    ///     revision,
    ///     path: "src/lib.rs".to_string(),
    ///     side: Side::New,
    ///     line,
    /// };
    /// let thread = Thread {
    ///     node: "reef/central/C36".parse().unwrap(),
    ///     number: 1,
    ///     version: 2,
    ///     target: Target::Line(at(1, 12)),
    ///     followed: vec![at(2, 15)],
    ///     outdated: Some(3),
    ///     state: ThreadState::Open,
    ///     comments: Vec::new(),
    /// };
    /// assert_eq!(thread.anchor_in(2).map(|anchor| anchor.line), Some(15));
    /// assert!(thread.anchor_in(3).is_none());
    /// ```
    pub fn anchor_in(&self, revision: u32) -> Option<&Anchor> {
        // where it was started, then each place it was followed to
        self.target
            .anchor()
            .into_iter()
            .chain(&self.followed)
            .find(|anchor| anchor.revision == revision)
    }
}

/// A new thread, its first comment and what it is on
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThread {
    /// What it is on
    pub target: Target,
    /// Its first comment's body, as markdown
    pub body: String,
}

/// A reply to a thread
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewReply {
    /// Its body, as markdown
    pub body: String,
}

/// A thread marked open or resolved, over the version its writer read
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkThread {
    /// The state to mark it
    pub state: ThreadState,
    /// The version of the thread this mark was made against
    pub read: u64,
}

/// What came of a new thread or a reply
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Commented {
    /// The comment was kept
    Added {
        /// The thread as it now stands, the comment last in it
        thread: Thread,
        /// The comment's number
        comment: u32,
    },
    /// The node is not in its plan, so nothing was written
    NotInPlan {
        /// The node that isn't in its plan
        node: NodeName,
    },
    /// The target names no line of its node's revisions, so nothing was written
    NoLine {
        /// The target that names no line
        target: Target,
    },
    /// The thread replied to isn't in the plan, so nothing was written
    NoThread {
        /// The thread's number
        thread: u32,
    },
}

/// What came of marking a thread
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Marked {
    /// The thread was marked
    Marked {
        /// The thread as it now stands
        thread: Thread,
    },
    /// The thread has changed since the version the mark named, so nothing was written
    Stale {
        /// The thread as it stands
        thread: Thread,
    },
    /// The thread isn't in the plan, so nothing was written
    NoThread {
        /// The thread's number
        thread: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::{Comment, Commented, Marked, NewThread, Target, Thread, ThreadState};
    use crate::revision::{Anchor, Side};
    use serde_json::json;

    /// A line of `src/lib.rs` in revision 3
    fn anchor() -> Anchor {
        Anchor {
            revision: 3,
            path: "src/lib.rs".to_string(),
            side: Side::New,
            line: 12,
        }
    }

    /// A thread as JSON
    ///
    /// # Arguments
    ///
    /// * `thread` - The thread
    fn value_of(thread: &Thread) -> serde_json::Value {
        serde_json::to_value(thread).unwrap()
    }

    /// A target names what it is on, a line's anchor beside it
    #[test]
    fn targets_are_tagged_by_what_they_are_on() {
        // a general comment is on the node, and nothing else
        assert_eq!(
            serde_json::to_value(Target::Node).unwrap(),
            json!({"on": "node"})
        );
        // a line comment's anchor sits beside its tag
        let line = Target::Line(anchor());
        let value = serde_json::to_value(&line).unwrap();
        assert_eq!(
            value,
            json!({"on": "line", "revision": 3, "path": "src/lib.rs", "side": "new", "line": 12})
        );
        assert_eq!(serde_json::from_value::<Target>(value).unwrap(), line);
        assert_eq!(line.anchor(), Some(&anchor()));
        assert_eq!(Target::Node.anchor(), None);
    }

    /// A thread reads back as it was, an unrendered comment leaving its HTML out
    #[test]
    fn threads_read_back_as_written() {
        // a resolved thread on a line, with a reply
        let thread = Thread {
            node: "reef/central/C34".parse().unwrap(),
            number: 4,
            version: 2,
            target: Target::Line(anchor()),
            followed: Vec::new(),
            outdated: None,
            state: ThreadState::Resolved,
            comments: vec![
                Comment {
                    number: 4,
                    at: 10,
                    body: "Why a lock?".to_string(),
                    html: String::new(),
                },
                Comment {
                    number: 6,
                    at: 12,
                    body: "*Two* writers.".to_string(),
                    html: "<p><em>Two</em> writers.</p>\n".to_string(),
                },
            ],
        };
        let value = serde_json::to_value(&thread).unwrap();
        assert_eq!(value["target"]["on"], "line");
        assert_eq!(value["state"], "resolved");
        assert!(value["comments"][0].get("html").is_none());
        assert_eq!(
            value["comments"][1]["html"],
            "<p><em>Two</em> writers.</p>\n"
        );
        assert_eq!(serde_json::from_value::<Thread>(value).unwrap(), thread);
        assert!(!thread.is_open());
        // a thread never followed leaves its trail out, and is where it was started
        assert!(value_of(&thread).get("followed").is_none());
        assert!(value_of(&thread).get("outdated").is_none());
        assert_eq!(thread.latest_anchor(), Some(&anchor()));
        // one followed to revision 4 and lost in 5 says so, and is last where 4 had it
        let moved = Anchor {
            revision: 4,
            line: 14,
            ..anchor()
        };
        let followed = Thread {
            followed: vec![moved.clone()],
            outdated: Some(5),
            ..thread.clone()
        };
        let value = value_of(&followed);
        assert_eq!(value["followed"][0]["line"], 14);
        assert_eq!(value["outdated"], 5);
        assert_eq!(serde_json::from_value::<Thread>(value).unwrap(), followed);
        assert!(followed.is_outdated() && !thread.is_outdated());
        assert_eq!(followed.latest_anchor(), Some(&moved));
        assert_eq!(followed.anchor_in(3), Some(&anchor()));
        assert_eq!(followed.anchor_in(4), Some(&moved));
        assert_eq!(followed.anchor_in(5), None);
        // and a general thread is at no line in any revision
        let general = Thread {
            target: Target::Node,
            ..thread.clone()
        };
        assert_eq!(general.latest_anchor(), None);
        assert_eq!(general.anchor_in(3), None);
        // and a new thread is its target and its first body
        let new = NewThread {
            target: Target::Node,
            body: "Looks good".to_string(),
        };
        let json = serde_json::to_string(&new).unwrap();
        assert_eq!(json, r#"{"target":{"on":"node"},"body":"Looks good"}"#);
    }

    /// Outcomes are tagged by their kind in kebab-case
    #[test]
    fn outcomes_are_tagged_in_kebab_case() {
        // a target that names no line says which
        let none = Commented::NoLine {
            target: Target::Line(anchor()),
        };
        assert_eq!(
            serde_json::to_value(&none).unwrap(),
            json!({"no-line": {"target": {"on": "line", "revision": 3, "path": "src/lib.rs", "side": "new", "line": 12}}})
        );
        // a node not in its plan, and a thread that isn't there, name what is missing
        let missing = Commented::NotInPlan {
            node: "reef/central/C99".parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&missing).unwrap(),
            r#"{"not-in-plan":{"node":"reef/central/C99"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Marked::NoThread { thread: 7 }).unwrap(),
            r#"{"no-thread":{"thread":7}}"#
        );
        assert_eq!(ThreadState::Resolved.as_str(), "resolved");
    }
}
