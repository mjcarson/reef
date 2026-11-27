//! The follower: the Claude Code session the user is in, attached to central reef as itself, its
//! transcript streamed to the app as it works
//!
//! ```text
//! reef loop run      # typed in a Claude Code session, as `! reef loop run`
//! reef loop detach   # typed in the same session, as `! reef loop detach`
//! reef hook follow --session ID --pid PID --claude ID --pane KEY --project NAME --cwd DIR --inbox PATH [--plan NAME] [--transcript PATH]
//! ```
//!
//! - **The session the user is in, not a second one.** Typed in a Claude Code session,
//!   `reef loop run` attaches that session, on its terminal's pane, under an id of its own, and
//!   returns at once. Nothing is spawned but the follower, and the session is used as it was:
//!   every prompt, question and plan is answered in its terminal, and nothing waits on the app.
//! - **A follower in the background** ([`spawn`]), started as the injector is, in a process
//!   group of its own, its output in `$XDG_STATE_HOME/reef/follow.log`. It reads the transcript
//!   Claude Code writes on disk every quarter second, each complete line once
//!   ([`Tail`]), as entries ([`Recorder`]), and reports them through a [`Relay`], the runner's own
//!   bookkeeping, so they reach central reef once and in order.
//! - **An inbox for each Claude Code process,** `$XDG_STATE_HOME/reef/follow/<CLAUDE_PID>.jsonl`,
//!   a JSON event a line ([`Event`]). The attach writes who holds it, and a second `reef loop
//!   run` finds it held. SessionStart writes each session the process starts, and `reef loop
//!   detach` asks the follower to let go.
//! - **A clear moves the session to a new file.** `/clear` starts a new Claude Code session in
//!   the same process, which SessionStart names in the inbox. The follower reads the old file to
//!   its end, then follows the new file from its start, noting the new session in the mode its
//!   first lines name.
//! - **Live while it asks for orders.** It long-polls for orders, as a runner does, which keeps
//!   the session from reading as gone, and attaches again if central reef forgot it. It carries
//!   out none, since the app's acts on a terminal's session are refused.
//! - **It ends** once detached, once the Claude Code process is gone, or once another session
//!   attached on its pane replaced it. It reads the file to its end, says why it ended, and
//!   detaches, so the app shows the session ended.

use reef_api::{Attach, Entry, EntryKind, Pane, SessionState};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::clear;
use crate::cli::{self, PlanPick};
use crate::client::Client;
use crate::run::transcript::Recorder;
use crate::run::{ORDERS_WAIT, RETRY, Relay, drive, host_name, now_ms, runner_id};

/// How often the follower reads the transcript and its inbox again
const POLL: Duration = Duration::from_millis(250);

/// How long `reef loop detach` waits for the follower to let go
const DETACH_WAIT: Duration = Duration::from_secs(5);

/// How often `reef loop detach` looks whether the follower let go
const DETACH_POLL: Duration = Duration::from_millis(100);

/// The log the follower writes to, in reef's state directory
pub const LOG: &str = "follow.log";

/// A Claude Code session this process runs in, as its environment names it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Here {
    /// The pane it runs in: its multiplexer's, else its Claude Code process
    pub pane: Pane,
    /// Claude Code's id for the session, `CLAUDE_CODE_SESSION_ID`
    pub claude: String,
    /// The Claude Code process it runs in, `CLAUDE_PID`
    pub pid: u32,
    /// The inbox of the follower for that process
    pub inbox: PathBuf,
    /// The transcript Claude Code writes for the session, if it has written one yet
    pub transcript: Option<PathBuf>,
}

/// The Claude Code session this process runs in, if Claude Code ran it outside a runner's pane
///
/// It needs `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID`, which Claude Code hands its Bash commands
/// and hooks. A runner's own sessions run on a `reef:` pane, and are none.
///
/// # Arguments
///
/// * `var` - Reads one of this process's environment variables
///
/// # Examples
///
/// ```
/// use reef::follow::in_claude;
///
/// let env = |name: &str| match name {
///     "CLAUDE_CODE_SESSION_ID" => Some("s1".to_string()),
///     "CLAUDE_PID" => Some("4242".to_string()),
///     "XDG_STATE_HOME" => Some("/state".to_string()),
///     _ => None,
/// };
/// let here = in_claude(env).unwrap().unwrap();
/// assert_eq!(here.pane.to_string(), "claude:4242");
/// assert_eq!(here.inbox.display().to_string(), "/state/reef/follow/4242.jsonl");
/// ```
pub fn in_claude<F: Fn(&str) -> Option<String>>(var: F) -> anyhow::Result<Option<Here>> {
    // a variable set to nothing is as good as unset
    let set = |name: &str| var(name).filter(|value| !value.trim().is_empty());
    let Some(claude) = set("CLAUDE_CODE_SESSION_ID") else {
        return Ok(None);
    };
    let Some(pid) = set("CLAUDE_PID").and_then(|pid| pid.trim().parse::<u32>().ok()) else {
        return Ok(None);
    };
    // the pane it runs in, which a runner's session names as the runner's
    let Some(pane) = Pane::from_env(&var).filter(|pane| !pane.runs()) else {
        return Ok(None);
    };
    let inbox = inbox_of(&var, pid)?;
    let transcript = transcript_of(&claude, &var);
    Ok(Some(Here {
        pane,
        claude,
        pid,
        inbox,
        transcript,
    }))
}

/// The inbox of the follower for a Claude Code process, in reef's state directory
///
/// # Arguments
///
/// * `var` - Reads one environment variable, `XDG_STATE_HOME` or `HOME`
/// * `pid` - The Claude Code process, `CLAUDE_PID`
pub fn inbox_of<F: Fn(&str) -> Option<String>>(var: F, pid: u32) -> anyhow::Result<PathBuf> {
    // one file a process, under reef's own state directory
    Ok(clear::state_dir(var)?
        .join("follow")
        .join(format!("{pid}.jsonl")))
}

/// The transcript Claude Code writes for a session, `projects/*/<session>.jsonl` under
/// `$CLAUDE_CONFIG_DIR`, else `~/.claude`, if it has written one
///
/// # Arguments
///
/// * `session` - Claude Code's id for the session
/// * `var` - Reads one environment variable, `CLAUDE_CONFIG_DIR` or `HOME`
pub fn transcript_of<F: Fn(&str) -> Option<String>>(session: &str, var: F) -> Option<PathBuf> {
    // an id that could reach out of its directory names no transcript
    if session.is_empty() || session.contains(['/', '\\']) || session.contains("..") {
        return None;
    }
    // Claude Code's directory, which `CLAUDE_CONFIG_DIR` moves
    let config = match var("CLAUDE_CONFIG_DIR").filter(|dir| !dir.trim().is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(var("HOME").filter(|home| !home.trim().is_empty())?).join(".claude"),
    };
    // each project's directory, the first that holds the session's file
    let file = format!("{session}.jsonl");
    let mut projects: Vec<PathBuf> = std::fs::read_dir(config.join("projects"))
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    projects.sort();
    projects
        .into_iter()
        .map(|project| project.join(&file))
        .find(|path| path.is_file())
}

/// One event in a follower's inbox
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// A follower took the inbox, for the session central reef keeps under an id
    Started {
        /// The follower's process
        follower: u32,
        /// The id central reef keeps the session under
        session: String,
    },
    /// A Claude Code session started in the process, as SessionStart says: a clear's new one,
    /// or the one followed, started again or compacted
    Moved {
        /// Claude Code's id for the session
        claude: String,
        /// The transcript Claude Code writes for it
        transcript: PathBuf,
    },
    /// The user asked for the session to be detached
    Detach,
}

/// Adds an event to an inbox that exists, a line of its own
///
/// # Arguments
///
/// * `inbox` - The inbox
/// * `event` - The event
pub fn post(inbox: &Path, event: &Event) -> anyhow::Result<()> {
    // one write of a whole line, appended, so a reader never sees half of it
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .append(true)
        .open(inbox)
        .map_err(|error| anyhow::anyhow!("opening {}: {error}", inbox.display()))?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// The events an inbox holds, in the order they were posted
///
/// # Arguments
///
/// * `inbox` - The inbox
pub fn events(inbox: &Path) -> Vec<Event> {
    // a line that doesn't read as an event, or a missing inbox, says nothing
    std::fs::read_to_string(inbox)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The follower holding an inbox and the session it follows, if one holds it and is alive
///
/// # Arguments
///
/// * `inbox` - The inbox
pub fn holder(inbox: &Path) -> Option<(u32, String)> {
    // the last follower that took it, if it still runs
    let (follower, session) = events(inbox)
        .into_iter()
        .rev()
        .find_map(|event| match event {
            Event::Started { follower, session } => Some((follower, session)),
            _ => None,
        })?;
    alive(follower).then_some((follower, session))
}

/// Whether a process runs: its `/proc` entry, unless it is a zombie, else what `kill -0` says
///
/// # Arguments
///
/// * `pid` - The process's id
pub fn alive(pid: u32) -> bool {
    // Linux's own view, where a process that exited but isn't reaped yet is a zombie
    let proc = Path::new("/proc");
    if proc.join("self").exists() {
        let Ok(stat) = std::fs::read_to_string(proc.join(pid.to_string()).join("stat")) else {
            return false;
        };
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next());
        return !matches!(state, Some('Z' | 'X'));
    }
    // elsewhere, a signal that is only checked
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A file read as it grows, a complete line at a time
#[derive(Debug, Clone)]
pub struct Tail {
    /// The file
    path: PathBuf,
    /// How far it has been read, in bytes
    offset: u64,
    /// The last line read, held back until it ends
    partial: Vec<u8>,
}

impl Tail {
    /// A tail of a file, read from its start
    ///
    /// # Arguments
    ///
    /// * `path` - The file, which may not exist yet
    pub fn new(path: PathBuf) -> Tail {
        Tail {
            path,
            offset: 0,
            partial: Vec::new(),
        }
    }

    /// The file it reads
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The complete lines added since the last read, a last line with no end held back until
    /// it has one, and nothing for a file that doesn't exist yet
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::follow::Tail;
    /// use std::io::Write;
    ///
    /// let path = std::env::temp_dir().join(format!("reef-tail-doc-{}", std::process::id()));
    /// let mut tail = Tail::new(path.clone());
    /// assert!(tail.lines().unwrap().is_empty());
    /// std::fs::write(&path, "one\ntw").unwrap();
    /// assert_eq!(tail.lines().unwrap(), ["one"]);
    /// let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    /// file.write_all(b"o\n").unwrap();
    /// assert_eq!(tail.lines().unwrap(), ["two"]);
    /// std::fs::remove_file(&path).unwrap();
    /// ```
    pub fn lines(&mut self) -> std::io::Result<Vec<String>> {
        // a file not written yet has nothing to read
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        // what was added since, after any part of a line held back
        file.seek(SeekFrom::Start(self.offset))?;
        let mut added = Vec::new();
        file.read_to_end(&mut added)?;
        self.offset += added.len() as u64;
        self.partial.extend_from_slice(&added);
        // every line up to the last end, the rest held back
        let Some(end) = self.partial.iter().rposition(|byte| *byte == b'\n') else {
            return Ok(Vec::new());
        };
        let rest = self.partial.split_off(end + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        Ok(String::from_utf8_lossy(&complete)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect())
    }
}

/// What a follower follows: the session as it attached, its Claude Code process, its inbox and
/// its transcript
#[derive(Debug, Clone, PartialEq)]
pub struct Follow {
    /// What the session attached with, its own id and Claude Code's among it
    pub attach: Attach,
    /// The Claude Code process the session runs in, `CLAUDE_PID`
    pub pid: u32,
    /// The follower's inbox for that process
    pub inbox: PathBuf,
    /// The transcript Claude Code writes for the session, else found once it is written
    pub transcript: Option<PathBuf>,
}

impl Follow {
    /// The id central reef keeps the session under
    pub fn id(&self) -> &str {
        self.attach.id.as_deref().unwrap_or_default()
    }

    /// The command line, after reef's own flags, that runs the follower, as [`spawn`] does
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::follow::Follow;
    /// use reef_api::{Attach, Pane, SessionState};
    ///
    /// let follow = Follow {
    ///     attach: Attach {
    ///         id: Some("k3x9-7".to_string()),
    ///         pane: Pane::Claude { pid: 4242 },
    ///         project: "reef".to_string(),
    ///         plan: None,
    ///         host: "box".to_string(),
    ///         cwd: "/src/reef".into(),
    ///         state: SessionState::Working,
    ///         entries: Vec::new(),
    ///         prompts: Vec::new(),
    ///         claude: Some("s1".to_string()),
    ///     },
    ///     pid: 4242,
    ///     inbox: "/state/reef/follow/4242.jsonl".into(),
    ///     transcript: None,
    /// };
    /// assert_eq!(
    ///     follow.args().join(" "),
    ///     "hook follow --session k3x9-7 --pid 4242 --claude s1 --pane claude:4242 --project reef \
    ///      --cwd /src/reef --inbox /state/reef/follow/4242.jsonl"
    /// );
    /// ```
    pub fn args(&self) -> Vec<String> {
        // each part by its flag, the pane by its key
        let mut args = vec![
            "hook".to_string(),
            "follow".to_string(),
            "--session".to_string(),
            self.id().to_string(),
            "--pid".to_string(),
            self.pid.to_string(),
            "--claude".to_string(),
            self.attach.claude.clone().unwrap_or_default(),
            "--pane".to_string(),
            self.attach.pane.to_string(),
            "--project".to_string(),
            self.attach.project.clone(),
        ];
        // the plan and the transcript only when known
        if let Some(plan) = &self.attach.plan {
            args.extend(["--plan".to_string(), plan.clone()]);
        }
        args.extend([
            "--cwd".to_string(),
            self.attach.cwd.display().to_string(),
            "--inbox".to_string(),
            self.inbox.display().to_string(),
        ]);
        if let Some(transcript) = &self.transcript {
            args.extend(["--transcript".to_string(), transcript.display().to_string()]);
        }
        args
    }
}

/// Starts the follower in the background, as the injector is started, returning its process's
/// id
///
/// # Arguments
///
/// * `follow` - What it follows
/// * `conf` - The config this reef read, passed on so the follower reaches the same service
/// * `credentials` - The credentials file this reef was named, passed on so it signs in as this
///   reef does
pub fn spawn(
    follow: &Follow,
    conf: Option<&Path>,
    credentials: Option<&Path>,
) -> anyhow::Result<u32> {
    use std::os::unix::process::CommandExt;
    // what it says goes to its log, since nothing would show it, or nowhere if that can't open
    let log = clear::open_log(LOG)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    // this same binary, the new one if it was replaced while running, on the same service,
    // signed in as this reef is
    let mut command = std::process::Command::new(crate::update::own_exe()?);
    if let Some(conf) = conf {
        command.arg("--conf").arg(conf);
    }
    if let Some(credentials) = credentials {
        command.arg("--credentials").arg(credentials);
    }
    command
        .args(follow.args())
        .current_dir(&follow.attach.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0);
    // never waited on: it outlives the command, which init reaps it for
    let child = command.spawn()?;
    let pid = child.id();
    drop(child);
    Ok(pid)
}

/// Attaches the Claude Code session this runs in to central reef, and starts its follower,
/// returning what it follows, or `None` if a follower holds the session already
///
/// # Arguments
///
/// * `client` - Central reef's client
/// * `cwd` - The working directory, whose checkout and project the session is shown with
/// * `here` - The session, its pane, its Claude Code process and its inbox
/// * `pick` - The project and plan named, if they were
/// * `spawn` - Starts the follower in the background, as [`spawn`] does, returning its process
/// * `out` - Where to print what came of it
pub async fn attach<W, S>(
    client: &Client,
    cwd: &Path,
    here: &Here,
    pick: &PlanPick,
    spawn: S,
    out: &mut W,
) -> anyhow::Result<Option<Follow>>
where
    W: Write,
    S: FnOnce(&Follow) -> anyhow::Result<u32>,
{
    // a session followed already is said to be
    if let Some((follower, session)) = holder(&here.inbox) {
        writeln!(
            out,
            "this session is attached to central reef already, as `{session}`, its follower \
             process {follower}: type `! reef loop detach` to detach it"
        )?;
        return Ok(None);
    }
    // the session, working, on its terminal's pane under an id of its own
    let project = cli::picked(client, cwd, &pick.project).await?;
    let id = runner_id();
    let attach = Attach {
        id: Some(id.clone()),
        pane: here.pane.clone(),
        project: project.name.clone(),
        plan: pick.plan.clone(),
        host: host_name(),
        cwd: drive::checkout_root(cwd)?,
        state: SessionState::Working,
        entries: Vec::new(),
        prompts: Vec::new(),
        claude: Some(here.claude.clone()),
    };
    client.attach_session(&attach).await?;
    // the inbox, emptied, for SessionStart's moves and a detach
    if let Some(dir) = here.inbox.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&here.inbox, "")?;
    let follow = Follow {
        attach,
        pid: here.pid,
        inbox: here.inbox.clone(),
        transcript: here.transcript.clone(),
    };
    // the follower, which central reef lets go of again if it doesn't start
    let follower = match spawn(&follow) {
        Ok(follower) => follower,
        Err(error) => {
            let _ = client.detach_session(&id).await;
            let _ = std::fs::remove_file(&here.inbox);
            return Err(error.context("starting the session's follower"));
        }
    };
    post(
        &here.inbox,
        &Event::Started {
            follower,
            session: id.clone(),
        },
    )?;
    writeln!(
        out,
        "attached this Claude Code session to `{}` as `{id}`: reef's app shows its transcript as \
         it works, and its prompts are answered here as ever. Type `! reef loop detach` to \
         detach it.",
        project.name
    )?;
    Ok(Some(follow))
}

/// Asks the follower of the Claude Code session this runs in to let go of it, and waits a
/// while for it to
///
/// # Arguments
///
/// * `here` - The session, its Claude Code process and its inbox
/// * `out` - Where to print what came of it
pub async fn detach<W: Write>(here: &Here, out: &mut W) -> anyhow::Result<()> {
    // only a session followed can be let go of
    let Some((follower, session)) = holder(&here.inbox) else {
        writeln!(
            out,
            "this session isn't attached to central reef: type `! reef loop run` to attach it"
        )?;
        return Ok(());
    };
    post(&here.inbox, &Event::Detach)?;
    // let go of once the follower removes its inbox
    let deadline = tokio::time::Instant::now() + DETACH_WAIT;
    while here.inbox.exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(DETACH_POLL).await;
    }
    match here.inbox.exists() {
        true => writeln!(
            out,
            "asked the follower of `{session}`, process {follower}, to detach it, and it hasn't \
             let go yet: its log is {LOG} in reef's state directory"
        )?,
        false => writeln!(
            out,
            "detached this session, `{session}`, from central reef: reef's app shows it ended"
        )?,
    }
    Ok(())
}

/// Why a follower stopped following its session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The user detached it, with `reef loop detach`
    Detached,
    /// Its Claude Code process is gone
    Exited,
    /// Another session attached on its pane, which central reef shows instead
    Replaced,
    /// The follower itself was stopped
    Stopped,
}

impl Ended {
    /// Why it ended, as the transcript's last note says
    pub fn describe(self) -> &'static str {
        match self {
            Ended::Detached => "the session was detached in its terminal",
            Ended::Exited => "the session's Claude Code exited",
            Ended::Replaced => "another session attached on its pane",
            Ended::Stopped => "the session's follower stopped",
        }
    }
}

/// A follower: its relay, the reader of its transcript, and the inbox it reads
struct Follower<'a, W: Write> {
    /// Its side of central reef: the transcript held, where it stands and what is unsent
    relay: Relay<'a>,
    /// Reads the transcript's lines as entries
    recorder: Recorder,
    /// The transcript read so far, once it is known
    tail: Option<Tail>,
    /// Claude Code's id for the session followed
    claude: String,
    /// A clear's new session, noted once its file names the mode it runs in, or says something
    unnoted: Option<String>,
    /// The inbox, read as it grows
    inbox: Tail,
    /// Where it logs
    out: &'a mut W,
}

impl<W: Write> Follower<'_, W> {
    /// Reads the transcript's new lines into the relay, finding the file first if it isn't
    /// known yet
    fn read_transcript(&mut self) {
        // a transcript Claude Code hadn't written at the attach is looked for again
        if self.tail.is_none() {
            let var = |name: &str| std::env::var(name).ok();
            self.tail = transcript_of(&self.claude, var).map(Tail::new);
        }
        let Some(tail) = self.tail.as_mut() else {
            return;
        };
        let lines = match tail.lines() {
            Ok(lines) => lines,
            Err(error) => {
                let _ = writeln!(self.out, "reef: reading {}: {error}", tail.path().display());
                return;
            }
        };
        // each line's entries, and where the session stands after it
        for line in lines {
            let recorded = self.recorder.read(&line);
            // a clear's new session first, once a line names its mode or says something
            if recorded.mode.is_some() || !recorded.entries.is_empty() {
                self.note_session();
            }
            recorded
                .entries
                .into_iter()
                .for_each(|entry| self.relay.push(entry));
            if let Some(state) = recorded.state {
                self.relay.stand(state);
            }
        }
    }

    /// Notes a clear's new session, if one is still to be, in the mode the session last named
    fn note_session(&mut self) {
        // only once, before anything the new session says
        let Some(id) = self.unnoted.take() else {
            return;
        };
        let mode = self.recorder.mode();
        self.relay.push(Entry {
            at: now_ms(),
            kind: EntryKind::Session { id, mode },
        });
    }

    /// Reads the inbox's new events, following a clear's new session, and says whether one
    /// asked it to let go
    fn read_inbox(&mut self) -> Option<Ended> {
        // a missing inbox was removed by hand, which asks nothing
        let lines = self.inbox.lines().unwrap_or_default();
        for event in lines
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
        {
            match event {
                Event::Detach => return Some(Ended::Detached),
                // the session followed, started again or compacted, stays in its file
                Event::Moved { claude, .. } if claude == self.claude => {}
                // a clear's new session: the old file read to its end, then the new from its start
                Event::Moved { claude, transcript } => {
                    self.read_transcript();
                    self.note_session();
                    let _ = writeln!(
                        self.out,
                        "reef: following session {claude}, in {}",
                        transcript.display()
                    );
                    // noted once its file says the mode it runs in
                    self.unnoted = Some(claude.clone());
                    self.relay.run_claude(Some(claude.clone()));
                    self.relay.stand(SessionState::Idle);
                    self.claude = claude;
                    self.tail = Some(Tail::new(transcript));
                }
                Event::Started { .. } => {}
            }
        }
        None
    }
}

/// Follows a session until it ends, as the follower `reef hook follow` runs it, until it is
/// detached, its Claude Code exits, or the follower is signalled to stop
///
/// # Arguments
///
/// * `client` - Central reef's client
/// * `follow` - What it follows
/// * `out` - Where it logs
pub async fn run<W: Write>(client: &Client, follow: &Follow, out: &mut W) -> anyhow::Result<Ended> {
    // a signal to stop is a stop like any other, so it still detaches
    let stop = async {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(_) => return std::future::pending().await,
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    };
    run_until(client, follow, out, stop).await
}

/// Follows a session until it ends or a future does, as [`run`] does until a signal
///
/// # Arguments
///
/// * `client` - Central reef's client
/// * `follow` - What it follows
/// * `out` - Where it logs
/// * `stop` - Ends the following once it is ready
pub async fn run_until<W: Write>(
    client: &Client,
    follow: &Follow,
    out: &mut W,
    stop: impl std::future::Future<Output = ()>,
) -> anyhow::Result<Ended> {
    tokio::pin!(stop);
    let id = follow.id().to_string();
    let claude = follow.attach.claude.clone().unwrap_or_default();
    writeln!(out, "reef: following session {claude} as `{id}`")?;
    // attached already, holding nothing yet
    let mut follower = Follower {
        relay: Relay::new(client, follow.attach.clone(), id.clone()),
        recorder: Recorder::default(),
        tail: follow.transcript.clone().map(Tail::new),
        claude,
        unnoted: None,
        inbox: Tail::new(follow.inbox.clone()),
        out,
    };
    // the ask for orders, kept across reads so a read never cancels it
    let mut after = 0;
    let mut asking = Box::pin(client.session_orders(&id, after, ORDERS_WAIT));
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let ended = loop {
        tokio::select! {
            _ = &mut stop => break Ended::Stopped,
            _ = tick.tick() => {
                // what was written since, then what the inbox asks, then whether Claude runs
                follower.read_transcript();
                if let Some(ended) = follower.read_inbox() {
                    break ended;
                }
                if !alive(follow.pid) {
                    break Ended::Exited;
                }
                // a session let go of while it was followed was replaced on its pane
                let reported = follower.relay.report(Vec::new(), &mut *follower.out).await?;
                if reported.is_some_and(|session| session.state == SessionState::Detached) {
                    break Ended::Replaced;
                }
            }
            asked = &mut asking => {
                match asked {
                    // none is carried out, but the next report says whether it was replaced
                    Ok(Some(orders)) => {
                        after = orders.iter().map(|ordered| ordered.n).max().unwrap_or(after);
                        follower.relay.dirty = true;
                    }
                    // forgotten by a service that restarted: attached again, orders from 0
                    Ok(None) => {
                        follower.relay.attach(Vec::new()).await?;
                        after = 0;
                    }
                    Err(error) => {
                        let _ = writeln!(follower.out, "reef: asking central reef for orders failed: {error:#}");
                        tokio::time::sleep(RETRY).await;
                    }
                }
                asking = Box::pin(client.session_orders(&id, after, ORDERS_WAIT));
            }
        }
    };
    // the rest of the transcript, a new session that said nothing yet, why it ended, and
    // central reef told
    follower.read_transcript();
    follower.note_session();
    follower.relay.push(Entry {
        at: now_ms(),
        kind: EntryKind::Note {
            text: ended.describe().to_string(),
        },
    });
    follower.relay.stand(SessionState::Detached);
    let _ = follower.relay.report(Vec::new(), &mut *follower.out).await;
    let _ = client.detach_session(&id).await;
    // the inbox removed, unless a later attach took it
    let taken = events(&follow.inbox)
        .into_iter()
        .any(|event| matches!(event, Event::Started { session, .. } if session != id));
    if !taken {
        let _ = std::fs::remove_file(&follow.inbox);
    }
    writeln!(
        follower.out,
        "reef: stopped following `{id}`: {}",
        ended.describe()
    )?;
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::{Event, Tail, events, in_claude, post, transcript_of};
    use std::io::Write;

    /// A directory of the test's own, empty
    ///
    /// # Arguments
    ///
    /// * `name` - The test's name
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reef-follow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// A Claude Code session is found from its variables, but not without its process, and not
    /// on a runner's pane, whose sessions the app shows already
    #[test]
    fn a_session_is_found_from_its_environment() {
        // the variables Claude Code hands a `!` command in a tmux pane
        let env = |name: &str| match name {
            "CLAUDE_CODE_SESSION_ID" => Some("s1".to_string()),
            "CLAUDE_PID" => Some("4242".to_string()),
            "TMUX" => Some("/tmp/tmux-1000/default,1,0".to_string()),
            "TMUX_PANE" => Some("%3".to_string()),
            "HOME" => Some("/home/me".to_string()),
            _ => None,
        };
        let here = in_claude(env).unwrap().expect("a session");
        assert_eq!(here.pane.to_string(), "tmux:/tmp/tmux-1000/default:%3");
        assert_eq!((here.claude.as_str(), here.pid), ("s1", 4242));
        assert_eq!(
            here.inbox.display().to_string(),
            "/home/me/.local/state/reef/follow/4242.jsonl"
        );
        // no process named, no session
        let no_pid = |name: &str| (name != "CLAUDE_PID").then(|| env(name)).flatten();
        assert_eq!(in_claude(no_pid).unwrap(), None);
        // and a runner's session is the runner's
        let runner = |name: &str| match name {
            "REEF_PANE" => Some("reef:k3x9-7".to_string()),
            other => env(other),
        };
        assert_eq!(in_claude(runner).unwrap(), None);
        // outside Claude Code, none
        assert_eq!(in_claude(|_| None).unwrap(), None);
    }

    /// A session's transcript is found in whichever project's directory holds it, under the
    /// config directory named, and an id that could leave it finds none
    #[test]
    fn a_transcript_is_found_under_claudes_directory() {
        let dir = scratch("transcript");
        let project = dir.join("projects").join("-src-reef");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(dir.join("projects").join("-src-other")).unwrap();
        std::fs::write(project.join("s1.jsonl"), "{}\n").unwrap();
        let config = dir.display().to_string();
        let env = |name: &str| (name == "CLAUDE_CONFIG_DIR").then(|| config.clone());
        assert_eq!(transcript_of("s1", env), Some(project.join("s1.jsonl")));
        assert_eq!(transcript_of("s2", env), None);
        assert_eq!(transcript_of("../s1", env), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Events posted to an inbox read back as they were posted, and an inbox that doesn't exist
    /// takes none
    #[test]
    fn an_inbox_reads_back_as_posted() {
        let dir = scratch("inbox");
        let inbox = dir.join("4242.jsonl");
        // nothing is posted to an inbox no attach made
        assert!(post(&inbox, &Event::Detach).is_err());
        std::fs::write(&inbox, "").unwrap();
        let posted = [
            Event::Started {
                follower: 7,
                session: "k3x9-7".to_string(),
            },
            Event::Moved {
                claude: "s2".to_string(),
                transcript: dir.join("s2.jsonl"),
            },
            Event::Detach,
        ];
        for event in &posted {
            post(&inbox, event).unwrap();
        }
        assert_eq!(events(&inbox), posted);
        // each a line of its own, tagged by its event
        let text = std::fs::read_to_string(&inbox).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert_eq!(text.lines().last(), Some(r#"{"event":"detach"}"#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tail returns each complete line once, holding a partial last line back until it ends
    #[test]
    fn a_tail_returns_complete_lines_once() {
        let dir = scratch("tail");
        let path = dir.join("t.jsonl");
        let mut tail = Tail::new(path.clone());
        // a file not written yet reads as nothing
        assert!(tail.lines().unwrap().is_empty());
        std::fs::write(&path, "{\"a\":1}\n{\"b\":").unwrap();
        assert_eq!(tail.lines().unwrap(), [r#"{"a":1}"#]);
        // nothing new is nothing, and a line ends in a later write
        assert!(tail.lines().unwrap().is_empty());
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"2}\n{\"c\":3}\n").unwrap();
        assert_eq!(tail.lines().unwrap(), [r#"{"b":2}"#, r#"{"c":3}"#]);
        assert!(tail.lines().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
