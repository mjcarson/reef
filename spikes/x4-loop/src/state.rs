//! The stub plan and the loop's state, shared by the MCP server, the hooks and the CLI
//!
//! Every process that touches the loop opens the same state dir. The state lives in
//! `state.json`, and every hook event, tool call and injector step is appended to
//! `events.jsonl`, which `report` reads back.

use anyhow::{Context, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::instrument;

/// The prompt that starts a node, typed by the user or injected by reef
pub const KICKOFF: &str =
    "Take the next reef node: call the x4 next_task tool and follow what it says.";

/// Where a stub node is in its life
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Nobody has claimed this node yet
    Todo,
    /// The loop has claimed this node and Claude is planning or building it
    InProgress,
    /// Claude finished this node
    Done,
}

impl Status {
    /// The name this status is written with
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in-progress",
            Status::Done => "done",
        }
    }
}

/// How the loop moves from a finished node to the next
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The Stop hook lets Claude stop and types `/clear` into the session's own pane
    Inject,
    /// The Stop hook lets Claude stop and tells the user to type `/clear`
    Manual,
    /// The Stop hook blocks, and the next plan's approval clears the context
    Approval,
}

/// One node of the stub plan
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    /// The node's id
    pub id: String,
    /// The node's title
    pub title: String,
    /// The problem this node solves
    pub why: String,
    /// What is true once this node is done
    pub what: String,
    /// How this node is known to be done
    pub acceptance: Vec<String>,
    /// Extra steps the loop asks for while this node is planned
    #[serde(default)]
    pub probe: Option<String>,
    /// Where this node is in its life
    pub status: Status,
    /// Notes Claude left on this node through the `note` tool
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Node {
    /// Creates a new todo node
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `title` - The node's title
    /// * `why` - The problem this node solves
    /// * `what` - What is true once this node is done
    pub fn new(id: &str, title: &str, why: &str, what: &str) -> Self {
        Node {
            id: id.to_owned(),
            title: title.to_owned(),
            why: why.to_owned(),
            what: what.to_owned(),
            acceptance: vec!["`cargo test` passes, with a test for the new function".to_owned()],
            probe: None,
            status: Status::Todo,
            notes: Vec::new(),
        }
    }
}

/// What next_task found when asked for work
#[derive(Debug, PartialEq)]
pub enum Claim {
    /// The loop already holds this unfinished node
    Holding(Node),
    /// The loop just claimed this node and marked it in progress
    Claimed(Node),
    /// The loop is paused, so nothing new is handed out
    Paused,
    /// Every node is done
    Finished,
}

impl PartialEq for Node {
    /// Nodes are the same node when their ids match
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// The stub plan and where the loop is in it
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
    /// The stub nodes in plan order
    pub nodes: Vec<Node>,
    /// The node the loop holds, once it has claimed one
    pub current: Option<String>,
    /// Whether the loop stops after the current node
    pub paused: bool,
    /// How the loop moves from a finished node to the next
    pub mode: Mode,
    /// The finished node whose end the Stop hook already acted on
    pub handled_for: Option<String>,
}

impl State {
    /// Creates the stub plan: three tiny functions for a `textkit` crate
    ///
    /// # Arguments
    ///
    /// * `mode` - How the loop moves from a finished node to the next
    pub fn stub(mode: Mode) -> Self {
        // the first node also probes which tools plan mode lets through
        let mut n1 = Node::new(
            "N1",
            "N1 slugify",
            "textkit has no way to turn a title into a URL slug.",
            "`textkit::slugify(&str) -> String` lowercases a title and joins its words with `-`. \
             Whether non-ASCII letters are kept or dropped is the user's call.",
        );
        n1.probe = Some(
            "While you are in plan mode, call each of these x4 tools once: get_node (id N1), next, \
             check, and note (id N1, a one-line planning note). Say in your plan which of them \
             worked and which were refused."
                .to_owned(),
        );
        let n2 = Node::new(
            "N2",
            "N2 word_count",
            "textkit can't count the words in a string.",
            "`textkit::word_count(&str) -> usize` counts words split on whitespace. Whether a \
             hyphenated word counts once or as several is the user's call.",
        );
        let n3 = Node::new(
            "N3",
            "N3 title_case",
            "textkit can't title-case a heading.",
            "`textkit::title_case(&str) -> String` capitalises each word. Whether small words \
             such as \"of\" and \"the\" stay lowercase is the user's call.",
        );
        State {
            nodes: vec![n1, n2, n3],
            current: None,
            paused: false,
            mode,
            handled_for: None,
        }
    }

    /// Gets a node by its id
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Gets a node by its id to change it
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// Gets the node the loop holds, if it holds one
    pub fn current_node(&self) -> Option<&Node> {
        self.current.as_deref().and_then(|id| self.node(id))
    }

    /// Gets the first node nobody has claimed yet
    pub fn pending(&self) -> Option<&Node> {
        self.nodes.iter().find(|node| node.status == Status::Todo)
    }

    /// Hands out the node Claude should work on now, claiming a new one if needed
    ///
    /// The loop holds one node at a time, so a second call before that node is done returns
    /// the same node rather than claiming another.
    pub fn claim_next(&mut self) -> Claim {
        // an unfinished node the loop already holds comes back as it is
        if let Some(node) = self.current_node()
            && node.status != Status::Done
        {
            return Claim::Holding(node.clone());
        }
        // a paused loop hands out nothing new
        if self.paused {
            return Claim::Paused;
        }
        // claim the first todo node in plan order
        let Some(id) = self.pending().map(|node| node.id.clone()) else {
            return Claim::Finished;
        };
        self.current = Some(id.clone());
        let node = self.node_mut(&id).expect("pending node exists");
        node.status = Status::InProgress;
        Claim::Claimed(node.clone())
    }
}

/// The current time in milliseconds since the Unix epoch
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// The loop's state dir: its state, its event log and its lock
#[derive(Clone, Debug)]
pub struct Store {
    /// The dir every process of the loop shares
    dir: PathBuf,
}

impl Store {
    /// Opens the state dir, creating it if needed
    ///
    /// # Arguments
    ///
    /// * `dir` - The dir every process of the loop shares
    pub fn new<P: Into<PathBuf>>(dir: P) -> Result<Self, Error> {
        let dir = dir.into();
        // make sure the dir exists before anything is written to it
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Store { dir })
    }

    /// The state dir
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of a file in the state dir
    ///
    /// # Arguments
    ///
    /// * `name` - The file's name
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Takes the state dir's lock, held until the returned file is dropped
    fn lock(&self) -> Result<File, Error> {
        // the lock is a file of its own, so the state can be swapped by rename under it
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path("state.lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    /// Reads the state without the lock, for a look that changes nothing
    #[instrument(name = "Store::load", skip_all, err(Debug))]
    pub fn load(&self) -> Result<State, Error> {
        // take the lock so a half-swapped state is never read
        let _lock = self.lock()?;
        self.read()
    }

    /// Reads the state file
    fn read(&self) -> Result<State, Error> {
        let path = self.path("state.json");
        let raw = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(serde_json::from_slice(&raw)?)
    }

    /// Writes the state file through a temporary file and a rename
    ///
    /// # Arguments
    ///
    /// * `state` - The state to write
    fn write(&self, state: &State) -> Result<(), Error> {
        // write beside the real file so the rename stays on one filesystem
        let tmp = self.path("state.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, self.path("state.json"))?;
        Ok(())
    }

    /// Replaces the state with a new one
    ///
    /// # Arguments
    ///
    /// * `state` - The state to start from
    #[instrument(name = "Store::reset", skip_all, err(Debug))]
    pub fn reset(&self, state: &State) -> Result<(), Error> {
        // take the lock so no hook reads a half-written state
        let _lock = self.lock()?;
        self.write(state)?;
        // start a new event log, keeping the old one beside it
        let events = self.path("events.jsonl");
        if events.exists() {
            let kept = self.path(&format!("events-{}.jsonl", now_ms()));
            std::fs::rename(&events, kept)?;
        }
        // the context series starts again too
        let context = self.path("context.jsonl");
        if context.exists() {
            let kept = self.path(&format!("context-{}.jsonl", now_ms()));
            std::fs::rename(&context, kept)?;
        }
        Ok(())
    }

    /// Changes the state under the lock and writes it back
    ///
    /// # Arguments
    ///
    /// * `change` - The change to make, whose result is returned
    #[instrument(name = "Store::update", skip_all, err(Debug))]
    pub fn update<T, F: FnOnce(&mut State) -> T>(&self, change: F) -> Result<T, Error> {
        // hold the lock across the read, the change and the write
        let _lock = self.lock()?;
        let mut state = self.read()?;
        let out = change(&mut state);
        self.write(&state)?;
        Ok(out)
    }

    /// Appends an event to the event log, stamped with the time
    ///
    /// # Arguments
    ///
    /// * `kind` - What happened: a hook's event name, `tool`, `decision` or `inject`
    /// * `body` - What it carried
    #[instrument(name = "Store::log", skip(self, body), err(Debug))]
    pub fn log(&self, kind: &str, body: Value) -> Result<(), Error> {
        // one line per event, written in one call under the lock
        let line = serde_json::json!({ "at": now_ms(), "kind": kind, "body": body });
        let _lock = self.lock()?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path("events.jsonl"))?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Reads every event logged so far
    pub fn events(&self) -> Result<Vec<Value>, Error> {
        read_jsonl(&self.path("events.jsonl"))
    }
}

/// Reads a file of one JSON value per line, skipping lines that don't parse
///
/// # Arguments
///
/// * `path` - The file to read
pub fn read_jsonl(path: &Path) -> Result<Vec<Value>, Error> {
    // a file that was never written holds no lines
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second next_task before the node is done hands back the same node
    #[test]
    fn claim_holds_one_node_at_a_time() {
        let mut state = State::stub(Mode::Inject);
        // the first call claims N1
        assert!(matches!(state.claim_next(), Claim::Claimed(node) if node.id == "N1"));
        // a second call before N1 is done returns N1 again
        assert!(matches!(state.claim_next(), Claim::Holding(node) if node.id == "N1"));
        // once N1 is done, N2 is claimed
        state.node_mut("N1").unwrap().status = Status::Done;
        assert!(matches!(state.claim_next(), Claim::Claimed(node) if node.id == "N2"));
    }

    /// A paused loop hands out nothing new, but still hands back the node it holds
    #[test]
    fn paused_loop_claims_nothing_new() {
        let mut state = State::stub(Mode::Inject);
        state.claim_next();
        state.paused = true;
        // the held node still comes back
        assert!(matches!(state.claim_next(), Claim::Holding(node) if node.id == "N1"));
        // once it is done, nothing new is handed out
        state.node_mut("N1").unwrap().status = Status::Done;
        assert_eq!(state.claim_next(), Claim::Paused);
    }
}
