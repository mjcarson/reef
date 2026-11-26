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

use reef_model::{Met, NodeName, Status};
use serde::{Deserialize, Serialize};

use crate::queue::Place;
use crate::user::Author;

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
            Edited::Other => "an edit this reef can't read".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Edited, HistoryPage, Logged};
    use crate::comment::Writer;
    use crate::queue::Place;
    use crate::user::Author;
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
    }
}
