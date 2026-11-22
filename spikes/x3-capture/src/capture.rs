//! Starting a node and capturing its changes, from its own local checkout
//!
//! `start` is what `reef start` would do: record where the node's work happens and what it
//! started from, and pin that start under `refs/reef/` so no gc prunes it. `capture` is what
//! `reef node capture` would do: snapshot the worktree, find the base by a rule, diff the two,
//! and store the result. Both read only local git data; neither needs a remote or a push.

use serde_json::json;
use std::path::Path;
use tracing::instrument;

use crate::diff::Diff;
use crate::git::Git;
use crate::rule::{self, Inputs, Rule};
use crate::store::{Capture, Status, Store, Work, now_ms};

/// Which backend runs a capture's steps
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Backend {
    /// git's CLI for every step
    Cli,
    /// gix for every step but the snapshot, which gix can't stage, and git's CLI for that
    Hybrid,
}

impl Backend {
    /// The backend reef keeps: gix wherever it matched the CLI, so only the snapshot needs git
    pub const CHOSEN: Backend = Backend::Hybrid;
}

/// How a capture is taken
#[derive(Debug, Clone)]
pub struct Options {
    /// The rule that finds the base
    pub rule: Rule,
    /// The backend that runs its steps
    pub backend: Backend,
    /// What started the capture, for its record
    pub trigger: String,
    /// Whether this capture seals the node's revision, or only keeps its draft current
    pub seal: bool,
    /// Whether to record HEAD's patch-id through the CLI, which only the rejected patch-id rule
    /// reads, so only the scenarios ask for it
    pub patch_id: bool,
}

/// The triggers that seal a revision: the end of Claude's turn on a done node, and a capture
/// asked for by hand
const SEALING: &[&str] = &["Stop", "cli", "scenario"];

impl Options {
    /// The chosen rule through the chosen backend, started by this trigger
    ///
    /// Stop and the CLI seal a revision; every other trigger, PostToolUse and git's hooks,
    /// keeps the node's draft revision current.
    ///
    /// # Arguments
    ///
    /// * `trigger` - What started the capture
    pub fn chosen(trigger: &str) -> Self {
        Options {
            rule: Rule::CHOSEN,
            backend: Backend::CHOSEN,
            trigger: trigger.to_owned(),
            seal: SEALING.contains(&trigger),
            patch_id: false,
        }
    }
}

/// What a capture did to the node's revisions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// It added a new revision
    Added,
    /// It took the place of the draft revision, which wasn't sealed
    Replaced,
    /// Nothing had changed, and it sealed the draft revision as it was
    Sealed,
    /// Nothing had changed, and there was nothing to seal
    Unchanged,
}

impl Stored {
    /// The word the event log uses
    pub fn name(self) -> &'static str {
        match self {
            Stored::Added => "added",
            Stored::Replaced => "replaced",
            Stored::Sealed => "sealed",
            Stored::Unchanged => "unchanged",
        }
    }
}

/// The ref that pins a node's start
///
/// # Arguments
///
/// * `node` - The node
pub fn pin_ref(node: &str) -> String {
    format!("refs/reef/start/{node}")
}

/// Starts a node in a checkout: records its work and pins what it started from
///
/// # Arguments
///
/// * `store` - The state dir
/// * `git` - git in the checkout the node's work happens in
/// * `node` - The node
/// * `base_branch` - The branch other people's work comes in on, such as `main`
/// * `needs` - The nodes it needs
#[instrument(name = "capture::start", skip(store, git, needs), err(Debug))]
pub fn start(
    store: &Store,
    git: &Git,
    node: &str,
    base_branch: Option<&str>,
    needs: &[String],
) -> anyhow::Result<Work> {
    // where the work happens and what is checked out there
    let worktree = git.toplevel()?;
    let git = git.at(&worktree);
    let branch = git.branch()?;
    let start = git
        .commit("HEAD")?
        .ok_or_else(|| anyhow::anyhow!("{} has no commit to start from", worktree.display()))?;
    // the worktree as it is, so anything left uncommitted is the start's, not this node's
    let start_tree = git.snapshot()?;
    // working on the base branch itself, only its upstream brings anyone else's work in
    let base_branch = match base_branch {
        Some(base) if branch.as_deref() == Some(base) => upstream(&git, base)?,
        Some(base) => Some(base.to_owned()),
        None => None,
    };
    let fork = match &base_branch {
        Some(base) => git.merge_base(&start, base)?,
        None => None,
    };
    // pin the start's tree and commit, so a rebase and a gc can't take them
    let pinned = git.commit_tree(&start_tree, Some(&start), &format!("reef: {node} started"))?;
    let pin = pin_ref(node);
    git.update_ref(&pin, &pinned)?;
    let work = Work {
        node: node.to_owned(),
        worktree,
        branch,
        base_branch,
        start,
        start_tree,
        fork,
        pin,
        needs: needs.to_vec(),
        started_at: now_ms(),
    };
    // record it, and mark the node in progress as next_task would
    let recorded = work.clone();
    store.update(move |state| {
        state.work.insert(recorded.node.clone(), recorded);
        state.nodes.entry(node.to_owned()).or_default().status = Status::InProgress;
    })?;
    store.log("start", json!({ "node": node, "work": work }))?;
    Ok(work)
}

/// The upstream a branch tracks, if it tracks one: a local ref, never fetched here
///
/// # Arguments
///
/// * `git` - git in the checkout
/// * `branch` - The branch
fn upstream(git: &Git, branch: &str) -> anyhow::Result<Option<String>> {
    // `@{upstream}` fails for a branch that tracks nothing
    let spec = format!("{branch}@{{upstream}}");
    if git.succeeds(&["rev-parse", "--verify", "--quiet", &spec])? {
        Ok(Some(git.run(&["rev-parse", "--abbrev-ref", &spec])?))
    } else {
        Ok(None)
    }
}

/// Reads the diff between a base and a tree, through gix if its repo is open, else the CLI
///
/// # Arguments
///
/// * `git` - git in the checkout
/// * `repo` - gix's view of the repo, for the hybrid backend
/// * `base` - The base tree or commit
/// * `tree` - The tree it ends at
pub fn read_diff(
    git: &Git,
    repo: Option<&gix::Repository>,
    base: &str,
    tree: &str,
) -> anyhow::Result<Diff> {
    match repo {
        Some(repo) => crate::gix::diff(repo, base, tree),
        None => git.diff(base, tree),
    }
}

/// A capture worked out but not yet stored
pub struct Taken {
    /// The commit HEAD was at
    pub head: Option<String>,
    /// The worktree as one tree
    pub tree: String,
    /// The base the rule found
    pub base: rule::Base,
    /// The diff from the base to the tree
    pub diff: Diff,
}

/// Works out a node's changes by one rule, storing nothing
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
/// * `rule` - The rule that finds the base
/// * `backend` - The backend that runs its steps
#[instrument(name = "capture::take", skip(store), err(Debug))]
pub fn take(store: &Store, node: &str, rule: Rule, backend: Backend) -> anyhow::Result<Taken> {
    // the node's work record says where its changes are
    let state = store.load()?;
    let work = state.work.get(node).ok_or_else(|| {
        anyhow::anyhow!("{node} hasn't been started, so nothing says where it is")
    })?;
    let git = store.git(&work.worktree);
    take_in(&git, &state, work, rule, backend)
}

/// Works out a node's changes in a checkout, from a state already read
///
/// # Arguments
///
/// * `git` - git in the node's worktree
/// * `state` - Everything captured so far
/// * `work` - The node's work record
/// * `rule` - The rule that finds the base
/// * `backend` - The backend that runs its steps
pub fn take_in(
    git: &Git,
    state: &crate::store::State,
    work: &Work,
    rule: Rule,
    backend: Backend,
) -> anyhow::Result<Taken> {
    // the hybrid opens the repo once in gix, for every step but the snapshot
    let repo = match backend {
        Backend::Hybrid => Some(crate::gix::open(git.dir())?),
        Backend::Cli => None,
    };
    // the head commit
    let head = match &repo {
        Some(repo) => crate::gix::head(repo)?,
        None => git.commit("HEAD")?,
    }
    .ok_or_else(|| anyhow::anyhow!("{} has no commit", work.worktree.display()))?;
    // the worktree as it is, uncommitted changes and all: git's CLI with either backend
    let tree = git.snapshot()?;
    // the base, by the rule
    let base = rule::base(
        rule,
        &Inputs {
            git,
            work,
            state,
            head: &head,
            gix: repo.as_ref(),
        },
    )?;
    let diff = read_diff(git, repo.as_ref(), &base.base, &tree)?;
    Ok(Taken {
        head: Some(head),
        tree,
        base,
        diff,
    })
}

/// Captures a node's changes and stores them as its draft revision, or seals it
///
/// A node has at most one draft, its latest revision: each draft capture takes its place, so
/// git's hooks can capture every commit without making a revision of each. A sealing capture
/// closes the draft as a revision, adding one first if anything changed. Returns the latest
/// revision and what was done to it, or `None` when nothing changed and nothing was sealed.
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
/// * `options` - The rule, the backend and the trigger
#[instrument(name = "capture::capture", skip(store, options), fields(trigger = %options.trigger), err(Debug))]
pub fn capture(store: &Store, node: &str, options: &Options) -> anyhow::Result<Option<Capture>> {
    // work it out, then fit it into the node's revisions
    let taken = take(store, node, options.rule, options.backend)?;
    let head_patch_id = if options.patch_id {
        head_patch_id(store, node, taken.head.as_deref())?
    } else {
        None
    };
    let rule = options.rule.name().to_owned();
    let trigger = options.trigger.clone();
    let seal = options.seal;
    let (stored, latest) = store.update(move |state| {
        let worktree = state
            .work
            .get(node)
            .map(|work| work.worktree.clone())
            .unwrap_or_default();
        // a seal covers the node's latest done mark, so Stop takes each done mark once
        let done_at = state
            .nodes
            .get(node)
            .filter(|entry| entry.status == Status::Done)
            .and_then(|entry| entry.done_at);
        let captures = state.captures.entry(node.to_owned()).or_default();
        let same = captures
            .last()
            .is_some_and(|last| last.base == taken.base.base && last.tree == taken.tree);
        let draft = captures.last().is_some_and(|last| !last.sealed);
        let stored = match (same, draft, seal) {
            // the same as the latest revision: seal it if asked, else leave it
            (true, true, true) => Stored::Sealed,
            (true, _, _) => Stored::Unchanged,
            // changed, over a draft that this takes the place of
            (false, true, _) => Stored::Replaced,
            // changed, after a sealed revision or none
            (false, false, _) => Stored::Added,
        };
        // a draft whose tree a commit now holds names that commit, as the SHAs it was read at
        if same
            && draft
            && let Some(last) = captures.last_mut()
        {
            last.head = taken.head.clone();
            last.head_patch_id = head_patch_id.clone();
        }
        match stored {
            Stored::Sealed => {
                if let Some(last) = captures.last_mut() {
                    last.sealed = true;
                    last.sealed_by = Some(trigger.clone());
                }
            }
            Stored::Unchanged => {}
            Stored::Added | Stored::Replaced => {
                // a replaced draft keeps its revision number
                if stored == Stored::Replaced {
                    captures.pop();
                }
                let capture = Capture {
                    node: node.to_owned(),
                    revision: captures.len() as u32 + 1,
                    trigger: trigger.clone(),
                    rule,
                    at: now_ms(),
                    worktree,
                    head: taken.head,
                    head_patch_id,
                    tree: taken.tree,
                    base: taken.base.base,
                    how: taken.base.how,
                    conflicts: taken.base.conflicts,
                    sealed: seal,
                    sealed_by: seal.then(|| trigger.clone()),
                    diff: taken.diff,
                };
                captures.push(capture);
            }
        }
        let latest = captures.last().cloned();
        // the node's done mark is taken once it is sealed
        if seal && let Some(entry) = state.nodes.get_mut(node) {
            entry.sealed_for = done_at;
        }
        (stored, latest)
    })?;
    // log what was done to the revisions
    let (added, removed) = latest
        .as_ref()
        .map(|capture| capture.diff.counts())
        .unwrap_or_default();
    store.log(
        "capture",
        json!({
            "node": node,
            "trigger": options.trigger,
            "stored": stored.name(),
            "revision": latest.as_ref().map(|capture| capture.revision),
            "sealed": latest.as_ref().map(|capture| capture.sealed),
            "files": latest.as_ref().map(|capture| capture.diff.files.len()),
            "added": added,
            "removed": removed,
            "head": latest.as_ref().and_then(|capture| capture.head.clone()),
            "how": latest.as_ref().map(|capture| capture.how.clone()),
        }),
    )?;
    Ok(match stored {
        Stored::Unchanged => None,
        _ => latest,
    })
}

/// The patch-id of HEAD's own change, unless HEAD is still the node's start
///
/// Only the rejected patch-id rule reads it, and gix has no patch-id, so it is the CLI's and
/// only the scenarios record it.
///
/// # Arguments
///
/// * `store` - The state dir
/// * `node` - The node
/// * `head` - The commit HEAD was at
fn head_patch_id(store: &Store, node: &str, head: Option<&str>) -> anyhow::Result<Option<String>> {
    let state = store.load()?;
    let (Some(work), Some(head)) = (state.work.get(node), head) else {
        return Ok(None);
    };
    // the start's commit isn't the node's change
    if head == work.start {
        return Ok(None);
    }
    let git = store.git(&work.worktree);
    Ok(git.patch_ids(&[head.to_owned()])?.remove(head))
}

/// The root of the worktree a directory is in
///
/// # Arguments
///
/// * `store` - The state dir, which says how git is run
/// * `dir` - Any directory in the worktree
pub fn worktree_of(store: &Store, dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    store.git(dir).toplevel()
}
