//! A headless Claude session's stream-json lines, read as transcript entries and prompts, and the
//! app's answers written back as the session reads them
//!
//! X5 found these shapes with Claude Code 2.1.330 (`spikes/x5-headless`):
//!
//! - **A prompt is a `control_request`,** `subtype` `can_use_tool`, naming the tool, its input and
//!   a `request_id`. The session waits until a `control_response` names that id.
//! - **A question is answered by allowing it,** its input given back with `answers` added: each
//!   question's text, and the label picked or the answer written.
//! - **A plan is approved by allowing ExitPlanMode, and rejected by denying it** with feedback,
//!   which Claude plans again from.
//! - **Each turn starts with a `system` `init`,** naming Claude Code's session and its mode, and
//!   ends with a `result`, after the Stop hook has run.

use reef_api::session::cut;
use reef_api::{Answer, EntryKind, Prompt, PromptKind, Question};
use serde_json::{Value, json};

/// The tool Claude asks the user questions with
pub const ASK: &str = "AskUserQuestion";

/// The tool Claude asks for its plan's approval with
pub const EXIT_PLAN: &str = "ExitPlanMode";

/// The most characters a tool's input keeps in one line of the transcript
const INPUT_LINE_MAX: usize = 200;

/// What one line the session wrote says
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    /// Entries for the transcript
    Said(Vec<EntryKind>),
    /// A prompt raised, with the tool's input as Claude asked it, which an answer gives back
    Raised {
        /// The prompt
        prompt: Prompt,
        /// The tool's input
        input: Value,
    },
    /// A request the runner doesn't answer as a prompt, which it refuses so nothing waits on it
    Other {
        /// The request's id
        id: String,
        /// What kind of request it is
        subtype: String,
    },
    /// A prompt the session no longer waits on
    Withdrawn {
        /// The prompt's id
        id: String,
    },
    /// A turn began, in a Claude Code session
    Began {
        /// Claude Code's id for the session
        id: String,
        /// The permission mode it runs in
        mode: String,
    },
    /// The turn ended
    Ended {
        /// How many turns of the model it took
        turns: u64,
        /// What the session has cost so far, in dollars
        cost: f64,
        /// How long the turn took, in milliseconds
        ms: u64,
    },
    /// Nothing the transcript keeps
    Nothing,
}

/// Reads one line the session wrote
///
/// A line that isn't JSON, such as a wrapper's own output, is nothing.
///
/// # Arguments
///
/// * `line` - The line
///
/// # Examples
///
/// ```
/// use reef::run::transcript::{Read, read};
///
/// let line = r#"{"type":"result","num_turns":3,"total_cost_usd":0.15,"duration_ms":900}"#;
/// assert_eq!(read(line), Read::Ended { turns: 3, cost: 0.15, ms: 900 });
/// assert_eq!(read("running 1 test"), Read::Nothing);
/// ```
pub fn read(line: &str) -> Read {
    // only JSON is the session's
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return Read::Nothing;
    };
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    match msg["type"].as_str().unwrap_or_default() {
        // each turn's start names the session and its mode
        "system" if msg["subtype"] == "init" => Read::Began {
            id: text(&msg["session_id"]),
            mode: text(&msg["permissionMode"]),
        },
        // a hook's message to the user, such as Stop's
        "system" if msg["subtype"] == "informational" => Read::Said(vec![EntryKind::Note {
            text: cut(&text(&msg["content"])),
        }]),
        "assistant" => Read::Said(said(&msg)),
        "user" => Read::Said(outputs(&msg)),
        "control_request" => requested(&msg),
        "control_cancel_request" => Read::Withdrawn {
            id: text(&msg["request_id"]),
        },
        "result" => Read::Ended {
            turns: msg["num_turns"].as_u64().unwrap_or(0),
            cost: msg["total_cost_usd"].as_f64().unwrap_or(0.0),
            ms: msg["duration_ms"].as_u64().unwrap_or(0),
        },
        _ => Read::Nothing,
    }
}

/// What Claude said in one message: its text, and each tool it called
///
/// A question or a plan is left out, since the prompt it raises shows it.
///
/// # Arguments
///
/// * `msg` - An `assistant` line
fn said(msg: &Value) -> Vec<EntryKind> {
    // each block a text or a tool's call, thinking left out
    let blocks = msg["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or_default().trim();
                (!text.is_empty()).then(|| EntryKind::Text { text: cut(text) })
            }
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or_default();
                (name != ASK && name != EXIT_PLAN).then(|| EntryKind::Tool {
                    name: name.to_string(),
                    input: one_line(&block["input"]),
                })
            }
            _ => None,
        })
        .collect()
}

/// What each tool answered in one message
///
/// # Arguments
///
/// * `msg` - A `user` line
fn outputs(msg: &Value) -> Vec<EntryKind> {
    // only tool results: a user's own message is already in the transcript as sent
    let blocks = msg["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter(|block| block["type"] == "tool_result")
        .map(|block| EntryKind::Output {
            text: cut(&content_text(&block["content"])),
            error: block["is_error"] == true,
        })
        .collect()
}

/// A tool result's content as text: its own text, or its text blocks joined
///
/// # Arguments
///
/// * `content` - The result's `content`
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        // text blocks joined, anything else said by its type
        Value::Array(blocks) => blocks
            .iter()
            .map(|block| match block["text"].as_str() {
                Some(text) => text.to_string(),
                None => format!("[{}]", block["type"].as_str().unwrap_or("content")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A tool's input in one line, cut if long: a command or a path as it is, else its JSON
///
/// # Arguments
///
/// * `input` - The tool's input
fn one_line(input: &Value) -> String {
    // what most tools are called with, said plainly
    let plain = ["command", "file_path", "path", "pattern", "url"]
        .iter()
        .find_map(|key| input[*key].as_str().map(str::to_string));
    let line = plain
        .unwrap_or_else(|| input.to_string())
        .replace('\n', " ");
    let count = line.chars().count();
    match count > INPUT_LINE_MAX {
        true => format!("{}…", line.chars().take(INPUT_LINE_MAX).collect::<String>()),
        false => line,
    }
}

/// A control request read as a prompt, or as one the runner refuses
///
/// # Arguments
///
/// * `msg` - A `control_request` line
fn requested(msg: &Value) -> Read {
    let id = msg["request_id"].as_str().unwrap_or_default().to_string();
    let request = &msg["request"];
    // only a tool's permission is the user's to answer
    if request["subtype"] != "can_use_tool" {
        return Read::Other {
            id,
            subtype: request["subtype"].as_str().unwrap_or_default().to_string(),
        };
    }
    let tool = request["tool_name"].as_str().unwrap_or_default();
    let input = request["input"].clone();
    // a question, a plan, or any other tool's permission
    let questions = (tool == ASK)
        .then(|| serde_json::from_value::<Vec<Question>>(input["questions"].clone()).ok())
        .flatten();
    let kind = match (tool, questions, input["plan"].as_str()) {
        (ASK, Some(questions), _) => PromptKind::Question { questions },
        (EXIT_PLAN, _, Some(plan)) => PromptKind::Plan {
            plan: plan.to_string(),
            html: String::new(),
        },
        _ => PromptKind::Permission {
            tool: tool.to_string(),
            input: cut(&serde_json::to_string_pretty(&input).unwrap_or_default()),
        },
    };
    Read::Raised {
        prompt: Prompt { id, kind },
        input,
    }
}

/// A user's message, as the session reads one on its stdin
///
/// # Arguments
///
/// * `text` - What the user says
pub fn user(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null,
        "session_id": "",
    })
}

/// The answer to a prompt, as the session reads it: the tool allowed with its input, a
/// question's answers added, or refused with what Claude is told
///
/// # Arguments
///
/// * `id` - The prompt's id
/// * `input` - The tool's input, as Claude asked it
/// * `answer` - The app's answer
///
/// # Examples
///
/// ```
/// use reef::run::transcript::respond;
/// use reef_api::Answer;
/// use serde_json::json;
///
/// let answer = Answer::Reject { feedback: "Name the test.".to_string() };
/// let sent = respond("r1", &json!({"plan": "# Plan"}), &answer);
/// assert_eq!(sent["response"]["response"]["behavior"], "deny");
/// assert_eq!(sent["response"]["response"]["message"], "Name the test.");
/// ```
pub fn respond(id: &str, input: &Value, answer: &Answer) -> Value {
    let decision = match answer {
        // a question's answers go back in its input, each by its question's text
        Answer::Answers { answers } => {
            let mut updated = input.as_object().cloned().unwrap_or_default();
            updated.insert("answers".to_string(), json!(answers));
            json!({ "behavior": "allow", "updatedInput": updated })
        }
        // anything else allowed runs as Claude asked it
        Answer::Allow | Answer::Approve => json!({ "behavior": "allow", "updatedInput": input }),
        // a refusal says why, or that the user refused
        Answer::Deny { message } => json!({
            "behavior": "deny",
            "message": or(message, "The user denied this in reef's app."),
        }),
        Answer::Reject { feedback } => json!({
            "behavior": "deny",
            "message": or(feedback, "The user rejected the plan in reef's app."),
        }),
    };
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": id, "response": decision },
    })
}

/// A refusal of a request the runner doesn't answer, so the session waits on nothing
///
/// # Arguments
///
/// * `id` - The request's id
/// * `subtype` - What kind of request it was
pub fn refuse(id: &str, subtype: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": id,
            "error": format!("reef's runner doesn't answer `{subtype}` requests"),
        },
    })
}

/// A text, or a default if it is blank
///
/// # Arguments
///
/// * `text` - The text
/// * `default` - What stands in for a blank one
fn or(text: &str, default: &str) -> String {
    match text.trim().is_empty() {
        true => default.to_string(),
        false => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Read, read, respond};
    use reef_api::{Answer, EntryKind, PromptKind};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// X5's run A, trimmed: what Claude Code 2.1.330 wrote while a node was planned, a question
    /// answered, a plan rejected and approved, and the node done
    const RUN: &str = include_str!("fixtures/x5-run-a.jsonl");

    /// Every line of a real run reads, each into what the transcript keeps
    #[test]
    fn a_real_run_reads_as_its_transcript() {
        let reads: Vec<Read> = RUN.lines().map(read).collect();
        // the turn began in a session, in the account's default mode
        assert!(
            matches!(&reads[0], Read::Began { mode, .. } if mode == "default"),
            "{:?}",
            reads[0]
        );
        // a write tool, a question and two plans were raised, in that order
        let raised: Vec<&PromptKind> = reads
            .iter()
            .filter_map(|read| match read {
                Read::Raised { prompt, .. } => Some(&prompt.kind),
                _ => None,
            })
            .collect();
        assert_eq!(raised.len(), 4, "{raised:?}");
        assert!(
            matches!(raised[0], PromptKind::Permission { tool, .. } if tool == "mcp__x4__note")
        );
        let PromptKind::Question { questions } = raised[1] else {
            panic!("the second prompt is the question: {:?}", raised[1]);
        };
        assert_eq!(questions[0].header, "Non-ASCII");
        assert_eq!(questions[0].options[0].label, "Keep (Recommended)");
        assert!(!questions[0].multi_select);
        assert!(matches!(raised[2], PromptKind::Plan { plan, .. } if plan.starts_with("# N1")));
        assert!(matches!(raised[3], PromptKind::Plan { .. }));
        // what Claude said and what its tools answered, the question and plans left to prompts
        let said: Vec<&EntryKind> = reads
            .iter()
            .filter_map(|read| match read {
                Read::Said(kinds) => Some(kinds),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(said.iter().any(
            |kind| matches!(kind, EntryKind::Tool { name, .. } if name == "mcp__x4__next_task")
        ));
        assert!(
            !said
                .iter()
                .any(|kind| matches!(kind, EntryKind::Tool { name, .. } if name == "ExitPlanMode"))
        );
        assert!(said.iter().any(|kind| matches!(kind, EntryKind::Output { error: true, text } if text.starts_with("Not yet"))));
        assert!(
            said.iter().any(
                |kind| matches!(kind, EntryKind::Note { text } if text.starts_with("Stop says"))
            )
        );
        // and the turn ended, after 18 turns of the model
        assert!(
            reads
                .iter()
                .any(|read| matches!(read, Read::Ended { turns: 18, .. }))
        );
    }

    /// An answer goes back as the session reads it: a question's in its input, a refusal as a
    /// message, and a blank one said for the user
    #[test]
    fn answers_go_back_as_the_session_reads_them() {
        let input = json!({"questions": [{"question": "Keep?", "options": []}]});
        // a question's answers added to its input, the rest of it kept
        let answers = Answer::Answers {
            answers: BTreeMap::from([("Keep?".to_string(), "Keep".to_string())]),
        };
        let sent = respond("r2", &input, &answers);
        assert_eq!(sent["type"], "control_response");
        assert_eq!(sent["response"]["request_id"], "r2");
        let decision = &sent["response"]["response"];
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedInput"]["answers"]["Keep?"], "Keep");
        assert_eq!(decision["updatedInput"]["questions"], input["questions"]);
        // an approval runs as asked, and a blank denial says the user refused
        let approved = respond("r3", &json!({"plan": "p"}), &Answer::Approve);
        assert_eq!(
            approved["response"]["response"]["updatedInput"]["plan"],
            "p"
        );
        let denied = respond(
            "r4",
            &json!({}),
            &Answer::Deny {
                message: " ".into(),
            },
        );
        assert!(
            denied["response"]["response"]["message"]
                .as_str()
                .unwrap()
                .contains("denied this in reef's app")
        );
    }
}
