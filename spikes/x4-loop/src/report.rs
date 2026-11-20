//! The status line's context log and the run's report

use anyhow::Error;
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use tracing::instrument;

use crate::state::{Store, now_ms, read_jsonl};

/// The context a status line update reports: fresh input, cache writes and cache reads
///
/// # Arguments
///
/// * `input` - The status line's input
fn context_tokens(input: &Value) -> Option<u64> {
    // the last call's usage is null before the first call and after a compaction
    let usage = input.pointer("/context_window/current_usage")?;
    if usage.is_null() {
        return None;
    }
    Some(
        [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|key| usage.get(*key).and_then(Value::as_u64))
        .sum(),
    )
}

/// Runs the status line: records the session's context and prints a short line
///
/// # Arguments
///
/// * `store` - The loop's state dir
#[instrument(name = "report::status_line", skip_all, err(Debug))]
pub fn status_line(store: &Store) -> Result<(), Error> {
    // read the status line's input
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw)?;
    let session = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tokens = context_tokens(&input);
    // keep the latest whole input for the Stop hook to read
    std::fs::write(store.path("context.json"), &raw)?;
    // append a sample whenever the context changes
    let series = read_jsonl(&store.path("context.jsonl"))?;
    let last = series.last();
    let changed = last
        .and_then(|sample| sample.get("tokens"))
        .and_then(Value::as_u64)
        != tokens
        || last
            .and_then(|sample| sample.get("session"))
            .and_then(Value::as_str)
            != Some(session);
    if changed {
        let sample = json!({
            "at": now_ms(),
            "session": session,
            "tokens": tokens,
            "window": input.pointer("/context_window/context_window_size"),
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(store.path("context.jsonl"))?;
        writeln!(file, "{sample}")?;
    }
    // print what the loop holds and the context
    let state = store.load().ok();
    let held = state
        .as_ref()
        .and_then(|state| state.current_node())
        .map(|node| format!("{} {}", node.id, node.status.as_str()))
        .unwrap_or_else(|| "no node".to_owned());
    let mode = state
        .map(|state| format!("{:?}", state.mode).to_lowercase())
        .unwrap_or_default();
    let ctx = tokens
        .map(|tokens| format!("{:.1}k", tokens as f64 / 1000.0))
        .unwrap_or_else(|| "-".to_owned());
    println!("x4 {mode} · {held} · ctx {ctx}");
    Ok(())
}

/// Cuts a string to its first line and a length, for the timeline
///
/// # Arguments
///
/// * `text` - The text to cut
/// * `len` - The most characters to keep
fn short(text: &str, len: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    let cut: String = line.chars().take(len).collect();
    if cut.len() < line.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

/// Reads a string at a JSON pointer, or an empty string
///
/// # Arguments
///
/// * `value` - The value to read from
/// * `pointer` - Where in it to read
fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

/// Shortens a session id to its first block
///
/// # Arguments
///
/// * `id` - The session id
fn sid(id: &str) -> &str {
    id.split('-').next().unwrap_or(id)
}

/// Says in one line what an event was
///
/// # Arguments
///
/// * `event` - The logged event
fn describe(event: &Value) -> String {
    let body = &event["body"];
    let input = &body["input"];
    match event["kind"].as_str().unwrap_or("") {
        "SessionStart" => format!(
            "source={} session={} context_tokens={}",
            text(input, "/source"),
            sid(text(input, "/session_id")),
            input.get("context_tokens").unwrap_or(&Value::Null)
        ),
        "SessionEnd" => format!(
            "reason={} session={}",
            text(input, "/reason"),
            sid(text(input, "/session_id"))
        ),
        "UserPromptSubmit" => format!(
            "mode={} prompt={:?}",
            text(input, "/permission_mode"),
            short(text(input, "/prompt"), 60)
        ),
        "PreToolUse" | "PostToolUse" | "PermissionRequest" | "PostToolUseFailure" => format!(
            "{} mode={}",
            text(input, "/tool_name"),
            text(input, "/permission_mode")
        ),
        "Stop" => format!(
            "stop_hook_active={} mode={} session={}",
            input.get("stop_hook_active").unwrap_or(&Value::Null),
            text(input, "/permission_mode"),
            sid(text(input, "/session_id"))
        ),
        "PreCompact" => format!("trigger={}", text(input, "/trigger")),
        "decision" => match body.get("action") {
            Some(action) => short(action.as_str().unwrap_or(""), 90),
            None => format!(
                "SessionStart({}) -> {}",
                text(body, "/session_start"),
                if body["output"].is_null() {
                    "nothing".to_owned()
                } else {
                    short(&body["output"].to_string(), 70)
                }
            ),
        },
        "tool" => format!(
            "{} {} -> {}",
            text(body, "/tool"),
            body["args"],
            short(text(body, "/reply"), 50)
        ),
        "inject" | "mcp" => {
            // everything but the step, compactly
            let mut rest = body.clone();
            let step = rest.as_object_mut().and_then(|obj| obj.remove("step"));
            format!(
                "{} {}",
                step.and_then(|s| s.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                short(&rest.to_string(), 90)
            )
        }
        _ => short(&body.to_string(), 90),
    }
}

/// The first context sample at or after a time, and the last one before it
///
/// # Arguments
///
/// * `series` - The status line's samples
/// * `at` - The time to look around
fn around(series: &[Value], at: u64) -> (Option<u64>, Option<u64>) {
    let time = |sample: &Value| sample["at"].as_u64().unwrap_or(0);
    let before = series
        .iter()
        .filter(|sample| time(sample) <= at)
        .filter_map(|sample| sample["tokens"].as_u64())
        .next_back();
    let after = series
        .iter()
        .filter(|sample| time(sample) > at)
        .find_map(|sample| sample["tokens"].as_u64());
    (before, after)
}

/// Prints the run's timeline and its context at each node's start and end
///
/// # Arguments
///
/// * `store` - The loop's state dir
#[instrument(name = "report::report", skip_all, err(Debug))]
pub fn report(store: &Store) -> Result<(), Error> {
    let events = store.events()?;
    let series = read_jsonl(&store.path("context.jsonl"))?;
    let Some(t0) = events.first().and_then(|event| event["at"].as_u64()) else {
        println!("No events.");
        return Ok(());
    };
    // the timeline, one line per event
    println!("| t (s) | event | detail |\n|---:|---|---|");
    for event in &events {
        let at = event["at"].as_u64().unwrap_or(t0);
        let detail = describe(event).replace('|', "\\|");
        println!(
            "| {:.1} | {} | {} |",
            (at - t0) as f64 / 1000.0,
            event["kind"].as_str().unwrap_or(""),
            detail
        );
    }
    // when each node was claimed and finished, from the tool calls
    let is_tool =
        |event: &Value, name: &str| event["kind"] == "tool" && event["body"]["tool"] == name;
    let mut nodes: Vec<(String, u64, Option<u64>)> = Vec::new();
    for event in &events {
        let at = event["at"].as_u64().unwrap_or(0);
        if is_tool(event, "next_task") {
            // only a fresh claim starts a node
            if let Some(rest) = text(&event["body"], "/reply").strip_prefix("Your next node: ") {
                let id = rest.split_whitespace().next().unwrap_or("").to_owned();
                nodes.push((id, at, None));
            }
        }
        if is_tool(event, "set_status") && event["body"]["args"]["status"] == "done" {
            let id = text(&event["body"], "/args/id");
            if let Some(node) = nodes.iter_mut().find(|node| node.0 == id) {
                node.2.get_or_insert(at);
            }
        }
    }
    // the clears, by time
    let clears: Vec<u64> = events
        .iter()
        .filter(|event| {
            event["kind"] == "SessionStart" && event["body"]["input"]["source"] == "clear"
        })
        .filter_map(|event| event["at"].as_u64())
        .collect();
    let claims: Vec<u64> = nodes.iter().map(|node| node.1).collect();
    println!(
        "\n| node | claimed (s) | done (s) | context at claim | context at end | after the next clear |\n|---|---:|---:|---:|---:|---:|"
    );
    for (id, claimed, done) in &nodes {
        let fmt = |tokens: Option<u64>| {
            tokens
                .map(|t| t.to_string())
                .unwrap_or_else(|| "-".to_owned())
        };
        let at_claim = around(&series, *claimed).1;
        let (at_end, cleared) = match done {
            Some(done) => {
                // a node ends at the next clear or the next claim, whichever comes first
                let next_clear = clears.iter().find(|clear| **clear > *done).copied();
                let next_claim = claims.iter().find(|claim| **claim > *done).copied();
                let boundary = next_clear
                    .into_iter()
                    .chain(next_claim)
                    .min()
                    .unwrap_or(u64::MAX);
                let end = around(&series, boundary).0;
                (end, next_clear.and_then(|clear| around(&series, clear).1))
            }
            None => (None, None),
        };
        println!(
            "| {id} | {:.1} | {} | {} | {} | {} |",
            (claimed - t0) as f64 / 1000.0,
            done.map(|done| format!("{:.1}", (done - t0) as f64 / 1000.0))
                .unwrap_or_else(|| "-".to_owned()),
            fmt(at_claim),
            fmt(at_end),
            fmt(cleared),
        );
    }
    Ok(())
}
