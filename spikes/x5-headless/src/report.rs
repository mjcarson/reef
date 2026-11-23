//! A run's timeline: every line the session and the host wrote, and X4's hook events, by time

use anyhow::{Context, Error};
use serde_json::Value;
use std::path::Path;

/// Reads a JSON-lines file, skipping lines that don't parse, with what names the machine taken
/// out first
///
/// # Arguments
///
/// * `path` - The file
/// * `scrub` - Each text to replace, and what replaces it, in order
fn lines(path: &Path, scrub: &[(String, String)]) -> Result<Vec<Value>, Error> {
    // the raw text scrubbed, so nothing cut later keeps half a path
    let mut text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    for (from, to) in scrub {
        text = text.replace(from.as_str(), to);
    }
    // a record a line
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// What names the machine in a run's logs, and what replaces each: the repo, the scratch
/// directory around it, each as Claude Code dashes it in a transcript's path, then the home
/// directory and the user's name
///
/// # Arguments
///
/// * `log` - The run's log, whose `init` names the repo
fn scrubs(log: &Path) -> Result<Vec<(String, String)>, Error> {
    // the repo from the session's own `init`
    let repo = lines(log, &[])?
        .iter()
        .find_map(|line| line["msg"]["cwd"].as_str().map(str::to_string));
    let mut scrub = Vec::new();
    let dashed = |path: &str| path.replace(['/', '.'], "-");
    if let Some(repo) = repo {
        let scratch = Path::new(&repo)
            .parent()
            .map(|parent| parent.display().to_string());
        scrub.push((repo.clone(), "<repo>".to_string()));
        scrub.push((dashed(&repo), "<repo>".to_string()));
        if let Some(scratch) = scratch {
            scrub.push((scratch.clone(), "<scratch>".to_string()));
            scrub.push((dashed(&scratch), "<scratch>".to_string()));
        }
    }
    // then the home directory, and the user's name wherever else it is
    if let Ok(home) = std::env::var("HOME") {
        scrub.push((home, "~".to_string()));
    }
    if let Ok(user) = std::env::var("USER") {
        scrub.push((user, "user".to_string()));
    }
    Ok(scrub)
}

/// Cuts a text to its first line and at most a number of characters
///
/// # Arguments
///
/// * `text` - The text
/// * `max` - The most characters kept
fn short(text: &str, max: usize) -> String {
    // the first line, cut, its pipes escaped for the table
    let first = text.lines().next().unwrap_or_default();
    let mut cut: String = first.chars().take(max).collect();
    if first.chars().count() > max || text.lines().count() > 1 {
        cut.push('…');
    }
    cut.replace('|', "\\|")
}

/// Says what one line of the run was, in a few words
///
/// # Arguments
///
/// * `from` - Who wrote it
/// * `msg` - The line
fn what(from: &str, msg: &Value) -> Vec<String> {
    // a line can say several things, an assistant message holding several blocks
    match (from, msg["type"].as_str().unwrap_or_default()) {
        ("claude", "system") if msg["subtype"] == "init" => vec![format!(
            "init: mode `{}`, model `{}`, project MCP {}, {} tools",
            msg["permissionMode"].as_str().unwrap_or_default(),
            msg["model"].as_str().unwrap_or_default(),
            project_servers(msg),
            msg["tools"].as_array().map_or(0, Vec::len)
        )],
        ("claude", "system") => vec![format!(
            "system `{}` {}",
            msg["subtype"].as_str().unwrap_or_default(),
            short(&msg.to_string(), 120)
        )],
        ("claude", "assistant") => msg["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| match block["type"].as_str() {
                Some("text") => Some(format!(
                    "says: {}",
                    short(block["text"].as_str().unwrap_or_default(), 100)
                )),
                Some("tool_use") => Some(format!(
                    "calls `{}` {}",
                    block["name"].as_str().unwrap_or_default(),
                    short(&block["input"].to_string(), 80)
                )),
                _ => None,
            })
            .collect(),
        ("claude", "user") => msg["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "tool_result")
            .map(|block| {
                let content = match &block["content"] {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                format!(
                    "result{}: {}",
                    if block["is_error"] == true {
                        " (error)"
                    } else {
                        ""
                    },
                    short(&content, 100)
                )
            })
            .collect(),
        ("claude", "control_request") => vec![format!(
            "**asks** `{}` `{}` {}",
            msg["request"]["subtype"].as_str().unwrap_or_default(),
            msg["request"]["tool_name"].as_str().unwrap_or_default(),
            short(&msg["request"]["input"].to_string(), 80)
        )],
        ("claude", "result") => vec![format!(
            "**result** `{}`, {} turns, ${}, {} denials",
            msg["subtype"].as_str().unwrap_or_default(),
            msg["num_turns"],
            msg["total_cost_usd"],
            msg["permission_denials"].as_array().map_or(0, Vec::len)
        )],
        ("host", "control_response") => vec![format!(
            "**answers** {}",
            short(&msg["response"]["response"].to_string(), 120)
        )],
        ("host", "user") => vec![format!(
            "sends: {}",
            short(
                msg["message"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default(),
                100
            )
        )],
        (_, kind) => vec![format!("{kind} {}", short(&msg.to_string(), 120))],
    }
}

/// The MCP servers the repo's own `.mcp.json` started, each with its status, leaving out the
/// account's own connectors
///
/// # Arguments
///
/// * `init` - The session's `init` line
fn project_servers(init: &Value) -> String {
    // only the repo's servers say anything about the repo's setup
    let servers: Vec<String> = init["mcp_servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|server| server["source"] == "project")
        .map(|server| {
            format!(
                "`{}` {}",
                server["name"].as_str().unwrap_or_default(),
                server["status"].as_str().unwrap_or_default()
            )
        })
        .collect();
    servers.join(", ")
}

/// Lays a run out as a markdown table, X4's hook events merged in by time
///
/// # Arguments
///
/// * `log` - The run's log
/// * `events` - X4's event log, if its hooks ran
pub fn timeline(log: &Path, events: Option<&Path>) -> Result<String, Error> {
    // every row as its time, who and what, with nothing naming the machine
    let scrub = scrubs(log)?;
    let mut rows: Vec<(u64, String, String)> = Vec::new();
    for line in lines(log, &scrub)? {
        let at = line["at"].as_u64().unwrap_or(0);
        let from = line["from"].as_str().unwrap_or_default().to_string();
        for said in what(&from, &line["msg"]) {
            rows.push((at, from.clone(), said));
        }
    }
    // the hooks' events, from the run's start on
    let start = rows.first().map_or(0, |row| row.0);
    if let Some(events) = events {
        for event in lines(events, &scrub)? {
            let at = event["at"].as_u64().unwrap_or(0);
            if at < start {
                continue;
            }
            let body = short(&event["body"].to_string(), 100);
            rows.push((
                at,
                "hook".to_string(),
                format!("{} {body}", event["kind"].as_str().unwrap_or_default()),
            ));
        }
    }
    rows.sort_by_key(|row| row.0);
    // the table, times from the run's start
    let mut out = String::from("| s | from | what |\n|---:|---|---|\n");
    for (at, from, said) in rows {
        let secs = at.saturating_sub(start) as f64 / 1000.0;
        out.push_str(&format!("| {secs:.1} | {from} | {said} |\n"));
    }
    Ok(out)
}
