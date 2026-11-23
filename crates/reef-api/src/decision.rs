//! Decisions made on a node's work: each plan a session asked approval for and its verdict, and
//! each question it asked and the answer picked, kept with the node
//!
//! - **Read from the session's transcript.** Claude Code writes each ExitPlanMode and
//!   AskUserQuestion it calls, and what came of it, to the session's transcript, whether the
//!   session runs in a terminal or under a runner. reef's hooks read them from there at each
//!   turn's end and at the session's, so every session's decisions are kept the same way.
//! - **Kept by the call that asked.** A decision is Claude Code's tool call, named by its id, so
//!   reading a transcript again changes nothing, and a call answered since it was first read
//!   takes its outcome.
//! - **Numbered on its node,** from 1 in the order they were first kept.

use crate::session::Question;
use reef_model::NodeName;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a decision was answered
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Via {
    /// In reef's app, for a session a runner ran
    App,
    /// In the terminal the session ran in
    Terminal,
}

impl Via {
    /// Where it was answered, as a person reads it
    pub fn describe(self) -> &'static str {
        match self {
            Via::App => "in reef's app",
            Via::Terminal => "in the terminal",
        }
    }
}

/// What a decision asked
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DecisionKind {
    /// A plan, for approval
    Plan {
        /// The plan as markdown
        plan: String,
        /// The plan as HTML, raw HTML in it escaped, which central reef renders when it is read
        #[serde(default, skip_serializing_if = "String::is_empty")]
        html: String,
    },
    /// Questions, each with its choices
    Question {
        /// The questions
        questions: Vec<Question>,
    },
}

/// What came of a decision
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Outcome {
    /// The plan was approved
    Approved,
    /// The plan was rejected, with what to change
    Rejected {
        /// The feedback Claude was given, which may be empty
        #[serde(default)]
        feedback: String,
    },
    /// The questions were answered
    Answered {
        /// Each question's text, and the answer picked or written for it
        answers: BTreeMap<String, String>,
    },
    /// The questions were left unanswered
    Declined {
        /// What Claude was told
        #[serde(default)]
        message: String,
    },
}

impl Outcome {
    /// What came of it, as a person reads it
    pub fn describe(&self) -> String {
        match self {
            Outcome::Approved => "approved".to_string(),
            Outcome::Rejected { feedback } if feedback.is_empty() => "rejected".to_string(),
            Outcome::Rejected { feedback } => format!("rejected: {feedback}"),
            Outcome::Answered { answers } => answers
                .iter()
                .map(|(question, answer)| format!("{question} → {answer}"))
                .collect::<Vec<_>>()
                .join("; "),
            Outcome::Declined { message } if message.is_empty() => "not answered".to_string(),
            Outcome::Declined { message } => format!("not answered: {message}"),
        }
    }
}

/// A decision as a hook read it from a session's transcript, to be kept on its node
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decided {
    /// Claude Code's id for the tool call that asked, which keeps it
    pub call: String,
    /// The Claude Code session it was asked in
    pub session: String,
    /// Where it was answered
    pub via: Via,
    /// When it was asked, in milliseconds since the epoch
    pub asked_at: u64,
    /// When it was answered, if it was
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<u64>,
    /// What it asked
    #[serde(flatten)]
    pub kind: DecisionKind,
    /// What came of it, once it was answered
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

/// One decision kept on a node
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Its number on its node, from 1
    pub number: u32,
    /// Bumped by every write of it
    pub version: u64,
    /// The decision, as read from its session
    #[serde(flatten)]
    pub decided: Decided,
}

/// Decisions to keep on a node, each kept by its call
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordDecisions {
    /// The decisions, in the order they were asked
    pub decisions: Vec<Decided>,
}

/// What came of keeping a node's decisions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionsRecorded {
    /// They were kept: those new to the node, and those whose outcome or plan changed
    Recorded {
        /// The node
        node: NodeName,
        /// How many were new
        added: u32,
        /// How many were kept already and changed
        updated: u32,
    },
    /// The node isn't in its plan, so nothing was kept
    NotInPlan {
        /// The node
        node: NodeName,
    },
}

#[cfg(test)]
mod tests {
    use super::{Decided, Decision, DecisionKind, Outcome, Via};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A decision flattens what it asked and what came of it beside its call, and reads back
    #[test]
    fn a_decision_reads_as_written() {
        let decision = Decision {
            number: 2,
            version: 1,
            decided: Decided {
                call: "toolu_1".to_string(),
                session: "s1".to_string(),
                via: Via::Terminal,
                asked_at: 10,
                answered_at: Some(12),
                kind: DecisionKind::Plan {
                    plan: "# Plan".to_string(),
                    html: String::new(),
                },
                outcome: Some(Outcome::Rejected {
                    feedback: "In capitals.".to_string(),
                }),
            },
        };
        let wire = serde_json::to_value(&decision).unwrap();
        assert_eq!(
            wire,
            json!({
                "number": 2, "version": 1, "call": "toolu_1", "session": "s1", "via": "terminal",
                "asked_at": 10, "answered_at": 12, "kind": "plan", "plan": "# Plan",
                "outcome": {"outcome": "rejected", "feedback": "In capitals."},
            })
        );
        assert_eq!(serde_json::from_value::<Decision>(wire).unwrap(), decision);
        // and an outcome says what came of it in words
        let answered = Outcome::Answered {
            answers: BTreeMap::from([("Which?".to_string(), "Hi".to_string())]),
        };
        assert_eq!(answered.describe(), "Which? → Hi");
        assert_eq!(
            Outcome::Rejected {
                feedback: String::new()
            }
            .describe(),
            "rejected"
        );
    }
}
