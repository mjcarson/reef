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
//! - **A line comment names a line of a sealed revision.** A thread on a line is kept only if
//!   the line's revision is sealed and lists its file, and the file has a line of that number on
//!   that side. A draft's lines change under it, so none is named until it is sealed.
//! - **A line's thread follows its line.** Each capture follows the node's threads from the
//!   revision their line was last in to the latest sealed one, a revision at a time, passing
//!   over any draft, by comparing the two revisions' hunks ([`follow`](super::follow)), and
//!   writes each one's row again over the version read. A thread keeps where its line was in
//!   each revision, and the revision that lost it, which outdates it. A thread started on an
//!   older revision is followed at once.
//! - **A change resolves a thread, and stays linked.** A resolve, by a commit's trailer at its
//!   capture or by hand, marks the thread resolved and links a revision to it, kept on its first
//!   row, so the link outlives the commit once it is squashed. A trailer whose change resolved
//!   the thread once already, by its author, when it was authored and its summary, which a
//!   rebase keeps, never resolves it again, so a thread reopened stays open.
//! - **A review holds its comments pending.** A review is a row of its own in the plan's
//!   partition, numbered as a comment is, holding whether it is open, submitted or discarded. A
//!   comment written in it names it, and is pending while it is open: left out of every read but
//!   one that asks for pending comments, and never marked or resolved. Submitting it is one write
//!   of its row, so every comment in it is published at once, and discarding it drops them.
//! - **Each write names its author.** A comment, a review started, a mark, a resolve and a
//!   review submitted or discarded each keep who made it, from the caller's token. Rows kept
//!   before authors were read back naming nobody; a comment kept before writers were said reads
//!   as the user's.
//! - **Writes take a lock per project; reads take none.** Each comment is one row's write, so
//!   every step leaves the plan's comments reading whole.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as another plan's comments.

use reef_api::{
    Anchor, Author, COMMENT_BODY_MAX, Comment, Commented, FileDiff, FollowedThread, MarkReview,
    MarkThread, Marked, NewReply, NewThread, Resolution, ResolveThread, Resolved, Resolves, Review,
    ReviewState, Reviewed, Revision, Target, Thread, ThreadMark, ThreadState, TrailerOutcome,
    TrailerRead,
};
use reef_model::{NodeName, PlanName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// Who wrote it: the user signed in, if one was, and whether Claude wrote it, the user's
    /// unless it says so, as rows kept before either was said read
    #[serde(flatten)]
    author: Author,
    /// The review it was written in, if it was, as rows kept before reviews read
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review: Option<u32>,
    /// Its body, as markdown
    body: String,
    /// Whether it starts a thread, replies in one, or is a review
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
        /// Each time it was marked open or resolved by hand, oldest first, as rows kept before
        /// marks were read
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        marks: Vec<ThreadMark>,
    },
    /// A reply in a thread
    Reply {
        /// The thread's number
        thread: u32,
    },
    /// A review of the node, whose comments are pending until it is submitted
    Review {
        /// Whether it is open, submitted or discarded
        state: ReviewState,
        /// When it was submitted or discarded, if it was
        #[serde(default, skip_serializing_if = "Option::is_none")]
        marked_at: Option<u64>,
        /// Who submitted or discarded it
        #[serde(default, skip_serializing_if = "Author::is_unknown")]
        marked_by: Author,
    },
}

/// Whether a read serves the threads and comments pending in a review not yet submitted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// Serve them, marked pending, as the app shows them to their reviewer
    Include,
    /// Leave them out, as everything but the app reads, Claude included
    Leave,
}

/// Where a comment stands with the review it was written in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// Published: written in no review, or in one submitted
    Published,
    /// Pending in this review, still open
    Pending(u32),
    /// Dropped with a review discarded, never served
    Dropped,
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
    /// * `pending` - Whether to serve what is pending in a review not yet submitted
    #[instrument(name = "Store::threads", skip_all, err(Debug))]
    pub async fn threads(&self, plan: &PlanName, pending: Pending) -> anyhow::Result<Vec<Thread>> {
        // every comment of the plan, grouped into its thread
        let rows = self.comment_rows(plan).await?;
        threads_of(plan, rows, pending)
    }

    /// Lists one node's comment threads, by number
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `pending` - Whether to serve what is pending in a review not yet submitted
    #[instrument(name = "Store::node_threads", skip_all, err(Debug))]
    pub async fn node_threads(
        &self,
        node: &NodeName,
        pending: Pending,
    ) -> anyhow::Result<Vec<Thread>> {
        // the plan's threads, those on this node kept
        let mut threads = self.threads(&node.plan, pending).await?;
        threads.retain(|thread| thread.node.id == node.id);
        Ok(threads)
    }

    /// Gets one comment thread of a plan, if a thread starts at that number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `number` - The thread's number
    /// * `pending` - Whether to serve it if it is pending in a review not yet submitted
    #[instrument(name = "Store::thread", skip_all, err(Debug))]
    pub async fn thread(
        &self,
        plan: &PlanName,
        number: u32,
        pending: Pending,
    ) -> anyhow::Result<Option<Thread>> {
        // the plan's threads, the one of this number kept, since its replies are anywhere after it
        let threads = self.threads(plan, pending).await?;
        Ok(threads.into_iter().find(|thread| thread.number == number))
    }

    /// Lists a plan's open reviews, by number
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::open_reviews", skip_all, err(Debug))]
    pub async fn open_reviews(&self, plan: &PlanName) -> anyhow::Result<Vec<Review>> {
        // every review of the plan, those still being written kept
        let rows = self.comment_rows(plan).await?;
        let mut reviews = reviews_of(plan, &rows)?;
        reviews.retain(|review| review.state == ReviewState::Open);
        Ok(reviews)
    }

    /// Gets a node's open review, if it has one
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::node_review", skip_all, err(Debug))]
    pub async fn node_review(&self, node: &NodeName) -> anyhow::Result<Option<Review>> {
        // the plan's open reviews, the one of this node kept
        let reviews = self.open_reviews(&node.plan).await?;
        Ok(reviews.into_iter().find(|review| review.node.id == node.id))
    }

    /// Submits a node's open review, publishing every comment in it at once, or discards it,
    /// dropping them, over the version its writer read
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `mark` - Whether it is submitted or discarded, and the version it was marked against
    /// * `author` - Who marks it
    #[instrument(name = "Store::mark_review", skip_all, err(Debug))]
    pub async fn mark_review(
        &self,
        node: &NodeName,
        mark: MarkReview,
        author: &Author,
    ) -> anyhow::Result<Reviewed> {
        // a review is only ever closed, never opened by a mark
        if mark.state == ReviewState::Open {
            let message = "a review is marked submitted or discarded, never open".to_string();
            return Err(Unkeepable(message).into());
        }
        // the node's open review, under the project's comment lock
        let lock = self.comment_lock(&node.plan.project);
        let _held = lock.lock().await;
        let rows = self.comment_rows(&node.plan).await?;
        let Some(open) = open_review(&rows, &node.id) else {
            return Ok(Reviewed::NoReview { node: node.clone() });
        };
        let Some((_, version, kept)) = rows.iter().find(|(number, _, _)| *number == open) else {
            return Ok(Reviewed::NoReview { node: node.clone() });
        };
        // a mark made against another version than the one stored never saw what changed
        let review = |rows: &[Read]| -> anyhow::Result<Review> {
            reviews_of(&node.plan, rows)?
                .into_iter()
                .find(|review| review.number == open)
                .ok_or_else(|| {
                    anyhow::anyhow!("review {open} of `{}` doesn't read back", node.plan)
                })
        };
        if *version != mark.read {
            return Ok(Reviewed::Stale {
                review: review(&rows)?,
            });
        }
        // its state changed in one write of its row, over the version read, with who marked it
        let marked = Kept {
            on: KeptOn::Review {
                state: mark.state,
                marked_at: Some(now_ms()),
                marked_by: author.clone(),
            },
            ..kept.clone()
        };
        let row = comment_row(&node.plan, open, version + 1, &marked)?;
        let written = self
            .backend
            .write_comment(row, Expect::Version(*version))
            .await?;
        let rows = self.comment_rows(&node.plan).await?;
        match written {
            Written::Applied => Ok(Reviewed::Marked {
                review: review(&rows)?,
            }),
            // a refusal means another process wrote it since the read
            Written::Refused(_) => Ok(Reviewed::Stale {
                review: review(&rows)?,
            }),
        }
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
    /// * `author` - Who writes it, Claude's if the request's body also says so
    #[instrument(name = "Store::start_thread", skip_all, err(Debug))]
    pub async fn start_thread(
        &self,
        node: &NodeName,
        new: NewThread,
        author: &Author,
    ) -> anyhow::Result<Commented> {
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
        let mut number = next_number(&rows)?;
        // the user signed in, said to be Claude's if either the header or the body says so
        let author = author.claiming(new.by);
        // written in the node's open review, started for it if it has none
        let review = match new.review {
            true => Some(
                self.joined(&node.plan, &node.id, &rows, &mut number, &author)
                    .await?,
            ),
            false => None,
        };
        // a line of an older revision is followed at once to the latest, read under the lock
        // so a capture's follow after it finds the thread where this left it
        let (followed, outdated) = match &new.target {
            Target::Line(anchor) => {
                let revisions = self.sealed_revisions(node).await?;
                let mut files = Files::new();
                self.trail(node, &revisions, anchor.clone(), &mut files)
                    .await?
            }
            Target::Node => (Vec::new(), None),
        };
        let kept = Kept {
            node: node.id.clone(),
            at: now_ms(),
            author,
            review,
            body: new.body,
            on: KeptOn::Thread {
                target: new.target,
                followed,
                outdated,
                state: ThreadState::Open,
                resolutions: Vec::new(),
                marks: Vec::new(),
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
    /// * `author` - Who writes it, Claude's if the request's body also says so
    #[instrument(name = "Store::reply", skip_all, err(Debug))]
    pub async fn reply(
        &self,
        plan: &PlanName,
        thread: u32,
        reply: NewReply,
        author: &Author,
    ) -> anyhow::Result<Commented> {
        // a body with nothing to say, or too much, is never kept
        check_body(&reply.body)?;
        // numbered after the plan's last comment, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let rows = self.comment_rows(plan).await?;
        // the thread must start at the number named, and a reply's number starts none
        let Some((node, standing)) = rows.iter().find_map(|(number, _, kept)| {
            (*number == thread && matches!(kept.on, KeptOn::Thread { .. }))
                .then(|| (kept.node.clone(), standing(&rows, kept)))
        }) else {
            return Ok(Commented::NoThread { thread });
        };
        let mut number = next_number(&rows)?;
        // the user signed in, said to be Claude's if either the header or the body says so
        let author = author.claiming(reply.by);
        // a pending thread takes replies only in its own review, and a dropped one none at all
        let review = match (standing, reply.review) {
            (Standing::Dropped, _) | (Standing::Pending(_), false) => {
                return Ok(Commented::NoThread { thread });
            }
            (Standing::Pending(review), true) => Some(review),
            (Standing::Published, true) => Some(
                self.joined(plan, &node, &rows, &mut number, &author)
                    .await?,
            ),
            (Standing::Published, false) => None,
        };
        let kept = Kept {
            node,
            at: now_ms(),
            author,
            review,
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
    /// * `author` - Who marks it
    #[instrument(name = "Store::mark_thread", skip_all, err(Debug))]
    pub async fn mark_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        mark: MarkThread,
        author: &Author,
    ) -> anyhow::Result<Marked> {
        // the thread's first row, which holds its state, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let Some((version, mut kept)) = self.thread_row(plan, thread).await? else {
            return Ok(Marked::NoThread { thread });
        };
        // a thread pending in a review is marked once the review is submitted, and one dropped
        // with its review never
        match self.standing_of(plan, &kept).await? {
            Standing::Published => {}
            Standing::Pending(_) => return Ok(Marked::Pending { thread }),
            Standing::Dropped => return Ok(Marked::NoThread { thread }),
        }
        // a mark made against another version than the one stored would undo what it never saw
        if version != mark.read {
            let thread = self.kept_thread(plan, thread).await?;
            return Ok(Marked::Stale { thread });
        }
        // its state changed, with who marked it and when, written only over the version read
        if let KeptOn::Thread { state, marks, .. } = &mut kept.on {
            *state = mark.state;
            marks.push(ThreadMark {
                state: mark.state,
                at: now_ms(),
                author: author.clone(),
            });
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
    /// * `author` - Who resolves it
    #[instrument(name = "Store::resolve_thread", skip_all, err(Debug))]
    pub async fn resolve_thread(
        &self,
        plan: &PlanName,
        thread: u32,
        resolve: ResolveThread,
        author: &Author,
    ) -> anyhow::Result<Resolved> {
        // the thread's first row, which holds its state, under the project's comment lock
        let lock = self.comment_lock(&plan.project);
        let _held = lock.lock().await;
        let Some((version, mut kept)) = self.thread_row(plan, thread).await? else {
            return Ok(Resolved::NoThread { thread });
        };
        // a thread pending in a review is resolved once the review is submitted, and one dropped
        // with its review never
        match self.standing_of(plan, &kept).await? {
            Standing::Published => {}
            Standing::Pending(_) => return Ok(Resolved::Pending { thread }),
            Standing::Dropped => return Ok(Resolved::NoThread { thread }),
        }
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
                author: author.clone(),
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
    /// * `author` - Who sent the capture
    #[instrument(name = "Store::resolve_by_trailers", skip_all, err(Debug))]
    pub async fn resolve_by_trailers(
        &self,
        node: &NodeName,
        revision: u32,
        resolves: Vec<Resolves>,
        author: &Author,
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
            // a thread pending in a review stays open, and one dropped with its review is none
            match self.standing_of(plan, &kept).await? {
                Standing::Published => {}
                Standing::Pending(_) => {
                    read.push((outcome(TrailerOutcome::Pending), None));
                    continue;
                }
                Standing::Dropped => {
                    read.push((outcome(TrailerOutcome::NoThread), None));
                    continue;
                }
            }
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
                author: author.clone(),
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

    /// Follows the threads on a node's lines to its latest sealed revision, and answers what it
    /// wrote
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
        // the node's sealed revisions, read under the lock, so a thread started before it was
        // followed as far as its start saw; a draft holds no thread until it is sealed
        let revisions = self.sealed_revisions(node).await?;
        let Some(latest) = revisions.last().map(|revision| revision.revision) else {
            return Ok(None);
        };
        let rows = self.comment_rows(&node.plan).await?;
        let states = review_states(&rows);
        let mut files = Files::new();
        let mut written = Vec::new();
        for (number, version, mut kept) in rows {
            // only this node's threads on a line, still on it, and behind its latest revision;
            // one pending in a review is followed too, so it is where its line is once published,
            // and one dropped with its review never
            if kept.node != node.id || standing_in(&states, &kept) == Standing::Dropped {
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
        match self.thread(plan, thread, Pending::Include).await? {
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

    /// The number of a node's open review among a plan's comments, starting one under the next
    /// number if it has none, which moves the next number on
    ///
    /// The caller holds the project's comment lock, and read the rows under it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `node` - The node's id
    /// * `rows` - The plan's comments, in number order
    /// * `next` - The number the next row of the plan takes
    /// * `author` - Who writes the comment, who starts the review if one is started
    async fn joined(
        &self,
        plan: &PlanName,
        node: &str,
        rows: &[Read],
        next: &mut u32,
        author: &Author,
    ) -> anyhow::Result<u32> {
        // the node's review still being written, if it has one
        if let Some(open) = open_review(rows, node) {
            return Ok(open);
        }
        // else one started under the next number, before the comment it was started for
        let number = *next;
        let review = Kept {
            node: node.to_string(),
            at: now_ms(),
            author: author.clone(),
            review: None,
            body: String::new(),
            on: KeptOn::Review {
                state: ReviewState::Open,
                marked_at: None,
                marked_by: Author::default(),
            },
        };
        self.keep_comment(plan, number, review).await?;
        *next = number
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("a plan can hold no more than {} comments", u32::MAX))?;
        Ok(number)
    }

    /// Where a comment stands with the review it was written in, its review's row read alone
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `kept` - The comment
    async fn standing_of(&self, plan: &PlanName, kept: &Kept) -> anyhow::Result<Standing> {
        // written in no review, it is published
        let Some(review) = kept.review else {
            return Ok(Standing::Published);
        };
        // else as its review's row says
        let key = sort_key(review);
        let rows = self
            .backend
            .comments(&plan.project, &plan.plan, Some(&key))
            .await?;
        for row in rows {
            if row.number != key {
                continue;
            }
            let (_, _, row) = comment(plan, row)?;
            if let KeptOn::Review { state, .. } = row.on {
                return Ok(standing_by(review, state));
            }
        }
        // a review a comment names is written before it, so one not there drops it
        Ok(Standing::Dropped)
    }

    /// Whether a node's sealed revisions hold the line an anchor names
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `anchor` - The line's revision, file, side and number
    async fn names_a_line(&self, node: &NodeName, anchor: &Anchor) -> anyhow::Result<bool> {
        // a draft's lines change under it, so only a sealed revision's are named
        let sealed = self
            .node_revision(node, anchor.revision)
            .await?
            .is_some_and(|revision| !revision.draft);
        if !sealed {
            return Ok(false);
        }
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
/// A comment pending in an open review is served only if pending ones are asked for, marked
/// with its review, and one dropped with a discarded review never is, nor any reply to a thread
/// left out.
///
/// # Arguments
///
/// * `plan` - The plan the comments were read from
/// * `rows` - Its comments, in number order
/// * `pending` - Whether to serve what is pending in a review not yet submitted
fn threads_of(plan: &PlanName, rows: Vec<Read>, pending: Pending) -> anyhow::Result<Vec<Thread>> {
    // each review's state, by number, since a comment's review comes before it
    let states = review_states(&rows);
    let mut threads: BTreeMap<u32, Thread> = BTreeMap::new();
    let mut left_out: HashSet<u32> = HashSet::new();
    for (number, version, kept) in rows {
        // a review is no comment, and a comment left out leaves out a thread it starts
        let standing = standing_in(&states, &kept);
        let review = match (standing, pending) {
            (Standing::Published, _) => None,
            (Standing::Pending(review), Pending::Include) => Some(review),
            (Standing::Pending(_), Pending::Leave) | (Standing::Dropped, _) => {
                if matches!(kept.on, KeptOn::Thread { .. }) {
                    left_out.insert(number);
                }
                continue;
            }
        };
        // each comment as a thread shows it
        let comment = Comment {
            number,
            at: kept.at,
            author: kept.author,
            review,
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
                marks,
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
                        marks,
                        pending: review.is_some(),
                        comments: vec![comment],
                    },
                );
            }
            // a reply follows in its thread, which comes before it, unless that was left out
            KeptOn::Reply { thread } => match threads.get_mut(&thread) {
                Some(thread) => thread.comments.push(comment),
                None if left_out.contains(&thread) => {}
                None => anyhow::bail!(
                    "comment {number} of `{plan}` replies in thread {thread}, which isn't stored"
                ),
            },
            // a review is no comment, and its state was read already
            KeptOn::Review { .. } => {}
        }
    }
    Ok(threads.into_values().collect())
}

/// Each review of a plan's comments and where it stands, by number
///
/// # Arguments
///
/// * `rows` - The plan's comments, in number order
fn review_states(rows: &[Read]) -> HashMap<u32, ReviewState> {
    rows.iter()
        .filter_map(|(number, _, kept)| match kept.on {
            KeptOn::Review { state, .. } => Some((*number, state)),
            _ => None,
        })
        .collect()
}

/// Where a comment stands with the review it was written in, by the plan's reviews' states
///
/// A review is itself published, so it is never left out by its own state.
///
/// # Arguments
///
/// * `states` - Each review of the plan and where it stands
/// * `kept` - The comment
fn standing_in(states: &HashMap<u32, ReviewState>, kept: &Kept) -> Standing {
    match kept.review {
        None => Standing::Published,
        // a review a comment names is written before it, so one not there drops it
        Some(review) => match states.get(&review) {
            Some(state) => standing_by(review, *state),
            None => Standing::Dropped,
        },
    }
}

/// Where a comment written in a review stands, by the review's state
///
/// # Arguments
///
/// * `review` - The review's number
/// * `state` - Where the review stands
fn standing_by(review: u32, state: ReviewState) -> Standing {
    match state {
        ReviewState::Open => Standing::Pending(review),
        ReviewState::Submitted => Standing::Published,
        ReviewState::Discarded => Standing::Dropped,
    }
}

/// Where a comment stands with its review, among a plan's comments
///
/// # Arguments
///
/// * `rows` - The plan's comments, in number order
/// * `kept` - The comment
fn standing(rows: &[Read], kept: &Kept) -> Standing {
    standing_in(&review_states(rows), kept)
}

/// The number of a node's open review among a plan's comments, if it has one
///
/// # Arguments
///
/// * `rows` - The plan's comments, in number order
/// * `node` - The node's id
fn open_review(rows: &[Read], node: &str) -> Option<u32> {
    rows.iter().find_map(|(number, _, kept)| {
        // a review row of the node, still being written
        let open = matches!(
            kept.on,
            KeptOn::Review {
                state: ReviewState::Open,
                ..
            }
        );
        (kept.node == node && open).then_some(*number)
    })
}

/// A plan's reviews, by number, each with how many comments are written in it
///
/// # Arguments
///
/// * `plan` - The plan the comments were read from
/// * `rows` - Its comments, in number order
fn reviews_of(plan: &PlanName, rows: &[Read]) -> anyhow::Result<Vec<Review>> {
    let mut reviews = Vec::new();
    for (number, version, kept) in rows {
        let KeptOn::Review {
            state,
            marked_at,
            marked_by,
        } = &kept.on
        else {
            continue;
        };
        // each comment that names it
        let comments = rows
            .iter()
            .filter(|(_, _, comment)| comment.review == Some(*number))
            .count();
        reviews.push(Review {
            node: plan.node(kept.node.clone())?,
            number: *number,
            version: *version,
            state: *state,
            at: kept.at,
            comments: u32::try_from(comments)?,
            author: kept.author.clone(),
            marked_at: *marked_at,
            marked_by: marked_by.clone(),
        });
    }
    Ok(reviews)
}

#[cfg(test)]
mod tests {
    use super::{Kept, KeptOn, Pending, comment_row, next_number, sort_key};
    use crate::store::Store;
    use reef_api::{Author, ReviewState, Target, ThreadState, Writer};
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
            author: Author::default(),
            review: None,
            body: "Looks good".to_string(),
            on: KeptOn::Thread {
                target: Target::Node,
                followed: Vec::new(),
                outdated: None,
                state: ThreadState::Open,
                resolutions: Vec::new(),
                marks: Vec::new(),
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
        let threads = store.threads(&plan, Pending::Include).await?;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].state, ThreadState::Resolved);
        assert!(threads[0].resolutions.is_empty());
        // and a thread with none linked is kept without the key, as the old binary wrote it
        let kept = serde_json::to_string(&general("C34"))?;
        assert!(!kept.contains("resolutions"), "{kept}");
        // a comment kept before writers were said is the user's, and the user's says nothing
        assert_eq!(threads[0].comments[0].author.by, Writer::User);
        assert!(!kept.contains("\"by\""), "{kept}");
        let claudes = Kept {
            author: Author {
                by: Writer::Claude,
                ..Author::default()
            },
            ..general("C34")
        };
        assert!(serde_json::to_string(&claudes)?.contains(r#""by":"claude""#));
        Ok(())
    }

    /// Rows kept before reviews read back published, whether or not pending ones are asked for,
    /// and a comment in no review is kept without the key, as the old binary wrote it
    #[tokio::test]
    async fn rows_kept_before_reviews_read_back() -> anyhow::Result<()> {
        // a thread and its reply as C43's binary kept them, with no review in their JSON
        let store = Store::memory();
        let plan = PlanName::new("reef", "central")?;
        let rows = [
            (
                1,
                r#"{"node":"C34","at":1,"body":"Why?","on":{"thread":{"target":{"on":"node"},"state":"open"}}}"#,
            ),
            (
                2,
                r#"{"node":"C34","at":2,"by":"claude","body":"Because.","on":{"reply":{"thread":1}}}"#,
            ),
        ];
        for (number, record) in rows {
            let row = crate::schema::CommentRow {
                project: "reef".to_string(),
                plan: "central".to_string(),
                number: sort_key(number),
                version: 1,
                record: record.to_string(),
            };
            store.backend().plant_comment("reef", "central", row);
        }
        // both read as published, the same whichever reads them
        let left = store.threads(&plan, Pending::Leave).await?;
        assert_eq!(left, store.threads(&plan, Pending::Include).await?);
        assert_eq!(left.len(), 1);
        assert!(!left[0].pending);
        assert!(
            left[0]
                .comments
                .iter()
                .all(|comment| comment.review.is_none())
        );
        assert_eq!(left[0].comments[1].author.by, Writer::Claude);
        assert!(store.open_reviews(&plan).await?.is_empty());
        // and a comment in no review says nothing of one
        let kept = serde_json::to_string(&general("C34"))?;
        assert!(!kept.contains("review"), "{kept}");
        Ok(())
    }

    /// Rows kept before authors were read back naming nobody: a comment, a thread never marked,
    /// a review never said to be started or marked by anyone, and a resolution linked by nobody;
    /// and what nobody wrote is kept without the keys, as the old binary wrote it
    #[tokio::test]
    async fn rows_kept_before_authors_read_back() -> anyhow::Result<()> {
        // a review submitted, a thread in it resolved by hand, and Claude's reply, as C49 kept them
        let store = Store::memory();
        let plan = PlanName::new("reef", "central")?;
        let rows = [
            (
                1,
                r#"{"node":"C34","at":1,"body":"","on":{"review":{"state":"submitted"}}}"#,
            ),
            (
                2,
                r#"{"node":"C34","at":2,"review":1,"body":"Why?","on":{"thread":{"target":{"on":"node"},"state":"resolved","resolutions":[{"node":"reef/central/C34","revision":1,"at":3}]}}}"#,
            ),
            (
                3,
                r#"{"node":"C34","at":4,"by":"claude","body":"Because.","on":{"reply":{"thread":2}}}"#,
            ),
        ];
        for (number, record) in rows {
            let row = crate::schema::CommentRow {
                project: "reef".to_string(),
                plan: "central".to_string(),
                number: sort_key(number),
                version: 1,
                record: record.to_string(),
            };
            store.backend().plant_comment("reef", "central", row);
        }
        // each reads back naming nobody, Claude's reply still Claude's
        let threads = store.threads(&plan, Pending::Leave).await?;
        assert_eq!(threads.len(), 1);
        let thread = &threads[0];
        assert_eq!(thread.comments[0].author, Author::default());
        assert_eq!(
            thread.comments[1].author,
            Author {
                by: Writer::Claude,
                ..Author::default()
            }
        );
        assert!(thread.marks.is_empty() && thread.last_mark().is_none());
        assert!(thread.resolutions[0].author.is_unknown());
        let review = store
            .node_review(&plan.node("C34")?)
            .await?
            .map(|review| review.state);
        assert_eq!(review, None);
        // and nobody's review row, read among the plan's reviews, names nobody either
        let rows = store.comment_rows(&plan).await?;
        let reviews = super::reviews_of(&plan, &rows)?;
        assert_eq!(reviews[0].state, ReviewState::Submitted);
        assert!(reviews[0].author.is_unknown() && reviews[0].marked_by.is_unknown());
        assert_eq!(reviews[0].marked_at, None);
        // what nobody wrote is kept with no author's keys at all
        let kept = serde_json::to_string(&general("C34"))?;
        for key in ["user", "since", "marks", "marked"] {
            assert!(!kept.contains(key), "{kept}");
        }
        // and a signed-in user's comment keeps their name beside its other fields
        let adas = Kept {
            author: Author {
                user: Some("ada".to_string()),
                since: Some(5),
                by: Writer::User,
            },
            ..general("C34")
        };
        let json = serde_json::to_string(&adas)?;
        assert!(json.contains(r#""user":"ada","since":5"#), "{json}");
        assert_eq!(serde_json::from_str::<Kept>(&json)?, adas);
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
            .threads(&ours, Pending::Include)
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
            .threads(&ours, Pending::Include)
            .await
            .expect_err("a reply with no thread was served");
        assert!(format!("{error:#}").contains("isn't stored"), "{error:#}");
        Ok(())
    }
}
