//! A stand-in for shoal: nodes' statuses, their work records and their captures, as JSON
//!
//! It lives in a state dir outside every checkout, so nothing a capture keeps is in git. One
//! lock file guards it, since two worktrees' hooks can capture at the same moment.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::instrument;

use crate::diff::Diff;
use crate::git::Git;

/// Where a node's work stands, with the same words as reef's
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    clap::ValueEnum,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Not started
    #[default]
    Todo,
    /// Started
    InProgress,
    /// Finished
    Done,
    /// Stuck
    Blocked,
}

/// A node as the stub plan holds it
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Node {
    /// Where its work stands
    pub status: Status,
    /// When it was last marked done, in ms since the epoch
    pub done_at: Option<u64>,
    /// The done mark its latest sealed revision covers, so Stop seals each done mark once
    #[serde(default)]
    pub sealed_for: Option<u64>,
}

/// C30's work record: where a node's work happens and what it started from
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    /// The node
    pub node: String,
    /// The root of the worktree its work happens in
    pub worktree: PathBuf,
    /// The branch checked out there when it started, if HEAD wasn't detached
    pub branch: Option<String>,
    /// The local ref whose new commits are someone else's work, if any: `main`, or a
    /// tracking ref such as `origin/main` when the node works on `main` itself
    pub base_branch: Option<String>,
    /// The commit HEAD was at when it started
    pub start: String,
    /// The worktree as it was when it started, uncommitted changes included
    pub start_tree: String,
    /// Where the start left the base branch, `merge-base(start, base_branch)`
    pub fork: Option<String>,
    /// The ref that keeps the start's tree and commit from being pruned
    pub pin: String,
    /// The nodes it needs
    pub needs: Vec<String>,
    /// When it started, in ms since the epoch
    pub started_at: u64,
}

/// One revision of a node's changes, as extracted from its checkout
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    /// The node
    pub node: String,
    /// Its revision, from 1
    pub revision: u32,
    /// What started it: a hook's event, or `cli`
    pub trigger: String,
    /// The rule that found its base
    pub rule: String,
    /// When it was taken, in ms since the epoch
    pub at: u64,
    /// The worktree it was taken in
    pub worktree: PathBuf,
    /// The commit HEAD was at, if any: provenance only, it may stop resolving
    pub head: Option<String>,
    /// The patch-id of HEAD's own change, which a rebase keeps
    pub head_patch_id: Option<String>,
    /// The worktree as one tree, uncommitted changes included
    pub tree: String,
    /// The tree or commit the diff starts from
    pub base: String,
    /// How the rule came to that base
    pub how: String,
    /// The files that conflicted carrying the start forward, whose diff may hold how an
    /// earlier node's change was resolved
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// Whether this revision is sealed; the latest, while unsealed, is the node's draft
    #[serde(default)]
    pub sealed: bool,
    /// What sealed it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_by: Option<String>,
    /// The changes themselves
    pub diff: Diff,
}

/// Everything the stub keeps
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// The stub plan's nodes, by id
    pub nodes: BTreeMap<String, Node>,
    /// Each started node's work record, by id
    pub work: BTreeMap<String, Work>,
    /// Each node's captures, oldest first, by id
    pub captures: BTreeMap<String, Vec<Capture>>,
}

impl State {
    /// A node's latest capture, if it has one
    ///
    /// # Arguments
    ///
    /// * `node` - The node
    pub fn latest(&self, node: &str) -> Option<&Capture> {
        self.captures.get(node).and_then(|captures| captures.last())
    }

    /// The node a worktree is working on: the one that started there last
    ///
    /// A commit made after a node is done but before the next one starts still belongs to it,
    /// so the last to start is the one, whatever its status.
    ///
    /// # Arguments
    ///
    /// * `worktree` - The worktree's root
    pub fn current(&self, worktree: &Path) -> Option<&Work> {
        self.work
            .values()
            .filter(|work| work.worktree == worktree)
            .max_by_key(|work| work.started_at)
    }
}

/// The milliseconds since the epoch
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// The state dir: its state, its event log and its lock
#[derive(Debug, Clone)]
pub struct Store {
    /// The dir every process shares
    dir: PathBuf,
}

impl Store {
    /// Opens the state dir, creating it if needed
    ///
    /// # Arguments
    ///
    /// * `dir` - The dir every process shares
    pub fn new<P: Into<PathBuf>>(dir: P) -> anyhow::Result<Self> {
        let dir = dir.into();
        // make sure the dir exists before anything is written to it
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Store { dir })
    }

    /// Opens a state dir whose git ignores the machine's config, as the tests' repos do
    ///
    /// A marker file says so, so the hooks git starts in other processes do the same.
    ///
    /// # Arguments
    ///
    /// * `dir` - The dir every process shares
    pub fn isolated<P: Into<PathBuf>>(dir: P) -> anyhow::Result<Self> {
        let store = Store::new(dir)?;
        // the marker is all a later process needs to see
        std::fs::write(store.path("isolated"), b"")?;
        Ok(store)
    }

    /// The state dir
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// git in a checkout, isolated from the machine's config if this state dir is
    ///
    /// # Arguments
    ///
    /// * `dir` - The checkout
    pub fn git<P: Into<PathBuf>>(&self, dir: P) -> Git {
        if self.path("isolated").exists() {
            Git::isolated(dir)
        } else {
            Git::new(dir)
        }
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
    fn lock(&self) -> anyhow::Result<File> {
        // the lock is a file of its own, so the state can be swapped by rename under it
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path("state.lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    /// Reads the state file, empty if there is none yet
    fn read(&self) -> anyhow::Result<State> {
        let path = self.path("state.json");
        // a new state dir starts empty
        if !path.exists() {
            return Ok(State::default());
        }
        let raw = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(serde_json::from_slice(&raw)?)
    }

    /// Writes the state file through a temporary file and a rename
    ///
    /// # Arguments
    ///
    /// * `state` - The state to write
    fn write(&self, state: &State) -> anyhow::Result<()> {
        // write beside the real file so the rename stays on one filesystem
        let tmp = self.path("state.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, self.path("state.json"))?;
        Ok(())
    }

    /// Reads the state under the lock
    #[instrument(name = "Store::load", skip_all, err(Debug))]
    pub fn load(&self) -> anyhow::Result<State> {
        // take the lock so a half-swapped state is never read
        let _lock = self.lock()?;
        self.read()
    }

    /// Changes the state under the lock and writes it back
    ///
    /// # Arguments
    ///
    /// * `change` - The change to make, whose result is returned
    #[instrument(name = "Store::update", skip_all, err(Debug))]
    pub fn update<T, F: FnOnce(&mut State) -> T>(&self, change: F) -> anyhow::Result<T> {
        // hold the lock across the read, the change and the write
        let _lock = self.lock()?;
        let mut state = self.read()?;
        let out = change(&mut state);
        self.write(&state)?;
        Ok(out)
    }

    /// Sets a node's status, stamping when it was marked done
    ///
    /// # Arguments
    ///
    /// * `node` - The node
    /// * `status` - Its new status
    pub fn set_status(&self, node: &str, status: Status) -> anyhow::Result<()> {
        self.update(|state| {
            let entry = state.nodes.entry(node.to_owned()).or_default();
            entry.status = status;
            // a done mark is what the Stop hook compares captures against
            if status == Status::Done {
                entry.done_at = Some(now_ms());
            }
        })
    }

    /// Appends an event to the event log, stamped with the time
    ///
    /// # Arguments
    ///
    /// * `kind` - What happened: a hook's event, `capture` or `tool`
    /// * `body` - What it carried
    #[instrument(name = "Store::log", skip(self, body), err(Debug))]
    pub fn log(&self, kind: &str, body: Value) -> anyhow::Result<()> {
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

    /// Reads the event log back
    pub fn events(&self) -> anyhow::Result<Vec<Value>> {
        let path = self.path("events.jsonl");
        // no log yet is no events
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&path)?;
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| Ok(serde_json::from_str(line)?))
            .collect()
    }
}
