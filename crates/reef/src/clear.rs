//! The injector: `/clear` and the next node's kickoff typed into the session's own pane, once a
//! loop's node is done
//!
//! ```text
//! reef hook clear --pane KEY --session ID --pid PID --after NODE   # Stop's, in the background
//! ```
//!
//! - **Started by Stop, in the background.** No hook output, tool or command clears an
//!   interactive session. So once the node the pane's loop holds is done and another is next,
//!   Stop starts this as a process of its own ([`spawn`]) and lets Claude stop.
//! - **Or by a start between nodes.** Stop acts on a node's end once, so after a paused loop's
//!   node, or once the injector stepped aside, `! reef loop start` hands off as Stop did
//!   ([`hand_off`]). Claude Code answers a `!` command with a turn of its own, busy from the
//!   command's Enter to the reply's end, so the injector, started inside it, waits it out as it
//!   waits out the turn that ended a node.
//! - **Only into the session's own pane**, zellij's or tmux's, as X4 typed into it: zellij's
//!   `write-chars` and `write 13`, tmux's `send-keys`. A session with no multiplexer has no pane
//!   reef can reach, so its user types `/clear` and `next`.
//! - **Only once Claude is idle**, as `claude agents --json` says of the Claude Code process
//!   `CLAUDE_PID` names, and only while that process runs the session it waits on.
//! - **Never over a draft.** It reads the pane's input box with its colours, since Claude Code
//!   draws its own suggestions there faint, and waits while the box holds anything else. It
//!   presses Enter only on a box that reads as what it typed, so a line that is partly the
//!   user's is never sent.
//! - **It steps aside** without typing once the loop is paused, stopped or on another node, the
//!   session is cleared or gone, a turn starts that it didn't type, or it has waited ten minutes.
//!   The user's own `/clear` and `next` then start the next node.
//! - **Then the kickoff,** once the clear's new session is idle, since `initialUserMessage`
//!   starts no turn after an interactive clear (X4). SessionStart(clear) names the next node to
//!   Claude ([`crate::loops::session_start`]).
//! - **What it did goes to a log,** `$XDG_STATE_HOME/reef/clear.log`, since nothing shows a
//!   background process's output.

use reef_api::{Loop, LoopState, Pane};
use reef_model::NodeName;
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::time::Instant;

use crate::client::Client;
use crate::task::Ending;

/// What the injector types once the clear's new session is idle
pub const KICKOFF: &str = "Take the next node from reef's loop with next_task";

/// How often it looks again while it waits, for its first half minute
const POLL: Duration = Duration::from_millis(500);

/// How often it looks again once it has waited half a minute, on a draft left in the box
const SLOW_POLL: Duration = Duration::from_secs(2);

/// How long it looks often before it looks slowly
const QUICK: Duration = Duration::from_secs(30);

/// How long it waits on the session before it steps aside
const PATIENCE: Duration = Duration::from_secs(600);

/// How long a typed line is given to settle before the box is read back, since a slash opens
/// Claude Code's command menu (X4)
const SETTLE: Duration = Duration::from_millis(400);

/// How long it waits for a clear's new session before it steps aside
const CLEARING: Duration = Duration::from_secs(10);

/// What the injector is handed, by Stop or by a start between nodes: where to type, and after
/// which node
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clear {
    /// The pane the session runs in, zellij's or tmux's
    pub pane: Pane,
    /// The session running in the pane, which is cleared
    pub session: String,
    /// The Claude Code process the session runs in, `CLAUDE_PID`, which a clear keeps
    pub pid: u32,
    /// The node whose end the loop acted on
    pub after: NodeName,
}

impl Clear {
    /// The command line that runs it, after the program and any `--conf`
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::clear::Clear;
    ///
    /// let order = Clear {
    ///     pane: "zellij:work:3".parse().unwrap(),
    ///     session: "s1".to_string(),
    ///     pid: 4242,
    ///     after: "reef/central/C61".parse().unwrap(),
    /// };
    /// assert_eq!(
    ///     order.args().join(" "),
    ///     "hook clear --pane zellij:work:3 --session s1 --pid 4242 --after reef/central/C61"
    /// );
    /// ```
    pub fn args(&self) -> Vec<String> {
        // each part by its flag, the pane and node by their keys
        vec![
            "hook".to_string(),
            "clear".to_string(),
            "--pane".to_string(),
            self.pane.to_string(),
            "--session".to_string(),
            self.session.clone(),
            "--pid".to_string(),
            self.pid.to_string(),
            "--after".to_string(),
            self.after.to_string(),
        ]
    }
}

/// What the injector is handed, when a node's end has another next and the session can be typed
/// into: its pane is a multiplexer's, and its session and Claude Code process are known
///
/// # Arguments
///
/// * `ending` - What came after the loop's node
/// * `pane` - The pane the session runs in, if it could be found
/// * `session` - The session running there, if Claude Code named it
/// * `pid` - The Claude Code process the session runs in, `CLAUDE_PID`, if it was named
pub fn order(
    ending: &Ending,
    pane: Option<&Pane>,
    session: Option<String>,
    pid: Option<u32>,
) -> Option<Clear> {
    // only the end of a node with another next is cleared after
    let Ending::Next { done, .. } = ending else {
        return None;
    };
    // and only in a pane reef can type into, watching a session it can name
    let pane = pane.filter(|pane| pane.typed())?;
    let session = session.filter(|session| !session.trim().is_empty())?;
    Some(Clear {
        pane: pane.clone(),
        session,
        pid: pid?,
        after: done.clone(),
    })
}

/// Starts the injector once a node's end has another next and the session can be typed into,
/// then says what the user is told of the end, if anything
///
/// Stop hands off at a node's end, and `reef loop start` when it sets a loop running between
/// nodes. Where the injector can't start, the user is told to clear the session, and why.
///
/// # Arguments
///
/// * `ending` - What came after the loop's node
/// * `pane` - The pane the session runs in, if it could be found
/// * `session` - The session running there, if Claude Code named it
/// * `pid` - The Claude Code process the session runs in, `CLAUDE_PID`, if it was named
/// * `spawn` - Starts the injector in the background, as [`spawn`] does
///
/// # Examples
///
/// ```
/// use reef::clear::hand_off;
/// use reef::task::Ending;
/// use reef_api::Pane;
///
/// let next = Ending::Next {
///     done: "reef/central/C61".parse().unwrap(),
///     next: "reef/central/C71".parse().unwrap(),
///     round: false,
/// };
/// // a Claude Code process with no multiplexer is the user's to clear
/// let bare = Pane::Claude { pid: 4242 };
/// let said = hand_off(&next, Some(&bare), Some("s1".to_string()), Some(4242), |_| unreachable!());
/// assert!(said.unwrap().ends_with("`/clear`, then say `next`"));
/// ```
pub fn hand_off<S>(
    ending: &Ending,
    pane: Option<&Pane>,
    session: Option<String>,
    pid: Option<u32>,
    spawn: S,
) -> Option<String>
where
    S: FnOnce(&Clear) -> anyhow::Result<()>,
{
    // a runner's session is never cleared: the runner starts what comes next in a fresh one
    if pane.is_some_and(Pane::runs) {
        return ending.run_message();
    }
    // another node next clears the session, wherever reef can type into its pane
    let started = order(ending, pane, session, pid).map(|order| spawn(&order));
    let message = ending.message(matches!(started, Some(Ok(()))))?;
    // an injector that couldn't start leaves the clear to the user, saying why
    match started {
        Some(Err(error)) => Some(format!(
            "{message} (reef couldn't clear it itself: {error:#})"
        )),
        _ => Some(message),
    }
}

/// Starts the injector as a process of its own, so the hook or command that starts it returns at
/// once
///
/// It runs this same binary, in a process group of its own so nothing Claude Code sends the
/// hook's group reaches it, with nothing piped back to Claude Code, and its output appended to
/// the log.
///
/// # Arguments
///
/// * `order` - Where to type, and after which node
/// * `conf` - The config this reef was named, passed on so the injector reaches the same service
pub fn spawn(order: &Clear, conf: Option<&Path>) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    // what it says goes to the log, since nothing would show it, or nowhere if that can't open
    let log = log_file()
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    // this same binary, on the same service
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if let Some(conf) = conf {
        command.arg("--conf").arg(conf);
    }
    command
        .args(order.args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0);
    // never waited on: it outlives the hook, which init reaps it for
    let child = command.spawn()?;
    drop(child);
    Ok(())
}

/// Opens the injector's log for appending, under `$XDG_STATE_HOME`, else `~/.local/state`
fn log_file() -> anyhow::Result<std::fs::File> {
    // the state directory, as the XDG spec places it
    let state = match std::env::var_os("XDG_STATE_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| anyhow::anyhow!("neither XDG_STATE_HOME nor HOME is set"))?;
            PathBuf::from(home).join(".local/state")
        }
    };
    // reef's own directory in it
    let dir = state.join("reef");
    std::fs::create_dir_all(&dir)?;
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("clear.log"))?)
}

/// Types into a zellij or tmux pane and reads its screen back
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typist {
    /// A zellij pane, written to and read by zellij's actions
    Zellij {
        /// The zellij session the pane is in
        session: String,
        /// The pane's id
        pane: String,
    },
    /// A tmux pane, written to by `send-keys` and read by `capture-pane`
    Tmux {
        /// The server's socket, or empty for the default server
        server: String,
        /// The pane's id, such as `%3`
        pane: String,
    },
}

impl Typist {
    /// The typist for a pane, or `None` for a Claude Code process with no multiplexer
    ///
    /// # Arguments
    ///
    /// * `pane` - The pane the session runs in
    pub fn for_pane(pane: &Pane) -> Option<Typist> {
        // only a multiplexer's pane can be typed into
        match pane {
            Pane::Zellij { session, pane } => Some(Typist::Zellij {
                session: session.clone(),
                pane: pane.clone(),
            }),
            Pane::Tmux { server, pane } => Some(Typist::Tmux {
                server: server.clone(),
                pane: pane.clone(),
            }),
            Pane::Claude { .. } | Pane::Reef { .. } => None,
        }
    }

    /// The multiplexer's command, aimed at this pane's session or server
    ///
    /// # Arguments
    ///
    /// * `args` - What follows `zellij --session S action`, or `tmux [-S SOCKET]`
    fn command(&self, args: &[&str]) -> tokio::process::Command {
        // zellij's actions go by session, and tmux's commands by server
        let mut command = match self {
            Typist::Zellij { session, .. } => {
                let mut command = tokio::process::Command::new("zellij");
                command.args(["--session", session.as_str(), "action"]);
                command
            }
            Typist::Tmux { server, .. } => {
                let mut command = tokio::process::Command::new("tmux");
                if !server.is_empty() {
                    command.args(["-S", server.as_str()]);
                }
                command
            }
        };
        command.args(args).stdin(Stdio::null());
        command
    }

    /// Types text into the pane, as if from its keyboard, without pressing Enter
    ///
    /// # Arguments
    ///
    /// * `text` - What to type
    pub async fn text(&self, text: &str) -> anyhow::Result<()> {
        // as literal characters, never as key names
        let command = match self {
            Typist::Zellij { pane, .. } => {
                self.command(&["write-chars", "--pane-id", pane.as_str(), text])
            }
            Typist::Tmux { pane, .. } => {
                self.command(&["send-keys", "-t", pane.as_str(), "-l", text])
            }
        };
        ran(command).await.map(|_| ())
    }

    /// Presses Enter in the pane
    pub async fn enter(&self) -> anyhow::Result<()> {
        // a carriage return, which is what the Enter key sends
        let command = match self {
            Typist::Zellij { pane, .. } => {
                self.command(&["write", "--pane-id", pane.as_str(), "13"])
            }
            Typist::Tmux { pane, .. } => self.command(&["send-keys", "-t", pane.as_str(), "Enter"]),
        };
        ran(command).await.map(|_| ())
    }

    /// The pane's screen as it is drawn, with its colours, so faint text can be told apart
    pub async fn screen(&self) -> anyhow::Result<String> {
        // the visible screen to standard output, escape codes and all
        let command = match self {
            Typist::Zellij { pane, .. } => {
                self.command(&["dump-screen", "--pane-id", pane.as_str(), "--ansi"])
            }
            Typist::Tmux { pane, .. } => {
                self.command(&["capture-pane", "-p", "-e", "-t", pane.as_str()])
            }
        };
        let out = ran(command).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Runs a command to its end, failing with what it said if it fails
///
/// # Arguments
///
/// * `command` - The command
async fn ran(mut command: tokio::process::Command) -> anyhow::Result<Vec<u8>> {
    // its output kept, to return or to say why it failed
    let out = command
        .output()
        .await
        .map_err(|error| anyhow::anyhow!("running {command:?}: {error}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "{command:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// What is typed in the input box of the Claude Code screen given, or `None` if no box is on it
///
/// The box is the last prompt line, `❯`, right under a rule, with what follows it up to the
/// rule that closes it. Text drawn faint is left out: it is Claude Code's own, a placeholder
/// or a suggested prompt, never the user's.
///
/// # Arguments
///
/// * `screen` - The screen, with its escape codes, as zellij's `dump-screen --ansi` or tmux's
///   `capture-pane -e` prints it
///
/// # Examples
///
/// ```
/// use reef::clear::input_box;
///
/// let rule = "─".repeat(40);
/// let suggested = format!("{rule}\n❯\u{a0}\x1b[2mcommit the change\x1b[0m\n{rule}\n");
/// assert_eq!(input_box(&suggested).as_deref(), Some(""));
/// let drafted = format!("{rule}\n❯\u{a0}half a thought\n{rule}\n");
/// assert_eq!(input_box(&drafted).as_deref(), Some("half a thought"));
/// assert_eq!(input_box("user@host:~/src$ "), None);
/// ```
pub fn input_box(screen: &str) -> Option<String> {
    // each line's text, and the part of it not drawn faint
    let lines: Vec<(String, String)> = screen.lines().map(styled).collect();
    // the box opens with the last prompt line right under a rule
    let open = (1..lines.len())
        .rev()
        .find(|&at| rule(&lines[at - 1].0) && lines[at].0.trim_start().starts_with(PROMPT))?;
    // and closes at the next rule
    let close = (open + 1..lines.len()).find(|&at| rule(&lines[at].0))?;
    // what was typed: the prompt's sign dropped, each line's end trimmed
    let typed: Vec<&str> = lines[open..close]
        .iter()
        .enumerate()
        .map(|(at, (_, kept))| match at {
            0 => kept.trim_start().trim_start_matches(PROMPT),
            _ => kept.as_str(),
        })
        .map(str::trim_end)
        .collect();
    Some(typed.join("\n").trim().to_string())
}

/// The sign Claude Code's input box starts with
const PROMPT: char = '❯';

/// Whether a line of text is one of the rules Claude Code draws above and below its input box
///
/// Once Claude Code names a session, it writes the name into the top rule's right end, as
/// `──── write-a-txt-file ─`, so a rule may hold a label between its dashes.
///
/// # Arguments
///
/// * `text` - The line's text, without its escape codes
fn rule(text: &str) -> bool {
    // a line of the box-drawing dash, end to end, around any label
    let text = text.trim();
    let dashes = text.chars().filter(|c| *c == '─').count();
    dashes >= 10 && text.starts_with('─') && text.ends_with('─')
}

/// A screen line's text without its escape codes, and the part of that text not drawn faint
///
/// # Arguments
///
/// * `line` - The line, escape codes and all
fn styled(line: &str) -> (String, String) {
    let mut text = String::new();
    let mut kept = String::new();
    let mut faint = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        // a character drawn on screen, kept unless it is faint
        if c != '\x1b' {
            text.push(c);
            if !faint {
                kept.push(c);
            }
            continue;
        }
        match chars.next() {
            // a control sequence runs to its final byte, and SGR's sets how text is drawn
            Some('[') => {
                let mut params = String::new();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        if c == 'm' {
                            faint = sgr(&params, faint);
                        }
                        break;
                    }
                    params.push(c);
                }
            }
            // an operating system command runs to BEL or ST
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            // any other escape is two characters long
            _ => {}
        }
    }
    (text, kept)
}

/// Whether text is drawn faint after an SGR sequence, given whether it was before
///
/// # Arguments
///
/// * `params` - The sequence's parameters, such as `38;2;153;153;153` or `2`
/// * `faint` - Whether text was faint before it
fn sgr(params: &str, mut faint: bool) -> bool {
    let codes: Vec<&str> = params.split(';').collect();
    let mut at = 0;
    while at < codes.len() {
        match codes[at] {
            // a reset, and normal intensity, end faint text
            "" | "0" | "22" => faint = false,
            // faint begins it
            "2" => faint = true,
            // a colour's own numbers are no codes of their own, so `38;2;…` isn't faint
            "38" | "48" | "58" => match codes.get(at + 1) {
                Some(&"5") => at += 2,
                Some(&"2") => at += 4,
                _ => {}
            },
            _ => {}
        }
        at += 1;
    }
    faint
}

/// What `claude agents --json` says of the interactive session a Claude Code process runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// The session it runs now, which a clear changes
    pub session: String,
    /// Whether it waits on the user, rather than working through a turn
    pub idle: bool,
}

/// One entry `claude agents --json` lists, the parts reef reads
#[derive(Debug, Deserialize)]
struct Listed {
    /// The Claude Code process, which interactive sessions name
    #[serde(default)]
    pid: Option<u32>,
    /// `interactive`, or `background` for a session running on its own
    #[serde(default)]
    kind: Option<String>,
    /// The session's id
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    /// `idle` or `busy`, for an interactive session
    #[serde(default)]
    status: Option<String>,
}

/// The interactive session a Claude Code process runs, from what `claude agents --json`
/// printed, or `None` if the process runs none
///
/// # Arguments
///
/// * `listed` - What `claude agents --json` printed
/// * `pid` - The Claude Code process, `CLAUDE_PID`
///
/// # Examples
///
/// ```
/// use reef::clear::agent;
///
/// let listed = r#"[{"pid": 7, "kind": "interactive", "sessionId": "s1", "status": "idle"}]"#;
/// let found = agent(listed, 7).unwrap().unwrap();
/// assert_eq!((found.session.as_str(), found.idle), ("s1", true));
/// assert_eq!(agent(listed, 8).unwrap(), None);
/// ```
pub fn agent(listed: &str, pid: u32) -> anyhow::Result<Option<Agent>> {
    // every session Claude Code runs on this machine
    let listed: Vec<Listed> = serde_json::from_str(listed)
        .map_err(|error| anyhow::anyhow!("`claude agents --json` printed no list: {error}"))?;
    // the interactive one this process runs
    let found = listed
        .into_iter()
        .find(|entry| entry.pid == Some(pid) && entry.kind.as_deref() == Some("interactive"));
    Ok(found.and_then(|entry| {
        Some(Agent {
            session: entry.session_id?,
            idle: entry.status.as_deref() == Some("idle"),
        })
    }))
}

/// Asks Claude Code what session a process of it runs, and whether it is idle
///
/// # Arguments
///
/// * `pid` - The Claude Code process, `CLAUDE_PID`
async fn agent_of(pid: u32) -> anyhow::Result<Option<Agent>> {
    // the sessions Claude Code runs, as JSON for scripts
    let mut command = tokio::process::Command::new("claude");
    command.args(["agents", "--json"]).stdin(Stdio::null());
    let listed = ran(command).await?;
    agent(&String::from_utf8_lossy(&listed), pid)
}

/// What the injector waits on before it types into a session
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    /// The node whose end the loop acted on, which it must still hold
    pub after: NodeName,
    /// The session typed into, which the Claude Code process must still run
    pub session: String,
    /// Whether the session was seen idle, so a turn after it is one reef didn't type
    pub idle_seen: bool,
}

impl Watch {
    /// Waits on a session, not seen idle yet
    ///
    /// # Arguments
    ///
    /// * `after` - The node whose end the loop acted on
    /// * `session` - The session to type into
    pub fn new(after: NodeName, session: String) -> Watch {
        Watch {
            after,
            session,
            idle_seen: false,
        }
    }
}

/// What the injector does next
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Look again in a moment
    Wait,
    /// Type now: the session is idle and its box empty
    Type,
    /// Step aside without typing, for this reason
    Quit(String),
}

/// Judges whether the injector types now, waits or steps aside, from where the pane's loop, the
/// Claude Code process and the input box stand
///
/// # Arguments
///
/// * `watch` - What it waits on, which records the session being seen idle
/// * `feeding` - The pane's loop, if it still has one
/// * `agent` - What the Claude Code process runs, if it still runs
/// * `input` - What the input box holds, or `None` if no box is on screen
pub fn judge(
    watch: &mut Watch,
    feeding: Option<&Loop>,
    agent: Option<&Agent>,
    input: Option<&str>,
) -> Step {
    // the loop must still be between the node it acted on and the next
    let Some(feeding) = feeding else {
        return Step::Quit("the pane's loop was stopped".to_string());
    };
    if feeding.state != LoopState::Running {
        return Step::Quit("the loop was paused".to_string());
    }
    if feeding.node.as_ref() != Some(&watch.after) || feeding.acted.as_ref() != Some(&watch.after) {
        return Step::Quit(format!("the loop moved on from `{}`", watch.after));
    }
    // and Claude Code must still run the session typed into
    let Some(agent) = agent else {
        return Step::Quit("Claude Code no longer runs in the pane".to_string());
    };
    if agent.session != watch.session {
        return Step::Quit(format!("session {} was cleared already", watch.session));
    }
    // a turn after it was idle is one the user, or something else, started
    if !agent.idle {
        return match watch.idle_seen {
            true => Step::Quit("a turn started that reef didn't type".to_string()),
            false => Step::Wait,
        };
    }
    watch.idle_seen = true;
    // typed only into an empty box, never over a draft or where no box is drawn
    match input {
        Some("") => Step::Type,
        _ => Step::Wait,
    }
}

/// Clears the session running in the pane and asks for the next node, once it is idle and its
/// box empty, or steps aside, saying why
///
/// # Arguments
///
/// * `client` - The service's client
/// * `order` - Where to type, and after which node
/// * `log` - Where to say what it did
pub async fn run<W: Write>(client: &Client, order: &Clear, log: &mut W) -> anyhow::Result<()> {
    // only a multiplexer's pane can be typed into
    let Some(typist) = Typist::for_pane(&order.pane) else {
        anyhow::bail!(
            "{} is no zellij or tmux pane, so reef can't type into it",
            order.pane.describe()
        );
    };
    let said = |log: &mut W, what: &str| writeln!(log, "reef: {}: {what}", order.pane);
    said(
        log,
        &format!("clearing session {} after `{}`", order.session, order.after),
    )?;
    let started = Instant::now();
    // the session that stopped is cleared once it is idle with nothing typed
    let mut watch = Watch::new(order.after.clone(), order.session.clone());
    if let Some(why) = settle(client, order, &typist, &mut watch, started).await? {
        return said(log, &format!("stepped aside before `/clear`: {why}")).map_err(Into::into);
    }
    if !typed(&typist, "/clear").await? {
        return said(
            log,
            "the box held more than `/clear`, so Enter wasn't pressed",
        )
        .map_err(Into::into);
    }
    said(log, "typed `/clear`")?;
    // then the clear's new session, in the same process
    let Some(session) = cleared(order).await? else {
        return said(log, "no new session followed `/clear`").map_err(Into::into);
    };
    // which asks for the next node once it too is idle with nothing typed
    let mut watch = Watch::new(order.after.clone(), session.clone());
    if let Some(why) = settle(client, order, &typist, &mut watch, started).await? {
        return said(log, &format!("stepped aside before the kickoff: {why}")).map_err(Into::into);
    }
    if !typed(&typist, KICKOFF).await? {
        return said(
            log,
            "the box held more than the kickoff, so Enter wasn't pressed",
        )
        .map_err(Into::into);
    }
    said(log, &format!("asked session {session} for the next node"))?;
    Ok(())
}

/// Waits until a session can be typed into, or says why it never will be
///
/// # Arguments
///
/// * `client` - The service's client
/// * `order` - Where to type, and after which node
/// * `typist` - Types into the pane and reads it
/// * `watch` - What it waits on
/// * `started` - When the injector started, which its patience runs from
async fn settle(
    client: &Client,
    order: &Clear,
    typist: &Typist,
    watch: &mut Watch,
    started: Instant,
) -> anyhow::Result<Option<String>> {
    loop {
        // where the loop, the Claude Code process and the box stand now
        let loops = client.loops().await?;
        let feeding = loops.iter().find(|feeding| feeding.pane == order.pane);
        let agent = agent_of(order.pid).await?;
        let input = input_box(&typist.screen().await?);
        match judge(watch, feeding, agent.as_ref(), input.as_deref()) {
            Step::Type => return Ok(None),
            Step::Quit(why) => return Ok(Some(why)),
            // a draft left in the box is waited on, though not for ever
            Step::Wait if started.elapsed() >= PATIENCE => {
                return Ok(Some(format!(
                    "waited {} minutes on a busy session or a draft",
                    PATIENCE.as_secs() / 60
                )));
            }
            Step::Wait => {
                // often at first, then slowly while a draft sits in the box
                let poll = match started.elapsed() < QUICK {
                    true => POLL,
                    false => SLOW_POLL,
                };
                tokio::time::sleep(poll).await;
            }
        }
    }
}

/// Types a line into an empty box, then presses Enter only if the box reads as that line alone
///
/// Returns whether Enter was pressed.
///
/// # Arguments
///
/// * `typist` - Types into the pane and reads it
/// * `line` - What to type
async fn typed(typist: &Typist, line: &str) -> anyhow::Result<bool> {
    // the line, then a moment for the TUI to settle, since a slash opens its command menu
    typist.text(line).await?;
    tokio::time::sleep(SETTLE).await;
    // a key the user pressed meanwhile is theirs, so their line is never sent
    let input = input_box(&typist.screen().await?);
    if !input.is_some_and(|input| same_words(&input, line)) {
        return Ok(false);
    }
    typist.enter().await?;
    Ok(true)
}

/// Whether the box reads as a line, however a narrow pane wrapped it
///
/// # Arguments
///
/// * `input` - What the box holds
/// * `line` - What was typed
fn same_words(input: &str, line: &str) -> bool {
    // a wrap splits the line between words, so the words alone are compared
    input.split_whitespace().eq(line.split_whitespace())
}

/// Waits for the session a clear starts in the same Claude Code process, which has a new id
///
/// # Arguments
///
/// * `order` - The session cleared, and the process it ran in
async fn cleared(order: &Clear) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    loop {
        // the process's session, once it isn't the one cleared
        match agent_of(order.pid).await? {
            None => return Ok(None),
            Some(agent) if agent.session != order.session => return Ok(Some(agent.session)),
            Some(_) if started.elapsed() >= CLEARING => return Ok(None),
            Some(_) => tokio::time::sleep(POLL).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Agent, Clear, Step, Watch, agent, hand_off, input_box, judge, order, same_words, sgr,
    };
    use crate::task::Ending;
    use reef_api::{Loop, LoopState, Pane};
    use reef_model::NodeName;

    /// Names a node of `reef/central`
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    fn named(id: &str) -> NodeName {
        format!("reef/central/{id}").parse().unwrap()
    }

    /// A running loop in a zellij pane, holding a node whose end it acted on
    ///
    /// # Arguments
    ///
    /// * `node` - The id of the node it holds and acted on
    fn between(node: &str) -> Loop {
        Loop {
            pane: "zellij:work:3".parse::<Pane>().unwrap(),
            project: "reef".to_string(),
            plan: None,
            session: Some("s1".to_string()),
            node: Some(named(node)),
            acted: Some(named(node)),
            state: LoopState::Running,
            version: 4,
            started_at: 1,
            width: 1,
            opens: true,
            pid: None,
            members: Vec::new(),
        }
    }

    /// An idle or busy session
    ///
    /// # Arguments
    ///
    /// * `session` - The session's id
    /// * `idle` - Whether it waits on the user
    fn running(session: &str, idle: bool) -> Agent {
        Agent {
            session: session.to_string(),
            idle,
        }
    }

    /// The box is read as Claude Code 2.1.330 draws it in zellij and in tmux: empty under a
    /// faint placeholder or suggestion, under a rule holding the session's name, a draft of one
    /// line or two, `/clear` with its menu open above it, and the last box after a clear's own
    /// line in the transcript
    #[test]
    fn the_box_is_read_from_zellij_and_tmux_screens() {
        // what each screen's box holds, as each multiplexer dumped it
        let screens = [
            (include_str!("clear/zellij-placeholder.ansi"), ""),
            (include_str!("clear/tmux-placeholder.ansi"), ""),
            (include_str!("clear/tmux-suggestion.ansi"), ""),
            (include_str!("clear/zellij-busy.ansi"), ""),
            (include_str!("clear/tmux-busy.ansi"), ""),
            (include_str!("clear/zellij-cleared.ansi"), ""),
            (include_str!("clear/zellij-named.ansi"), ""),
            (include_str!("clear/tmux-cleared.ansi"), ""),
            (include_str!("clear/zellij-draft.ansi"), "half a thought"),
            (include_str!("clear/tmux-draft.ansi"), "half a thought"),
            (
                include_str!("clear/zellij-draft-lines.ansi"),
                "half a thought\n  and a second line",
            ),
            (
                include_str!("clear/tmux-draft-lines.ansi"),
                "half a thought\n  and a second line",
            ),
            (include_str!("clear/zellij-clear-menu.ansi"), "/clear"),
            (include_str!("clear/tmux-clear-menu.ansi"), "/clear"),
        ];
        for (at, (screen, typed)) in screens.iter().enumerate() {
            assert_eq!(input_box(screen).as_deref(), Some(*typed), "screen {at}");
        }
    }

    /// A screen with no input box on it, a shell's or a dialog's, holds no box to type into
    #[test]
    fn a_screen_without_the_box_has_none() {
        // a shell once Claude Code has exited
        assert_eq!(input_box("user@host:~/src$ \n\n"), None);
        // the trust dialog, whose choice is a prompt line under no rule of its own
        let rule = "─".repeat(60);
        let dialog = format!(
            "{rule}\n Accessing workspace:\n /src/tui\n\n ❯ No, exit\n   Yes, I trust this \
             folder\n\n Enter to confirm · Esc to cancel\n"
        );
        assert_eq!(input_box(&dialog), None);
        // a box never closed by its rule is no box
        assert_eq!(input_box(&format!("{rule}\n❯ \n")), None);
        // nor is a line of dashes that a label ends, as a tool's output might draw
        let labelled = format!("{rule} notes\n❯ \n{rule}\n");
        assert_eq!(input_box(&labelled), None);
    }

    /// A colour's own numbers never read as faint, and a reset or normal intensity ends it
    #[test]
    fn only_sgr_two_is_faint() {
        // a truecolour or 256-colour grey isn't faint, though a 2 is among its numbers
        assert!(!sgr("38;2;153;153;153", false));
        assert!(!sgr("38;5;2", false));
        assert!(!sgr("48;2;2;2;2", false));
        // faint, alone or after other codes
        assert!(sgr("2", false));
        assert!(sgr("39;49;2", false));
        assert!(sgr("38;2;1;2;3;2", false));
        // and a reset or normal intensity ends it, as zellij writes before each run of text
        for ended in ["", "0", "22", "2;22"] {
            assert!(!sgr(ended, true), "{ended}");
        }
        // other codes leave it as it was
        assert!(sgr("39", true));
        assert!(!sgr("1", false));
    }

    /// The process's interactive session is found by its pid, and a background one by the same
    /// process is never it
    #[test]
    fn the_session_is_found_by_its_process() {
        // an interactive session busy, another idle, and a background session that has no pid
        let listed = r#"[
            {"id": "3c59e498", "kind": "background", "sessionId": "b1", "state": "blocked"},
            {"pid": 8700, "kind": "interactive", "sessionId": "s1", "status": "idle"},
            {"pid": 4242, "kind": "interactive", "sessionId": "s2", "status": "busy"}
        ]"#;
        assert_eq!(agent(listed, 8700).unwrap(), Some(running("s1", true)));
        assert_eq!(agent(listed, 4242).unwrap(), Some(running("s2", false)));
        // a process that runs no session, or isn't Claude Code's
        assert_eq!(agent(listed, 1).unwrap(), None);
        assert_eq!(agent("[]", 8700).unwrap(), None);
        // and output that isn't a list is an error
        assert!(agent("not json", 8700).is_err());
    }

    /// It waits while the session works through the turn that ended the node, and while the box
    /// holds a draft or isn't drawn, then types once the session is idle and the box empty
    #[test]
    fn it_types_only_into_an_idle_session_with_an_empty_box() {
        let feeding = between("C61");
        let mut watch = Watch::new(named("C61"), "s1".to_string());
        // still busy, just after the Stop, is waited on
        let busy = running("s1", false);
        assert_eq!(
            judge(&mut watch, Some(&feeding), Some(&busy), Some("")),
            Step::Wait
        );
        assert!(!watch.idle_seen);
        // idle with a draft, or with no box drawn, is waited on too
        let idle = running("s1", true);
        for input in [Some("half a thought"), Some("/cl"), None] {
            assert_eq!(
                judge(&mut watch, Some(&feeding), Some(&idle), input),
                Step::Wait,
                "{input:?}"
            );
        }
        assert!(watch.idle_seen);
        // idle with an empty box is typed into
        assert_eq!(
            judge(&mut watch, Some(&feeding), Some(&idle), Some("")),
            Step::Type
        );
    }

    /// It steps aside once the user sends a prompt of their own, the loop is paused, stopped or
    /// moved on, or the session is cleared or gone
    #[test]
    fn it_steps_aside_when_the_session_or_loop_moves_on() {
        let feeding = between("C61");
        let idle = running("s1", true);
        let quits = |watch: &mut Watch, feeding: Option<&Loop>, agent: Option<&Agent>| {
            matches!(judge(watch, feeding, agent, Some("")), Step::Quit(_))
        };
        // a turn once it was seen idle is the user's own prompt
        let mut watch = Watch::new(named("C61"), "s1".to_string());
        judge(&mut watch, Some(&feeding), Some(&idle), Some("a draft"));
        assert!(quits(
            &mut watch,
            Some(&feeding),
            Some(&running("s1", false))
        ));
        // a loop stopped, paused, or holding another node
        let mut watch = Watch::new(named("C61"), "s1".to_string());
        assert!(quits(&mut watch, None, Some(&idle)));
        let paused = Loop {
            state: LoopState::Paused,
            ..feeding.clone()
        };
        assert!(quits(&mut watch, Some(&paused), Some(&idle)));
        let took = Loop {
            node: Some(named("C60")),
            ..feeding.clone()
        };
        assert!(quits(&mut watch, Some(&took), Some(&idle)));
        let unacted = Loop {
            acted: Some(named("C41")),
            ..feeding.clone()
        };
        assert!(quits(&mut watch, Some(&unacted), Some(&idle)));
        // a session cleared already, by the user, or a Claude Code that exited
        assert!(quits(
            &mut watch,
            Some(&feeding),
            Some(&running("s2", true))
        ));
        assert!(quits(&mut watch, Some(&feeding), None));
    }

    /// A line wrapped by a narrow pane still reads as the line typed, and one with more in it
    /// doesn't
    #[test]
    fn a_wrapped_line_reads_as_typed() {
        let kickoff = "Take the next node from reef's loop with next_task";
        assert!(same_words(
            "Take the next node from reef's\n  loop with next_task",
            kickoff
        ));
        assert!(same_words("/clear", "/clear"));
        assert!(!same_words("half a thought/clear", "/clear"));
        assert!(!same_words("", "/clear"));
    }

    /// The injector is handed a node's end only when another is next, in a zellij or tmux pane,
    /// with the session and its Claude Code process named
    #[test]
    fn only_a_next_node_in_a_typed_pane_is_cleared_after() {
        let done = named("C41");
        let next = Ending::Next {
            done: done.clone(),
            next: named("C61"),
            round: false,
        };
        let zellij: Pane = "zellij:work:3".parse().unwrap();
        let session = || Some("s1".to_string());
        // a zellij or tmux pane, its session and its process named, is cleared after
        assert_eq!(
            order(&next, Some(&zellij), session(), Some(4242)),
            Some(Clear {
                pane: zellij.clone(),
                session: "s1".to_string(),
                pid: 4242,
                after: done.clone(),
            })
        );
        let tmux: Pane = "tmux:/tmp/tmux-1000/default:%3".parse().unwrap();
        assert!(order(&next, Some(&tmux), session(), Some(4242)).is_some());
        // a Claude Code process with no multiplexer, or no pane, is the user's to clear
        let bare = Pane::Claude { pid: 4242 };
        assert_eq!(order(&next, Some(&bare), session(), Some(4242)), None);
        assert_eq!(order(&next, None, session(), Some(4242)), None);
        // as is a session or a process Claude Code didn't name
        assert_eq!(order(&next, Some(&zellij), None, Some(4242)), None);
        assert_eq!(
            order(&next, Some(&zellij), Some(" ".to_string()), Some(4242)),
            None
        );
        assert_eq!(order(&next, Some(&zellij), session(), None), None);
        // and a pause, a finished plan or nothing to act on clears nothing
        for ending in [
            Ending::Paused(done.clone()),
            Ending::Finished(done.clone()),
            Ending::Nothing,
        ] {
            assert_eq!(order(&ending, Some(&zellij), session(), Some(4242)), None);
        }
    }

    /// Handing off starts the injector once where it can type, and says reef clears; where it
    /// can't, or the injector fails to start, it says `/clear`, then `next`, and why
    #[test]
    fn a_hand_off_says_who_clears() {
        let next = Ending::Next {
            done: named("C61"),
            next: named("C71"),
            round: false,
        };
        let zellij: Pane = "zellij:work:3".parse().unwrap();
        // a pane reef can type into starts the injector once, and says reef clears
        let mut started = Vec::new();
        let said = hand_off(
            &next,
            Some(&zellij),
            Some("s1".to_string()),
            Some(4242),
            |order| {
                started.push(order.clone());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(started.len(), 1);
        assert!(
            said.ends_with(
                "reef clears the context and asks for it once Claude is idle and the input box is \
                 empty"
            ),
            "{said}"
        );
        // an injector that fails to start leaves the clear to the user, saying why
        let said = hand_off(
            &next,
            Some(&zellij),
            Some("s1".to_string()),
            Some(4242),
            |_| anyhow::bail!("no such program"),
        )
        .unwrap();
        assert!(
            said.ends_with(
                "`/clear`, then say `next` (reef couldn't clear it itself: no such program)"
            ),
            "{said}"
        );
        // and nothing to act on starts nothing and says nothing
        let said = hand_off(
            &Ending::Nothing,
            Some(&zellij),
            Some("s1".to_string()),
            Some(4242),
            |_| unreachable!(),
        );
        assert_eq!(said, None);
    }
}
