//! The candidate rules for the base a node's diff starts from
//!
//! Each finds a base from the node's checkout, its work record and the other nodes' captures.
//! The diff then runs from that base to the worktree as it is now. The matrix tries every rule
//! on every scenario, so the rule reef keeps is the one that held up.

use std::collections::BTreeSet;

use crate::git::Git;
use crate::store::{State, Work};

/// A rule for the base a node's diff starts from
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum Rule {
    /// R1: where the head left the base branch, `merge-base(head, base_branch)`
    MergeBase,
    /// R2: the worktree as it was when the node started
    Start,
    /// R3: the newest of the merge-base, the start commit and other nodes' captured heads
    /// that the head descends from
    Captured,
    /// R4: the parent of the oldest commit whose `Reef-Node` trailer names the node
    Trailers,
    /// R5: R3, also finding other nodes' captured heads that a rebase rewrote, by patch-id
    PatchId,
    /// R6: the start, carried forward over whatever the base branch brought in since
    Carried,
}

impl Rule {
    /// Every rule, in the matrix's order
    pub const ALL: [Rule; 6] = [
        Rule::MergeBase,
        Rule::Start,
        Rule::Captured,
        Rule::Trailers,
        Rule::PatchId,
        Rule::Carried,
    ];

    /// The rule reef keeps
    pub const CHOSEN: Rule = Rule::Carried;

    /// The rule's name, as the CLI and the matrix spell it
    pub fn name(self) -> &'static str {
        match self {
            Rule::MergeBase => "merge-base",
            Rule::Start => "start",
            Rule::Captured => "captured",
            Rule::Trailers => "trailers",
            Rule::PatchId => "patch-id",
            Rule::Carried => "carried",
        }
    }
}

/// The base a rule found
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// The tree or commit the diff starts from
    pub base: String,
    /// How the rule came to it, for the matrix and the capture's record
    pub how: String,
    /// The files that conflicted carrying the start forward: their diff may hold how an
    /// earlier node's change was resolved, not only this node's
    pub conflicts: Vec<String>,
}

impl Base {
    /// A base found plainly
    ///
    /// # Arguments
    ///
    /// * `base` - The tree or commit
    /// * `how` - How it was found
    fn plain(base: String, how: impl Into<String>) -> Self {
        Base {
            base,
            how: how.into(),
            conflicts: Vec::new(),
        }
    }
}

/// What every rule needs to find a base
pub struct Inputs<'a> {
    /// git in the node's worktree
    pub git: &'a Git,
    /// The node's work record
    pub work: &'a Work,
    /// Everything captured so far, every node's
    pub state: &'a State,
    /// The commit HEAD is at
    pub head: &'a str,
}

/// The empty tree's id, the base of a diff that starts before the first commit
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Whether a trailer's value names a node: `project/plan/id`, or the id alone
///
/// # Arguments
///
/// * `value` - The trailer's value
/// * `node` - The node's id
fn names(value: &str, node: &str) -> bool {
    value == node || value.ends_with(&format!("/{node}"))
}

/// The merge-base of the head and the base branch, if there is a base branch
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
fn merge_base(inputs: &Inputs) -> anyhow::Result<Option<String>> {
    match &inputs.work.base_branch {
        Some(branch) => inputs.git.merge_base(inputs.head, branch),
        None => Ok(None),
    }
}

/// Picks the candidate every other candidate is an ancestor of, among those the head descends from
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
/// * `candidates` - Each candidate commit with where it came from
fn newest(inputs: &Inputs, candidates: Vec<(String, String)>) -> anyhow::Result<Base> {
    // keep the candidates that still exist and that the head descends from
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (commit, from) in candidates {
        if !seen.insert(commit.clone()) {
            continue;
        }
        if inputs.git.exists(&commit)? && inputs.git.is_ancestor(&commit, inputs.head)? {
            kept.push((commit, from));
        }
    }
    // the newest is the one that descends from all the others
    for (commit, from) in &kept {
        let mut descends = true;
        for (other, _) in &kept {
            if other != commit && !inputs.git.is_ancestor(other, commit)? {
                descends = false;
                break;
            }
        }
        if descends {
            return Ok(Base::plain(commit.clone(), from.clone()));
        }
    }
    // none at all, or two the history has no order for, as a merge of two of them leaves
    let names: Vec<&str> = kept.iter().map(|(_, from)| from.as_str()).collect();
    anyhow::bail!("no newest base among [{}]", names.join(", "))
}

/// The candidates R3 starts from: the merge-base, the start, and other nodes' captured heads
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
fn captured_candidates(inputs: &Inputs) -> anyhow::Result<Vec<(String, String)>> {
    let mut candidates = Vec::new();
    // where the head left the base branch
    if let Some(base) = merge_base(inputs)? {
        candidates.push((base, "merge-base".to_owned()));
    }
    // what the node started from
    candidates.push((inputs.work.start.clone(), "start".to_owned()));
    // every other node's latest captured head
    for (node, captures) in &inputs.state.captures {
        if node == &inputs.work.node {
            continue;
        }
        if let Some(head) = captures.last().and_then(|capture| capture.head.clone()) {
            candidates.push((head, format!("{node}'s head")));
        }
    }
    Ok(candidates)
}

/// R4: the parent of the oldest commit since the merge-base whose trailer names the node
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
fn trailers(inputs: &Inputs) -> anyhow::Result<Base> {
    // the commits on the head's own line, back to where it left the base branch
    let base = merge_base(inputs)?;
    let commits = inputs.git.first_parents(base.as_deref(), inputs.head)?;
    let found = inputs.git.trailers(&commits)?;
    // newest first, so the last one that names the node is the oldest
    let oldest = commits.iter().rev().find(|commit| {
        found
            .get(commit.as_str())
            .is_some_and(|nodes| nodes.iter().any(|value| names(value, &inputs.work.node)))
    });
    match oldest {
        // its parent, or the empty tree for a root commit
        Some(commit) => match inputs.git.commit(&format!("{commit}^"))? {
            Some(parent) => Ok(Base::plain(parent, "before its oldest trailer")),
            None => Ok(Base::plain(
                EMPTY_TREE.to_owned(),
                "a root commit's trailer",
            )),
        },
        // nothing committed yet, so only the uncommitted changes are the node's
        None => Ok(Base::plain(
            inputs.head.to_owned(),
            "no trailer, so the head",
        )),
    }
}

/// R5: R3's candidates, plus commits whose patch-id matches another node's captured head
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
fn patch_id(inputs: &Inputs) -> anyhow::Result<Base> {
    let mut candidates = captured_candidates(inputs)?;
    // the patch-ids other nodes' heads had when they were captured
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (node, captures) in &inputs.state.captures {
        if node == &inputs.work.node {
            continue;
        }
        if let Some(id) = captures
            .last()
            .and_then(|capture| capture.head_patch_id.clone())
        {
            wanted.push((id, node.clone()));
        }
    }
    // a rebase rewrote those heads if a commit since the merge-base has the same patch
    if !wanted.is_empty() {
        let base = merge_base(inputs)?;
        let commits = inputs.git.first_parents(base.as_deref(), inputs.head)?;
        let ids = inputs.git.patch_ids(&commits)?;
        for (commit, id) in ids {
            for (want, node) in &wanted {
                if &id == want {
                    candidates.push((commit.clone(), format!("{node}'s head, rewritten")));
                }
            }
        }
    }
    newest(inputs, candidates)
}

/// R6: the start's tree, with whatever the base branch brought in since merged into it
///
/// Nothing came in when the head still leaves the base branch where the start did, so the
/// base is the start's tree. Otherwise the start is carried forward as a rebase would carry
/// it: a three-way merge of the start's tree and the new merge-base's, from the fork's.
///
/// # Arguments
///
/// * `inputs` - What the rule has to go on
fn carried(inputs: &Inputs) -> anyhow::Result<Base> {
    let work = inputs.work;
    // with no base branch, nothing comes in from anyone else
    let (Some(_), Some(fork)) = (&work.base_branch, &work.fork) else {
        return Ok(Base::plain(
            work.start_tree.clone(),
            "the start, no base branch",
        ));
    };
    let Some(now) = merge_base(inputs)? else {
        return Ok(Base::plain(
            work.start_tree.clone(),
            "the start, no merge-base",
        ));
    };
    // the head still leaves the base branch where the start did
    if &now == fork {
        return Ok(Base::plain(work.start_tree.clone(), "the start"));
    }
    // carry the start over what came in between the fork and the new merge-base
    let merged = inputs.git.merge_trees(
        &inputs.git.tree(fork)?,
        &work.start_tree,
        &inputs.git.tree(&now)?,
    )?;
    Ok(Base {
        base: merged.tree,
        how: if merged.conflicts.is_empty() {
            "the start carried forward".to_owned()
        } else {
            format!(
                "the start carried forward, {} conflicted",
                merged.conflicts.join(", ")
            )
        },
        conflicts: merged.conflicts,
    })
}

/// Finds the base a node's diff starts from, by one rule
///
/// # Arguments
///
/// * `rule` - The rule to follow
/// * `inputs` - What it has to go on
pub fn base(rule: Rule, inputs: &Inputs) -> anyhow::Result<Base> {
    match rule {
        Rule::MergeBase => match merge_base(inputs)? {
            Some(base) => Ok(Base::plain(base, "merge-base")),
            None => anyhow::bail!("no base branch to take a merge-base with"),
        },
        Rule::Start => Ok(Base::plain(inputs.work.start_tree.clone(), "the start")),
        Rule::Captured => newest(inputs, captured_candidates(inputs)?),
        Rule::Trailers => trailers(inputs),
        Rule::PatchId => patch_id(inputs),
        Rule::Carried => carried(inputs),
    }
}
