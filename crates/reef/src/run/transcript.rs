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
//!
//! A session in a terminal is read from the transcript Claude Code writes for it on disk instead,
//! by a [`Recorder`]: its `assistant` and `user` records take the stream's shapes, one block an
//! `assistant` record, and a `system` `turn_duration` ends each turn.

use reef_api::session::cut;
use reef_api::{
    Answer, DecisionKind, Entry, EntryKind, Outcome, Prompt, PromptKind, Question, SessionState,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

use crate::decisions::{self, iso_ms};

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

/// What one line of a session's on-disk transcript adds: its entries, and where the session
/// stands after it, if the line says
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recorded {
    /// The entries, in the order the line holds them
    pub entries: Vec<Entry>,
    /// Where the session stands after it, if the line says
    pub state: Option<SessionState>,
    /// The permission mode the line names the session running in, if it names one
    pub mode: Option<String>,
}

/// Reads a Claude Code session's on-disk transcript a line at a time, as the entries the app
/// shows of a session in a terminal
///
/// - **Each record once.** Claude Code writes some records twice, so a `uuid` read already is
///   passed over.
/// - **The session's own lines.** What Claude Code injects itself (`isMeta`) and a subagent's
///   lines (`isSidechain`) are left out.
/// - **Questions and plans as history.** An AskUserQuestion or ExitPlanMode call reads as the
///   prompt it asked, and its result as the answer it got, as the hooks read decisions
///   ([`crate::decisions`]). Nothing waits on the app, since the terminal answered it.
/// - **Where it stands.** A message typed starts a turn, and `turn_duration` ends it.
#[derive(Debug, Clone, Default)]
pub struct Recorder {
    /// Each question or plan asked and not answered yet, by its call's id, with the tool called
    asks: HashMap<String, String>,
    /// Each record's `uuid` read already
    seen: HashSet<String>,
    /// The permission mode the session last ran in, if a line has said
    mode: Option<String>,
}

impl Recorder {
    /// The permission mode the session last ran in, `default` until a line says
    pub fn mode(&self) -> String {
        self.mode.clone().unwrap_or_else(|| "default".to_string())
    }

    /// Reads one line of the transcript as its entries, each at the record's own time
    ///
    /// A line that isn't JSON, a subagent's, one Claude Code injected and one read already add
    /// nothing.
    ///
    /// # Arguments
    ///
    /// * `line` - The line
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::run::transcript::Recorder;
    /// use reef_api::{EntryKind, SessionState};
    ///
    /// let mut recorder = Recorder::default();
    /// let line = r#"{"type":"user","uuid":"u1","timestamp":"2026-11-25T10:00:00Z","message":{"role":"user","content":"Add a test"}}"#;
    /// let read = recorder.read(line);
    /// assert_eq!(read.entries[0].kind, EntryKind::User { text: "Add a test".to_string() });
    /// assert_eq!(read.state, Some(SessionState::Working));
    /// assert!(recorder.read(line).entries.is_empty());
    /// ```
    pub fn read(&mut self, line: &str) -> Recorded {
        // only JSON is the transcript's
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return Recorded::default();
        };
        // a subagent's line is its own, not the session's
        if record["isSidechain"] == true {
            return Recorded::default();
        }
        // the mode the session runs in, from any record that names it
        let mode = record["permissionMode"].as_str().map(str::to_string);
        if mode.is_some() {
            self.mode.clone_from(&mode);
        }
        // what Claude Code injected itself, and a record written twice, add nothing more
        let named = Recorded {
            mode,
            ..Recorded::default()
        };
        if record["isMeta"] == true {
            return named;
        }
        if let Some(uuid) = record["uuid"].as_str()
            && !self.seen.insert(uuid.to_string())
        {
            return named;
        }
        // each entry at the record's time, else now
        let at = record["timestamp"]
            .as_str()
            .and_then(iso_ms)
            .unwrap_or_else(super::now_ms);
        let (kinds, state) = match record["type"].as_str().unwrap_or_default() {
            "assistant" => (self.said(&record), Some(SessionState::Working)),
            "user" => self.heard(&record),
            "system" if record["subtype"] == "turn_duration" => {
                (Vec::new(), Some(SessionState::Idle))
            }
            // a hook's message to the user, such as Stop's
            "system" if record["subtype"] == "informational" => {
                let text = record["content"].as_str().unwrap_or_default();
                (vec![EntryKind::Note { text: cut(text) }], None)
            }
            _ => (Vec::new(), None),
        };
        let entries = kinds.into_iter().map(|kind| Entry { at, kind }).collect();
        Recorded {
            entries,
            state,
            ..named
        }
    }

    /// What Claude said in one `assistant` record: its text, each tool it called, and each
    /// question or plan it asked, remembered for its answer
    ///
    /// # Arguments
    ///
    /// * `record` - The record
    fn said(&mut self, record: &Value) -> Vec<EntryKind> {
        // each block a text, a call, or thinking, which is left out
        let blocks = record["message"]["content"]
            .as_array()
            .into_iter()
            .flatten();
        let mut kinds = Vec::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default().trim();
                    if !text.is_empty() {
                        kinds.push(EntryKind::Text { text: cut(text) });
                    }
                }
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or_default();
                    let id = block["id"].as_str().unwrap_or_default();
                    // a question or a plan, as the prompt it asked, else the call as any other
                    let asked = (name == ASK || name == EXIT_PLAN)
                        .then(|| decisions::asked(name, &block["input"]))
                        .flatten();
                    match asked {
                        Some(kind) => {
                            self.asks.insert(id.to_string(), name.to_string());
                            let prompt = Prompt {
                                id: id.to_string(),
                                kind: prompt_kind(kind),
                            };
                            kinds.push(EntryKind::Asked { prompt });
                        }
                        None => kinds.push(EntryKind::Tool {
                            name: name.to_string(),
                            input: one_line(&block["input"]),
                        }),
                    }
                }
                _ => {}
            }
        }
        kinds
    }

    /// What one `user` record says: a message typed, or what each tool answered, a question's
    /// or a plan's as its answer, and whether it started a turn
    ///
    /// # Arguments
    ///
    /// * `record` - The record
    fn heard(&mut self, record: &Value) -> (Vec<EntryKind>, Option<SessionState>) {
        let content = &record["message"]["content"];
        // a message typed is its text alone
        if let Some(text) = content.as_str() {
            return match typed(text) {
                Some((kind, true)) => (vec![kind], Some(SessionState::Working)),
                Some((kind, false)) => (vec![kind], None),
                None => (Vec::new(), None),
            };
        }
        let mut kinds = Vec::new();
        let mut state = None;
        for block in content.as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default();
                    if let Some((kind, starts)) = typed(text) {
                        kinds.push(kind);
                        if starts {
                            state = Some(SessionState::Working);
                        }
                    }
                }
                Some("tool_result") => kinds.push(self.result(record, block)),
                _ => {}
            }
        }
        (kinds, state)
    }

    /// One tool's result: the answer to a question or a plan asked, else its output
    ///
    /// # Arguments
    ///
    /// * `record` - The `user` record holding it, whose `toolUseResult` records the answers
    /// * `block` - The `tool_result` block
    fn result(&mut self, record: &Value, block: &Value) -> EntryKind {
        let id = block["tool_use_id"].as_str().unwrap_or_default();
        let refused = block["is_error"] == true;
        // a question's or a plan's result read as the hooks read it
        if let Some(tool) = self.asks.remove(id) {
            let said = decisions::content_text(&block["content"]);
            let outcome = decisions::outcome(&tool, refused, &said, &record["toolUseResult"]);
            return EntryKind::Answered {
                prompt: id.to_string(),
                answer: answer_of(outcome),
            };
        }
        EntryKind::Output {
            text: cut(&content_text(&block["content"])),
            error: refused,
        }
    }
}

/// A decision's kind as the prompt it would have raised
///
/// # Arguments
///
/// * `kind` - What the call asked
fn prompt_kind(kind: DecisionKind) -> PromptKind {
    match kind {
        DecisionKind::Plan { plan, html } => PromptKind::Plan { plan, html },
        DecisionKind::Question { questions } => PromptKind::Question { questions },
    }
}

/// What came of a question or a plan, as the answer the app would have given
///
/// # Arguments
///
/// * `outcome` - What came of it, as its result says
fn answer_of(outcome: Outcome) -> Answer {
    match outcome {
        Outcome::Approved => Answer::Approve,
        Outcome::Rejected { feedback } => Answer::Reject { feedback },
        Outcome::Answered { answers } => Answer::Answers { answers },
        Outcome::Declined { message } => Answer::Deny { message },
    }
}

/// The text between a tag and its end, if a text holds both
///
/// # Arguments
///
/// * `text` - The text
/// * `name` - The tag's name
fn tagged<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    // the first of each, in order
    let (_, rest) = text.split_once(&format!("<{name}>"))?;
    let (inner, _) = rest.split_once(&format!("</{name}>"))?;
    Some(inner)
}

/// A user's message as the app shows it, and whether it started a turn: a `!` command or a
/// slash command as it was typed, their output as a tool's, and anything else as it was said
///
/// # Arguments
///
/// * `text` - The message
fn typed(text: &str) -> Option<(EntryKind, bool)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // a `!` command, as it was typed
    if let Some(command) = tagged(text, "bash-input") {
        let text = format!("! {}", command.trim());
        return Some((EntryKind::User { text }, false));
    }
    // and what it printed
    let stdout = tagged(text, "bash-stdout").or_else(|| tagged(text, "local-command-stdout"));
    let stderr = tagged(text, "bash-stderr");
    if stdout.is_some() || stderr.is_some() {
        let said: Vec<&str> = [stdout, stderr]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        let error = stderr.is_some_and(|stderr| !stderr.trim().is_empty());
        let text = cut(&said.join("\n"));
        return Some((EntryKind::Output { text, error }, false));
    }
    // a slash command, its arguments after it
    if let Some(name) = tagged(text, "command-name") {
        let args = tagged(text, "command-args").unwrap_or_default().trim();
        let text = match args.is_empty() {
            true => name.trim().to_string(),
            false => format!("{} {args}", name.trim()),
        };
        return Some((EntryKind::User { text }, false));
    }
    // anything else said to Claude starts a turn
    Some((EntryKind::User { text: cut(text) }, true))
}

#[cfg(test)]
mod tests {
    use super::{Read, Recorder, read, respond};
    use reef_api::{Answer, Choice, EntryKind, Prompt, PromptKind, Question, SessionState};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// X5's run A, trimmed: what Claude Code 2.1.330 wrote while a node was planned, a question
    /// answered, a plan rejected and approved, and the node done
    const RUN: &str = include_str!("fixtures/x5-run-a.jsonl");

    /// A terminal session's transcript, built in the shapes Claude Code writes on disk: a message
    /// typed, a caveat it injected, thinking, text, a Bash call and its result, a question
    /// answered, a plan rejected with feedback and then approved, a subagent's line, a record
    /// written twice, a line that isn't JSON, the turn's end, and a `!` command with its output
    const TERMINAL: &str = include_str!("fixtures/terminal.jsonl");

    /// A plan prompt by its call's id
    ///
    /// # Arguments
    ///
    /// * `id` - The ExitPlanMode call's id
    /// * `plan` - The plan's markdown
    fn plan(id: &str, plan: &str) -> EntryKind {
        EntryKind::Asked {
            prompt: Prompt {
                id: id.to_string(),
                kind: PromptKind::Plan {
                    plan: plan.to_string(),
                    html: String::new(),
                },
            },
        }
    }

    /// A terminal's transcript reads as its entries, each once and in order, its question and
    /// plans as asked and answered, what Claude Code injected and a subagent's left out, and
    /// where the session stands after each turn
    #[test]
    fn a_terminals_transcript_reads_as_its_entries() {
        let mut recorder = Recorder::default();
        let read: Vec<_> = TERMINAL.lines().map(|line| recorder.read(line)).collect();
        let kinds: Vec<EntryKind> = read
            .iter()
            .flat_map(|recorded| recorded.entries.iter().map(|entry| entry.kind.clone()))
            .collect();
        let question = "Which greeting should welcome.txt hold?";
        let choice = |label: &str, description: &str| Choice {
            label: label.to_string(),
            description: description.to_string(),
        };
        let asked = EntryKind::Asked {
            prompt: Prompt {
                id: "toolu_q1".to_string(),
                kind: PromptKind::Question {
                    questions: vec![Question {
                        question: question.to_string(),
                        header: "Greeting".to_string(),
                        options: vec![choice("Hello", "The usual"), choice("Hi", "Shorter")],
                        multi_select: false,
                    }],
                },
            },
        };
        let answered = |prompt: &str, answer: Answer| EntryKind::Answered {
            prompt: prompt.to_string(),
            answer,
        };
        let expected = vec![
            EntryKind::User {
                text: "Add a greeting to welcome.txt".to_string(),
            },
            EntryKind::Text {
                text: "I'll look at the file first.".to_string(),
            },
            EntryKind::Tool {
                name: "Bash".to_string(),
                input: "cat welcome.txt".to_string(),
            },
            EntryKind::Output {
                text: "Hello".to_string(),
                error: false,
            },
            asked,
            answered(
                "toolu_q1",
                Answer::Answers {
                    answers: BTreeMap::from([(question.to_string(), "Hi".to_string())]),
                },
            ),
            plan("toolu_p1", "# Plan\n\nWrite Hi to welcome.txt."),
            answered(
                "toolu_p1",
                Answer::Reject {
                    feedback: "Say it in capitals.".to_string(),
                },
            ),
            plan("toolu_p2", "# Plan\n\nWrite HI to welcome.txt."),
            answered("toolu_p2", Answer::Approve),
            EntryKind::Text {
                text: "Done: welcome.txt says HI.".to_string(),
            },
            EntryKind::User {
                text: "! git status --short".to_string(),
            },
            EntryKind::Output {
                text: "M welcome.txt".to_string(),
                error: false,
            },
        ];
        assert_eq!(kinds, expected);
        // each entry at its record's time
        let first = &read[1].entries[0];
        assert_eq!(
            first.at,
            crate::decisions::iso_ms("2026-11-25T10:00:00.000Z").unwrap()
        );
        // the message typed started the turn, and its end left the session idle, which the `!`
        // command after it didn't change
        let states: Vec<SessionState> = read.iter().filter_map(|recorded| recorded.state).collect();
        assert_eq!(states.first(), Some(&SessionState::Working));
        assert_eq!(states.last(), Some(&SessionState::Idle));
        // and the mode is the one it last ran in
        assert_eq!(recorder.mode(), "default");
    }

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
