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
//! - **Writes take a lock per project; reads take none.** Each comment is one row's write, so
//!   every step leaves the plan's comments reading whole.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as another plan's comments.

use reef_api::{
    Anchor, COMMENT_BODY_MAX, Comment, Commented, MarkThread, Marked, NewReply, NewThread, Target,
    Thread, ThreadState,
};
use reef_model::{NodeName, PlanName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::instrument;

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
        /// What the thread is on
        target: Target,
        /// Whether the thread is open or resolved
        state: ThreadState,
    },
    /// A reply in a thread
    Reply {
        /// The thread's number
        thread: u32,
    },
}

/// One comment row as read: its number, the version its row is at, and what it keeps
type Read = (u32, u64, Kept);

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
        let kept = Kept {
            node: node.id.clone(),
            at: now_ms(),
            body: new.body,
            on: KeptOn::Thread {
                target: new.target,
                state: ThreadState::Open,
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
            KeptOn::Thread { target, state } => {
                let node = plan.node(kept.node)?;
                threads.insert(
                    number,
                    Thread {
                        node,
                        number,
                        version,
                        target,
                        state,
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
                state: ThreadState::Open,
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
