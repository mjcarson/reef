//! Each plan's comments: a row a comment in the plan's partition, sorted by its number
//!
//! - **A plan's comments in one partition.** A node's threads and the whole plan's are each read
//!   from the one partition, and grouped into threads as they are read.
//! - **Numbered in the plan, and never reset.** Every comment, a reply included, takes the
//!   number after the last one stored in its plan, so a thread is named by one number wherever
//!   in the plan it is read from, and no restart reuses one.
//! - **A thread's first comment holds the thread.** Its target and its state are kept on its
//!   first comment's row, which a mark writes again over the version it read. A reply is a row
//!   of its own, naming its thread, so a reply and a mark never write the same row.
//! - **A line comment names a line of a stored revision.** A thread on a line is kept only if
//!   the line's revision lists its file, and the file has a line of that number on that side.
//! - **A line's thread follows its line.** Each capture follows the node's threads from the
//!   revision their line was last in to the latest, a revision at a time, by comparing the two
//!   revisions' hunks ([`follow`](super::follow)), and writes each one's row again over the
//!   version read. A thread keeps where its line was in each revision, and the revision that
//!   lost it, which outdates it. A thread started on an older revision is followed at once.
//! - **A change resolves a thread, and stays linked.** A resolve, by a commit's trailer at its
//!   capture or by hand, marks the thread resolved and links a revision to it, kept on its first
//!   row, so the link outlives the commit once it is squashed. A trailer whose change resolved
//!   the thread once already, by its author, when it was authored and its summary, which a
//!   rebase keeps, never resolves it again, so a thread reopened stays open.
//! - **Writes take a lock per project; reads take none.** Each comment is one row's write, so
//!   every step leaves the plan's comments reading whole.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as another plan's comments.

use reef_api::{
    Anchor, COMMENT_BODY_MAX, Comment, Commented, FileDiff, FollowedThread, MarkThread, Marked,
    NewReply, NewThread, Resolution, ResolveThread, Resolved, Resolves, Revision, Target, Thread,
    ThreadState, TrailerOutcome, TrailerRead,
};
use reef_model::{NodeName, PlanName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tracing::instrument;

use super::follow::{follow_line, next_file};
use super::{Store, Unkeepable, work::now_ms};
use crate::backend::{Backend, Expect, Written};
use crate::schema::CommentRow;

/// What a comment row keeps as JSON: everything of a comment but its plan and number
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// The id of the node its thread is on
    node: String,
    /// When it was written, in milliseconds since the epoch
    at: u64,
    /// Its body, as markdown
    body: String,
    /// Whether it starts a thread or replies in one
    on: KeptOn,
}

/// Whether a comment starts a thread, holding the thread's target and state, or replies in one
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum KeptOn {
    /// A thread's first comment
    Thread {
        /// What the thread is on, as it was started
        target: Target,
        /// Where its line was in each revision after its target's, oldest first
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        followed: Vec<Anchor>,
        /// The revision that no longer held its line
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outdated: Option<u32>,
        /// Whether the thread is open or resolved
        state: ThreadState,
        /// Each change linked to it as one that answered it, oldest first
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        resolutions: Vec<Resolution>,
    },
    /// A reply in a thread
    Reply {
        /// The thread's number
        thread: u32,
    },
}

/// One comment row as read: its number, the version its row is at, and what it keeps
type Read = (u32, u64, Kept);

/// The files of a node's revisions read so far, by revision and path, each `None` if its
/// revision doesn't list it
type Files = HashMap<(u32, String), Option<Arc<FileDiff>>>;

/// The threads a follow wrote, and the revision it followed them to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Followed {
    /// The node's latest revision
    pub revision: u32,
    /// Each thread written, by number
    pub threads: Vec<FollowedThread>,
}

impl<B: Backend> Store<B> {
    /// Finds a project's comment lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose comments' lock to find
    fn comment_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.comments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(project.to_string())
            .or_default()
            .clone()
    }

    /// Lists a plan's comment threads, by number, on every node of it
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::threads", skip_all, err(Debug))]
    pub async fn threads(&self, plan: &PlanName) -> anyhow::Result<Vec<Thread>> {
        // every comment of the plan, grouped into its thread
        let rows = self.comment_rows(plan).await?;
        threads_of(plan, rows)
    }

    /// Lists one node's comment threads, by number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::node_threads", skip_all, err(Debug))]
    pub async fn node_threads(&self, node: &NodeName) -> anyhow::Result<Vec<Thread>> {
        // the plan's threads, those on this node kept
        let mut threads = self.threads(&node.plan).await?;
        threads.retain(|thread| thread.node.id == node.id);
        Ok(threads)
    }

    /// Gets one comment thread of a plan, if a thread starts at that number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `number` - The thread's number
    #[instrument(name = "Store::thread", skip_all, err(Debug))]
    pub async fn thread(&self, plan: &PlanName, number: u32) -> anyhow::Result<Option<Thread>> {
        // the plan's threads, the one of this number kept, since its replies are anywhere after it
        let threads = self.threads(plan).await?;
        Ok(threads.into_iter().find(|thread| thread.number == number))
    }

    /// Starts a thread on a node, or on a line of one of its revisions
    ///
    /// The caller has found the node in its plan. A line it names must be a line of a revision
    /// the node has stored, or nothing is written.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `new` - What the thread is on, and its first comment's body
    #[instrument(name = "Store::start_thread", skip_all, err(Debug))]
    pub async fn start_thread(&self, node: &NodeName, new: NewThread) -> anyhow::Result<Commented> {
        // a body with nothing to say, or too much, is never kept
        check_body(&new.body)?;
        // a line comment must name a line the node's revisions hold
        if let Target::Line(anchor) = &new.target
            && !self.names_a_line(node, anchor).await?
        {
            return Ok(Commented::NoLine { target: new.target });
        }
        // numbered after the plan's last comment, under the project's comment lock
        let lock = self.comment_lock(&node.plan.project);
        let _held = lock.lock().await;
        let rows = self.comment_rows(&node.plan).await?;
        let number = next_number(&rows)?;
        // a line of an older revision is followed at once to the latest, read under the lock
        // so a capture's follow after it finds the thread where this left it
        let (followed, outdated) = match &new.target {
            Target::Line(anchor) => {
                let revisions = self.revisions(node).await?;
                let mut files = Files::new();
                self.trail(node, &revisions, anchor.clone(), &mut files)
                    .await?
            }
            Target::Node => (Vec::new(), None),
        };
        let kept = Kept {
            node: node.id.clone(),
            at: now_ms(),
            body: new.body,
            on: KeptOn::Thread {
                target: new.target,
                followed,
                outdated,
                state: ThreadState::Open,
                resolutions: Vec::new(),
            },
        };
        self.keep_comment(&node.plan, number, kept).await?;
        // the thread as it now stands, read back with the rest
        let thread = self.kept_thread(&node.plan, number).await?;
        Ok(Commented::Added {
            thread,
            comment: number,
        })
    }

    /// Replies in a thread of a plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `reply` - The reply's body
    #[instrument(name = "Store::reply", skip_all, err(Debug))]
    pub async fn reply(
        &self,
        plan: &PlanName,
        thread: u32,
        reply: NewReply,
    ) -> anyhow::Result<Commented> {
        // a body with nothing to say, or too much, is never kept
        check_body(&reply.body)?;
        // numbered after the plan's last comment, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let rows = self.comment_rows(plan).await?;
        // the thread must start at the number named, and a reply's number starts none
        let Some(node) = rows.iter().find_map(|(number, _, kept)| {
            (*number == thread && matches!(kept.on, KeptOn::Thread { .. }))
                .then(|| kept.node.clone())
        }) else {
            return Ok(Commented::NoThread { thread });
        };
        let number = next_number(&rows)?;
        let kept = Kept {
            node,
            at: now_ms(),
            body: reply.body,
            on: KeptOn::Reply { thread },
        };
        self.keep_comment(plan, number, kept).await?;
        // the thread as it now stands, the reply last in it
        let thread = self.kept_thread(plan, thread).await?;
        Ok(Commented::Added {
            thread,
            comment: number,
        })
    }

    /// Marks a thread open or resolved, over the version its writer read
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `mark` - The state to mark it, and the version it was marked against
    #[instrument(name = "Store::mark_thread", skip_all, err(Debug))]
    pub async fn mark_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        mark: MarkThread,
    ) -> anyhow::Result<Marked> {
        // the thread's first row, which holds its state, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let Some((version, mut kept)) = self.thread_row(plan, thread).await? else {
            return Ok(Marked::NoThread { thread });
        };
        // a mark made against another version than the one stored would undo what it never saw
        if version != mark.read {
            let thread = self.kept_thread(plan, thread).await?;
            return Ok(Marked::Stale { thread });
        }
        // its state changed, written only over the version read
        if let KeptOn::Thread { state, .. } = &mut kept.on {
            *state = mark.state;
        }
        let row = comment_row(plan, thread, version + 1, &kept)?;
        let written = self
            .backend
            .write_comment(row, Expect::Version(version))
            .await?;
        // a refusal means another process wrote it since the read, so this mark is stale
        let thread = self.kept_thread(plan, thread).await?;
        match written {
            Written::Applied => Ok(Marked::Marked { thread }),
            Written::Refused(_) => Ok(Marked::Stale { thread }),
        }
    }

    /// Resolves a thread by hand, over the version its writer read, linking a revision to it
    ///
    /// The caller has found the revision linked among its node's.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    /// * `resolve` - The revision that answered it, and the version it was resolved against
    #[instrument(name = "Store::resolve_thread", skip_all, err(Debug))]
    pub async fn resolve_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        resolve: ResolveThread,
    ) -> anyhow::Result<Resolved> {
        // the thread's first row, which holds its state, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let Some((version, mut kept)) = self.thread_row(plan, thread).await? else {
            return Ok(Resolved::NoThread { thread });
        };
        // a resolve made against another version than the one stored never saw what changed
        if version != resolve.read {
            let thread = self.kept_thread(plan, thread).await?;
            return Ok(Resolved::Stale { thread });
        }
        // resolved, the revision linked after any before it, written over the version read
        if let KeptOn::Thread {
            state, resolutions, ..
        } = &mut kept.on
        {
            *state = ThreadState::Resolved;
            resolutions.push(Resolution {
                node: resolve.node,
                revision: resolve.revision,
                at: now_ms(),
                commit: None,
            });
        }
        let row = comment_row(plan, thread, version + 1, &kept)?;
        let written = self
            .backend
            .write_comment(row, Expect::Version(version))
            .await?;
        // a refusal means another process wrote it since the read, so this resolve is stale
        let thread = self.kept_thread(plan, thread).await?;
        match written {
            Written::Applied => Ok(Resolved::Resolved { thread }),
            Written::Refused(_) => Ok(Resolved::Stale { thread }),
        }
    }

    /// Resolves the threads a capture's trailers name, linking the revision it kept or matched,
    /// and answers what came of each, in the order they were named
    ///
    /// A thread is named in the plan of the node captured, whichever node it is on. An open one
    /// is resolved, and one resolved already only linked. A trailer whose change, by its author,
    /// when it was authored and its summary, is linked to the thread already writes nothing: a
    /// thread reopened since stays open, so a rebase that rewrites the commit never undoes the
    /// reopen. Each thread written is moved on a version.
    ///
    /// # Arguments
    ///
    /// * `node` - The node captured
    /// * `revision` - The revision the capture kept, or the latest it matched
    /// * `resolves` - Each thread a trailer named, with its commit
    #[instrument(name = "Store::resolve_by_trailers", skip_all, err(Debug))]
    pub async fn resolve_by_trailers(
        &self,
        node: &NodeName,
        revision: u32,
        resolves: Vec<Resolves>,
    ) -> anyhow::Result<Vec<(TrailerRead, Option<Thread>)>> {
        // under the project's comment lock, so no mark lands between a read and its write
        let plan = &node.plan;
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let at = now_ms();
        let mut read = Vec::with_capacity(resolves.len());
        for resolves in resolves {
            // each thread read again, so two commits naming one see each other's write
            let outcome = |outcome| TrailerRead {
                thread: resolves.thread,
                commit: resolves.commit.commit.clone(),
                outcome,
            };
            let Some((version, mut kept)) = self.thread_row(plan, resolves.thread).await? else {
                read.push((outcome(TrailerOutcome::NoThread), None));
                continue;
            };
            let KeptOn::Thread {
                state, resolutions, ..
            } = &mut kept.on
            else {
                read.push((outcome(TrailerOutcome::NoThread), None));
                continue;
            };
            // a change linked already resolves nothing again, whether or not it was reopened
            let seen = resolutions.iter().any(|linked| {
                linked
                    .commit
                    .as_ref()
                    .is_some_and(|commit| commit.same_change(&resolves.commit))
            });
            if seen {
                let reopened = *state == ThreadState::Open;
                let kind = match reopened {
                    true => TrailerOutcome::Reopened,
                    false => TrailerOutcome::Seen,
                };
                read.push((outcome(kind), None));
                continue;
            }
            // else resolved if it was open, and the revision linked either way
            let kind = match *state {
                ThreadState::Open => TrailerOutcome::Resolved,
                ThreadState::Resolved => TrailerOutcome::Linked,
            };
            *state = ThreadState::Resolved;
            resolutions.push(Resolution {
                node: node.clone(),
                revision,
                at,
                commit: Some(resolves.commit.clone()),
            });
            let row = comment_row(plan, resolves.thread, version + 1, &kept)?;
            match self
                .backend
                .write_comment(row, Expect::Version(version))
                .await?
            {
                Written::Applied => {
                    let thread = self.kept_thread(plan, resolves.thread).await?;
                    read.push((outcome(kind), Some(thread)));
                }
                // under the lock, only a writer in another process could have moved it
                Written::Refused(refusal) => anyhow::bail!(
                    "thread {} of `{plan}` wasn't resolved by {}, since another writer wrote it \
                     meanwhile: {}",
                    resolves.thread,
                    resolves.commit.commit,
                    refusal.describe()
                ),
            }
        }
        Ok(read)
    }

    /// Follows the threads on a node's lines to its latest revision, and answers what it wrote
    ///
    /// Each thread whose line was last in an earlier revision is followed a revision at a time,
    /// until it reaches the latest or a revision no longer holds its line, which outdates it,
    /// and its row is written again over the version read. Resolved threads are followed too,
    /// so one reopened is where its line is, but an outdated one is followed no further. A row
    /// another process wrote since the read is left as it is, for the next follow to take on
    /// from where it is.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::follow_threads", skip_all, err(Debug))]
    pub async fn follow_threads(&self, node: &NodeName) -> anyhow::Result<Option<Followed>> {
        // under the project's comment lock, so no mark or new thread lands between a read and
        // its write
        let lock = self.comment_lock(&node.plan.project);
        let _held = lock.lock().await;
        // the node's revisions, read under the lock, so a thread started before it was followed
        // as far as its start saw
        let revisions = self.revisions(node).await?;
        let Some(latest) = revisions.last().map(|revision| revision.revision) else {
            return Ok(None);
        };
        let rows = self.comment_rows(&node.plan).await?;
        let mut files = Files::new();
        let mut written = Vec::new();
        for (number, version, mut kept) in rows {
            // only this node's threads on a line, still on it, and behind its latest revision
            if kept.node != node.id {
                continue;
            }
            let KeptOn::Thread {
                target: Target::Line(anchor),
                followed,
                outdated,
                ..
            } = &mut kept.on
            else {
                continue;
            };
            let from = followed.last().unwrap_or(anchor).clone();
            if outdated.is_some() || from.revision >= latest {
                continue;
            }
            // followed a revision at a time, then written over the version read
            let (moved, lost) = self.trail(node, &revisions, from, &mut files).await?;
            followed.extend(moved);
            *outdated = lost;
            let row = comment_row(&node.plan, number, version + 1, &kept)?;
            match self
                .backend
                .write_comment(row, Expect::Version(version))
                .await?
            {
                Written::Applied => written.push(FollowedThread {
                    thread: number,
                    version: version + 1,
                    outdated: lost.is_some(),
                }),
                // under the lock, only a writer in another process could have moved it
                Written::Refused(_) => {}
            }
        }
        Ok((!written.is_empty()).then_some(Followed {
            revision: latest,
            threads: written,
        }))
    }

    /// Follows a line through each revision after the one it is in, until one no longer holds
    /// it, and answers where it was in each, and the revision that lost it if one did
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revisions` - The node's revisions, oldest first
    /// * `from` - The line, in the revision it is in
    /// * `files` - The files of the node's revisions read so far
    async fn trail(
        &self,
        node: &NodeName,
        revisions: &[Revision],
        from: Anchor,
        files: &mut Files,
    ) -> anyhow::Result<(Vec<Anchor>, Option<u32>)> {
        let start = from.revision;
        let mut at = from;
        let mut moved = Vec::new();
        for next in revisions
            .iter()
            .filter(|revision| revision.revision > start)
        {
            // the file the line is in, as its revision lists it, and the one it became
            let listed = revisions
                .iter()
                .find(|revision| revision.revision == at.revision)
                .and_then(|revision| revision.files.iter().find(|file| file.path() == at.path));
            let Some(became) = listed.and_then(|file| next_file(file, &next.files)) else {
                return Ok((moved, Some(next.revision)));
            };
            // the line in the file it became, if the next revision's hunks hold it
            let from_file = self.file_of(node, at.revision, &at.path, files).await?;
            let to_file = self
                .file_of(node, next.revision, became.path(), files)
                .await?;
            let followed = match (from_file, to_file) {
                (Some(from_file), Some(to_file)) => {
                    follow_line(&at, &from_file, next.revision, &to_file)
                }
                _ => None,
            };
            match followed {
                Some(anchor) => {
                    moved.push(anchor.clone());
                    at = anchor;
                }
                None => return Ok((moved, Some(next.revision))),
            }
        }
        Ok((moved, None))
    }

    /// Reads one file of a node's revision, once for each follow
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `revision` - The revision's number
    /// * `path` - The path the file is shown under
    /// * `files` - The files of the node's revisions read so far
    async fn file_of(
        &self,
        node: &NodeName,
        revision: u32,
        path: &str,
        files: &mut Files,
    ) -> anyhow::Result<Option<Arc<FileDiff>>> {
        // a file read already is read from what was kept of it
        let key = (revision, path.to_string());
        if let Some(file) = files.get(&key) {
            return Ok(file.clone());
        }
        let file = self
            .revision_file(node, revision, path)
            .await?
            .map(Arc::new);
        files.insert(key, file.clone());
        Ok(file)
    }

    /// Reads every comment row of a plan, in number order, each checked against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    async fn comment_rows(&self, plan: &PlanName) -> anyhow::Result<Vec<Read>> {
        // the plan's one partition
        let rows = self
            .backend
            .comments(&plan.project, &plan.plan, None)
            .await?;
        rows.into_iter().map(|row| comment(plan, row)).collect()
    }

    /// Reads a thread's first row, the version it is at and what it keeps, if a thread starts
    /// at that number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    async fn thread_row(
        &self,
        plan: &PlanName,
        thread: u32,
    ) -> anyhow::Result<Option<(u64, Kept)>> {
        // the one row under the thread's number
        let key = sort_key(thread);
        let rows = self
            .backend
            .comments(&plan.project, &plan.plan, Some(&key))
            .await?;
        for row in rows {
            // a row read under another key is not this one
            if row.number != key {
                continue;
            }
            let (_, version, kept) = comment(plan, row)?;
            // a reply's number starts no thread
            return Ok(matches!(kept.on, KeptOn::Thread { .. }).then_some((version, kept)));
        }
        Ok(None)
    }

    /// Reads back a thread just written, which must be there
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `thread` - The thread's number
    async fn kept_thread(&self, plan: &PlanName, thread: u32) -> anyhow::Result<Thread> {
        match self.thread(plan, thread).await? {
            Some(thread) => Ok(thread),
            None => anyhow::bail!("thread {thread} of `{plan}` was written, but doesn't read back"),
        }
    }

    /// Keeps a new comment under its number, which the caller found free under the lock
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `number` - The comment's number
    /// * `kept` - What it keeps
    async fn keep_comment(&self, plan: &PlanName, number: u32, kept: Kept) -> anyhow::Result<()> {
        // written once, where nothing is
        let row = comment_row(plan, number, 1, &kept)?;
        match self.backend.write_comment(row, Expect::Absent).await? {
            Written::Applied => Ok(()),
            // under the lock, only a writer in another process could have taken the number
            Written::Refused(refusal) => anyhow::bail!(
                "comment {number} of `{plan}` wasn't kept, since another writer stored one under \
                 its number meanwhile: {}",
                refusal.describe()
            ),
        }
    }

    /// Whether a node's revisions hold the line an anchor names
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `anchor` - The line's revision, file, side and number
    async fn names_a_line(&self, node: &NodeName, anchor: &Anchor) -> anyhow::Result<bool> {
        // the file as the revision lists it, if it does
        let Some(file) = self
            .revision_file(node, anchor.revision, &anchor.path)
            .await?
        else {
            return Ok(false);
        };
        // one of its lines anchors where this one does
        let found =
            file.hunks.iter().flat_map(|hunk| &hunk.lines).any(|line| {
                Anchor::of(anchor.revision, &anchor.path, line).as_ref() == Some(anchor)
            });
        Ok(found)
    }
}

/// Refuses a comment's body with nothing in it, or more than [`COMMENT_BODY_MAX`] bytes
///
/// # Arguments
///
/// * `body` - The body as written
fn check_body(body: &str) -> anyhow::Result<()> {
    if body.trim().is_empty() {
        return Err(Unkeepable("a comment needs a body".to_string()).into());
    }
    if body.len() > COMMENT_BODY_MAX {
        let message = format!(
            "a comment's body is {} bytes, more than the {COMMENT_BODY_MAX} one may hold",
            body.len()
        );
        return Err(Unkeepable(message).into());
    }
    Ok(())
}

/// The number after a plan's last comment, or 1 for its first
///
/// # Arguments
///
/// * `rows` - The plan's comments, in number order
fn next_number(rows: &[Read]) -> anyhow::Result<u32> {
    // the rows are in number order, so the last is the highest
    match rows.last() {
        None => Ok(1),
        Some((last, _, _)) => last
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("a plan can hold no more than {} comments", u32::MAX)),
    }
}

/// A comment's sort key: its number padded with zeros, so the partition sorts by number
///
/// # Arguments
///
/// * `number` - The comment's number
fn sort_key(number: u32) -> String {
    format!("{number:010}")
}

/// A comment's row, at a version
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `number` - The comment's number
/// * `version` - The version the row is written at
/// * `kept` - What it keeps
fn comment_row(
    plan: &PlanName,
    number: u32,
    version: u64,
    kept: &Kept,
) -> anyhow::Result<CommentRow> {
    Ok(CommentRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        number: sort_key(number),
        version,
        record: serde_json::to_string(kept)?,
    })
}

/// A comment row read back, checked against the plan it was read for
///
/// # Arguments
///
/// * `plan` - The plan whose partition was read
/// * `row` - The row
fn comment(plan: &PlanName, row: CommentRow) -> anyhow::Result<Read> {
    // a row of another plan whose key hashed the same is a collision, not this plan's
    anyhow::ensure!(
        row.project == plan.project && row.plan == plan.plan,
        "comment row `{}` of `{}/{}` was stored where `{plan}`'s comments are: their partition \
         keys' hashes collide",
        row.number,
        row.project,
        row.plan
    );
    // its number is its padded sort key
    let number: u32 = row.number.parse().map_err(|_| {
        anyhow::anyhow!(
            "comment row `{}` of `{plan}` isn't named by its number",
            row.number
        )
    })?;
    let kept: Kept = serde_json::from_str(&row.record).map_err(|error| {
        anyhow::anyhow!("comment {number} of `{plan}` doesn't read as one: {error}")
    })?;
    Ok((number, row.version, kept))
}

/// A plan's comments grouped into their threads, by number
///
/// # Arguments
///
/// * `plan` - The plan the comments were read from
/// * `rows` - Its comments, in number order
fn threads_of(plan: &PlanName, rows: Vec<Read>) -> anyhow::Result<Vec<Thread>> {
    let mut threads: BTreeMap<u32, Thread> = BTreeMap::new();
    for (number, version, kept) in rows {
        // each comment as a thread shows it
        let comment = Comment {
            number,
            at: kept.at,
            body: kept.body,
            html: String::new(),
        };
        match kept.on {
            // a thread's first comment starts it, on its node
            KeptOn::Thread {
                target,
                followed,
                outdated,
                state,
                resolutions,
            } => {
                let node = plan.node(kept.node)?;
                threads.insert(
                    number,
                    Thread {
                        node,
                        number,
                        version,
                        target,
                        followed,
                        outdated,
                        state,
                        resolutions,
                        comments: vec![comment],
                    },
                );
            }
            // a reply follows in its thread, which comes before it
            KeptOn::Reply { thread } => match threads.get_mut(&thread) {
                Some(thread) => thread.comments.push(comment),
                None => anyhow::bail!(
                    "comment {number} of `{plan}` replies in thread {thread}, which isn't stored"
                ),
            },
        }
    }
    Ok(threads.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::{Kept, KeptOn, comment_row, next_number, sort_key};
    use crate::store::Store;
    use reef_api::{Target, ThreadState};
    use reef_model::PlanName;

    /// A general comment on a node, starting a thread
    ///
    /// # Arguments
    ///
    /// * `node` - The node's id
    fn general(node: &str) -> Kept {
        Kept {
            node: node.to_string(),
            at: 1,
            body: "Looks good".to_string(),
            on: KeptOn::Thread {
                target: Target::Node,
                followed: Vec::new(),
                outdated: None,
                state: ThreadState::Open,
                resolutions: Vec::new(),
            },
        }
    }

    /// Comments sort as numbers do, and the next one comes after the last
    #[test]
    fn comments_sort_by_number_and_count_on() {
        // padded numbers sort in their order
        let mut keys = [sort_key(10), sort_key(9), sort_key(100)];
        keys.sort();
        assert_eq!(keys, [sort_key(9), sort_key(10), sort_key(100)]);
        // a plan's first comment is 1, and any other is the last one's after it
        assert_eq!(next_number(&[]).unwrap(), 1);
        assert_eq!(next_number(&[(7, 1, general("C34"))]).unwrap(), 8);
        assert!(next_number(&[(u32::MAX, 1, general("C34"))]).is_err());
    }

    /// A thread's row kept before threads were resolved by changes reads back with none linked,
    /// so its table needs no fresh data directory
    #[tokio::test]
    async fn rows_kept_before_resolutions_read_back() -> anyhow::Result<()> {
        // a resolved general thread as C36's binary kept it, with no resolutions in its JSON
        let store = Store::memory();
        let plan = PlanName::new("reef", "central")?;
        let row = crate::schema::CommentRow {
            project: "reef".to_string(),
            plan: "central".to_string(),
            number: sort_key(1),
            version: 2,
            record: r#"{"node":"C34","at":1,"body":"Looks good","on":{"thread":{"target":{"on":"node"},"state":"resolved"}}}"#.to_string(),
        };
        store.backend().plant_comment("reef", "central", row);
        // it reads as it was, linked to no change
        let threads = store.threads(&plan).await?;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].state, ThreadState::Resolved);
        assert!(threads[0].resolutions.is_empty());
        // and a thread with none linked is kept without the key, as the old binary wrote it
        let kept = serde_json::to_string(&general("C34"))?;
        assert!(!kept.contains("resolutions"), "{kept}");
        Ok(())
    }

    /// Rows stored in a plan's partition under another plan's key are an error, never served
    #[tokio::test]
    async fn collided_comment_rows_are_errors() -> anyhow::Result<()> {
        // a comment of another project's plan, landing in reef's central's partition
        let store = Store::memory();
        let ours = PlanName::new("reef", "central")?;
        let theirs = PlanName::new("other", "central")?;
        let stray = comment_row(&theirs, 1, 1, &general("C34"))?;
        store.backend().plant_comment("reef", "central", stray);
        let error = store
            .threads(&ours)
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        // and a reply naming a thread that isn't stored is an error too
        let store = Store::memory();
        let reply = Kept {
            on: KeptOn::Reply { thread: 3 },
            ..general("C34")
        };
        let row = comment_row(&ours, 4, 1, &reply)?;
        store.backend().plant_comment("reef", "central", row);
        let error = store
            .threads(&ours)
            .await
            .expect_err("a reply with no thread was served");
        assert!(format!("{error:#}").contains("isn't stored"), "{error:#}");
        Ok(())
    }
}
