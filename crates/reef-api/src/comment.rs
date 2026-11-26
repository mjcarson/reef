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
//!
//! A comment may be written in a review of its node, as a merge request's review is: it stays
//! pending, shown only where it is asked for and never resolved, until the review is submitted,
//! which publishes every comment in it at once, or discarded, which drops them.
//!
//! A thread is resolved by a change. A commit's `Reef-Resolves: <n>` trailer, read when the commit
//! is captured, resolves thread `n` of the capture's plan and links it to the revision the capture
//! kept, as a [`Resolution`]. `reef comment resolve` links a revision by hand. The link names the
//! revision, which reef keeps, so it outlives the commit once that is squashed away.

use reef_model::NodeName;
use serde::{Deserialize, Serialize};

use crate::revision::Anchor;
use crate::user::Author;

/// The most bytes a comment's body may hold
pub const COMMENT_BODY_MAX: usize = 64 << 10;

/// The trailer a commit names the threads it answers with, such as `Reef-Resolves: 12`
pub const RESOLVES_TRAILER: &str = "Reef-Resolves";

/// The thread numbers a `Reef-Resolves` trailer's value names, or `None` if any part of it isn't
/// one
///
/// A value names one thread or several, split by commas or spaces, each number with or without
/// a `#` before it.
///
/// # Arguments
///
/// * `value` - The trailer's value, after its `:`
///
/// # Examples
///
/// ```
/// use reef_api::comment::trailer_threads;
///
/// assert_eq!(trailer_threads("12"), Some(vec![12]));
/// assert_eq!(trailer_threads("#12, 14 #15"), Some(vec![12, 14, 15]));
/// assert_eq!(trailer_threads("the lock"), None);
/// assert_eq!(trailer_threads(" "), None);
/// ```
pub fn trailer_threads(value: &str) -> Option<Vec<u32>> {
    // each part split off by commas or spaces, a `#` before it allowed
    let threads = value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let digits = part.strip_prefix('#').unwrap_or(part);
            // a sign or anything else but digits is no number of a thread
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        })
        .collect::<Option<Vec<u32>>>()?;
    // a value naming nothing names no thread either
    (!threads.is_empty()).then_some(threads)
}

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

/// Whether the user or Claude made a write
///
/// A comment is the user's unless Claude wrote it through reef's tools, so a thread whose last
/// word is Claude's waits on the user rather than on Claude. The user is named beside it, in its
/// [`Author`], from the token that sent it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Writer {
    /// The user, in the app or the terminal
    #[default]
    User,
    /// Claude, through reef's tools while it works the loop
    Claude,
}

impl Writer {
    /// Whether the user wrote it, which a comment leaves unsaid
    pub fn is_user(&self) -> bool {
        *self == Writer::User
    }

    /// The name this writer takes in JSON and on the command line
    pub fn as_str(self) -> &'static str {
        match self {
            Writer::User => "user",
            Writer::Claude => "claude",
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
    /// Who wrote it: the user signed in, if one was, and whether Claude wrote it for them
    #[serde(flatten)]
    pub author: Author,
    /// The review it is pending in, unsubmitted, if it is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<u32>,
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
    /// Each change linked to it as one that answered it, oldest first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<Resolution>,
    /// Each time it was marked open or resolved by hand, oldest first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<ThreadMark>,
    /// Whether its first comment is pending in a review not yet submitted, so it is shown only
    /// where pending threads are asked for, and can't be resolved
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// Its comments, the first one first
    pub comments: Vec<Comment>,
}

impl Thread {
    /// Whether it still wants an answer
    pub fn is_open(&self) -> bool {
        self.state == ThreadState::Open
    }

    /// Whether it wants an answer from Claude: it is open and published, and the user wrote its
    /// last published comment
    ///
    /// A thread Claude replied to last, with a question back, waits on the user instead, and one
    /// pending in a review waits on the review.
    pub fn wants_answer(&self) -> bool {
        // open, published, and not left waiting on the user by Claude's own reply
        self.is_open()
            && !self.pending
            && self
                .comments
                .iter()
                .rev()
                .find(|comment| comment.review.is_none())
                .is_none_or(|comment| comment.author.by.is_user())
    }

    /// Whether it is open and published, as the user counts what still wants an answer
    pub fn is_open_published(&self) -> bool {
        self.is_open() && !self.pending
    }

    /// The change linked to it last as the one that answered it, if one was
    pub fn resolved_by(&self) -> Option<&Resolution> {
        self.resolutions.last()
    }

    /// The last time it was marked open or resolved by hand, if it was
    pub fn last_mark(&self) -> Option<&ThreadMark> {
        self.marks.last()
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
    ///     resolutions: Vec::new(),
    ///     marks: Vec::new(),
    ///     pending: false,
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

/// A thread marked open or resolved by hand: the state it was marked, when, and by whom
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMark {
    /// The state it was marked
    pub state: ThreadState,
    /// When, in milliseconds since the epoch
    pub at: u64,
    /// Who marked it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
}

/// A new thread, its first comment and what it is on
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThread {
    /// What it is on
    pub target: Target,
    /// Its first comment's body, as markdown
    pub body: String,
    /// Who writes it, the user unless it says so
    #[serde(default, skip_serializing_if = "Writer::is_user")]
    pub by: Writer,
    /// Whether it is written in the node's review, started for it if none is open, so it stays
    /// pending until the review is submitted
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
}

/// A reply to a thread
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewReply {
    /// Its body, as markdown
    pub body: String,
    /// Who writes it, the user unless it says so
    #[serde(default, skip_serializing_if = "Writer::is_user")]
    pub by: Writer,
    /// Whether it is written in the thread's node's review, started for it if none is open, so
    /// it stays pending until the review is submitted; a reply to a pending thread must be
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
}

/// Where a review of a node stands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    /// Still being written: its comments are pending
    Open,
    /// Submitted: its comments are published, all at once
    Submitted,
    /// Discarded: its comments are dropped, never served again
    Discarded,
}

impl ReviewState {
    /// The name this state takes in JSON and on the command line
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::Open => "open",
            ReviewState::Submitted => "submitted",
            ReviewState::Discarded => "discarded",
        }
    }
}

/// A review of a node's work, as a merge request's review: comments written in it stay pending
/// until it is submitted
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// The node it reviews
    pub node: NodeName,
    /// Its number in its plan, numbered as comments are
    pub number: u32,
    /// The version its row is at, which a mark names to change it
    pub version: u64,
    /// Whether it is still being written, submitted or discarded
    pub state: ReviewState,
    /// When it was started, in milliseconds since the epoch
    pub at: u64,
    /// How many comments are written in it
    pub comments: u32,
    /// Who started it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
    /// When it was submitted or discarded, in milliseconds since the epoch, if it was
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marked_at: Option<u64>,
    /// Who submitted or discarded it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub marked_by: Author,
}

/// A node's open review submitted or discarded, over the version its writer read
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkReview {
    /// What becomes of it: submitted or discarded
    pub state: ReviewState,
    /// The version of the review this was made against
    pub read: u64,
}

/// What came of submitting or discarding a node's review
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reviewed {
    /// The review was marked, its comments published or dropped at once
    Marked {
        /// The review as it now stands
        review: Review,
    },
    /// The review has changed since the version named, so nothing was written
    Stale {
        /// The review as it stands
        review: Review,
    },
    /// The node has no open review, so nothing was written
    NoReview {
        /// The node named
        node: NodeName,
    },
}

/// A thread marked open or resolved, over the version its writer read
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkThread {
    /// The state to mark it
    pub state: ThreadState,
    /// The version of the thread this mark was made against
    pub read: u64,
}

/// A commit as a capture read it: one whose `Reef-Resolves` trailer named a thread, or one of
/// a node's own commits
///
/// Its id may stop resolving once it is rewritten, but its author, when it was authored and its
/// summary find it again (`same_change`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvingCommit {
    /// The commit's id, which may stop resolving once it is squashed
    pub commit: String,
    /// Its message's first line
    pub summary: String,
    /// Its author's email
    pub author: String,
    /// When it was authored, in seconds since the epoch, which a rebase, a cherry-pick and an
    /// amend all keep
    pub authored: i64,
}

impl ResolvingCommit {
    /// Whether two commits are one change: by one author, authored at one time, under one
    /// summary, as a rebase, a cherry-pick or an amend that keeps the message leaves a commit it
    /// rewrites
    ///
    /// # Arguments
    ///
    /// * `other` - The other commit
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::ResolvingCommit;
    ///
    /// let picked = ResolvingCommit {
    ///     commit: "a".repeat(40),
    ///     summary: "Take the lock first".to_string(),
    ///     author: "dev@example.com".to_string(),
    ///     authored: 1_700_000_000,
    /// };
    /// let rebased = ResolvingCommit { commit: "b".repeat(40), ..picked.clone() };
    /// assert!(picked.same_change(&rebased));
    /// ```
    pub fn same_change(&self, other: &ResolvingCommit) -> bool {
        // a commit rewritten keeps its author, when it was authored and its summary, whatever
        // its id
        self.author == other.author
            && self.authored == other.authored
            && self.summary == other.summary
    }
}

/// A change linked to a thread as one that answered it: a revision of a node's changes, and the
/// commit that named the thread, if one did
///
/// reef keeps the revision, so the link outlives the commit once that is squashed away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// The node whose revision answered it, which may be another than the thread's
    pub node: NodeName,
    /// That revision's number
    pub revision: u32,
    /// When it was linked, in milliseconds since the epoch
    pub at: u64,
    /// The commit whose trailer named the thread, or `None` for one resolved by hand
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<ResolvingCommit>,
    /// Who linked it: whoever resolved it by hand, or sent the capture that read the trailer
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
}

/// A thread a commit's `Reef-Resolves` trailer names, sent with the capture that read it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolves {
    /// The thread's number in the plan of the node captured
    pub thread: u32,
    /// The commit whose trailer named it
    pub commit: ResolvingCommit,
}

/// What one trailer read at a capture did to the thread it names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrailerOutcome {
    /// It resolved its thread, which was open, and linked the capture's revision to it
    Resolved,
    /// Its thread was resolved already, so the capture's revision was only linked to it
    Linked,
    /// Its change was linked to the thread already, which is still resolved, so nothing was
    /// written
    Seen,
    /// Its change resolved the thread once already, and the thread was reopened since, so the
    /// trailer was passed over and the thread left open
    Reopened,
    /// No thread starts at its number, so nothing was written
    NoThread,
    /// Its thread is pending in a review not yet submitted, so it was left open
    Pending,
}

/// One thread a capture's trailers named, and what came of it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrailerRead {
    /// The thread's number
    pub thread: u32,
    /// The commit whose trailer named it
    pub commit: String,
    /// What came of it
    pub outcome: TrailerOutcome,
}

/// A thread resolved by hand, a revision linked to it as the one that answered it, over the
/// version its writer read
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveThread {
    /// The node whose revision answered it, in the thread's project
    pub node: NodeName,
    /// That revision's number
    pub revision: u32,
    /// The version of the thread this was made against
    pub read: u64,
}

/// What came of resolving a thread by hand
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Resolved {
    /// The thread was resolved, and the revision linked to it
    Resolved {
        /// The thread as it now stands
        thread: Thread,
    },
    /// The thread has changed since the version named, so nothing was written
    Stale {
        /// The thread as it stands
        thread: Thread,
    },
    /// The thread isn't in the plan, so nothing was written
    NoThread {
        /// The thread's number
        thread: u32,
    },
    /// The thread is pending in a review not yet submitted, so it can't be resolved
    Pending {
        /// The thread's number
        thread: u32,
    },
    /// The node has no revision of that number in the thread's project, so nothing was written
    NoRevision {
        /// The node named
        node: NodeName,
        /// The revision's number
        revision: u32,
    },
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
    /// The thread is pending in a review not yet submitted, so it can't be marked
    Pending {
        /// The thread's number
        thread: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        Comment, Commented, MarkReview, Marked, NewReply, NewThread, Resolution, Resolved,
        ResolvingCommit, Review, ReviewState, Reviewed, Target, Thread, ThreadState,
        TrailerOutcome, TrailerRead, Writer, trailer_threads,
    };
    use crate::revision::{Anchor, Side};
    use crate::user::Author;
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
            resolutions: Vec::new(),
            marks: Vec::new(),
            comments: vec![
                Comment {
                    number: 4,
                    at: 10,
                    author: Author::default(),
                    body: "Why a lock?".to_string(),
                    html: String::new(),
                    review: None,
                },
                Comment {
                    number: 6,
                    at: 12,
                    author: Author::default(),
                    body: "*Two* writers.".to_string(),
                    html: "<p><em>Two</em> writers.</p>\n".to_string(),
                    review: None,
                },
            ],
            pending: false,
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
            by: Writer::User,
            review: false,
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

    /// A trailer's value names its threads, and nothing else
    #[test]
    fn trailers_name_thread_numbers() {
        // one number, several split by commas or spaces, each with a `#` or not
        assert_eq!(trailer_threads("4"), Some(vec![4]));
        assert_eq!(trailer_threads(" #4,#5  6 "), Some(vec![4, 5, 6]));
        // anything that isn't a number names nothing, and spoils the rest
        assert_eq!(trailer_threads("4, the lock"), None);
        assert_eq!(trailer_threads("+4"), None);
        assert_eq!(trailer_threads("#"), None);
        assert_eq!(trailer_threads("99999999999"), None);
        assert_eq!(trailer_threads(""), None);
    }

    /// A thread's resolutions read back as they were, a hand's leaving its commit out, and the
    /// last is the one it was resolved by
    #[test]
    fn resolutions_read_back_as_linked() {
        // a commit that named the thread, and a revision linked by hand after it
        let commit = ResolvingCommit {
            commit: "a".repeat(40),
            summary: "Take the lock first".to_string(),
            author: "dev@example.com".to_string(),
            authored: 1_700_000_000,
        };
        let by_trailer = Resolution {
            node: "reef/central/C37".parse().unwrap(),
            revision: 2,
            at: 10,
            commit: Some(commit.clone()),
            author: Author::default(),
        };
        let by_hand = Resolution {
            node: "reef/central/C38".parse().unwrap(),
            revision: 1,
            at: 12,
            commit: None,
            author: Author {
                user: Some("ada".to_string()),
                ..Author::default()
            },
        };
        let thread = Thread {
            node: "reef/central/C37".parse().unwrap(),
            number: 4,
            version: 3,
            target: Target::Node,
            followed: Vec::new(),
            outdated: None,
            state: ThreadState::Resolved,
            resolutions: vec![by_trailer.clone(), by_hand.clone()],
            marks: Vec::new(),
            comments: Vec::new(),
            pending: false,
        };
        let value = serde_json::to_value(&thread).unwrap();
        assert_eq!(value["resolutions"][0]["node"], "reef/central/C37");
        assert_eq!(value["resolutions"][0]["commit"]["authored"], 1_700_000_000);
        assert!(value["resolutions"][1].get("commit").is_none());
        assert!(value["resolutions"][0].get("author").is_none());
        assert_eq!(value["resolutions"][1]["author"]["user"], "ada");
        assert_eq!(serde_json::from_value::<Thread>(value).unwrap(), thread);
        assert_eq!(thread.resolved_by(), Some(&by_hand));
        // a thread never resolved by a change leaves them out
        let open = Thread {
            resolutions: Vec::new(),
            ..thread.clone()
        };
        assert!(
            serde_json::to_value(&open)
                .unwrap()
                .get("resolutions")
                .is_none()
        );
        assert_eq!(open.resolved_by(), None);
        // a commit rewritten is the same change, and another of the same author isn't, even
        // within the same second
        let amended = ResolvingCommit {
            commit: "b".repeat(40),
            ..commit.clone()
        };
        let later = ResolvingCommit {
            authored: commit.authored + 1,
            ..commit.clone()
        };
        let reworded = ResolvingCommit {
            commit: "c".repeat(40),
            summary: "Take the lock first, always".to_string(),
            ..commit.clone()
        };
        assert!(commit.same_change(&amended));
        assert!(!commit.same_change(&later));
        assert!(!commit.same_change(&reworded));
        // and what a trailer did, and a hand's resolve, are tagged in kebab-case
        let read = TrailerRead {
            thread: 4,
            commit: "a".repeat(40),
            outcome: TrailerOutcome::NoThread,
        };
        assert_eq!(serde_json::to_value(&read).unwrap()["outcome"], "no-thread");
        let missing = Resolved::NoRevision {
            node: "reef/central/C37".parse().unwrap(),
            revision: 9,
        };
        assert_eq!(
            serde_json::to_string(&missing).unwrap(),
            r#"{"no-revision":{"node":"reef/central/C37","revision":9}}"#
        );
    }

    /// A comment is the user's unless it says it is Claude's, and a thread waits on Claude only
    /// while it is open and the user wrote its last comment
    #[test]
    fn threads_want_an_answer_while_the_user_spoke_last() {
        // a comment and a reply, each with who wrote it
        let said = |number, by| Comment {
            number,
            at: u64::from(number),
            author: Author {
                by,
                ..Author::default()
            },
            review: None,
            body: "Why?".to_string(),
            html: String::new(),
        };
        let thread = Thread {
            node: "reef/central/C43".parse().unwrap(),
            number: 1,
            version: 1,
            target: Target::Node,
            followed: Vec::new(),
            outdated: None,
            state: ThreadState::Open,
            resolutions: Vec::new(),
            marks: Vec::new(),
            comments: vec![said(1, Writer::User)],
            pending: false,
        };
        // the user's leaves the writer unsaid, and Claude's says so
        let value = serde_json::to_value(&thread.comments[0]).unwrap();
        assert!(value.get("by").is_none());
        let claudes = serde_json::to_value(said(2, Writer::Claude)).unwrap();
        assert_eq!(claudes["by"], "claude");
        assert_eq!(
            serde_json::from_value::<Comment>(claudes)
                .unwrap()
                .author
                .by,
            Writer::Claude
        );
        // a comment kept before writers were said reads back as the user's
        let old: Comment =
            serde_json::from_value(json!({"number": 3, "at": 4, "body": "Old"})).unwrap();
        assert_eq!(old.author, Author::default());
        // a signed-in user's comment names them beside its writer, at the comment's own level
        let ada = Comment {
            author: Author {
                user: Some("ada".to_string()),
                since: Some(9),
                by: Writer::Claude,
            },
            ..said(5, Writer::User)
        };
        let value = serde_json::to_value(&ada).unwrap();
        assert_eq!(
            (
                value["user"].clone(),
                value["since"].clone(),
                value["by"].clone()
            ),
            (json!("ada"), json!(9), json!("claude"))
        );
        assert_eq!(serde_json::from_value::<Comment>(value).unwrap(), ada);
        // open, the user's word last, it wants an answer
        assert!(thread.wants_answer());
        // Claude's reply last leaves it waiting on the user
        let asked = Thread {
            comments: vec![said(1, Writer::User), said(2, Writer::Claude)],
            ..thread.clone()
        };
        assert!(asked.is_open() && !asked.wants_answer());
        // the user answering wants an answer again
        let answered = Thread {
            comments: vec![
                said(1, Writer::User),
                said(2, Writer::Claude),
                said(3, Writer::User),
            ],
            ..thread.clone()
        };
        assert!(answered.wants_answer());
        // and a resolved thread wants none
        let resolved = Thread {
            state: ThreadState::Resolved,
            ..thread
        };
        assert!(!resolved.wants_answer());
        // a reply sent as Claude's says so, and the user's leaves it out
        let reply = NewReply {
            body: "Is two enough?".to_string(),
            by: Writer::Claude,
            review: false,
        };
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"body":"Is two enough?","by":"claude"}"#
        );
        assert_eq!(Writer::Claude.as_str(), "claude");
    }

    /// A thread pending in a review wants no answer and counts as no open one, its comments
    /// name their review, and a review and what came of marking one are tagged in kebab-case
    #[test]
    fn pending_threads_wait_on_their_review() {
        // a thread whose first comment is pending in review 3
        let thread = Thread {
            node: "reef/central/C73".parse().unwrap(),
            number: 4,
            version: 1,
            target: Target::Node,
            followed: Vec::new(),
            outdated: None,
            state: ThreadState::Open,
            resolutions: Vec::new(),
            marks: Vec::new(),
            pending: true,
            comments: vec![Comment {
                number: 4,
                at: 1,
                author: Author::default(),
                review: Some(3),
                body: "Why?".to_string(),
                html: String::new(),
            }],
        };
        assert!(thread.is_open() && !thread.is_open_published() && !thread.wants_answer());
        let value = serde_json::to_value(&thread).unwrap();
        assert_eq!(
            (
                value["pending"].clone(),
                value["comments"][0]["review"].clone()
            ),
            (json!(true), json!(3))
        );
        assert_eq!(serde_json::from_value::<Thread>(value).unwrap(), thread);
        // published, it wants an answer, and a pending reply after Claude's word doesn't count
        let published = Thread {
            pending: false,
            comments: vec![
                Comment {
                    review: None,
                    ..thread.comments[0].clone()
                },
                Comment {
                    number: 5,
                    author: Author {
                        by: Writer::Claude,
                        ..Author::default()
                    },
                    review: None,
                    ..thread.comments[0].clone()
                },
                Comment {
                    number: 6,
                    review: Some(7),
                    ..thread.comments[0].clone()
                },
            ],
            ..thread.clone()
        };
        assert!(published.is_open_published() && !published.wants_answer());
        let value = serde_json::to_value(&published).unwrap();
        assert!(value.get("pending").is_none() && value["comments"][0].get("review").is_none());
        // a new thread in a review says so, and one published at once leaves it out
        let new = NewThread {
            target: Target::Node,
            body: "Later".to_string(),
            by: Writer::User,
            review: true,
        };
        assert_eq!(serde_json::to_value(&new).unwrap()["review"], true);
        let reply: NewReply = serde_json::from_value(json!({"body": "Now"})).unwrap();
        assert!(!reply.review);
        // a review, and what came of marking one
        let review = Review {
            node: "reef/central/C73".parse().unwrap(),
            number: 3,
            version: 2,
            state: ReviewState::Submitted,
            at: 1,
            comments: 2,
            author: Author::default(),
            marked_at: Some(3),
            marked_by: Author {
                user: Some("bob".to_string()),
                ..Author::default()
            },
        };
        let value = serde_json::to_value(&review).unwrap();
        assert_eq!(value["state"], "submitted");
        assert!(value.get("author").is_none());
        assert_eq!(value["marked_by"]["user"], "bob");
        assert_eq!(serde_json::from_value::<Review>(value).unwrap(), review);
        assert_eq!(ReviewState::Discarded.as_str(), "discarded");
        let none = Reviewed::NoReview {
            node: "reef/central/C73".parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"no-review":{"node":"reef/central/C73"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Marked::Pending { thread: 4 }).unwrap(),
            r#"{"pending":{"thread":4}}"#
        );
        assert_eq!(
            serde_json::to_value(TrailerOutcome::Pending).unwrap(),
            json!("pending")
        );
        let mark: MarkReview =
            serde_json::from_value(json!({"state": "submitted", "read": 2})).unwrap();
        assert_eq!(mark.state, ReviewState::Submitted);
    }
}
