//! The runner: `reef loop run`, which runs a loop's sessions headless on the user's own machine,
//! and relays each one's transcript and prompts through central reef to the app
//!
//! - **The server runs nothing.** The user starts the runner in their repo's checkout. It
//!   attaches to central reef as a session of its own and spawns Claude itself, so central reef
//!   only relays.
//! - **A pane of its own.** The runner is a [`Pane::Reef`], which it names to each session in
//!   `REEF_PANE`, so the session's MCP server and hooks find its loop as a terminal session's
//!   find its multiplexer's pane.
//! - **A fresh session a node.** Once the Stop hook has acted on a node's end, which it does
//!   before the turn's `result`, the runner ends the session and starts the next node in a new
//!   one, with the terminal loop's kickoff. Nothing is typed, and nothing is cleared.
//! - **It follows its loop** ([`decide`]), at each turn's end and whenever central reef wakes it.
//! - **Every prompt waits for the app.** A question, a plan, or a tool plan mode or the
//!   permission mode asks about is reported as waiting, and answered when the app's answer
//!   comes as an order.
//! - **Orders by number.** It asks central reef for the orders after the last it read, so a
//!   dropped connection misses none, and a service that restarted, and so forgot it, is attached
//!   to again with the transcript the runner holds.

pub mod drive;
pub mod transcript;

use reef_api::session::KEPT_ENTRIES;
use reef_api::{
    Attach, EditLoop, Entry, EntryKind, Loop, LoopEdit, LoopState, Order, Ordered, Pane, Prompt,
    Report, Session, SessionState,
};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::clear::KICKOFF;
use crate::cli::{self, PlanPick};
use crate::client::Client;
use crate::loops::between;
use crate::task::{Ending, upcoming};
use drive::{Claude, Line, Spawn};
use transcript::Read;

/// How long one ask for orders may wait for one
pub(crate) const ORDERS_WAIT: Duration = Duration::from_secs(25);

/// How long a session is given to end once its stdin is closed, before it is killed
const END_WAIT: Duration = Duration::from_secs(10);

/// How long to wait before asking again after central reef didn't answer
pub(crate) const RETRY: Duration = Duration::from_secs(2);

/// What `reef loop run` was asked
#[derive(Debug, Clone, Default)]
pub struct Runs {
    /// The Claude Code to run, else `claude`
    pub claude: Option<String>,
    /// Whether to start the loop at once, rather than wait for the app to
    pub start: bool,
    /// The permission mode each session starts in, if not Claude Code's default
    pub mode: Option<String>,
    /// More arguments for each session, such as `--model`
    pub args: Vec<String>,
    /// The config this reef was named, handed to each session's MCP server and hooks
    pub conf: Option<PathBuf>,
    /// The credentials file this reef was named, handed to each session's MCP server and hooks
    pub credentials: Option<PathBuf>,
}

/// Where the runner's Claude stands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proc {
    /// No session runs
    None,
    /// A session is working a turn
    Busy,
    /// A session's turn ended, and it waits for a message
    Idle,
}

/// What made the runner read its loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// It attached
    Attached,
    /// Central reef woke it, the app having changed its loop
    Woken,
    /// Its session's turn ended
    TurnEnded,
    /// Its session exited of its own accord
    Exited,
}

/// What the runner does once it has read its loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Leave things as they are, standing as said
    Stay(SessionState),
    /// Start a session, for the loop's next node
    Spawn,
    /// End the session, and start a fresh one for the loop's next node
    Restart,
    /// End the session, standing as said
    End(SessionState),
}

/// Decides what the runner does, from its loop, its session and what made it look
///
/// - **No loop:** it ends any session, and waits for a start.
/// - **A turn under way:** it lets the turn go on.
/// - **No session:** a running loop gets one when the runner attaches or is woken, never when a
///   session just ended, so a session that keeps failing can't spin it. A paused one waits.
/// - **A turn ended:** between nodes, the next node starts in a fresh session, if one comes.
///   With nothing ready after the node done, or paused with it done, the session ends. Holding
///   nothing, which next_task took nothing for, it ends, unless the app has just asked again.
///   Mid-node, it waits for the user's reply.
///
/// # Arguments
///
/// * `feeding` - The runner's loop, if it has one
/// * `proc` - Where its Claude stands
/// * `why` - What made it look
/// * `coming` - Whether a node, or a round of one's comments, comes after the node its loop holds
///   done, as next_task would take it
///
/// # Examples
///
/// ```
/// use reef::run::{Proc, Step, Why, decide};
/// use reef_api::SessionState;
///
/// assert_eq!(decide(None, Proc::Idle, Why::TurnEnded, false), Step::End(SessionState::Idle));
/// assert_eq!(decide(None, Proc::None, Why::Woken, false), Step::Stay(SessionState::Idle));
/// ```
pub fn decide(feeding: Option<&Loop>, proc: Proc, why: Why, coming: bool) -> Step {
    // a stopped loop, or none started, has no session
    let Some(feeding) = feeding else {
        return match proc {
            Proc::None => Step::Stay(SessionState::Idle),
            Proc::Busy | Proc::Idle => Step::End(SessionState::Idle),
        };
    };
    let paused = feeding.state == LoopState::Paused;
    // whether the end of the node it holds was acted on
    let done = feeding.node.is_some() && feeding.acted == feeding.node;
    match proc {
        // a turn under way goes on
        Proc::Busy => Step::Stay(SessionState::Working),
        // a session only when asked for, never straight after one ended
        Proc::None if paused => Step::Stay(SessionState::Paused),
        Proc::None => match why {
            Why::Attached | Why::Woken => Step::Spawn,
            Why::TurnEnded | Why::Exited => Step::Stay(SessionState::Idle),
        },
        // a turn ended: the next node, a pause, nothing taken, or a reply awaited
        Proc::Idle if done && paused => Step::End(SessionState::Paused),
        Proc::Idle if done && coming => Step::Restart,
        Proc::Idle if done => Step::End(SessionState::Idle),
        Proc::Idle if feeding.node.is_none() => match why {
            Why::Woken => Step::Restart,
            _ => Step::End(SessionState::Idle),
        },
        Proc::Idle => Step::Stay(SessionState::Waiting),
    }
}

/// The current time in milliseconds since the epoch
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// A runner's id, or a terminal's session's: when it started, in base 36, and its process's id
pub(crate) fn runner_id() -> String {
    // the time in base 36, short and sorted by when it started
    let mut ms = now_ms();
    let mut digits = Vec::new();
    while ms > 0 {
        let digit = (ms % 36) as u32;
        digits.push(char::from_digit(digit, 36).unwrap_or('0'));
        ms /= 36;
    }
    let time: String = digits.iter().rev().collect();
    format!("{time}-{}", std::process::id())
}

/// The host the runner runs on, as the app shows it and a login's token is labelled with
pub(crate) fn host_name() -> String {
    // the kernel's name for it, else the `hostname` command's, else this one
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        })
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

/// One entry as the runner prints it in its own terminal, a line each
///
/// # Arguments
///
/// * `kind` - The entry
pub fn line_of(kind: &EntryKind) -> String {
    // the first line of anything long, so the terminal follows along
    let first = |text: &str| text.lines().next().unwrap_or_default().to_string();
    match kind {
        EntryKind::User { text } => format!("> {}", first(text)),
        EntryKind::Text { text } => format!("claude: {}", first(text)),
        EntryKind::Tool { name, input } => format!("  {name} {input}"),
        EntryKind::Output { text, error: true } => format!("  ! {}", first(text)),
        EntryKind::Output { .. } => "  ✓".to_string(),
        EntryKind::Asked { prompt } => {
            format!("? {} (answer it in reef's app)", prompt_line(prompt))
        }
        EntryKind::Answered { answer, .. } => format!("= {}", answer.describe()),
        EntryKind::Session { id, mode } => format!("session {id}, {mode} mode"),
        EntryKind::Turn { turns, cost, ms } => {
            format!(
                "turn ended: {turns} turns, {:.1} s, ${cost:.2} so far",
                *ms as f64 / 1000.0
            )
        }
        EntryKind::Note { text } => format!("reef: {text}"),
    }
}

/// What a prompt asks, in a line
///
/// # Arguments
///
/// * `prompt` - The prompt
fn prompt_line(prompt: &Prompt) -> String {
    match &prompt.kind {
        reef_api::PromptKind::Permission { tool, .. } => format!("may Claude run {tool}?"),
        reef_api::PromptKind::Question { questions } => questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join(" "),
        reef_api::PromptKind::Plan { .. } => "Claude's plan is ready for approval".to_string(),
    }
}

/// A session's side of central reef, the runner's and the follower's alike: what it attaches
/// with, the transcript it holds, what it hasn't reported yet, and where it stands
///
/// - **One writer, each report awaited.** A report is sent only once the last was answered, and
///   a failed one keeps its entries for the next, so they arrive once and in order.
/// - **Attached again whole.** A service that restarted forgets the session, so the next report
///   or ask for orders attaches again with the latest [`KEPT_ENTRIES`] entries held, as many as
///   central reef keeps.
pub(crate) struct Relay<'a> {
    /// Central reef's client
    client: &'a Client,
    /// What it attaches with: its pane, its own id if it names one, project, plan, host and
    /// checkout
    attach: Attach,
    /// Its session's id
    pub(crate) id: String,
    /// Where it stands, as last said
    pub(crate) state: SessionState,
    /// Claude Code's id for the session running, if one is
    pub(crate) claude: Option<String>,
    /// The transcript's latest entries, which it sends again if central reef forgets it
    held: VecDeque<Entry>,
    /// The entries not reported yet
    unsent: Vec<Entry>,
    /// Whether its state, prompts or Claude changed since its last report
    pub(crate) dirty: bool,
}

impl<'a> Relay<'a> {
    /// A relay for a session that isn't attached yet, standing as said
    ///
    /// # Arguments
    ///
    /// * `client` - Central reef's client
    /// * `attach` - What it attaches with, its state, entries and prompts set as it attaches
    /// * `id` - Its session's id
    pub(crate) fn new(client: &'a Client, attach: Attach, id: String) -> Relay<'a> {
        // standing as the attach says, holding nothing yet
        Relay {
            client,
            state: attach.state,
            claude: attach.claude.clone(),
            attach,
            id,
            held: VecDeque::new(),
            unsent: Vec::new(),
            dirty: false,
        }
    }

    /// Adds an entry to the transcript, letting go of the oldest held past [`KEPT_ENTRIES`]
    ///
    /// # Arguments
    ///
    /// * `entry` - The entry
    pub(crate) fn push(&mut self, entry: Entry) {
        // held to attach again with, and kept for the next report
        self.held.push_back(entry.clone());
        while self.held.len() > KEPT_ENTRIES {
            self.held.pop_front();
        }
        self.unsent.push(entry);
    }

    /// Stands as said, reporting it if it changed
    ///
    /// # Arguments
    ///
    /// * `state` - Where it now stands
    pub(crate) fn stand(&mut self, state: SessionState) {
        // only a change is worth reporting
        if self.state != state {
            self.state = state;
            self.dirty = true;
        }
    }

    /// Names the Claude Code session running, reporting it if it changed
    ///
    /// # Arguments
    ///
    /// * `claude` - Claude Code's id for it, if one runs
    pub(crate) fn run_claude(&mut self, claude: Option<String>) {
        // only a change is worth reporting
        if self.claude != claude {
            self.claude = claude;
            self.dirty = true;
        }
    }

    /// Attaches to central reef, again with everything it holds if it attached before
    ///
    /// # Arguments
    ///
    /// * `prompts` - The prompts waiting for an answer
    pub(crate) async fn attach(&mut self, prompts: Vec<Prompt>) -> anyhow::Result<Session> {
        // the transcript held, and where it stands now
        let attach = Attach {
            state: self.state,
            entries: self.held.iter().cloned().collect(),
            prompts,
            claude: self.claude.clone(),
            ..self.attach.clone()
        };
        let session = self.client.attach_session(&attach).await?;
        self.unsent.clear();
        self.dirty = false;
        Ok(session)
    }

    /// Reports the entries since the last report and where it stands, attaching again if
    /// central reef forgot it; a service that doesn't answer is told on the next report
    ///
    /// It returns the session as central reef now keeps it, or `None` if nothing was new or
    /// the service didn't answer.
    ///
    /// # Arguments
    ///
    /// * `prompts` - The prompts waiting for an answer
    /// * `out` - Where a failed report is said
    pub(crate) async fn report<W: Write>(
        &mut self,
        prompts: Vec<Prompt>,
        out: &mut W,
    ) -> anyhow::Result<Option<Session>> {
        // nothing new is nothing to say
        if self.unsent.is_empty() && !self.dirty {
            return Ok(None);
        }
        let report = Report {
            entries: self.unsent.clone(),
            state: self.state,
            claude: self.claude.clone(),
            prompts: prompts.clone(),
        };
        match self.client.report_session(&self.id, &report).await {
            Ok(Some(session)) => {
                self.unsent.clear();
                self.dirty = false;
                Ok(Some(session))
            }
            // a service that restarted forgot the session, so it is attached again whole
            Ok(None) => self.attach(prompts).await.map(Some),
            // kept for the next report
            Err(error) => {
                let _ = writeln!(out, "reef: reporting to central reef failed: {error:#}");
                Ok(None)
            }
        }
    }
}

/// A runner: its session's state, what it has reported and what it holds back
struct Runner<'a, W: Write> {
    /// Central reef's client
    client: &'a Client,
    /// How it starts each session
    spawn: Spawn,
    /// Its side of central reef: its transcript, where it stands and what it hasn't reported
    relay: Relay<'a>,
    /// Where it prints the transcript as it goes
    out: &'a mut W,
    /// The session running, if one is
    claude: Option<Claude>,
    /// Where that session stands
    proc: Proc,
    /// Each prompt waiting for an answer, with the tool's input as Claude asked it
    raised: Vec<(Prompt, Value)>,
}

impl<W: Write> Runner<'_, W> {
    /// Adds an entry to the transcript, and prints it
    ///
    /// # Arguments
    ///
    /// * `kind` - What the entry says
    fn note(&mut self, kind: EntryKind) {
        // printed as a line, kept whole
        let _ = writeln!(self.out, "{}", line_of(&kind));
        self.relay.push(Entry { at: now_ms(), kind });
    }

    /// The prompts waiting for an answer
    fn prompts(&self) -> Vec<Prompt> {
        self.raised
            .iter()
            .map(|(prompt, _)| prompt.clone())
            .collect()
    }

    /// Attaches to central reef, again with everything it holds if it attached before
    async fn attach(&mut self) -> anyhow::Result<()> {
        // the transcript held, and the prompts waiting
        let prompts = self.prompts();
        self.relay.attach(prompts).await?;
        Ok(())
    }

    /// Reports the entries since the last report and where it stands, attaching again if
    /// central reef forgot it; a service that doesn't answer is told on the next report
    async fn report(&mut self) -> anyhow::Result<()> {
        // the prompts waiting, beside what the relay holds
        let prompts = self.prompts();
        self.relay.report(prompts, &mut *self.out).await?;
        Ok(())
    }

    /// Stands as said, reporting it if it changed
    ///
    /// # Arguments
    ///
    /// * `state` - Where it now stands
    fn stand(&mut self, state: SessionState) {
        // only a change is worth reporting
        self.relay.stand(state);
    }

    /// Reads its loop, and does what [`decide`] says
    ///
    /// # Arguments
    ///
    /// * `why` - What made it look
    async fn reconcile(&mut self, why: Why) -> anyhow::Result<()> {
        // its own pane's loop, if it has one
        let loops = self.client.loops().await?;
        let feeding = loops.iter().find(|feeding| feeding.pane == self.spawn.pane);
        // and whether anything comes after a node it holds done, as the Stop hook asks
        let done = feeding.and_then(|feeding| between(feeding).map(|done| (feeding, done)));
        let coming = match done {
            Some((feeding, done)) => matches!(
                upcoming(self.client, feeding, done, &loops).await?,
                Ending::Next { .. }
            ),
            None => false,
        };
        match decide(feeding, self.proc, why, coming) {
            Step::Stay(state) => self.stand(state),
            Step::Spawn => self.start_session().await?,
            Step::Restart => {
                // the node done, or nothing taken, then a fresh session for what comes next
                let said = match feeding.and_then(|feeding| feeding.node.as_ref()) {
                    Some(node) => {
                        format!("`{node}` is done, so the next node starts in a fresh session")
                    }
                    None => "starting again in a fresh session".to_string(),
                };
                self.note(EntryKind::Note { text: said });
                self.end_session().await;
                self.start_session().await?;
            }
            Step::End(state) => {
                // a pause, nothing ready after the node done, or no node at all
                let finished = feeding.is_some_and(|feeding| between(feeding).is_some());
                let said = match (state, finished) {
                    (SessionState::Paused, _) => "the loop is paused, so the session ends here",
                    (_, true) => "nothing else is ready to take, so the session ends here",
                    _ => "the session ends, with no node to work",
                };
                self.note(EntryKind::Note {
                    text: said.to_string(),
                });
                self.end_session().await;
                self.stand(state);
            }
        }
        Ok(())
    }

    /// Starts a session, and asks it for the loop's next node
    async fn start_session(&mut self) -> anyhow::Result<()> {
        // the terminal loop's kickoff, in a session of its own
        let mut claude = Claude::spawn(&self.spawn)?;
        claude.send(&transcript::user(KICKOFF)).await?;
        self.claude = Some(claude);
        self.proc = Proc::Busy;
        self.note(EntryKind::User {
            text: KICKOFF.to_string(),
        });
        self.stand(SessionState::Working);
        Ok(())
    }

    /// Ends the session running, if one is: its stdin closed, then killed if it lingers
    async fn end_session(&mut self) {
        let Some(mut claude) = self.claude.take() else {
            return;
        };
        // what it says while it ends is kept, until it exits
        claude.close();
        let ended = tokio::time::timeout(END_WAIT, async {
            while let Some(line) = claude.next().await {
                match line {
                    Line::Out(line) => {
                        if let Read::Said(kinds) = transcript::read(&line) {
                            kinds.into_iter().for_each(|kind| self.note(kind));
                        }
                    }
                    Line::Err(_) => {}
                    Line::Exited(_) => break,
                }
            }
        })
        .await;
        if ended.is_err() {
            claude.kill();
            let _ = tokio::time::timeout(END_WAIT, claude.next()).await;
        }
        // nothing it raised waits any more
        self.proc = Proc::None;
        self.relay.run_claude(None);
        if !self.raised.is_empty() {
            self.raised.clear();
            self.relay.dirty = true;
        }
    }

    /// Takes one line the session wrote
    ///
    /// # Arguments
    ///
    /// * `line` - The line
    async fn on_line(&mut self, line: &str) -> anyhow::Result<()> {
        match transcript::read(line) {
            Read::Said(kinds) => kinds.into_iter().for_each(|kind| self.note(kind)),
            // a prompt waits for the app's answer
            Read::Raised { prompt, input } => {
                self.note(EntryKind::Asked {
                    prompt: prompt.clone(),
                });
                self.raised.push((prompt, input));
                self.relay.dirty = true;
            }
            Read::Withdrawn { id } => {
                self.raised.retain(|(prompt, _)| prompt.id != id);
                self.note(EntryKind::Note {
                    text: "Claude no longer waits on a prompt".to_string(),
                });
                self.relay.dirty = true;
            }
            // anything else asked of a host is refused, so the session waits on nothing
            Read::Other { id, subtype } => {
                if let Some(claude) = self.claude.as_mut() {
                    claude.send(&transcript::refuse(&id, &subtype)).await?;
                }
            }
            Read::Began { id, mode } => {
                if self.relay.claude.as_deref() != Some(id.as_str()) {
                    self.relay.run_claude(Some(id.clone()));
                    self.note(EntryKind::Session { id, mode });
                }
            }
            // the turn's end, after the Stop hook acted on the node's: the loop says what's next
            Read::Ended { turns, cost, ms } => {
                self.note(EntryKind::Turn { turns, cost, ms });
                self.proc = Proc::Idle;
                if !self.raised.is_empty() {
                    self.raised.clear();
                    self.relay.dirty = true;
                }
                self.reconcile(Why::TurnEnded).await?;
            }
            Read::Nothing => {}
        }
        Ok(())
    }

    /// Takes the session's exit
    ///
    /// # Arguments
    ///
    /// * `code` - Its exit code, if it had one
    async fn on_exit(&mut self, code: Option<i32>) -> anyhow::Result<()> {
        // an exit nobody asked for is said, and the loop asked what now
        self.claude = None;
        self.proc = Proc::None;
        self.relay.run_claude(None);
        if !self.raised.is_empty() {
            self.raised.clear();
            self.relay.dirty = true;
        }
        let how = code.map_or("with no code".to_string(), |code| {
            format!("with code {code}")
        });
        self.note(EntryKind::Note {
            text: format!("Claude exited {how}"),
        });
        self.reconcile(Why::Exited).await
    }

    /// Carries out one of central reef's orders
    ///
    /// # Arguments
    ///
    /// * `order` - The order
    async fn on_order(&mut self, order: Order) -> anyhow::Result<()> {
        match order {
            Order::Wake => self.reconcile(Why::Woken).await?,
            // an answer to a prompt still waiting, sent once
            Order::Answer { prompt, answer } => {
                let Some(at) = self
                    .raised
                    .iter()
                    .position(|(raised, _)| raised.id == prompt)
                else {
                    self.note(EntryKind::Note {
                        text: format!("no prompt `{prompt}` waits for an answer"),
                    });
                    return Ok(());
                };
                let (raised, input) = self.raised.remove(at);
                if let Some(claude) = self.claude.as_mut() {
                    claude
                        .send(&transcript::respond(&raised.id, &input, &answer))
                        .await?;
                }
                self.note(EntryKind::Answered { prompt, answer });
                self.relay.dirty = true;
            }
            // a reply, to a session whose turn ended mid-node
            Order::Say { text } => match (self.proc, self.claude.as_mut()) {
                (Proc::Idle, Some(claude)) => {
                    claude.send(&transcript::user(&text)).await?;
                    self.proc = Proc::Busy;
                    self.note(EntryKind::User { text });
                    self.stand(SessionState::Working);
                }
                _ => self.note(EntryKind::Note {
                    text: "Claude isn't waiting for a reply".to_string(),
                }),
            },
        }
        Ok(())
    }

    /// The next line from the session, or never if none runs
    async fn next_line(claude: &mut Option<Claude>) -> Option<Line> {
        match claude {
            Some(claude) => claude.next().await,
            None => std::future::pending().await,
        }
    }
}

/// Runs a loop's sessions headless here, relaying them through central reef, until Ctrl-C
///
/// It attaches, starts the loop if asked to, and then follows its loop and central reef's
/// orders. Once stopped, it ends any session, lets go of its loop's node, and detaches.
///
/// # Arguments
///
/// * `client` - Central reef's client
/// * `cwd` - The working directory, whose checkout the sessions run in and whose project they
///   take nodes from
/// * `runs` - How to run each session, and whether to start the loop at once
/// * `pick` - The project and plan named, if they were
/// * `out` - Where the transcript is printed as it goes
pub async fn run<W: Write>(
    client: &Client,
    cwd: &Path,
    runs: &Runs,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    run_until(client, cwd, runs, pick, out, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

/// Runs a loop's sessions headless here until a future ends, as [`run`] does until Ctrl-C
///
/// # Arguments
///
/// * `client` - Central reef's client
/// * `cwd` - The working directory
/// * `runs` - How to run each session, and whether to start the loop at once
/// * `pick` - The project and plan named, if they were
/// * `out` - Where the transcript is printed as it goes
/// * `stop` - Ends the run once it is ready
pub async fn run_until<W: Write>(
    client: &Client,
    cwd: &Path,
    runs: &Runs,
    pick: &PlanPick,
    out: &mut W,
    stop: impl std::future::Future<Output = ()>,
) -> anyhow::Result<()> {
    // the project, its checkout, and a pane of the runner's own
    let project = cli::picked(client, cwd, &pick.project).await?;
    let root = drive::checkout_root(cwd)?;
    let id = runner_id();
    let pane = Pane::Reef { id: id.clone() };
    let attach = Attach {
        id: None,
        pane: pane.clone(),
        project: project.name.clone(),
        plan: pick.plan.clone(),
        host: host_name(),
        cwd: root.clone(),
        state: SessionState::Idle,
        entries: Vec::new(),
        prompts: Vec::new(),
        claude: None,
    };
    let spawn = Spawn {
        claude: runs.claude.clone().unwrap_or_else(|| "claude".to_string()),
        args: runs.args.clone(),
        mode: runs.mode.clone(),
        cwd: root,
        pane: pane.clone(),
        conf: runs.conf.clone(),
        credentials: runs.credentials.clone(),
    };
    let mut runner = Runner {
        client,
        spawn,
        relay: Relay::new(client, attach, id.clone()),
        out,
        claude: None,
        proc: Proc::None,
        raised: Vec::new(),
    };
    runner.attach().await?;
    writeln!(
        runner.out,
        "attached {} to `{}`: start its loop from reef's app{}",
        pane.describe(),
        project.name,
        if runs.start { ", or now, as asked" } else { "" }
    )?;
    // started at once if asked, else when the app says
    if runs.start {
        let start = LoopEdit::Start {
            project: project.name.clone(),
            plan: pick.plan.clone(),
            session: None,
            width: None,
            opens: Some(false),
            pid: None,
        };
        client
            .edit_loop(&EditLoop {
                pane: pane.clone(),
                edit: start,
            })
            .await?;
    }
    runner.reconcile(Why::Attached).await?;
    let ran = follow(&mut runner, stop).await;
    // ended: no session, no node held, and central reef told
    runner.end_session().await;
    let stopped = client
        .edit_loop(&EditLoop {
            pane: pane.clone(),
            edit: LoopEdit::Stop,
        })
        .await;
    runner.note(EntryKind::Note {
        text: "the runner stopped".to_string(),
    });
    runner.stand(SessionState::Detached);
    let _ = runner.report().await;
    let _ = client.detach_session(&id).await;
    stopped?;
    ran
}

/// Follows the session's lines and central reef's orders until the run is stopped
///
/// # Arguments
///
/// * `runner` - The runner
/// * `stop` - Ends the run once it is ready
async fn follow<W: Write>(
    runner: &mut Runner<'_, W>,
    stop: impl std::future::Future<Output = ()>,
) -> anyhow::Result<()> {
    tokio::pin!(stop);
    let client = runner.client;
    let id = runner.relay.id.clone();
    // the ask for orders, kept across lines so a line never cancels it
    let mut after = 0;
    let mut asking = Box::pin(client.session_orders(&id, after, ORDERS_WAIT));
    loop {
        runner.report().await?;
        tokio::select! {
            _ = &mut stop => return Ok(()),
            line = Runner::<W>::next_line(&mut runner.claude) => match line {
                Some(Line::Out(line)) => runner.on_line(&line).await?,
                Some(Line::Err(line)) => {
                    let _ = writeln!(runner.out, "claude: {line}");
                }
                Some(Line::Exited(code)) => runner.on_exit(code).await?,
                None => runner.on_exit(None).await?,
            },
            asked = &mut asking => {
                match asked {
                    Ok(Some(orders)) => {
                        for Ordered { n, order, .. } in orders {
                            after = n;
                            runner.on_order(order).await?;
                        }
                    }
                    // forgotten by a service that restarted: attached again, orders from 0
                    Ok(None) => {
                        runner.attach().await?;
                        after = 0;
                    }
                    Err(error) => {
                        let _ = writeln!(runner.out, "reef: asking central reef for orders failed: {error:#}");
                        tokio::time::sleep(RETRY).await;
                    }
                }
                asking = Box::pin(client.session_orders(&id, after, ORDERS_WAIT));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Proc, Step, Why, decide};
    use reef_api::{Loop, LoopState, Pane, SessionState};

    /// A runner's loop, running or paused, holding a node or none, its end acted on or not
    ///
    /// # Arguments
    ///
    /// * `state` - Whether it runs
    /// * `node` - The node it holds, a bare id in `reef/central`
    /// * `acted` - Whether its node's end was acted on
    fn looped(state: LoopState, node: Option<&str>, acted: bool) -> Loop {
        let node = node.map(|id| format!("reef/central/{id}").parse().unwrap());
        Loop {
            changed_by: reef_api::Author::default(),
            started_by: reef_api::Author::default(),
            pane: Pane::Reef {
                id: "k3x9-4242".to_string(),
            },
            project: "reef".to_string(),
            plan: None,
            session: None,
            acted: if acted { node.clone() } else { None },
            node,
            state,
            version: 1,
            started_at: 0,
            width: 1,
            opens: false,
            pid: None,
            members: Vec::new(),
        }
    }

    /// The runner starts a session only when asked, moves on between nodes while a node comes,
    /// ends at a pause, with nothing after the node done or with nothing taken, waits mid-node,
    /// and never spins on a session that just ended
    #[test]
    fn the_runner_follows_its_loop() {
        let running = LoopState::Running;
        let paused = LoopState::Paused;
        let started = looped(running, None, false);
        let mid = looped(running, Some("C45"), false);
        let between = looped(running, Some("C45"), true);
        let paused_done = looped(paused, Some("C45"), true);
        let paused_mid = looped(paused, Some("C45"), false);
        // a session for a running loop when the runner attaches or is woken, and not after one
        // ended of its own accord
        assert_eq!(
            decide(Some(&started), Proc::None, Why::Woken, false),
            Step::Spawn
        );
        assert_eq!(
            decide(Some(&mid), Proc::None, Why::Attached, false),
            Step::Spawn
        );
        assert_eq!(
            decide(Some(&mid), Proc::None, Why::Exited, false),
            Step::Stay(SessionState::Idle)
        );
        assert_eq!(
            decide(Some(&paused_done), Proc::None, Why::Woken, true),
            Step::Stay(SessionState::Paused)
        );
        // a turn under way goes on, unless its loop is gone
        assert_eq!(
            decide(Some(&between), Proc::Busy, Why::Woken, true),
            Step::Stay(SessionState::Working)
        );
        assert_eq!(
            decide(None, Proc::Busy, Why::Woken, false),
            Step::End(SessionState::Idle)
        );
        // at a turn's end: the next node, a pause, nothing taken, or a reply awaited
        assert_eq!(
            decide(Some(&between), Proc::Idle, Why::TurnEnded, true),
            Step::Restart
        );
        // and with nothing after it, the session ends rather than start one that takes nothing
        assert_eq!(
            decide(Some(&between), Proc::Idle, Why::TurnEnded, false),
            Step::End(SessionState::Idle)
        );
        assert_eq!(
            decide(Some(&paused_done), Proc::Idle, Why::TurnEnded, true),
            Step::End(SessionState::Paused)
        );
        assert_eq!(
            decide(Some(&started), Proc::Idle, Why::TurnEnded, false),
            Step::End(SessionState::Idle)
        );
        assert_eq!(
            decide(Some(&started), Proc::Idle, Why::Woken, false),
            Step::Restart
        );
        assert_eq!(
            decide(Some(&mid), Proc::Idle, Why::TurnEnded, false),
            Step::Stay(SessionState::Waiting)
        );
        assert_eq!(
            decide(Some(&paused_mid), Proc::Idle, Why::Woken, false),
            Step::Stay(SessionState::Waiting)
        );
    }
}
