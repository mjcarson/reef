//! Claude Code's hooks: every event is logged, and Stop and SessionStart drive the loop

use anyhow::Error;
use serde_json::{Value, json};
use std::io::Read;
use std::process::{Command, Stdio};
use tracing::instrument;

use crate::state::{KICKOFF, Mode, State, Status, Store};

/// The environment variables that say which terminal pane a session runs in
pub const PANE_VARS: [&str; 3] = ["ZELLIJ_PANE_ID", "ZELLIJ_SESSION_NAME", "TMUX_PANE"];

/// What the Stop hook does once Claude's turn ends
#[derive(Debug, PartialEq)]
pub enum StopAction {
    /// Let Claude stop, optionally telling the user something
    Allow {
        /// Why Claude may stop, for the event log
        why: &'static str,
        /// A message shown to the user, if any
        message: Option<String>,
    },
    /// Keep Claude going with this reason as its next instruction
    Block {
        /// What Claude is told to do next
        reason: String,
    },
    /// Let Claude stop, then type `/clear` into its pane and start the next node
    Inject {
        /// The node whose end this acts on
        after: String,
    },
}

/// Decides what the Stop hook does, and records that it acted on this node's end
///
/// It never reads `stop_hook_active`. A loop that let Claude stop whenever the flag is true
/// would stop after its second node if the flag stays true through a plan approval, so
/// "never twice" is kept by `handled_for` instead: one action per finished node.
///
/// # Arguments
///
/// * `state` - The loop's state, changed when the hook acts on a node's end
pub fn decide(state: &mut State) -> StopAction {
    // nothing to do until the loop holds a node
    let Some(node) = state.current_node() else {
        return StopAction::Allow {
            why: "the loop holds no node",
            message: None,
        };
    };
    // Claude stopped mid-node, so it is waiting on the user
    if node.status != Status::Done {
        return StopAction::Allow {
            why: "the node isn't done",
            message: None,
        };
    }
    let id = node.id.clone();
    // this node's end was already acted on, so never act twice
    if state.handled_for.as_deref() == Some(id.as_str()) {
        return StopAction::Allow {
            why: "already acted on this node's end",
            message: None,
        };
    }
    // from here on the hook acts on this node's end, whatever it does
    state.handled_for = Some(id.clone());
    // a paused loop stops after the node it was on
    if state.paused {
        return StopAction::Allow {
            why: "the loop is paused",
            message: Some(format!("reef: {id} is done and the loop is paused.")),
        };
    }
    // with nothing left to claim, the plan is finished
    let Some(next) = state.pending().map(|node| node.id.clone()) else {
        return StopAction::Allow {
            why: "the plan is finished",
            message: Some(format!("reef: {id} is done, and so is the plan.")),
        };
    };
    // otherwise move on as the mode says
    match state.mode {
        Mode::Approval => StopAction::Block {
            reason: format!(
                "{id} is done. Call the x4 next_task tool for the next node ({next}), then call \
                 EnterPlanMode and plan it."
            ),
        },
        Mode::Inject => StopAction::Inject { after: id },
        Mode::Manual => StopAction::Allow {
            why: "the user clears by hand",
            message: Some(format!("reef: {id} is done. Type /clear to take {next}.")),
        },
    }
}

/// What the SessionStart hook adds to a session that just started or was cleared
///
/// # Arguments
///
/// * `state` - The loop's state
/// * `source` - Why the session started: `startup`, `resume`, `clear`, `compact` or `fork`
pub fn session_start(state: &State, source: &str) -> Option<Value> {
    // only a clear moves the loop on
    if source != "clear" {
        return None;
    }
    // a clear mid-node is a plan approval's: say which node the plan is for
    if let Some(node) = state.current_node()
        && node.status == Status::InProgress
    {
        let context = format!(
            "You are in reef's loop, on node {} ({}). If its plan was just approved, implement \
             it, run `cargo test`, then call the x4 set_status tool with id {} and status done.",
            node.id, node.title, node.id
        );
        return Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        }));
    }
    // a paused or finished loop starts nothing
    if state.paused {
        return None;
    }
    let next = state.pending()?;
    // otherwise start the next node without the user typing anything
    let context = format!("You are in reef's loop. The next node is {}.", next.id);
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context,
            "initialUserMessage": KICKOFF,
        }
    }))
}

/// Reads the context size of the last assistant message in a transcript
///
/// The transcript's format is internal to Claude Code, so this is only a cross-check on the
/// status line's documented `context_window`.
///
/// # Arguments
///
/// * `path` - The transcript's path, from the hook's input
pub fn transcript_context(path: &str) -> Option<Value> {
    // read the whole transcript, since a spike's sessions are small
    let raw = std::fs::read_to_string(path).ok()?;
    // walk back to the last line with usage on it
    raw.lines().rev().find_map(|line| {
        let value: Value = serde_json::from_str(line).ok()?;
        let usage = value.pointer("/message/usage")?;
        // the context is everything sent: fresh input, cache writes and cache reads
        let tokens = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|key| usage.get(*key).and_then(Value::as_u64))
        .sum::<u64>();
        Some(json!({ "tokens": tokens, "usage": usage }))
    })
}

/// Runs one hook: logs its input, and for Stop and SessionStart prints what Claude Code does
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `event` - The hook's event name
#[instrument(name = "hook::run", skip(store), err(Debug))]
pub fn run(store: &Store, event: &str) -> Result<(), Error> {
    // read the event's input from stdin
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    // log the input with the pane the session runs in
    let env: serde_json::Map<String, Value> = PANE_VARS
        .iter()
        .filter_map(|var| {
            std::env::var(var)
                .ok()
                .map(|val| ((*var).to_owned(), json!(val)))
        })
        .collect();
    store.log(event, json!({ "input": input, "env": env }))?;
    // only Stop and SessionStart do anything beyond logging
    match event {
        "Stop" => stop(store, &input),
        "SessionStart" => start(store, &input),
        _ => Ok(()),
    }
}

/// Decides what a Stop does, logs it and tells Claude Code
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `input` - The Stop hook's input
#[instrument(name = "hook::stop", skip_all, err(Debug))]
fn stop(store: &Store, input: &Value) -> Result<(), Error> {
    // decide under the lock so two stops never both act on one node
    let action = store.update(decide)?;
    // log the decision with the context at this point, from both sources
    let transcript = input
        .get("transcript_path")
        .and_then(Value::as_str)
        .and_then(transcript_context);
    let status_line = std::fs::read_to_string(store.path("context.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    store.log(
        "decision",
        json!({
            "action": format!("{action:?}"),
            "stop_hook_active": input.get("stop_hook_active"),
            "transcript_context": transcript,
            "status_line_context": status_line,
        }),
    )?;
    // tell Claude Code what to do
    match action {
        StopAction::Allow { message, .. } => {
            if let Some(message) = message {
                println!("{}", json!({ "systemMessage": message }));
            }
        }
        StopAction::Block { reason } => {
            println!("{}", json!({ "decision": "block", "reason": reason }));
        }
        StopAction::Inject { after } => {
            let session = input
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            spawn_injector(store, session, &after)?;
        }
    }
    Ok(())
}

/// Decides what a SessionStart adds, logs it and tells Claude Code
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `input` - The SessionStart hook's input
#[instrument(name = "hook::start", skip_all, err(Debug))]
fn start(store: &Store, input: &Value) -> Result<(), Error> {
    // work out what to add from the state as it is now
    let source = input.get("source").and_then(Value::as_str).unwrap_or("");
    let state = store.load()?;
    let output = session_start(&state, source);
    store.log(
        "decision",
        json!({ "session_start": source, "output": output }),
    )?;
    // print it for Claude Code, if there is anything to add
    if let Some(output) = output {
        println!("{output}");
    }
    Ok(())
}

/// Starts the injector in the background, so the Stop hook returns at once
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `session` - The session whose pane the injector types into
/// * `after` - The node whose end this acts on
#[instrument(name = "hook::spawn_injector", skip(store), err(Debug))]
fn spawn_injector(store: &Store, session: &str, after: &str) -> Result<(), Error> {
    use std::os::unix::process::CommandExt;
    // run this same binary's inject command, with no pipes back to Claude Code
    let child = Command::new(std::env::current_exe()?)
        .arg("--dir")
        .arg(store.dir())
        .args(["inject", "--session", session, "--after", after])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // a group of its own, so nothing Claude Code sends the hook's group reaches it
        .process_group(0)
        .spawn()?;
    store.log(
        "inject",
        json!({ "step": "spawned", "pid": child.id(), "after": after }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Claim;

    /// Claims the next node and marks it done, as Claude would by the end of a node
    fn finish_next(state: &mut State) -> String {
        let Claim::Claimed(node) = state.claim_next() else {
            panic!("expected a node to claim");
        };
        state.node_mut(&node.id).unwrap().status = Status::Done;
        node.id
    }

    /// The hook never acts twice on one node's end
    #[test]
    fn never_acts_twice_for_one_node() {
        for mode in [Mode::Approval, Mode::Inject, Mode::Manual] {
            let mut state = State::stub(mode);
            finish_next(&mut state);
            // the first stop after N1 is done acts
            assert!(!matches!(
                decide(&mut state),
                StopAction::Allow { message: None, .. }
            ));
            // a second stop with nothing new done lets Claude stop quietly
            assert_eq!(
                decide(&mut state),
                StopAction::Allow {
                    why: "already acted on this node's end",
                    message: None
                }
            );
        }
    }

    /// Nothing happens while a node is in progress
    #[test]
    fn waits_mid_node() {
        let mut state = State::stub(Mode::Approval);
        // before the loop holds anything
        assert!(matches!(decide(&mut state), StopAction::Allow { .. }));
        // while N1 is planned or built
        state.claim_next();
        assert_eq!(
            decide(&mut state),
            StopAction::Allow {
                why: "the node isn't done",
                message: None
            }
        );
    }

    /// A paused loop stops after its node and says so
    #[test]
    fn pause_stops_after_the_node() {
        let mut state = State::stub(Mode::Inject);
        finish_next(&mut state);
        state.paused = true;
        assert!(matches!(
            decide(&mut state),
            StopAction::Allow {
                why: "the loop is paused",
                message: Some(_)
            }
        ));
    }

    /// A finished plan starts nothing
    #[test]
    fn finished_plan_starts_nothing() {
        let mut state = State::stub(Mode::Approval);
        // finish every node, acting on each end as the loop goes
        for _ in 0..3 {
            finish_next(&mut state);
            decide(&mut state);
        }
        // the last decision was the plan's end, and a later stop does nothing
        assert!(matches!(decide(&mut state), StopAction::Allow { .. }));
        assert!(state.pending().is_none());
    }

    /// The chained loop blocks once for every finished node, in a chain of continuations
    ///
    /// Every stop here would carry `stop_hook_active: true` if it stays true across tool
    /// calls; `decide` keeps the loop going regardless.
    #[test]
    fn approval_blocks_once_per_node() {
        let mut state = State::stub(Mode::Approval);
        for _ in 0..2 {
            let id = finish_next(&mut state);
            // the node's end blocks, naming it
            assert!(
                matches!(decide(&mut state), StopAction::Block { reason } if reason.starts_with(&id))
            );
            // Claude ignoring the block and stopping again is let go
            assert!(matches!(decide(&mut state), StopAction::Allow { .. }));
        }
    }

    /// Only the chained loop ever blocks
    #[test]
    fn inject_and_manual_never_block() {
        for mode in [Mode::Inject, Mode::Manual] {
            let mut state = State::stub(mode);
            for _ in 0..3 {
                finish_next(&mut state);
                // every stop along the way lets Claude stop
                assert!(!matches!(decide(&mut state), StopAction::Block { .. }));
                assert!(!matches!(decide(&mut state), StopAction::Block { .. }));
            }
        }
    }

    /// A clear between nodes starts the next one, and a clear mid-node names the node
    #[test]
    fn session_start_after_clear() {
        let mut state = State::stub(Mode::Manual);
        // a startup adds nothing
        assert!(session_start(&state, "startup").is_none());
        // a clear before anything is claimed starts the first node
        let out = session_start(&state, "clear").unwrap();
        assert_eq!(out["hookSpecificOutput"]["initialUserMessage"], KICKOFF);
        // a clear mid-node, from a plan approval, names the node and starts nothing
        state.claim_next();
        let out = session_start(&state, "clear").unwrap();
        assert!(
            out["hookSpecificOutput"]
                .get("initialUserMessage")
                .is_none()
        );
        // a clear while paused between nodes starts nothing
        state.node_mut("N1").unwrap().status = Status::Done;
        state.paused = true;
        assert!(session_start(&state, "clear").is_none());
    }
}
