//! The injector: types `/clear` into the session's own terminal pane once Claude is idle
//!
//! It runs detached from the Stop hook, in the user's own zellij or tmux pane. reef never
//! starts or owns the session: it only types into the pane the session already runs in.

use anyhow::{Context, Error, bail};
use serde_json::{Value, json};
use std::process::Command;
use std::time::{Duration, Instant};
use tracing::instrument;

use crate::state::{KICKOFF, Store, now_ms};

/// How often the injector looks for what it is waiting on
const POLL: Duration = Duration::from_millis(250);

/// The terminal multiplexer a session runs in, and its pane
#[derive(Debug, PartialEq, Eq)]
pub enum Backend {
    /// A zellij pane, written to with `zellij action write-chars --pane-id`
    Zellij {
        /// The zellij session the pane is in
        session: String,
        /// The pane's id, as `ZELLIJ_PANE_ID` gives it
        pane: String,
    },
    /// A tmux pane, written to with `tmux send-keys -t`
    Tmux {
        /// The pane's id, as `TMUX_PANE` gives it
        pane: String,
    },
}

impl Backend {
    /// Finds the pane a session runs in from its environment, preferring zellij
    ///
    /// # Arguments
    ///
    /// * `var` - Looks up one environment variable
    pub fn from_env<F: Fn(&str) -> Option<String>>(var: F) -> Option<Self> {
        // zellij needs its session's name as well as the pane, since actions go by session
        if let (Some(session), Some(pane)) = (var("ZELLIJ_SESSION_NAME"), var("ZELLIJ_PANE_ID")) {
            return Some(Backend::Zellij { session, pane });
        }
        // tmux's pane ids are unique across its server
        var("TMUX_PANE").map(|pane| Backend::Tmux { pane })
    }

    /// The backend's name, for the event log
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Zellij { .. } => "zellij",
            Backend::Tmux { .. } => "tmux",
        }
    }

    /// Types a line into the pane and presses Enter
    ///
    /// # Arguments
    ///
    /// * `text` - The line to type
    #[instrument(name = "Backend::type_line", skip(self), err(Debug))]
    pub fn type_line(&self, text: &str) -> Result<(), Error> {
        // type the text first
        match self {
            Backend::Zellij { session, pane } => run(Command::new("zellij").args([
                "--session",
                session,
                "action",
                "write-chars",
                "--pane-id",
                pane,
                text,
            ]))?,
            Backend::Tmux { pane } => {
                run(Command::new("tmux").args(["send-keys", "-t", pane, "-l", text]))?
            }
        }
        // give the TUI a moment to settle, since a slash opens its command menu
        std::thread::sleep(Duration::from_millis(400));
        // then press Enter
        match self {
            Backend::Zellij { session, pane } => run(Command::new("zellij").args([
                "--session",
                session,
                "action",
                "write",
                "--pane-id",
                pane,
                "13",
            ])),
            Backend::Tmux { pane } => {
                run(Command::new("tmux").args(["send-keys", "-t", pane, "Enter"]))
            }
        }
    }

    /// Reads the last lines on the pane's screen, so the log shows what was typed into
    pub fn screen_tail(&self, lines: usize) -> Option<String> {
        // dump the visible screen
        let out = match self {
            Backend::Zellij { session, pane } => Command::new("zellij")
                .args([
                    "--session",
                    session,
                    "action",
                    "dump-screen",
                    "--pane-id",
                    pane,
                ])
                .output(),
            Backend::Tmux { pane } => Command::new("tmux")
                .args(["capture-pane", "-p", "-t", pane])
                .output(),
        }
        .ok()?;
        // keep its last non-blank lines
        let text = String::from_utf8_lossy(&out.stdout);
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        Some(kept[kept.len().saturating_sub(lines)..].join("\n"))
    }
}

/// Runs a command and fails if it does
///
/// # Arguments
///
/// * `cmd` - The command to run
fn run(cmd: &mut Command) -> Result<(), Error> {
    let out = cmd.output().with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!("{cmd:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Reads a session's status from `claude agents --json`, if it is listed
///
/// # Arguments
///
/// * `session` - The session's id
fn session_status(session: &str) -> Option<String> {
    // list the active sessions
    let out = Command::new("claude")
        .args(["agents", "--json"])
        .output()
        .ok()?;
    let sessions: Vec<Value> = serde_json::from_slice(&out.stdout).ok()?;
    // find ours and read its status (interactive sessions) or state (background ones)
    let ours = sessions
        .iter()
        .find(|entry| entry.get("sessionId").and_then(Value::as_str) == Some(session))?;
    ours.get("status")
        .or_else(|| ours.get("state"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Waits for an event the hooks or the MCP server log, after a point in time
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `since` - Only events at or after this time count
/// * `timeout` - How long to wait
/// * `matches` - Whether an event is the one waited on
fn wait_for_event<F: Fn(&Value) -> bool>(
    store: &Store,
    since: u64,
    timeout: Duration,
    matches: F,
) -> Option<Value> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        // look through what was logged since then
        let found = store.events().ok()?.into_iter().find(|event| {
            event.get("at").and_then(Value::as_u64).unwrap_or(0) >= since && matches(event)
        });
        if found.is_some() {
            return found;
        }
        std::thread::sleep(POLL);
    }
    None
}

/// Clears the session's context and starts the next node
///
/// # Arguments
///
/// * `store` - The loop's state dir
/// * `session` - The session whose pane is typed into
/// * `after` - The node whose end this acts on
#[instrument(name = "inject::run", skip(store), err(Debug))]
pub fn run_injector(store: &Store, session: &str, after: &str) -> Result<(), Error> {
    let started = now_ms();
    // find the pane from the environment the hook passed down
    let Some(backend) = Backend::from_env(|var| std::env::var(var).ok()) else {
        store.log("inject", json!({ "step": "no pane", "after": after }))?;
        bail!("the session runs in neither zellij nor tmux");
    };
    store.log(
        "inject",
        json!({ "step": "pane", "backend": backend.name(), "pane": format!("{backend:?}") }),
    )?;
    // wait until Claude's turn has ended and the session takes input again
    let start = Instant::now();
    let mut listed = false;
    let idle = loop {
        match session_status(session) {
            Some(status) => {
                listed = true;
                if status == "idle" {
                    break true;
                }
            }
            // a session that isn't listed can't be watched, so wait a fixed time instead
            None if start.elapsed() > Duration::from_millis(1500) => break false,
            None => {}
        }
        if start.elapsed() > Duration::from_secs(30) {
            break false;
        }
        std::thread::sleep(POLL);
    };
    store.log(
        "inject",
        json!({
            "step": "idle",
            "idle": idle,
            "listed": listed,
            "waited_ms": start.elapsed().as_millis() as u64,
            "screen": backend.screen_tail(4),
        }),
    )?;
    // clear the context
    let cleared_at = now_ms();
    backend.type_line("/clear")?;
    store.log("inject", json!({ "step": "typed /clear" }))?;
    // wait for the clear to reach the SessionStart hook
    let clear = wait_for_event(store, cleared_at, Duration::from_secs(10), |event| {
        event.get("kind").and_then(Value::as_str) == Some("SessionStart")
            && event.pointer("/body/input/source").and_then(Value::as_str) == Some("clear")
    });
    // a clear may give the session a new id, so watch that one from here on
    let new_session = clear
        .as_ref()
        .and_then(|event| event.pointer("/body/input/session_id"))
        .and_then(Value::as_str)
        .unwrap_or(session)
        .to_owned();
    store.log(
        "inject",
        json!({ "step": "cleared", "seen": clear.is_some(), "new_session": new_session }),
    )?;
    // the clear's initialUserMessage should start the next node; see whether a turn starts
    let turn = wait_for_event(store, cleared_at, Duration::from_secs(8), |event| {
        matches!(
            event.get("kind").and_then(Value::as_str),
            Some("UserPromptSubmit") | Some("PreToolUse") | Some("tool")
        )
    });
    // a session that is busy has started a turn even if no hook has seen it yet
    let busy = session_status(&new_session).as_deref() == Some("busy");
    if turn.is_some() || busy {
        store.log(
            "inject",
            json!({
                "step": "started by initialUserMessage",
                "seen": turn.as_ref().and_then(|event| event.get("kind")),
                "busy": busy,
                "after_ms": now_ms() - started,
            }),
        )?;
        return Ok(());
    }
    // no turn started by itself, so type the kickoff too
    backend.type_line(KICKOFF)?;
    store.log(
        "inject",
        json!({ "step": "typed kickoff", "after_ms": now_ms() - started }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Builds a lookup over a fixed set of variables
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, val)| ((*key).to_owned(), (*val).to_owned()))
            .collect();
        move |var| map.get(var).cloned()
    }

    /// zellij is found from its session and pane, and wins over tmux
    #[test]
    fn zellij_from_env() {
        let env = vars(&[
            ("ZELLIJ_SESSION_NAME", "work"),
            ("ZELLIJ_PANE_ID", "3"),
            ("TMUX_PANE", "%1"),
        ]);
        assert_eq!(
            Backend::from_env(env),
            Some(Backend::Zellij {
                session: "work".into(),
                pane: "3".into()
            })
        );
    }

    /// tmux is found from its pane
    #[test]
    fn tmux_from_env() {
        assert_eq!(
            Backend::from_env(vars(&[("TMUX_PANE", "%7")])),
            Some(Backend::Tmux { pane: "%7".into() })
        );
    }

    /// A zellij pane without its session's name isn't enough to write to it
    #[test]
    fn no_pane_without_both_zellij_vars() {
        assert_eq!(Backend::from_env(vars(&[("ZELLIJ_PANE_ID", "3")])), None);
        assert_eq!(Backend::from_env(vars(&[])), None);
    }
}
