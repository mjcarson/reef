//! Sessions reef runs: a runner the user starts attaches to central reef, which relays its Claude
//! session's transcript and prompts to the app, and the app's answers back
//!
//! - **The server runs nothing.** The runner, `reef loop run`, runs on the user's own machine and
//!   spawns Claude there. Central reef keeps what it reports, and queues what the app asks of it.
//! - **A runner is a pane of its own,** [`Pane::Reef`], so its sessions' MCP server and hooks find
//!   its loop as a terminal session's find its multiplexer's pane.
//! - **Kept in memory.** Claude Code keeps the whole transcript on disk, so a session's record
//!   lives as long as the service does, and a runner whose service restarted attaches again with
//!   the transcript it holds.
//! - **Prompts wait for the app.** Each prompt the session raises is pending until the app answers
//!   it, and several can be pending at once.
//! - **A terminal's session attaches itself.** `reef loop run` typed in a Claude Code session
//!   attaches that session under an id of its own, on its terminal's pane, and a follower streams
//!   its transcript. It raises no prompts, since its terminal answers them, and the app's acts on
//!   it are refused as [`SessionActed::InTerminal`].

use crate::loops::Pane;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How long a runner can go without asking for orders before it reads as gone, in milliseconds
pub const GONE_AFTER_MS: u64 = 60_000;

/// The most characters one entry's text keeps, past which it is cut
pub const ENTRY_TEXT_MAX: usize = 4_000;

/// The most entries central reef keeps of a session, the oldest let go past it, and so the most
/// a runner or a follower holds to attach again with
pub const KEPT_ENTRIES: usize = 2_000;

/// Where a runner's session stands, as the runner last said, or gone if it stopped saying
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    /// Attached, with no Claude running: its loop holds nothing, is stopped, or isn't started
    Idle,
    /// Claude is working a turn
    Working,
    /// Claude's turn ended mid-node, so it waits for the user's reply
    Waiting,
    /// Its loop is paused, and the node it was on is done
    Paused,
    /// The runner let go of it
    Detached,
    /// The runner hasn't asked for orders in a minute
    Gone,
}

impl SessionState {
    /// The state as a person reads it
    pub fn describe(self) -> &'static str {
        match self {
            SessionState::Idle => "idle",
            SessionState::Working => "working",
            SessionState::Waiting => "waiting for a reply",
            SessionState::Paused => "paused",
            SessionState::Detached => "detached",
            SessionState::Gone => "gone",
        }
    }
}

/// One choice a question offers
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// What the choice says, and what the answer names
    pub label: String,
    /// What picking it means
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// One question of an AskUserQuestion, in the shape Claude asks it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// The question, which its answer is keyed by
    pub question: String,
    /// A short label for it
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header: String,
    /// The choices it offers, beside an answer of the user's own
    pub options: Vec<Choice>,
    /// Whether several choices can be picked at once
    #[serde(default)]
    pub multi_select: bool,
}

/// What a prompt asks of the user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PromptKind {
    /// Whether a tool may run as Claude asked it to
    Permission {
        /// The tool's name as the session calls it, such as `Bash` or `mcp__reef__put`
        tool: String,
        /// The tool's input, as pretty JSON, cut if long
        input: String,
    },
    /// Claude's questions, each with its choices
    Question {
        /// The questions
        questions: Vec<Question>,
    },
    /// Claude's plan, for approval
    Plan {
        /// The plan as markdown
        plan: String,
        /// The plan as HTML, raw HTML in it escaped, which central reef renders when it is raised
        #[serde(default, skip_serializing_if = "String::is_empty")]
        html: String,
    },
}

/// A prompt the session raised, which waits for the app's answer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    /// The session's id for it, which the answer names
    pub id: String,
    /// What it asks
    #[serde(flatten)]
    pub kind: PromptKind,
}

/// An answer to a prompt
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub enum Answer {
    /// Let the tool run as asked
    Allow,
    /// Refuse the tool, saying why
    Deny {
        /// What Claude is told, which may be empty
        #[serde(default)]
        message: String,
    },
    /// Answer Claude's questions, each by its text
    Answers {
        /// Each question's text, and the answer picked or written for it
        answers: BTreeMap<String, String>,
    },
    /// Approve Claude's plan
    Approve,
    /// Reject Claude's plan, with feedback it plans again from
    Reject {
        /// What Claude is told to change
        feedback: String,
    },
}

impl Answer {
    /// Whether the answer lets the tool run, a question answered and a plan approved included
    pub fn allows(&self) -> bool {
        matches!(
            self,
            Answer::Allow | Answer::Answers { .. } | Answer::Approve
        )
    }

    /// The answer as a person reads it in a transcript
    pub fn describe(&self) -> String {
        match self {
            Answer::Allow => "allowed".to_string(),
            Answer::Deny { message } if message.is_empty() => "denied".to_string(),
            Answer::Deny { message } => format!("denied: {message}"),
            Answer::Answers { answers } => answers
                .iter()
                .map(|(question, answer)| format!("{question} → {answer}"))
                .collect::<Vec<_>>()
                .join("; "),
            Answer::Approve => "approved".to_string(),
            Answer::Reject { feedback } => format!("rejected: {feedback}"),
        }
    }
}

/// What one entry of a transcript says
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EntryKind {
    /// What was said to Claude: the runner's kickoff, or the user's reply
    User {
        /// The message
        text: String,
    },
    /// What Claude said
    Text {
        /// Its text, as markdown
        text: String,
    },
    /// A tool Claude called
    Tool {
        /// The tool's name
        name: String,
        /// Its input in a line, cut if long
        input: String,
    },
    /// What a tool answered
    Output {
        /// Its text, cut if long
        text: String,
        /// Whether it answered with an error
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        error: bool,
    },
    /// A prompt the session raised
    Asked {
        /// The prompt
        prompt: Prompt,
    },
    /// A prompt answered
    Answered {
        /// The prompt's id
        prompt: String,
        /// The answer
        answer: Answer,
    },
    /// A Claude Code session started, a fresh one a node
    Session {
        /// Claude Code's id for it
        id: String,
        /// The permission mode it started in
        mode: String,
    },
    /// A turn's end
    Turn {
        /// How many turns of the model it took
        turns: u64,
        /// What the session has cost so far, in dollars
        cost: f64,
        /// How long the turn took, in milliseconds
        ms: u64,
    },
    /// What the runner did, or saw, in words
    Note {
        /// The note
        text: String,
    },
}

/// One entry of a session's transcript
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// When it came, in milliseconds since the epoch
    pub at: u64,
    /// What it says
    #[serde(flatten)]
    pub kind: EntryKind,
}

/// One runner's session, as central reef keeps it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Its id, its pane's
    pub id: String,
    /// The runner's pane, which its loop is kept by
    pub pane: Pane,
    /// The project it takes nodes from
    pub project: String,
    /// The one plan of it the runner narrows to, if one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The host the runner runs on
    pub host: String,
    /// The checkout its sessions run in
    pub cwd: PathBuf,
    /// When it attached, in milliseconds since the epoch
    pub attached_at: u64,
    /// When the runner was last heard from, in milliseconds since the epoch
    pub seen_at: u64,
    /// Where it stands
    pub state: SessionState,
    /// Claude Code's id for the session running now, if one is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    /// How many entries its transcript has had
    pub entries: u64,
    /// The prompts waiting for an answer, oldest first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<Prompt>,
}

impl Session {
    /// The session as it reads at a time: gone once its runner hasn't been heard from in a
    /// minute, unless it let go first
    ///
    /// # Arguments
    ///
    /// * `now` - The time, in milliseconds since the epoch
    pub fn at(mut self, now: u64) -> Session {
        // a runner that let go said so, and one unheard from is gone
        if self.state != SessionState::Detached && now.saturating_sub(self.seen_at) > GONE_AFTER_MS
        {
            self.state = SessionState::Gone;
        }
        self
    }

    /// Whether its runner is there to take orders
    pub fn live(&self) -> bool {
        !matches!(self.state, SessionState::Detached | SessionState::Gone)
    }
}

/// A runner attaching to central reef, or attaching again after its service restarted, or a
/// Claude Code session in a terminal attaching itself
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attach {
    /// The session's own id, which a terminal's session names since its pane isn't a runner's;
    /// a runner's is its pane's
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The runner's pane, or the terminal's the session runs in
    pub pane: Pane,
    /// The project it takes nodes from
    pub project: String,
    /// The one plan of it the runner narrows to, if one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The host the runner runs on
    pub host: String,
    /// The checkout its sessions run in
    pub cwd: PathBuf,
    /// Where it stands
    pub state: SessionState,
    /// The transcript it holds, empty on its first attach
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// The prompts waiting for an answer
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<Prompt>,
    /// Claude Code's id for the session running as it attaches, if one is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
}

/// What a runner reports: its entries since its last report, and where it stands now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The entries since its last report
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// Where it stands
    pub state: SessionState,
    /// Claude Code's id for the session running now, if one is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    /// The prompts waiting for an answer
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<Prompt>,
}

/// Part of a session's transcript
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The session as it stands
    pub session: Session,
    /// The number of the first entry given, from 0
    pub from: u64,
    /// The entries from there on
    pub entries: Vec<Entry>,
}

/// What central reef asks of a runner
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "order", rename_all = "kebab-case")]
pub enum Order {
    /// Read its loop again, which the app changed, and do what it now says
    Wake,
    /// Answer a prompt the session raised
    Answer {
        /// The prompt's id
        prompt: String,
        /// The answer
        answer: Answer,
    },
    /// Say something to Claude, whose turn ended mid-node
    Say {
        /// The message
        text: String,
    },
}

/// An order, numbered from 1 in the order it was given
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ordered {
    /// Its number
    pub n: u64,
    /// The order
    #[serde(flatten)]
    pub order: Order,
}

/// What the app asks of a runner's session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "kebab-case")]
pub enum SessionAct {
    /// Start the runner's loop, or set its paused loop running again
    Start,
    /// Pause its loop, so it stops once the node it is on is done
    Pause,
    /// Stop its loop, letting go of its node, and end Claude's session
    Stop,
    /// Answer a prompt the session raised
    Answer {
        /// The prompt's id
        prompt: String,
        /// The answer
        answer: Answer,
    },
    /// Say something to Claude, whose turn ended mid-node
    Say {
        /// The message
        text: String,
    },
}

/// What came of what the app asked of a runner's session
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionActed {
    /// The runner was told
    Ordered {
        /// The session as it now stands
        session: Session,
    },
    /// The runner's loop was changed, as a loop's edit answered, and the runner told
    Looped {
        /// The session as it now stands
        session: Session,
        /// What came of the loop's change, boxed since a loop is large beside the rest
        edited: Box<crate::loops::LoopEdited>,
    },
    /// No prompt of that id waits for an answer, so nothing was sent
    NoPrompt {
        /// The prompt's id
        prompt: String,
        /// The session as it now stands
        session: Session,
    },
    /// Claude isn't waiting for a reply, so nothing was sent
    NotWaiting {
        /// The session as it now stands
        session: Session,
    },
    /// The runner let go of the session, or stopped asking for orders, so nothing was sent
    Gone {
        /// The session as it now stands
        session: Session,
    },
    /// The session is a terminal's, which only its terminal answers, so nothing was sent
    InTerminal {
        /// The session as it now stands
        session: Session,
    },
}

/// Cuts a text to at most [`ENTRY_TEXT_MAX`] characters, saying how much was left out
///
/// # Arguments
///
/// * `text` - The text
///
/// # Examples
///
/// ```
/// use reef_api::session::cut;
///
/// assert_eq!(cut("short"), "short");
/// assert!(cut(&"x".repeat(5000)).ends_with("… (1000 more characters)"));
/// ```
pub fn cut(text: &str) -> String {
    // most texts are short, and kept whole
    let count = text.chars().count();
    if count <= ENTRY_TEXT_MAX {
        return text.to_string();
    }
    let kept: String = text.chars().take(ENTRY_TEXT_MAX).collect();
    format!("{kept}… ({} more characters)", count - ENTRY_TEXT_MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        Answer, Entry, EntryKind, Order, Ordered, Prompt, PromptKind, Session, SessionState,
    };
    use crate::loops::Pane;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A session seen at a time
    ///
    /// # Arguments
    ///
    /// * `seen_at` - When its runner was last heard from
    /// * `state` - Where it stands
    fn seen(seen_at: u64, state: SessionState) -> Session {
        Session {
            id: "k3x9-4242".to_string(),
            pane: Pane::Reef {
                id: "k3x9-4242".to_string(),
            },
            project: "reef".to_string(),
            plan: None,
            host: "box".to_string(),
            cwd: "/src/reef".into(),
            attached_at: 0,
            seen_at,
            state,
            claude: None,
            entries: 0,
            prompts: Vec::new(),
        }
    }

    /// A runner unheard from for a minute reads as gone, unless it let go
    #[test]
    fn a_silent_runner_is_gone() {
        // heard from within the minute, it stands as it said
        let working = seen(1_000, SessionState::Working);
        assert_eq!(working.clone().at(61_000).state, SessionState::Working);
        assert!(working.clone().at(61_000).live());
        // a minute and a moment later, it is gone
        assert_eq!(working.at(61_001).state, SessionState::Gone);
        // and one that let go stays let go
        let detached = seen(1_000, SessionState::Detached);
        assert_eq!(detached.at(99_000).state, SessionState::Detached);
    }

    /// Prompts, answers, entries and orders take the shapes the runner and the app send
    #[test]
    fn the_wire_reads_as_written() {
        // a plan prompt flattens its kind beside its id
        let prompt = Prompt {
            id: "r1".to_string(),
            kind: PromptKind::Plan {
                plan: "# Plan".to_string(),
                html: String::new(),
            },
        };
        assert_eq!(
            serde_json::to_value(&prompt).unwrap(),
            json!({"id": "r1", "kind": "plan", "plan": "# Plan"})
        );
        // an answer to questions names each by its text
        let answer = Answer::Answers {
            answers: BTreeMap::from([("Keep?".to_string(), "Keep".to_string())]),
        };
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            json!({"answer": "answers", "answers": {"Keep?": "Keep"}})
        );
        assert!(
            answer.allows()
                && !Answer::Reject {
                    feedback: "no".into()
                }
                .allows()
        );
        // an entry flattens its kind beside its time
        let entry = Entry {
            at: 7,
            kind: EntryKind::Answered {
                prompt: "r1".to_string(),
                answer: Answer::Approve,
            },
        };
        let wire = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            wire,
            json!({"at": 7, "kind": "answered", "prompt": "r1", "answer": {"answer": "approve"}})
        );
        assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), entry);
        // and an order its number
        let ordered = Ordered {
            n: 3,
            order: Order::Say {
                text: "go on".to_string(),
            },
        };
        let wire = serde_json::to_value(&ordered).unwrap();
        assert_eq!(wire, json!({"n": 3, "order": "say", "text": "go on"}));
        assert_eq!(serde_json::from_value::<Ordered>(wire).unwrap(), ordered);
    }
}
