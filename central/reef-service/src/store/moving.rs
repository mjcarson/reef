//! Moving nodes from one plan of a project to another, everything of theirs with them
//!
//! - **A move is judged on both plans**, as every write is, under the project's queue, work,
//!   comment and decision locks and both plans' write locks, taken in one order. It is refused
//!   if it adds a problem to either plan, such as a need left across the two, so a whole
//!   subtree of needs moves at once or not at all. A group named moves with its members.
//! - **A node keeps its id.** One the other plan has already is refused, unless a killed move
//!   left it there as this one would write it, and one a loop holds is refused until let go.
//! - **Everything of a node's moves with it**: its work record, its captured revisions and their
//!   files, its decisions and its queue entry, under its new name, and its comments, numbered
//!   anew after the other plan's last. Its history is copied under its new name, and both plans'
//!   keep that it moved.
//! - **A move killed partway is finished by running it again.** Each step can be run twice: the
//!   rows of a node's own are copied where none is stored, and each copied comment names the one
//!   it came from. The nodes join the other plan before what they leave behind is deleted, and
//!   leave their own plan last.

use reef_api::{Author, Edited, NodeWrite, PageWrite, records};
use reef_model::{Diagnostic, NodeName, PlanName};
use std::collections::HashSet;
use std::future::Future;
use tracing::instrument;

use super::{
    Proposal, Store, StoredNode, Unkeepable, bundles, history, in_written_order, listed_order,
    settle,
};
use crate::backend::{Backend, Expect, RowKey};
use crate::rank;
use crate::schema::{DecisionRow, EditRow, QueueRow, RevisionFileRow, RevisionRow, WorkRow};

/// How many of a history's edits are read at a time while it is copied
const HISTORY_PAGE: usize = 500;

/// The two plans a move judges: the one its nodes leave and the one they join, each as it stands
/// and as the move would leave it
#[derive(Debug, Clone)]
pub struct MoveProposal {
    /// The plan the nodes leave
    pub from: Proposal,
    /// The plan they join
    pub to: Proposal,
}

/// What came of a move the store judged
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveSet {
    /// The nodes moved, everything of theirs with them
    Applied {
        /// Every node moved, by id, in the order the plan they left listed them
        nodes: Vec<String>,
        /// The nodes whose group stayed behind, which joined the group named, or none
        regrouped: Vec<String>,
        /// Each of their threads' numbers in the plan they left and the plan they joined
        threads: Vec<(u32, u32)>,
        /// The revision the plan they left is at now
        from_revision: u64,
        /// The revision the plan they joined is at now
        to_revision: u64,
        /// Whether any of them was queued, so both plans' queues changed
        queued: bool,
    },
    /// Some nodes named aren't in the plan they leave
    NotInPlan(Vec<String>),
    /// The plan they join has nodes of those ids already
    Taken(Vec<String>),
    /// A loop holds a node of the move
    Held {
        /// The node held
        node: NodeName,
        /// The pane whose loop holds it, as a person would name it
        pane: String,
    },
    /// The move would add problems to either plan
    Refused(Vec<Diagnostic>),
}

/// The ids a move takes: each named, and every member of each group among them, in the order
/// the plan lists them
///
/// # Arguments
///
/// * `nodes` - The plan's nodes, in its order
/// * `ids` - The ids named
fn members_of(nodes: &[StoredNode], ids: &[String]) -> Vec<String> {
    // the named ids, then each node whose group is among them, until none joins
    let mut taken: HashSet<String> = ids.iter().cloned().collect();
    loop {
        let joined: Vec<String> = nodes
            .iter()
            .filter(|node| !taken.contains(&node.spec.id))
            .filter(|node| {
                node.spec
                    .group
                    .as_ref()
                    .is_some_and(|group| taken.contains(group))
            })
            .map(|node| node.spec.id.clone())
            .collect();
        if joined.is_empty() {
            break;
        }
        taken.extend(joined);
    }
    nodes
        .iter()
        .filter(|node| taken.contains(&node.spec.id))
        .map(|node| node.spec.id.clone())
        .collect()
}

/// A node's name within its project, `plan/id`, as its rows of its own are keyed
///
/// # Arguments
///
/// * `plan` - The node's plan
/// * `id` - The node's id
fn node_key(plan: &PlanName, id: &str) -> String {
    format!("{}/{id}", plan.plan)
}

/// A plan's nodes as writes, in the order a check reads them
///
/// # Arguments
///
/// * `nodes` - The nodes, each with the version it would be written at
fn writes(nodes: &[(NodeWrite, u64)]) -> Vec<NodeWrite> {
    let mut written: Vec<NodeWrite> = nodes.iter().map(|(node, _)| node.clone()).collect();
    in_written_order(&mut written);
    written
}

impl<B: Backend> Store<B> {
    /// Moves nodes of a plan to another plan of its project, with everything of theirs, unless
    /// that would add a problem to either plan
    ///
    /// The check is handed both plans as they stand and as the move would leave them, under
    /// both plans' write locks, so no write to either lands between the check and the move.
    ///
    /// # Arguments
    ///
    /// * `from` - The plan the nodes leave
    /// * `to` - The plan they join
    /// * `ids` - The nodes to move, by id: a group named moves with its members
    /// * `group` - The group of `to` a node joins whose own group stays behind, or `None`
    /// * `author` - Who moves them, from the caller's token
    /// * `check` - Finds the problems the move would add to either plan
    #[instrument(name = "Store::move_nodes", skip_all, err(Debug))]
    pub async fn move_nodes<F, Fut>(
        &self,
        from: &PlanName,
        to: &PlanName,
        ids: &[String],
        group: Option<&str>,
        author: &Author,
        check: F,
    ) -> anyhow::Result<MoveSet>
    where
        F: FnOnce(MoveProposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // between two plans of one project
        if from.project != to.project {
            let message = format!("`{to}` is another project's plan: nodes move within a project");
            return Err(Unkeepable(message).into());
        }
        if from == to {
            let message = format!("the nodes are in `{to}` already");
            return Err(Unkeepable(message).into());
        }
        let project = from.project.as_str();
        // the project's locks, then both plans', each in one order, so no other write lands
        let queue_lock = self.queue_lock(project);
        let _queue = queue_lock.lock().await;
        let work_lock = self.work_lock(project);
        let _work = work_lock.lock().await;
        let comment_lock = self.comment_lock(project);
        let _comments = comment_lock.lock().await;
        let decision_lock = self.decision_lock(project);
        let _decisions = decision_lock.lock().await;
        let (first, second) = match from.plan < to.plan {
            true => (from, to),
            false => (to, from),
        };
        let _first = self.writing(first).await;
        let _second = self.writing(second).await;
        // both plans as they stand, after pruning what a killed write left
        let (from_row, from_recovered) = self.recover(from, self.plan_row(from).await?).await?;
        let (to_row, to_recovered) = self.recover(to, self.plan_row(to).await?).await?;
        let mut here = self.revision(from, from_row.current, None).await?;
        listed_order(&mut here);
        let mut there = self.revision(to, to_row.current, None).await?;
        listed_order(&mut there);
        // every node named must be in the plan it leaves
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !here.iter().any(|node| node.spec.id == **id))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Ok(MoveSet::NotInPlan(missing));
        }
        // the nodes it takes, a group's members with it, each as the other plan takes it: after
        // its own nodes, a node whose group stays behind joining the one named
        let moving = members_of(&here, ids);
        let taking: HashSet<&str> = moving.iter().map(String::as_str).collect();
        let base = there.iter().map(|node| node.order + 1).max().unwrap_or(0);
        let mut regrouped = Vec::new();
        let mut moved: Vec<(NodeWrite, u64)> = Vec::with_capacity(moving.len());
        for (i, node) in here
            .iter()
            .filter(|node| taking.contains(node.spec.id.as_str()))
            .enumerate()
        {
            let mut write = NodeWrite::from(node.clone());
            if write
                .spec
                .group
                .as_ref()
                .is_some_and(|kept| !taking.contains(kept.as_str()))
            {
                write.spec.group = group.map(str::to_string);
                regrouped.push(write.spec.id.clone());
            }
            write.order = base + i as u64;
            moved.push((write, node.version + 1));
        }
        // none may take an id the other plan has, unless a killed move put it there: as this one
        // would write it, its history copied there already
        let mut already: HashSet<String> = HashSet::new();
        let mut taken = Vec::new();
        for (write, _) in &moved {
            let Some(kept) = there.iter().find(|node| node.spec.id == write.spec.id) else {
                continue;
            };
            let same = kept.spec == write.spec && kept.body == write.body;
            match same && self.half_moved(from, to, &write.spec.id).await? {
                true => {
                    already.insert(write.spec.id.clone());
                }
                false => taken.push(write.spec.id.clone()),
            }
        }
        if !taken.is_empty() {
            return Ok(MoveSet::Taken(taken));
        }
        // and none may be held by a loop
        for feeding in self.loops().await? {
            if let Some(node) = feeding
                .group()
                .find(|node| node.plan == *from && taking.contains(node.id.as_str()))
            {
                return Ok(MoveSet::Held {
                    node: node.clone(),
                    pane: feeding.pane.describe(),
                });
            }
        }
        // both plans as the move would leave them
        let staying: Vec<(NodeWrite, u64)> = here
            .iter()
            .filter(|node| !taking.contains(node.spec.id.as_str()))
            .map(|node| (NodeWrite::from(node.clone()), node.version))
            .collect();
        let mut joined: Vec<(NodeWrite, u64)> = there
            .iter()
            .map(|node| (NodeWrite::from(node.clone()), node.version))
            .collect();
        joined.extend(
            moved
                .iter()
                .filter(|(write, _)| !already.contains(&write.spec.id))
                .cloned(),
        );
        let from_pages: Vec<PageWrite> = self
            .page_revision(from, from_row.pages_current, None)
            .await?
            .into_iter()
            .map(PageWrite::from)
            .collect();
        let to_pages: Vec<PageWrite> = self
            .page_revision(to, to_row.pages_current, None)
            .await?
            .into_iter()
            .map(PageWrite::from)
            .collect();
        let proposal = MoveProposal {
            from: Proposal {
                before: records(from, here.iter().cloned().map(NodeWrite::from).collect())?,
                after: records(from, writes(&staying))?,
                pages: from_pages,
                pages_after: None,
                branch: self.plan_branch(from).await?,
                docs: self.plan_docs(from).await?,
            },
            to: Proposal {
                before: records(to, there.iter().cloned().map(NodeWrite::from).collect())?,
                after: records(to, writes(&joined))?,
                pages: to_pages,
                pages_after: None,
                branch: self.plan_branch(to).await?,
                docs: self.plan_docs(to).await?,
            },
        };
        // judged on both, and nothing moved if either gains a problem
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(MoveSet::Refused(problems));
        }
        // 1. everything of each node's copied to its new name, its comments moved
        let mut threads = Vec::new();
        let mut left = Vec::new();
        for id in &moving {
            left.extend(self.copy_rows(from, to, id).await?);
            self.copy_history(from, to, id).await?;
            threads.extend(self.move_comments_held(from, to, id).await?);
        }
        // 2. the nodes written to the plan they join
        let to_revision = match joined.len() == there.len() {
            true => to_row.current,
            false => {
                self.rewrite(to, to_row, to_recovered, joined, None)
                    .await?
                    .revision
            }
        };
        // 3. what they leave behind deleted, then 4. the nodes taken out of their own plan
        self.backend.delete_rows(left).await?;
        let from_revision = self
            .rewrite(from, from_row, from_recovered, staying, None)
            .await?
            .revision;
        // 5. their queue entries moved, at their places
        let queued = self.move_queue_held(from, to, &taking).await?;
        // 6. and each plan's history keeps that they moved
        let relisted = Edited::Relisted {
            from: from.plan.clone(),
            to: to.plan.clone(),
        };
        let mut pending = Vec::with_capacity(moved.len() * 2);
        for (write, version) in &moved {
            let id = write.spec.id.as_str();
            pending.push(history::Pending {
                subject: history::node_subject(&from.node(id)?),
                version: None,
                edits: vec![relisted.clone()],
                went_done: false,
            });
            pending.push(history::Pending {
                subject: history::node_subject(&to.node(id)?),
                version: Some(*version),
                edits: vec![relisted.clone()],
                went_done: false,
            });
        }
        self.keep_edits(project, author, false, pending).await;
        Ok(MoveSet::Applied {
            nodes: moving,
            regrouped,
            threads,
            from_revision,
            to_revision,
            queued,
        })
    }

    /// Whether a node of the same id in both plans is one a killed move left half moved: its
    /// newest edit is the same in both histories, since a move copies the history before the
    /// node, and a node added to the other plan alone keeps an edit of its own
    ///
    /// # Arguments
    ///
    /// * `from` - The plan it leaves
    /// * `to` - The plan it joins
    /// * `id` - The node's id
    #[instrument(name = "Store::half_moved", skip_all, err(Debug))]
    async fn half_moved(&self, from: &PlanName, to: &PlanName, id: &str) -> anyhow::Result<bool> {
        let project = from.project.as_str();
        // the newest edit of each, by its key, which a copy keeps
        let here = history::node_subject(&from.node(id)?);
        let there = history::node_subject(&to.node(id)?);
        let newest_here = self.backend.edits(project, &here, None, 1).await?;
        let newest_there = self.backend.edits(project, &there, None, 1).await?;
        Ok(match (newest_here.first(), newest_there.first()) {
            (Some(here), Some(there)) => here.key == there.key,
            _ => false,
        })
    }

    /// Copies a node's rows of its own to its new name where none is stored yet: its work
    /// record, its revisions and their files, and its decisions, and answers the keys of those
    /// it leaves behind
    ///
    /// # Arguments
    ///
    /// * `from` - The plan it leaves
    /// * `to` - The plan it joins
    /// * `id` - The node's id
    #[instrument(name = "Store::copy_rows", skip_all, err(Debug))]
    async fn copy_rows(
        &self,
        from: &PlanName,
        to: &PlanName,
        id: &str,
    ) -> anyhow::Result<Vec<RowKey>> {
        let project = from.project.as_str();
        let (old, new) = (node_key(from, id), node_key(to, id));
        let mut left = Vec::new();
        // its work record, where none is stored under its new name
        for row in self.backend.work(project, Some(&old)).await? {
            if row.node != old {
                continue;
            }
            let copy = WorkRow {
                node: new.clone(),
                ..row
            };
            self.backend.write_work(copy, Expect::Absent).await?;
            left.push(RowKey::Work {
                project: project.to_string(),
                node: old.clone(),
            });
        }
        // each revision, its files first so a revision that reads has every file it lists
        for row in self.backend.revisions(project, &old, None).await? {
            let number: u64 = row.revision.parse().map_err(|_| {
                anyhow::anyhow!(
                    "revision row `{}` of `{old}` isn't named by its number",
                    row.revision
                )
            })?;
            let files = self
                .backend
                .revision_files(project, &old, number, None)
                .await?;
            for file in &files {
                left.push(RowKey::RevisionFile {
                    project: project.to_string(),
                    node: old.clone(),
                    revision: number,
                    path: file.path.clone(),
                });
            }
            let copies: Vec<RevisionFileRow> = files
                .into_iter()
                .map(|file| RevisionFileRow {
                    node: new.clone(),
                    ..file
                })
                .collect();
            for bundle in bundles(copies, |file| file.path.len() + file.file.len()) {
                self.backend.write_revision_files(bundle).await?;
            }
            left.push(RowKey::Revision {
                project: project.to_string(),
                node: old.clone(),
                revision: row.revision.clone(),
            });
            let copy = RevisionRow {
                node: new.clone(),
                ..row
            };
            self.backend.write_revision(copy, Expect::Absent).await?;
        }
        // and each decision
        for row in self.backend.decisions(project, &old).await? {
            left.push(RowKey::Decision {
                project: project.to_string(),
                node: old.clone(),
                number: row.number.clone(),
            });
            let copy = DecisionRow {
                node: new.clone(),
                ..row
            };
            self.backend.write_decision(copy, Expect::Absent).await?;
        }
        Ok(left)
    }

    /// Copies a node's history under its new name, each edit under its own key, so a copy run
    /// again writes each over itself
    ///
    /// # Arguments
    ///
    /// * `from` - The plan it leaves
    /// * `to` - The plan it joins
    /// * `id` - The node's id
    #[instrument(name = "Store::copy_history", skip_all, err(Debug))]
    async fn copy_history(&self, from: &PlanName, to: &PlanName, id: &str) -> anyhow::Result<()> {
        let project = from.project.as_str();
        let (old, new) = (
            history::node_subject(&from.node(id)?),
            history::node_subject(&to.node(id)?),
        );
        // a page at a time, newest first, until a page comes back short
        let mut older: Option<String> = None;
        loop {
            let page = self
                .backend
                .edits(project, &old, older.as_deref(), HISTORY_PAGE)
                .await?;
            let ended = page.len() < HISTORY_PAGE;
            older = page.last().map(|row| row.key.clone());
            let copies: Vec<EditRow> = page
                .into_iter()
                .map(|row| EditRow {
                    subject: new.clone(),
                    ..row
                })
                .collect();
            for bundle in bundles(copies, |row| row.subject.len() + row.record.len()) {
                // a plan's activity stays the plan's, so a copy carries none
                self.backend.write_edits(bundle, Vec::new()).await?;
            }
            if ended || older.is_none() {
                return Ok(());
            }
        }
    }

    /// Moves the queue entries of nodes leaving a plan to the plan they join, each to a rank
    /// just after its own, so it keeps its place, and answers whether any moved
    ///
    /// The caller holds the project's queue lock.
    ///
    /// # Arguments
    ///
    /// * `from` - The plan they leave
    /// * `to` - The plan they join
    /// * `ids` - The ids of the nodes moving
    #[instrument(name = "Store::move_queue_held", skip_all, err(Debug))]
    async fn move_queue_held(
        &self,
        from: &PlanName,
        to: &PlanName,
        ids: &HashSet<&str>,
    ) -> anyhow::Result<bool> {
        // the queue as it stands, after pruning what a killed write left
        let read = self.queue_rows(&from.project).await?;
        let mut seq = read.iter().map(|row| row.seq).max().unwrap_or(0);
        let (rows, leftovers) = settle(read);
        self.prune_queue(leftovers).await?;
        let mut moved = false;
        for (i, row) in rows.iter().enumerate() {
            if row.plan != from.plan || !ids.contains(row.id.as_str()) {
                continue;
            }
            // just after its own row, before the next, so it keeps its place in the project
            let next = rows.get(i + 1).map(|next| next.rank.as_str());
            let rank = rank::between(Some(&row.rank), next)?;
            seq += 1;
            let entry = QueueRow {
                project: row.project.clone(),
                rank,
                plan: to.plan.clone(),
                id: row.id.clone(),
                seq,
            };
            self.move_row(Some(row), entry).await?;
            moved = true;
        }
        Ok(moved)
    }
}
