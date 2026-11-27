//! reef's command for capturing a node's changes from its own checkout, sealed as a revision,
//! and the capture git's and Claude Code's hooks take to keep a node's draft current
//!
//! ```text
//! reef node capture [ID]   # the node this worktree is on, or the one named, worked here
//! ```
//!
//! - **The diff runs from the start, carried forward, to the worktree.** The start is the tree
//!   `reef start` wrote. When the base branch has brought something in since, by a rebase or a
//!   merge, the start is first carried over it with a three-way merge of trees, from where the
//!   start left the base branch to where HEAD leaves it now, as a rebase would carry it.
//! - **A seal takes what is tracked.** The worktree is written as a tree with every tracked file
//!   as it is, staged new files included, and untracked files left out unless the base already
//!   holds them, so a `__pycache__` no ignore file names stays out of the node's changes.
//! - **gix for every step but the snapshot,** which stays git's CLI on a copy of the index, so
//!   the user's own staging is left as it was.
//! - **Each file a grammar reads is outlined** ([`outline`](crate::outline)) from both of its
//!   sides once the diff is read, so the items its change touched go with it.
//! - **Only the local checkout is read.** Nothing is fetched, pushed or read from a remote, and
//!   the service is sent what was read, since it reads no checkout.
//! - **A capture the node's latest revision already holds** adds no revision. Its SHAs are still
//!   added to the node's record, naming that revision.
//! - **A draft or a seal.** `reef node capture` seals what it takes, over the node's draft if its
//!   latest revision is one, and the hooks ([`hook`](crate::hook)) keep the draft current. A
//!   node done whose latest revision is a draft, or that has none, is sealed by the Stop hook,
//!   and by the start of another node in its worktree ([`seal_if_done`]).
//! - **A capture reads its node's `Reef-Resolves` trailers.** Each commit HEAD reaches past the
//!   node's start and the base branch is read for them, and each thread one names is sent to be
//!   resolved, linked to the revision the capture keeps or matches. A value that names no thread
//!   number is said, and sent nowhere.

use reef_api::comment::trailer_threads;
use reef_api::{
    AddCapture, CaptureShas, Captured, ErrorCode, FileDiff, RESOLVES_TRAILER, Resolves,
    ResolvingCommit, Revision, TrailerOutcome, TrailerRead, WorkRecord, WorkStart,
};
use reef_model::{PlanName, Status};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::cli::{self, PlanPick};
use crate::client::{Client, is_code};
use crate::git::{Checkout, Walked};
use crate::outline;
use crate::start::{checkout_in, where_started};

/// Each trailer value that names no thread number: its commit and the value
pub type Unreadable = Vec<(String, String)>;

/// What a capture does with what it took
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// Keeps the node's draft current, as git's hooks do while it is worked on
    Draft,
    /// Seals it, as `reef node capture` and the Stop hook do
    Seal,
}

/// How much a capture prints of what came of it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// The revision's SHAs and every file, as `reef node capture` prints them
    Full,
    /// One line for a revision written, and nothing for one unchanged, as a hook prints after
    /// each commit
    Line,
}

/// A capture taken from a checkout but not yet sent
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// What it read: the head, the worktree's tree, the merge-base and the base
    pub shas: CaptureShas,
    /// Whether the start was carried over what the base branch brought in since
    pub carried: bool,
    /// The files that conflicted carrying the start forward
    pub conflicts: Vec<String>,
    /// Every changed file from the base to the worktree's tree
    pub files: Vec<FileDiff>,
    /// Each thread a `Reef-Resolves` trailer names, on the node's commits, oldest first
    pub resolves: Vec<Resolves>,
    /// The node's own commits, oldest first, none of them a merge
    pub commits: Vec<ResolvingCommit>,
    /// Each trailer value that names no thread number: its commit and the value
    pub unreadable: Unreadable,
}

/// The tree a node's diff starts from, and how it was found
struct Carry {
    /// The start's tree, carried forward if anything came in
    tree: String,
    /// Whether it was carried
    carried: bool,
    /// The files that conflicted carrying it
    conflicts: Vec<String>,
}

/// Captures a node's changes in the working directory's checkout and sends them as a revision
///
/// # Arguments
///
/// * `client` - The service's client
/// * `cwd` - The working directory, in the node's worktree
/// * `id` - The node's id, or `None` for the node this worktree is on
/// * `pick` - The project and plan named, if they were
/// * `out` - Where to print what came of it
pub async fn capture<W: Write>(
    client: &Client,
    cwd: &Path,
    id: Option<&str>,
    pick: &PlanPick,
    out: &mut W,
) -> anyhow::Result<()> {
    // the checkout, which must be part of the project's repo
    let (checkout, project) = checkout_in(client, cwd, &pick.project, "capture").await?;
    let record = match id {
        // a node named is in the plan picked, and must be worked here
        Some(id) => {
            let plan = cli::plan_in(client, cwd, project, pick.plan.as_deref()).await?;
            let node = plan.node(id)?;
            let record = client.work_of(&node).await?.ok_or_else(|| {
                anyhow::anyhow!("`{id}` has not been started: `reef start {id}` starts it here")
            })?;
            if record.start.worktree != checkout.root() {
                anyhow::bail!(
                    "`{id}` is worked in {}, not here: capture it from there",
                    record.start.worktree.display()
                );
            }
            record
        }
        // else the one started last in this worktree, in the plan its branch picks, else in any
        None => cli::worktree_record(client, &project, checkout.root())
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no node of `{}` has been started in {}: `reef start <ID>` starts one",
                    project.name,
                    checkout.root().display()
                )
            })?,
    };
    // what the node changed, read here, then sealed by the service
    capture_record(client, &checkout, &record, Keep::Seal, Report::Full, out).await?;
    Ok(())
}

/// Captures the changes of a node worked in a checkout, as a draft or a seal, and prints what
/// came of it
///
/// # Arguments
///
/// * `client` - The service's client
/// * `checkout` - The checkout the node's work happens in, which its record names
/// * `record` - The node's work record
/// * `keep` - Whether the capture keeps the node's draft current or seals it
/// * `report` - How much to print
/// * `out` - Where to print it
pub async fn capture_record<W: Write>(
    client: &Client,
    checkout: &Checkout,
    record: &WorkRecord,
    keep: Keep,
    report: Report,
    out: &mut W,
) -> anyhow::Result<Captured> {
    // what the node changed, read here, then kept by the service
    let taken = take(checkout, &record.start)?;
    let add = AddCapture {
        pinned: record.start.pinned.clone(),
        shas: taken.shas.clone(),
        conflicts: taken.conflicts.clone(),
        files: taken.files,
        resolves: taken.resolves.clone(),
        commits: taken.commits.clone(),
        draft: keep == Keep::Draft,
    };
    let id = &record.node.id;
    // each commit the trailers were read from, by id, for its summary
    let summaries: HashMap<&str, &str> = taken
        .resolves
        .iter()
        .map(|resolves| {
            (
                resolves.commit.commit.as_str(),
                resolves.commit.summary.as_str(),
            )
        })
        .collect();
    let read = Read {
        plan: &record.node.plan,
        summaries: &summaries,
        unreadable: &taken.unreadable,
    };
    let captured = client.add_capture(&record.node, &add).await?;
    match &captured {
        Captured::Added {
            revision, trailers, ..
        }
        | Captured::Replaced {
            revision, trailers, ..
        } => {
            // what was written, said as what became of the node's draft
            let number = revision.revision;
            let what = match (&captured, revision.draft) {
                (Captured::Added { .. }, true) => format!("drafted `{id}` as revision {number}"),
                (Captured::Added { .. }, false) => format!("captured `{id}` as revision {number}"),
                (_, true) => format!("updated `{id}`'s draft, revision {number}"),
                (_, false) => format!("sealed `{id}` as revision {number}"),
            };
            match report {
                Report::Full => {
                    writeln!(out, "{what} in {}", record.start.worktree.display())?;
                    show(revision, taken.carried, out)?;
                }
                Report::Line => {
                    let (added, removed) = revision.counts();
                    let files = revision.files.len();
                    let noun = if files == 1 { "file" } else { "files" };
                    writeln!(out, "{what}, {files} {noun} +{added} -{removed}")?;
                }
            }
            read.show(trailers, out)?;
        }
        Captured::Unchanged {
            revision, trailers, ..
        } => {
            // a hook says nothing of a capture that wrote no revision, but what its trailers did
            if report == Report::Full {
                let draft = if revision.draft { ", a draft" } else { "" };
                writeln!(
                    out,
                    "`{id}` is unchanged since revision {}{draft}",
                    revision.revision
                )?;
                show(revision, taken.carried, out)?;
            }
            read.show(trailers, out)?;
        }
        Captured::NotStarted { .. } => {
            anyhow::bail!("`{id}`'s record went meanwhile: nothing was captured")
        }
        Captured::Restarted { record } => anyhow::bail!(
            "`{id}` was started again meanwhile, {}: nothing was captured, so capture it again",
            where_started(record)
        ),
    }
    Ok(captured)
}

/// Seals a node's changes once it is done: its draft, or its first revision if it has none
///
/// A node not done, or whose latest revision is sealed already, or that its plan no longer
/// has, is left as it is. Answers whether it sealed one.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `checkout` - The checkout the node's work happens in, which its record names
/// * `record` - The node's work record
/// * `report` - How much to print
/// * `out` - Where to print it
pub async fn seal_if_done<W: Write>(
    client: &Client,
    checkout: &Checkout,
    record: &WorkRecord,
    report: Report,
    out: &mut W,
) -> anyhow::Result<bool> {
    // the node as its plan has it now, which may have dropped it since it started
    let node = match client.node(&record.node.plan, &record.node.id).await {
        Ok(node) => node,
        Err(error) if is_code(&error, ErrorCode::NotFound) => return Ok(false),
        Err(error) => return Err(error),
    };
    if node.spec.status != Some(Status::Done) {
        return Ok(false);
    }
    // a latest revision sealed already has nothing left to seal
    let revisions = client.revisions(&record.node).await?;
    if revisions.last().is_some_and(|latest| !latest.draft) {
        return Ok(false);
    }
    capture_record(client, checkout, record, Keep::Seal, report, out).await?;
    Ok(true)
}

/// Takes a capture of a node's changes in its checkout: its base, its worktree as a seal takes
/// it, and the diff between them
///
/// # Arguments
///
/// * `checkout` - The checkout the node's work happens in
/// * `start` - What the node's start read there
pub fn take(checkout: &Checkout, start: &WorkStart) -> anyhow::Result<Taken> {
    // the start's tree, which only its pin keeps once a rebase has moved past it
    if !checkout.has_object(&start.start_tree)? {
        anyhow::bail!(
            "the start's tree {} is gone from {}, which happens once its pin {} is deleted and git \
             prunes: `reef start --restart` starts the node again from here",
            start.start_tree,
            checkout.root().display(),
            start.pin
        );
    }
    // the commit HEAD is at
    let head = checkout
        .head()?
        .ok_or_else(|| anyhow::anyhow!("{} has no commit to capture", checkout.root().display()))?;
    // where HEAD meets the base branch now, if there is one to meet
    let merge_base = match &start.base_branch {
        Some(branch) => match checkout.commit(branch)? {
            Some(tip) => checkout.merge_base(&head, &tip)?,
            None => None,
        },
        None => None,
    };
    // the start, carried over whatever the base branch brought in since
    let carry = carry(checkout, start, merge_base.as_deref())?;
    // the worktree as a seal takes it, and the diff from the base to it
    let tree = checkout.seal_snapshot(&carry.tree)?;
    let mut files = checkout.diff(&carry.tree, &tree)?;
    // each file a grammar reads outlined from both its sides, while the blobs are at hand
    outline::outline_files(checkout, &mut files)?;
    // the node's own commits, past its start and not the base branch's, and the threads they
    // name
    let own = own_commits(checkout, &head, &start.start, merge_base.as_deref())?;
    let (resolves, unreadable) = resolving(&own);
    let commits = own
        .into_iter()
        .filter(|walked| !walked.merges)
        .map(|walked| walked.commit)
        .collect();
    Ok(Taken {
        shas: CaptureShas {
            head,
            tree,
            merge_base,
            base: carry.tree,
        },
        carried: carry.carried,
        conflicts: carry.conflicts,
        files,
        resolves,
        commits,
        unreadable,
    })
}

/// The node's own commits on a head, oldest first: those past its start that its base branch
/// doesn't hold
///
/// A rebase that rewrote the start itself, as squashing a round's fix into an earlier commit
/// does, leaves the start out of the head's history, so every commit it rewrote would read as
/// the node's. Those are left out by the change each was: a head's commit that is the same change
/// as one the start held is the start's, not the node's.
///
/// # Arguments
///
/// * `checkout` - The checkout the node's work happens in
/// * `head` - The commit HEAD is at
/// * `start` - The commit HEAD was at when the node started
/// * `merge_base` - Where HEAD meets the base branch, if there is one to meet
fn own_commits(
    checkout: &Checkout,
    head: &str,
    start: &str,
    merge_base: Option<&str>,
) -> anyhow::Result<Vec<Walked>> {
    // past the start, and not the base branch's
    let mut hidden = vec![start];
    hidden.extend(merge_base);
    let walked = checkout.walk(head, &hidden)?;
    // a start still in the head's history hides everything before the node
    if checkout.is_ancestor(start, head)? {
        return Ok(walked);
    }
    // otherwise what the start held since it and the head parted is rewritten into the head
    let Some(parted) = checkout.merge_base(head, start)? else {
        return Ok(walked);
    };
    let mut before = vec![parted.as_str()];
    before.extend(merge_base);
    let held = checkout.walk(start, &before)?;
    // so each commit the start held, rewritten, is left out
    let own = walked
        .into_iter()
        .filter(|walked| {
            !held
                .iter()
                .any(|was| was.commit.same_change(&walked.commit))
        })
        .collect();
    Ok(own)
}

/// Reads the threads the `Reef-Resolves` trailers on a node's own commits name, oldest first,
/// and each value that names no thread number
///
/// # Arguments
///
/// * `own` - The node's own commits, oldest first
fn resolving(own: &[Walked]) -> (Vec<Resolves>, Unreadable) {
    let mut resolves = Vec::new();
    let mut unreadable = Vec::new();
    for walked in own {
        // each thread once a commit, however often its trailers name it
        let mut named = Vec::new();
        for value in walked.values(RESOLVES_TRAILER) {
            match trailer_threads(&value) {
                Some(threads) => named.extend(threads),
                None => unreadable.push((walked.commit.commit.clone(), value.clone())),
            }
        }
        let mut seen = Vec::with_capacity(named.len());
        for thread in named {
            if seen.contains(&thread) {
                continue;
            }
            seen.push(thread);
            resolves.push(Resolves {
                thread,
                commit: walked.commit.clone(),
            });
        }
    }
    (resolves, unreadable)
}

/// What a capture's trailers named, for saying what came of each
struct Read<'a> {
    /// The plan the threads are in
    plan: &'a PlanName,
    /// Each commit's summary, by its id
    summaries: &'a HashMap<&'a str, &'a str>,
    /// Each trailer value that names no thread number: its commit and the value
    unreadable: &'a [(String, String)],
}

impl Read<'_> {
    /// Prints what each thread a trailer named came to, and each value that named none
    ///
    /// A trailer whose change resolved its thread already, which is still resolved, says
    /// nothing, since every capture reads it again.
    ///
    /// # Arguments
    ///
    /// * `trailers` - What the service did with each thread named
    /// * `out` - Where to print it
    fn show<W: Write>(&self, trailers: &[TrailerRead], out: &mut W) -> anyhow::Result<()> {
        for read in trailers {
            // a commit is named by its short id and its summary
            let short = &read.commit[..read.commit.len().min(12)];
            let summary = self
                .summaries
                .get(read.commit.as_str())
                .copied()
                .unwrap_or_default();
            let thread = read.thread;
            match read.outcome {
                TrailerOutcome::Resolved => {
                    writeln!(out, "resolved #{thread}, answered by {short} {summary}")?
                }
                TrailerOutcome::Linked => writeln!(
                    out,
                    "linked {short} {summary} to #{thread}, which was resolved already"
                )?,
                TrailerOutcome::Reopened => writeln!(
                    out,
                    "left #{thread} open: {short} resolved it before it was reopened, so a new \
                     commit naming it resolves it again"
                )?,
                TrailerOutcome::NoThread => writeln!(
                    out,
                    "{RESOLVES_TRAILER}: {thread} in {short} names no thread of `{}`",
                    self.plan
                )?,
                TrailerOutcome::Pending => writeln!(
                    out,
                    "left #{thread} open: it is pending in a review not yet submitted, and a \
                     commit naming it resolves it once the review is"
                )?,
                TrailerOutcome::Seen => {}
            }
        }
        // then each value that named no number
        for (commit, value) in self.unreadable {
            let short = &commit[..commit.len().min(12)];
            writeln!(
                out,
                "{RESOLVES_TRAILER}: {value} in {short} names no thread: give it the thread's \
                 number, such as `{RESOLVES_TRAILER}: 12`"
            )?;
        }
        Ok(())
    }
}

/// Carries a node's start over what its base branch brought in since, as a rebase would
///
/// Nothing came in when there is no base branch to meet, or when HEAD still leaves it where the
/// start did, so the base is the start's tree. Otherwise it is a three-way merge of the start's
/// tree and the new merge-base's, from the fork's.
///
/// # Arguments
///
/// * `checkout` - The checkout the node's work happens in
/// * `start` - What the node's start read there
/// * `now` - Where HEAD leaves the base branch now, if they meet
fn carry(checkout: &Checkout, start: &WorkStart, now: Option<&str>) -> anyhow::Result<Carry> {
    // the start as it was
    let as_started = || Carry {
        tree: start.start_tree.clone(),
        carried: false,
        conflicts: Vec::new(),
    };
    // with no fork or no merge-base, nothing came in from anyone else
    let (Some(fork), Some(now)) = (start.fork.as_deref(), now) else {
        return Ok(as_started());
    };
    // and HEAD may still leave the base branch where the start did
    if fork == now {
        return Ok(as_started());
    }
    // else the start is carried over what came in between the fork and the new merge-base
    let fork_tree = checkout.tree_of(fork)?;
    let now_tree = checkout.tree_of(now)?;
    let merged = checkout.merge_trees(&fork_tree, &start.start_tree, &now_tree)?;
    Ok(Carry {
        tree: merged.tree,
        carried: true,
        conflicts: merged.conflicts,
    })
}

/// Prints a revision: its files and what they changed, its SHAs and its conflicts
///
/// # Arguments
///
/// * `revision` - The revision
/// * `carried` - Whether this capture carried the start forward to find its base
/// * `out` - Where to print it
fn show<W: Write>(revision: &Revision, carried: bool, out: &mut W) -> anyhow::Result<()> {
    // what isn't there says so
    let or_none = |value: &Option<String>| value.clone().unwrap_or_else(|| "none".to_string());
    let (added, removed) = revision.counts();
    let how = if carried {
        "the start carried forward"
    } else {
        "the start"
    };
    let mut rows = vec![
        (
            "files",
            format!("{}, +{added} -{removed}", revision.files.len()),
        ),
        ("base", format!("{}, {how}", revision.shas.base)),
        ("tree", revision.shas.tree.clone()),
        ("head", revision.shas.head.clone()),
        ("merge-base", or_none(&revision.shas.merge_base)),
    ];
    if !revision.conflicts.is_empty() {
        rows.push(("conflicts", revision.conflicts.join(", ")));
    }
    let rows = rows
        .into_iter()
        .map(|(key, value)| (key.to_string(), value));
    cli::columns(out, rows)?;
    // then each file, with what happened to it and its lines
    let files = revision.files.iter().map(|file| {
        // a rename names where it came from
        let path = match (&file.old_path, &file.new_path) {
            (Some(old), Some(new)) if old != new => format!("{old} -> {new}"),
            _ => file.path().to_string(),
        };
        let lines = if file.binary {
            "binary".to_string()
        } else if file.too_large {
            "too large to keep".to_string()
        } else {
            format!("+{} -{}", file.added, file.removed)
        };
        (
            format!("  {}", status(file.status)),
            format!("{path}  {lines}"),
        )
    });
    cli::columns(out, files)
}

/// A file's status as a word
///
/// # Arguments
///
/// * `status` - What happened to the file
fn status(status: reef_api::FileStatus) -> &'static str {
    match status {
        reef_api::FileStatus::Added => "added",
        reef_api::FileStatus::Deleted => "deleted",
        reef_api::FileStatus::Modified => "modified",
        reef_api::FileStatus::Renamed => "renamed",
    }
}

#[cfg(test)]
mod tests {
    use super::show;
    use reef_api::{CaptureShas, FileStatus, FileSummary, Revision};

    /// A revision prints its counts, its SHAs, its conflicts and each file, a rename's source too
    #[test]
    fn revisions_print_their_files() {
        // one renamed file and one binary one, with a conflict carrying the start
        let file = |status, old: Option<&str>, new: Option<&str>, binary| FileSummary {
            status,
            old_path: old.map(str::to_string),
            new_path: new.map(str::to_string),
            old_mode: None,
            new_mode: None,
            old_id: None,
            new_id: None,
            binary,
            too_large: false,
            outline: None,
            added: 2,
            removed: 1,
        };
        let revision = Revision {
            node: "reef/central/C32".parse().unwrap(),
            revision: 2,
            at: 1,
            shas: CaptureShas {
                head: "a".repeat(40),
                tree: "b".repeat(40),
                merge_base: None,
                base: "c".repeat(40),
            },
            conflicts: vec!["old.rs".to_string()],
            files: vec![
                file(FileStatus::Renamed, Some("old.rs"), Some("new.rs"), false),
                file(FileStatus::Added, None, Some("logo.png"), true),
            ],
            commits: Vec::new(),
            draft: false,
            author: reef_api::Author::default(),
        };
        let mut out = Vec::new();
        show(&revision, true, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // the revision's own fields first
        assert!(out.contains("files       2, +4 -2\n"), "{out}");
        assert!(out.contains("the start carried forward"), "{out}");
        assert!(out.contains("merge-base  none\n"), "{out}");
        assert!(out.contains("conflicts   old.rs\n"), "{out}");
        // then each file
        assert!(
            out.contains("  renamed  old.rs -> new.rs  +2 -1\n"),
            "{out}"
        );
        assert!(out.contains("  added    logo.png  binary\n"), "{out}");
    }
}
