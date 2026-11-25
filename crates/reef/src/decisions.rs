//! A session's decisions, read from the transcript Claude Code writes for it: each plan it asked
//! approval for and its verdict, and each question it asked and the answer picked
//!
//! - **One reading for every session.** Claude Code writes each ExitPlanMode and AskUserQuestion
//!   call to the session's transcript, and then its result, whether the session runs in a
//!   terminal or under a runner, so the hooks read both the same way.
//! - **The outcome from the result.** A plan allowed is approved, and one refused is rejected,
//!   with the feedback after Claude Code's own words in a terminal, or the runner's message as
//!   it is. A question allowed is answered, its answers from the result's own record, else from
//!   its text; one refused is declined.
//! - **Each on the node the session last took.** next_task names the node it hands out in
//!   backticks on its result's first line, so a decision is kept on the node the last next_task
//!   before it named. One asked before any is left to the hook to place.
//! - **Each call once.** Claude Code writes some lines twice, so a call or a result already read
//!   is passed over.
//! - **An approval that clears the context is read from the session it starts.** Approving a
//!   plan with *clear context* ends the planning session's ExitPlanMode as a refusal with no
//!   words, which a refusal without feedback also writes. The new session's first message holds
//!   the plan approved, as `planContent`, beside a record naming the planning session's call,
//!   so the approval is read from there, as that call's.

use reef_api::{Decided, DecisionKind, Outcome, Question, Via};
use reef_model::NodeName;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The tool Claude asks for its plan's approval with
const EXIT_PLAN: &str = "ExitPlanMode";

/// The tool Claude asks the user questions with
const ASK: &str = "AskUserQuestion";

/// The tool reef's loop hands a session its node with
const NEXT_TASK: &str = "mcp__reef__next_task";

/// What Claude Code says a refused tool call's feedback after, in a terminal
const SAID: &str = "the user said:";

/// What Claude Code starts a refusal in a terminal with
const REFUSED: &str = "The user doesn't want to proceed with this tool use";

/// The line Claude Code keeps its record of a session's tool calls on
const DATA_FLOW: &str = "data-flow-records";

/// One decision read from a transcript, and the node the session had last taken, if it had
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// The node the last next_task before it named, if one did
    pub node: Option<NodeName>,
    /// The decision
    pub decided: Decided,
}

/// A call read from a transcript, waiting for its result
struct Call {
    /// Where it sits among the decisions read
    at: usize,
    /// The tool it called
    tool: String,
}

/// Reads a session's transcript as its decisions, in the order they were asked
///
/// # Arguments
///
/// * `text` - The transcript, a JSON object a line
/// * `via` - Where its answers were given
///
/// # Examples
///
/// ```
/// use reef::decisions::read;
/// use reef_api::{Outcome, Via};
///
/// let lines = [
///     r##"{"type":"assistant","sessionId":"s1","timestamp":"2026-11-23T17:00:00.000Z","message":{"content":[{"type":"tool_use","id":"t1","name":"ExitPlanMode","input":{"plan":"# Plan"}}]}}"##,
///     r#"{"type":"user","timestamp":"2026-11-23T17:00:05.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"User has approved your plan."}]}}"#,
/// ];
/// let read = read(&lines.join("\n"), Via::Terminal);
/// assert_eq!(read[0].decided.outcome, Some(Outcome::Approved));
/// assert_eq!(read[0].decided.answered_at, Some(read[0].decided.asked_at + 5000));
/// ```
pub fn read(text: &str, via: Via) -> Vec<Read> {
    // what was read, the calls waiting for their results, and the node the session last took
    let mut decisions: Vec<Read> = Vec::new();
    let mut calls: HashMap<String, Call> = HashMap::new();
    let mut taking: HashSet<String> = HashSet::new();
    let mut answered: HashSet<String> = HashSet::new();
    let mut node: Option<NodeName> = None;
    // the plans a clear's approval carried in, and the calls Claude Code's record names
    let mut carried: Vec<(String, u64, Option<NodeName>)> = Vec::new();
    let mut flowed: Vec<String> = Vec::new();
    let mut session = String::new();
    for line in text.lines() {
        // a line that isn't JSON, or isn't a message, says nothing
        let Ok(line) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = line["timestamp"].as_str().and_then(iso_ms).unwrap_or(0);
        if session.is_empty()
            && let Some(id) = line["sessionId"].as_str()
        {
            session = id.to_string();
        }
        // the calls Claude Code's record names, ExitPlanMode's among them
        if line["type"] == DATA_FLOW {
            let records = line["records"].as_array().into_iter().flatten();
            for record in records.filter(|record| record["tool_name"] == EXIT_PLAN) {
                let id = record["tool_use_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if !id.is_empty() && !flowed.contains(&id) {
                    flowed.push(id);
                }
            }
            continue;
        }
        // a plan a clear's approval carried into this session
        if line["type"] == "user"
            && let Some(plan) = line["planContent"].as_str()
        {
            carried.push((plan.to_string(), at, node.clone()));
        }
        let blocks = line["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for block in &blocks {
            match block["type"].as_str() {
                // a call: next_task's, or one that asks the user
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let tool = block["name"].as_str().unwrap_or_default();
                    if tool == NEXT_TASK {
                        taking.insert(id);
                        continue;
                    }
                    if calls.contains_key(&id) || (tool != EXIT_PLAN && tool != ASK) {
                        continue;
                    }
                    let Some(kind) = asked(tool, &block["input"]) else {
                        continue;
                    };
                    let session = line["sessionId"].as_str().unwrap_or_default().to_string();
                    let decided = Decided {
                        call: id.clone(),
                        session,
                        via,
                        asked_at: at,
                        answered_at: None,
                        kind,
                        outcome: None,
                    };
                    let tool = tool.to_string();
                    calls.insert(
                        id,
                        Call {
                            at: decisions.len(),
                            tool,
                        },
                    );
                    decisions.push(Read {
                        node: node.clone(),
                        decided,
                    });
                }
                // a result: the node next_task handed out, or what came of a call that asked
                Some("tool_result") => {
                    let id = block["tool_use_id"].as_str().unwrap_or_default();
                    let said = content_text(&block["content"]);
                    if taking.contains(id) {
                        if let Some(taken) = named_node(&said) {
                            node = Some(taken);
                        }
                        continue;
                    }
                    let Some(call) = calls.get(id) else {
                        continue;
                    };
                    if !answered.insert(id.to_string()) {
                        continue;
                    }
                    let refused = block["is_error"] == true;
                    let decided = &mut decisions[call.at].decided;
                    decided.outcome =
                        Some(outcome(&call.tool, refused, &said, &line["toolUseResult"]));
                    decided.answered_at = Some(at);
                }
                _ => {}
            }
        }
    }
    // each plan a clear's approval carried in, approved, as the planning session's last call
    // this session didn't make itself
    let call = flowed.iter().rev().find(|id| !calls.contains_key(*id));
    if let (Some((plan, at, node)), Some(call)) = (carried.last(), call) {
        let decided = Decided {
            call: call.clone(),
            session,
            via,
            asked_at: *at,
            answered_at: Some(*at),
            kind: DecisionKind::Plan {
                plan: plan.clone(),
                html: String::new(),
            },
            outcome: Some(Outcome::Approved),
        };
        decisions.push(Read {
            node: node.clone(),
            decided,
        });
    }
    decisions
}

/// What a call asked: a plan, or questions, or `None` for input that holds neither
///
/// # Arguments
///
/// * `tool` - The tool called
/// * `input` - Its input
pub(crate) fn asked(tool: &str, input: &Value) -> Option<DecisionKind> {
    match tool {
        EXIT_PLAN => Some(DecisionKind::Plan {
            plan: input["plan"].as_str()?.to_string(),
            html: String::new(),
        }),
        _ => Some(DecisionKind::Question {
            questions: serde_json::from_value::<Vec<Question>>(input["questions"].clone()).ok()?,
        }),
    }
}

/// What came of a call that asked, from its result
///
/// # Arguments
///
/// * `tool` - The tool called
/// * `refused` - Whether its result was an error, as a refusal's is
/// * `said` - Its result's text
/// * `record` - Claude Code's own record of its result
pub(crate) fn outcome(tool: &str, refused: bool, said: &str, record: &Value) -> Outcome {
    match (tool, refused) {
        (EXIT_PLAN, false) => Outcome::Approved,
        (EXIT_PLAN, true) => Outcome::Rejected {
            feedback: feedback(said),
        },
        (_, true) => Outcome::Declined {
            message: feedback(said),
        },
        // the answers as recorded, else as the result's text says them
        (_, false) => {
            let recorded: Option<BTreeMap<String, String>> =
                record["answers"].as_object().map(|answers| {
                    answers
                        .iter()
                        .map(|(question, answer)| {
                            let answer = match answer {
                                Value::String(text) => text.clone(),
                                other => other.to_string(),
                            };
                            (question.clone(), answer)
                        })
                        .collect()
                });
            Outcome::Answered {
                answers: recorded.unwrap_or_else(|| answers_said(said)),
            }
        }
    }
}

/// The feedback a refusal carried: what follows Claude Code's own words in a terminal, nothing
/// if the user gave none, else the refusal's whole message, as a runner sends it
///
/// # Arguments
///
/// * `said` - The refusal's text
///
/// # Examples
///
/// ```
/// use reef::decisions::feedback;
///
/// let terminal = "The user doesn't want to proceed with this tool use. The tool use was \
///     rejected. To tell you how to proceed, the user said:\nName the test.";
/// assert_eq!(feedback(terminal), "Name the test.");
/// assert_eq!(feedback("The user doesn't want to proceed with this tool use. STOP."), "");
/// assert_eq!(feedback("Name the test."), "Name the test.");
/// ```
pub fn feedback(said: &str) -> String {
    // the user's own words, after Claude Code's
    if let Some((_, words)) = said.split_once(SAID) {
        return words.trim().to_string();
    }
    match said.starts_with(REFUSED) {
        true => String::new(),
        false => said.trim().to_string(),
    }
}

/// The answers a question's result says, as `"question"="answer"` pairs
///
/// # Arguments
///
/// * `said` - The result's text
fn answers_said(said: &str) -> BTreeMap<String, String> {
    // each pair's two quoted halves, the question's text possibly holding quotes of its own
    said.split("\", \"")
        .flat_map(|pair| pair.split_once("\"=\""))
        .map(|(question, answer)| {
            let question = question.rsplit_once(": \"").map_or(question, |(_, q)| q);
            let question = question.trim_start_matches('"');
            let answer = answer.split("\".").next().unwrap_or(answer);
            (
                question.to_string(),
                answer.trim_end_matches('"').to_string(),
            )
        })
        .collect()
}

/// A tool result's content as text: its own text, or its text blocks joined
///
/// # Arguments
///
/// * `content` - The result's `content`
pub(crate) fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The node next_task named on its result's first line, in backticks
///
/// # Arguments
///
/// * `said` - next_task's result
///
/// # Examples
///
/// ```
/// use reef::decisions::named_node;
///
/// let took = "Took `reef/central/C75` for the loop in tmux pane %3. Plan it now";
/// assert_eq!(named_node(took).unwrap().to_string(), "reef/central/C75");
/// assert!(named_node("Nothing is ready to take for the loop").is_none());
/// ```
pub fn named_node(said: &str) -> Option<NodeName> {
    // the backticked parts of the first line, the first that names a node
    let first = said.lines().next()?;
    first
        .split('`')
        .skip(1)
        .step_by(2)
        .find_map(|quoted| quoted.parse::<NodeName>().ok())
}

/// A transcript's timestamp, `2026-11-23T17:43:41.123Z`, in milliseconds since the epoch
///
/// # Arguments
///
/// * `stamp` - The timestamp, in UTC
///
/// # Examples
///
/// ```
/// use reef::decisions::iso_ms;
///
/// assert_eq!(iso_ms("1970-01-02T00:00:01.500Z"), Some(86_401_500));
/// assert_eq!(iso_ms("2026-11-23T17:43:41Z"), Some(1_795_455_821_000));
/// assert_eq!(iso_ms("yesterday"), None);
/// ```
pub fn iso_ms(stamp: &str) -> Option<u64> {
    // the date and the time either side of the `T`, the time's fraction optional
    let (date, time) = stamp.trim_end_matches('Z').split_once('T')?;
    let mut ymd = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut hms = clock.splitn(3, ':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    // days since the epoch, by the civil calendar's own arithmetic
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    let ms = ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis;
    u64::try_from(ms).ok()
}

#[cfg(test)]
mod tests {
    use super::{answers_said, read};
    use reef_api::{DecisionKind, Outcome, Via};

    /// A runner's session from C47's live run, trimmed: a node taken, its question answered,
    /// its first plan rejected with feedback and its second approved, each line written twice
    const RUN: &str = include_str!("decisions/live-p2.jsonl");

    /// A real session reads as its question and two plans, each on the node it took, each once,
    /// with how it was answered
    #[test]
    fn a_real_session_reads_as_its_decisions() {
        let read = read(RUN, Via::App);
        assert_eq!(read.len(), 3, "{read:#?}");
        // each on the node next_task handed out
        for decision in &read {
            assert_eq!(
                decision.node.as_ref().unwrap().to_string(),
                "live/planned/P2"
            );
            assert_eq!(decision.decided.via, Via::App);
            assert!(decision.decided.answered_at >= Some(decision.decided.asked_at));
        }
        // the question, and the choice picked
        let DecisionKind::Question { questions } = &read[0].decided.kind else {
            panic!("the question first: {:?}", read[0]);
        };
        assert_eq!(questions[0].header, "Welcome");
        let Some(Outcome::Answered { answers }) = &read[0].decided.outcome else {
            panic!("answered: {:?}", read[0].decided.outcome);
        };
        assert_eq!(answers["Which welcome should welcome.txt hold?"], "Hi");
        // the first plan rejected with its feedback, the second approved
        assert_eq!(
            read[1].decided.outcome,
            Some(Outcome::Rejected {
                feedback: "Write the welcome in capital letters instead, and say so in the plan."
                    .to_string()
            })
        );
        assert!(
            matches!(&read[2].decided.kind, DecisionKind::Plan { plan, .. } if plan.contains("`HI`"))
        );
        assert_eq!(read[2].decided.outcome, Some(Outcome::Approved));
    }

    /// A plan approved with a clear is read from the session the clear starts, as the planning
    /// session's call, approved
    #[test]
    fn an_approval_that_clears_is_read_from_the_new_session() {
        // C75's terminal run, the clear's new session's first lines
        let lines = [
            r##"{"type":"user","message":{"role":"user","content":"Implement the following plan:\n\n# Plan"},"timestamp":"2026-11-23T18:57:44.200Z","planContent":"# Plan\n\nSLEEP WELL"}"##,
            r#"{"type":"data-flow-records","v":1,"sessionId":"s2","records":[{"tool_name":"Write","tool_use_id":"w1","no_message":true},{"tool_name":"ExitPlanMode","tool_use_id":"p2","no_message":true}]}"#,
        ];
        let read = read(&lines.join("\n"), Via::Terminal);
        assert_eq!(read.len(), 1, "{read:#?}");
        let decided = &read[0].decided;
        assert_eq!(
            (decided.call.as_str(), decided.session.as_str()),
            ("p2", "s2")
        );
        assert_eq!(decided.outcome, Some(Outcome::Approved));
        assert!(
            matches!(&decided.kind, DecisionKind::Plan { plan, .. } if plan.ends_with("SLEEP WELL"))
        );
        // a session that starts with no plan carried in reads as nothing
        assert!(super::read(lines[1], Via::Terminal).is_empty());
    }

    /// A question's answers read from its result's text when Claude Code kept no record of them
    #[test]
    fn answers_read_from_the_result_text() {
        let said = "Your questions have been answered: \"Which colour?\"=\"red\", \"How big?\"=\"Large\". \
                    You can now continue with these answers in mind.";
        let answers = answers_said(said);
        assert_eq!(answers["Which colour?"], "red");
        assert_eq!(answers["How big?"], "Large");
    }
}
