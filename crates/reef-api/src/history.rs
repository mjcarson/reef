//! Who did what: each node's, page's and queue's history of edits
//!
//! Every write to a node, a page or a project's queue keeps an entry in that one's history: who
//! made it, when, the version it left it at, and what it changed. A node's entry says its status
//! went from one to another, a criterion was met, keys were edited or its body was, rather than
//! holding the node again. A history is read newest first, a page of at most [`HISTORY_MAX`]
//! entries at a time: a [`HistoryPage`] with older entries kept names the cursor that reads them.
//!
//! An entry is kept once its write is, so a write whose entry failed to keep still stands, and
//! its history has a gap where it would be.
//!
//! Acts on what has no history of its own are kept the same way (C89): a project's and a
//! plan's registration, each act on a project's loops and sessions ([`LoopAct`]), and each act
//! on reef's users ([`UserAct`]).

use reef_model::{Met, NodeName, Status};
use serde::{Deserialize, Serialize};

use crate::loops::Pane;
use crate::queue::Place;
use crate::user::{Author, TokenKind};

/// The most entries one page of a history holds, newest first
pub const HISTORY_MAX: usize = 200;

/// Which page of a history to read: the newest, or the one older than a cursor a page named
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    /// The cursor a page named, to read the entries older than its own, or `None` for the newest
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
    /// The most entries the page may hold, `None` for [`HISTORY_MAX`], which is also the most
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl HistoryQuery {
    /// The page of a history older than a cursor, as full as a page may be
    ///
    /// # Arguments
    ///
    /// * `older` - The cursor the page before named
    pub fn older<T: Into<String>>(older: T) -> Self {
        HistoryQuery {
            older: Some(older.into()),
            limit: None,
        }
    }

    /// The most entries the page holds: its limit, from 1 to [`HISTORY_MAX`]
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{HISTORY_MAX, HistoryQuery};
    ///
    /// assert_eq!(HistoryQuery::default().page_size(), HISTORY_MAX);
    /// let query = HistoryQuery { older: None, limit: Some(0) };
    /// assert_eq!(query.page_size(), 1);
    /// let query = HistoryQuery { older: None, limit: Some(5000) };
    /// assert_eq!(query.page_size(), HISTORY_MAX);
    /// ```
    pub fn page_size(&self) -> usize {
        // none asks for a whole page, and none may ask for more, or for nothing
        self.limit.unwrap_or(HISTORY_MAX).clamp(1, HISTORY_MAX)
    }
}

/// One page of a history: its entries, newest first, and the cursor onto the older ones
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    /// The page's entries, newest first
    pub edits: Vec<Logged>,
    /// The cursor that reads the entries older than these, if any are kept
    ///
    /// It is opaque: hand it back as `older` to read the next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
}

/// One write a history kept: who made it, when, and what it changed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Logged {
    /// When it was made, in milliseconds since the epoch
    pub at: u64,
    /// Who made it
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub author: Author,
    /// The version it left its node or page at, or `None` for a removal or a queue's edit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// Whether it came of writing the whole plan at once, as an import does
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub whole_plan: bool,
    /// What it changed, each part of it in the order a reader is told them
    pub edits: Vec<Edited>,
}

impl Logged {
    /// What it changed, as a reader is told: each part, split by `; `
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{Author, Edited, Logged};
    /// use reef_model::Status;
    ///
    /// let logged = Logged {
    ///     at: 1,
    ///     author: Author::default(),
    ///     version: Some(4),
    ///     whole_plan: false,
    ///     edits: vec![
    ///         Edited::Status { from: Some(Status::InProgress), to: Some(Status::Done) },
    ///         Edited::Body,
    ///     ],
    /// };
    /// assert_eq!(logged.describe(), "status in-progress → done; edited the body");
    /// ```
    pub fn describe(&self) -> String {
        // each part in turn
        let parts: Vec<String> = self.edits.iter().map(Edited::describe).collect();
        parts.join("; ")
    }
}

/// One part of what a write changed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "kebab-case")]
pub enum Edited {
    /// The node or page was added
    Added,
    /// The node or page was removed
    Removed,
    /// The node's status was changed, each side `None` for one left unsaid
    Status {
        /// The status it had
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Status>,
        /// The status it has now
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Status>,
    },
    /// One of the node's acceptance criteria was marked, or named another test
    Criterion {
        /// Its number, from 1, in the order the node lists them
        number: u32,
        /// What it says
        text: String,
        /// Whether it was met, `None` for left unsaid
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Met>,
        /// Whether it is met now, `None` for left unsaid
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Met>,
        /// The test it names now, if that changed
        #[serde(default, skip_serializing_if = "Option::is_none")]
        test: Option<String>,
    },
    /// Other keys of the node's spec were edited, by name, `description`'s parts as
    /// `description.why` and so on
    Keys {
        /// The keys edited, in the order the spec writes them
        keys: Vec<String>,
    },
    /// The node's body was edited
    Body,
    /// The page's text was edited
    Text,
    /// A node was put in the queue
    Queued {
        /// The node queued
        node: NodeName,
        /// Where it was put
        at: Place,
    },
    /// A queued node was moved
    Moved {
        /// The node moved
        node: NodeName,
        /// Where it was moved to
        at: Place,
    },
    /// A node was taken off the queue
    Unqueued {
        /// The node taken off
        node: NodeName,
    },
    /// The node's work was started, by `reef start`, which leaves its version as it was
    Started {
        /// When its work record says it started, in milliseconds since the epoch
        started_at: u64,
        /// The branch checked out where it started, or `None` on a detached HEAD
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// The commit HEAD was at
        start: String,
        /// Whether it was started before, so this start replaced that one
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        again: bool,
    },
    /// An act on a pane's loop, or on the session running in it
    Loop {
        /// The pane whose loop or session it was
        pane: Pane,
        /// What was done
        act: LoopAct,
    },
    /// An act on one of reef's users
    User {
        /// The user's name
        name: String,
        /// What was done
        act: UserAct,
    },
    /// The branch the plan follows was set, changed or dropped, each side `None` for its
    /// project's own
    Branched {
        /// The branch it followed
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        /// The branch it follows now
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    /// The node moved from one plan of its project to another, its work and comments with it
    Relisted {
        /// The plan it left
        from: String,
        /// The plan it joined
        to: String,
    },
    /// A kind of edit a newer reef kept, which this one can't read
    #[serde(other)]
    Other,
}

impl Edited {
    /// What it changed, as a reader is told
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::Edited;
    /// use reef_model::Met;
    ///
    /// let met = Edited::Criterion {
    ///     number: 2,
    ///     text: "Rows read back".to_string(),
    ///     from: None,
    ///     to: Some(Met::Met),
    ///     test: Some("tests/store.rs::rows".to_string()),
    /// };
    /// assert_eq!(met.describe(), "criterion 2 met, by tests/store.rs::rows");
    /// let keys = Edited::Keys { keys: vec!["needs".to_string(), "title".to_string()] };
    /// assert_eq!(keys.describe(), "edited needs, title");
    /// let branched = Edited::Branched { from: None, to: Some("dev".to_string()) };
    /// assert_eq!(branched.describe(), "follows dev, not its project's branch");
    /// ```
    pub fn describe(&self) -> String {
        // a side left unsaid reads as unset
        let status = |status: &Option<Status>| status.map_or("unset", Status::as_str);
        match self {
            Edited::Added => "added".to_string(),
            Edited::Removed => "removed".to_string(),
            Edited::Status { from, to } => {
                format!("status {} → {}", status(from), status(to))
            }
            Edited::Criterion {
                number,
                from,
                to,
                test,
                ..
            } => {
                // its mark if that changed, then the test it names now if that did
                let mut said = format!("criterion {number}");
                if from != to {
                    said.push(' ');
                    said.push_str(to.map_or("unset", Met::as_str));
                }
                if let Some(test) = test {
                    said.push_str(match from != to {
                        true => ", by ",
                        false => " checked by ",
                    });
                    said.push_str(test);
                }
                said
            }
            Edited::Keys { keys } => format!("edited {}", keys.join(", ")),
            Edited::Body => "edited the body".to_string(),
            Edited::Text => "edited the text".to_string(),
            Edited::Queued { node, at } => format!("queued {node} {}", at.describe()),
            Edited::Moved { node, at } => format!("moved {node} {}", at.describe()),
            Edited::Unqueued { node } => format!("took {node} off the queue"),
            Edited::Loop { pane, act } => act.describe(pane),
            Edited::User { name, act } => act.describe(name),
            Edited::Started {
                branch,
                start,
                again,
                ..
            } => {
                // started, or again, then where: on its branch, at its commit's short form
                let mut said = match again {
                    true => "started again".to_string(),
                    false => "started".to_string(),
                };
                if let Some(branch) = branch {
                    said.push_str(&format!(" on {branch}"));
                }
                let short: String = start.chars().take(7).collect();
                said.push_str(&format!(" at {short}"));
                said
            }
            Edited::Branched { from, to } => {
                // the branch it follows now, then the one it followed
                let side = |branch: &Option<String>| match branch {
                    Some(branch) => branch.clone(),
                    None => "its project's branch".to_string(),
                };
                format!("follows {}, not {}", side(to), side(from))
            }
            Edited::Relisted { from, to } => format!("moved from plan `{from}` to `{to}`"),
            Edited::Other => "an edit this reef can't read".to_string(),
        }
    }
}

/// An act on a pane's loop, or on the session running in it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "kebab-case")]
pub enum LoopAct {
    /// A loop was started in the pane
    Started {
        /// The one plan it takes nodes from, or `None` for every plan of its project
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<String>,
        /// How many nodes it works at once
        width: u8,
    },
    /// The pane's paused loop was set running again
    Resumed,
    /// The pane's loop was paused, to stop once its node is done
    Paused,
    /// The pane's loop was stopped, letting go of what it held
    Stopped {
        /// The nodes it held, its own first
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        held: Vec<NodeName>,
    },
    /// The pane's loop was handed to another session, after a clear or a resume
    HandedOff {
        /// The session's id
        session: String,
    },
    /// The pane's loop took a node, and its group's members beside it
    Took {
        /// The node it works itself
        node: NodeName,
        /// The group's other nodes, each worked by a session of its own
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<NodeName>,
    },
    /// The pane's loop let go of everything it held
    LetGo {
        /// The nodes it held, its own first
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        held: Vec<NodeName>,
    },
    /// The pane joined another loop's member, to work its node
    Joined {
        /// The member's node
        node: NodeName,
        /// The pane whose loop holds it
        of: Pane,
    },
    /// A session was attached in the pane, to be shown in the app
    Attached {
        /// The session's id
        session: String,
    },
    /// The pane's session let go, as its runner or its terminal stopped
    Detached {
        /// The session's id
        session: String,
    },
    /// A prompt the pane's session raised was answered from the app
    Answered {
        /// The session's id
        session: String,
        /// The prompt's id
        prompt: String,
    },
    /// Something was said to Claude in the pane's session from the app, its turn ended mid-node
    Said {
        /// The session's id
        session: String,
    },
    /// An act a newer reef kept, which this one can't read
    #[serde(other)]
    Other,
}

impl LoopAct {
    /// What was done, as a reader is told, naming the pane it was done in
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane whose loop or session it was
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::LoopAct;
    /// use reef_api::loops::Pane;
    ///
    /// let pane = Pane::Claude { pid: 7 };
    /// let started = LoopAct::Started { plan: Some("central".to_string()), width: 2 };
    /// assert_eq!(
    ///     started.describe(&pane),
    ///     "started a loop in Claude Code process 7, feeding plan `central`, 2 nodes at once"
    /// );
    /// ```
    pub fn describe(&self, pane: &Pane) -> String {
        // the nodes held, by name, in the order held
        let named = |nodes: &[NodeName]| {
            let names: Vec<String> = nodes.iter().map(|node| format!("`{node}`")).collect();
            names.join(", ")
        };
        let pane = pane.describe();
        match self {
            LoopAct::Started { plan, width } => {
                // which plans it feeds, then how many nodes at once if more than one
                let mut said = match plan {
                    Some(plan) => format!("started a loop in {pane}, feeding plan `{plan}`"),
                    None => format!("started a loop in {pane}, feeding every plan"),
                };
                if *width > 1 {
                    said.push_str(&format!(", {width} nodes at once"));
                }
                said
            }
            LoopAct::Resumed => format!("set the loop in {pane} running again"),
            LoopAct::Paused => format!("paused the loop in {pane}"),
            LoopAct::Stopped { held } if held.is_empty() => format!("stopped the loop in {pane}"),
            LoopAct::Stopped { held } => {
                format!("stopped the loop in {pane}, letting go of {}", named(held))
            }
            LoopAct::HandedOff { session } => {
                format!("handed the loop in {pane} to session `{session}`")
            }
            LoopAct::Took { node, members } if members.is_empty() => {
                format!("the loop in {pane} took `{node}`")
            }
            LoopAct::Took { node, members } => {
                format!("the loop in {pane} took `{node}`, with {}", named(members))
            }
            LoopAct::LetGo { held } => {
                format!("the loop in {pane} let go of {}", named(held))
            }
            LoopAct::Joined { node, of } => {
                format!("{pane} joined `{node}` of the loop in {}", of.describe())
            }
            LoopAct::Attached { session } => format!("attached session `{session}` in {pane}"),
            LoopAct::Detached { session } => format!("detached session `{session}` in {pane}"),
            LoopAct::Answered { session, prompt } => {
                format!("answered prompt `{prompt}` of session `{session}` in {pane}")
            }
            LoopAct::Said { session } => {
                format!("replied to Claude in session `{session}` in {pane}")
            }
            LoopAct::Other => format!("an act on the loop in {pane} this reef can't read"),
        }
    }
}

/// An act on one of reef's users, which an admin's log keeps
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "kebab-case")]
pub enum UserAct {
    /// The user was added
    Added {
        /// Whether they were added as an admin
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        admin: bool,
    },
    /// The user was removed, with every token of theirs
    Removed {
        /// How many of their tokens were revoked with them
        revoked: usize,
    },
    /// The user's password was changed: their own, or set by an admin
    Password {
        /// Whether they changed their own, with their current one
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        own: bool,
        /// How many of their tokens were revoked with it
        #[serde(default)]
        revoked: usize,
    },
    /// One of the user's tokens was revoked, by its id
    TokenRevoked {
        /// The token's id, as a listing shows it
        id: String,
        /// What it was issued to
        kind: TokenKind,
        /// What it was for
        label: String,
    },
    /// The user was recovered as an admin on the service's host, by `reef-serve recover`
    Recovered {
        /// Whether no user had the name, so they were added
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        added: bool,
        /// How many of their tokens were revoked
        #[serde(default)]
        revoked: usize,
    },
    /// An act a newer reef kept, which this one can't read
    #[serde(other)]
    Other,
}

impl UserAct {
    /// What was done, as a reader is told, naming the user it was done to
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::UserAct;
    ///
    /// assert_eq!(UserAct::Added { admin: true }.describe("carol"), "added carol, an admin");
    /// assert_eq!(
    ///     UserAct::Password { own: false, revoked: 2 }.describe("bob"),
    ///     "set bob's password, revoking 2 tokens"
    /// );
    /// ```
    pub fn describe(&self, name: &str) -> String {
        // how many tokens went with it, if any did
        let revoking = |revoked: usize| match revoked {
            0 => String::new(),
            1 => ", revoking 1 token".to_string(),
            revoked => format!(", revoking {revoked} tokens"),
        };
        match self {
            UserAct::Added { admin: true } => format!("added {name}, an admin"),
            UserAct::Added { admin: false } => format!("added {name}"),
            UserAct::Removed { revoked } => format!("removed {name}{}", revoking(*revoked)),
            UserAct::Password { own: true, .. } => format!("{name} changed their own password"),
            UserAct::Password {
                own: false,
                revoked,
            } => {
                format!("set {name}'s password{}", revoking(*revoked))
            }
            UserAct::TokenRevoked { id, kind, label } => {
                // a terminal's token or a browser's, by its id and what it was for
                let kind = match kind {
                    TokenKind::Cli => "terminal",
                    TokenKind::Browser => "browser",
                };
                format!("revoked {name}'s {kind} token {id} ({label})")
            }
            UserAct::Recovered { added: true, .. } => {
                format!("added {name} as an admin, recovering")
            }
            UserAct::Recovered {
                added: false,
                revoked,
            } => {
                format!("recovered {name} as an admin{}", revoking(*revoked))
            }
            UserAct::Other => format!("an act on {name} this reef can't read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Edited, HistoryPage, Logged, LoopAct, UserAct};
    use crate::comment::Writer;
    use crate::loops::Pane;
    use crate::queue::Place;
    use crate::user::Author;
    use crate::user::TokenKind;
    use reef_model::NodeName;
    use reef_model::{Met, Status};
    use serde_json::json;

    /// An entry is tagged by its edits' kinds in kebab-case, and leaves out what it doesn't say
    #[test]
    fn entries_read_back_as_written() {
        let logged = Logged {
            at: 5,
            author: Author {
                user: Some("ada".to_string()),
                since: Some(1),
                by: Writer::Claude,
            },
            version: Some(3),
            whole_plan: false,
            edits: vec![
                Edited::Status {
                    from: None,
                    to: Some(Status::InProgress),
                },
                Edited::Criterion {
                    number: 1,
                    text: "Kept".to_string(),
                    from: Some(Met::Unmet),
                    to: Some(Met::Met),
                    test: None,
                },
                Edited::Queued {
                    node: "reef/central/C50".parse().unwrap(),
                    at: Place::Top,
                },
            ],
        };
        let value = serde_json::to_value(&logged).unwrap();
        assert_eq!(
            value,
            json!({
                "at": 5,
                "author": {"user": "ada", "since": 1, "by": "claude"},
                "version": 3,
                "edits": [
                    {"edit": "status", "to": "in-progress"},
                    {"edit": "criterion", "number": 1, "text": "Kept", "from": "unmet", "to": "met"},
                    {"edit": "queued", "node": "reef/central/C50", "at": "top"},
                ],
            })
        );
        assert_eq!(serde_json::from_value::<Logged>(value).unwrap(), logged);
        // an entry by nobody leaves its author out
        let nobody = Logged {
            author: Author::default(),
            version: None,
            edits: vec![Edited::Removed],
            ..logged
        };
        assert_eq!(
            serde_json::to_value(&nobody).unwrap(),
            json!({"at": 5, "edits": [{"edit": "removed"}]})
        );
    }

    /// A page leaves out a cursor when no older entry is kept, and reads back as written
    #[test]
    fn pages_name_their_cursor_only_when_older_edits_remain() {
        let page = HistoryPage {
            edits: vec![Logged {
                at: 2,
                author: Author::default(),
                version: Some(1),
                whole_plan: false,
                edits: vec![Edited::Added],
            }],
            older: None,
        };
        let value = serde_json::to_value(&page).unwrap();
        assert_eq!(
            value,
            json!({"edits": [{"at": 2, "version": 1, "edits": [{"edit": "added"}]}]})
        );
        assert_eq!(serde_json::from_value::<HistoryPage>(value).unwrap(), page);
        // one with older entries names the cursor onto them
        let paged = HistoryPage {
            older: Some("cursor".to_string()),
            ..page
        };
        let value = serde_json::to_value(&paged).unwrap();
        assert_eq!(value["older"], "cursor");
        assert_eq!(serde_json::from_value::<HistoryPage>(value).unwrap(), paged);
    }

    /// A loop's and a user's acts read back as written, and an act a newer reef keeps, fields
    /// and all, reads as one this reef can't, keeping the pane or the user it was done to
    #[test]
    fn acts_read_back_and_unknown_ones_keep_their_subject() {
        let pane = Pane::Claude { pid: 7 };
        // a start, tagged by its kind
        let started = Edited::Loop {
            pane: pane.clone(),
            act: LoopAct::Started {
                plan: None,
                width: 1,
            },
        };
        let value = serde_json::to_value(&started).unwrap();
        assert_eq!(
            value,
            json!({
                "edit": "loop",
                "pane": {"kind": "claude", "pid": 7},
                "act": {"act": "started", "width": 1},
            })
        );
        assert_eq!(serde_json::from_value::<Edited>(value).unwrap(), started);
        // an act no reef here knows, with a field of its own
        let unknown: Edited = serde_json::from_value(json!({
            "edit": "loop",
            "pane": {"kind": "claude", "pid": 7},
            "act": {"act": "renamed", "to": "elsewhere"},
        }))
        .unwrap();
        assert_eq!(
            unknown,
            Edited::Loop {
                pane,
                act: LoopAct::Other
            }
        );
        assert_eq!(
            unknown.describe(),
            "an act on the loop in Claude Code process 7 this reef can't read"
        );
        // a user's removal, and one of their acts no reef here knows
        let removed = Edited::User {
            name: "bob".to_string(),
            act: UserAct::Removed { revoked: 2 },
        };
        let value = serde_json::to_value(&removed).unwrap();
        assert_eq!(
            value,
            json!({"edit": "user", "name": "bob", "act": {"act": "removed", "revoked": 2}})
        );
        assert_eq!(serde_json::from_value::<Edited>(value).unwrap(), removed);
        assert_eq!(removed.describe(), "removed bob, revoking 2 tokens");
        let unknown: Edited = serde_json::from_value(json!({
            "edit": "user",
            "name": "bob",
            "act": {"act": "renamed", "to": "rob"},
        }))
        .unwrap();
        assert_eq!(unknown.describe(), "an act on bob this reef can't read");
    }

    /// Each act is told plainly, naming its pane or its user
    #[test]
    fn acts_are_told_plainly() {
        let pane = Pane::Claude { pid: 7 };
        let node = |id: &str| -> NodeName { format!("reef/central/{id}").parse().unwrap() };
        let loop_told = |act: LoopAct| act.describe(&pane);
        assert_eq!(
            loop_told(LoopAct::Stopped { held: Vec::new() }),
            "stopped the loop in Claude Code process 7"
        );
        assert_eq!(
            loop_told(LoopAct::LetGo {
                held: vec![node("C1"), node("C2")]
            }),
            "the loop in Claude Code process 7 let go of `reef/central/C1`, `reef/central/C2`"
        );
        assert_eq!(
            loop_told(LoopAct::Answered {
                session: "s1".to_string(),
                prompt: "p2".to_string()
            }),
            "answered prompt `p2` of session `s1` in Claude Code process 7"
        );
        let user_told = |act: UserAct| act.describe("bob");
        assert_eq!(
            user_told(UserAct::Password {
                own: true,
                revoked: 0
            }),
            "bob changed their own password"
        );
        assert_eq!(
            user_told(UserAct::TokenRevoked {
                id: "3f9a".to_string(),
                kind: TokenKind::Browser,
                label: "a phone".to_string()
            }),
            "revoked bob's browser token 3f9a (a phone)"
        );
        assert_eq!(
            user_told(UserAct::Recovered {
                added: false,
                revoked: 1
            }),
            "recovered bob as an admin, revoking 1 token"
        );
    }

    /// An edit a newer reef kept reads as one this reef can't, rather than failing the history
    #[test]
    fn unknown_edits_read_as_other() {
        let logged: Logged = serde_json::from_value(json!({
            "at": 1,
            "edits": [{"edit": "renamed"}, {"edit": "body"}],
        }))
        .unwrap();
        assert_eq!(logged.edits, vec![Edited::Other, Edited::Body]);
        assert_eq!(
            logged.describe(),
            "an edit this reef can't read; edited the body"
        );
    }

    /// Each kind of edit is told plainly
    #[test]
    fn edits_are_told_plainly() {
        let node = || "reef/central/C1".parse().unwrap();
        let told = |edit: Edited| edit.describe();
        assert_eq!(told(Edited::Added), "added");
        assert_eq!(
            told(Edited::Status {
                from: Some(Status::Todo),
                to: None
            }),
            "status todo → unset"
        );
        assert_eq!(
            told(Edited::Criterion {
                number: 4,
                text: "t".to_string(),
                from: Some(Met::Met),
                to: Some(Met::Met),
                test: Some("tests/a.rs::b".to_string()),
            }),
            "criterion 4 checked by tests/a.rs::b"
        );
        assert_eq!(told(Edited::Text), "edited the text");
        assert_eq!(
            told(Edited::Moved {
                node: node(),
                at: Place::Before("reef/central/C2".parse().unwrap()),
            }),
            "moved reef/central/C1 before reef/central/C2"
        );
        assert_eq!(
            told(Edited::Unqueued { node: node() }),
            "took reef/central/C1 off the queue"
        );
        assert_eq!(
            told(Edited::Started {
                started_at: 1,
                branch: Some("main".to_string()),
                start: "1a2b3c4d5e6f".to_string(),
                again: false,
            }),
            "started on main at 1a2b3c4"
        );
        assert_eq!(
            told(Edited::Started {
                started_at: 1,
                branch: None,
                start: "1a2b3c4d5e6f".to_string(),
                again: true,
            }),
            "started again at 1a2b3c4"
        );
    }

    /// A start reads back as written, leaving out a branch it hasn't and a first start's `again`
    #[test]
    fn starts_read_back_as_written() {
        let started = Edited::Started {
            started_at: 7,
            branch: None,
            start: "abc".to_string(),
            again: false,
        };
        let value = serde_json::to_value(&started).unwrap();
        assert_eq!(
            value,
            json!({"edit": "started", "started_at": 7, "start": "abc"})
        );
        assert_eq!(serde_json::from_value::<Edited>(value).unwrap(), started);
    }
}
