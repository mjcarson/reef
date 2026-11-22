//! Each node's work record, a row a node in its project's partition
//!
//! - **One record a node, across the project's plans.** A worktree finds the node it is on among
//!   its project's records, so every plan's share one partition, each sorted by `plan/id`.
//! - **A start writes its row whole.** A node not started is written `if_absent`, and one started
//!   again only over the version the start named, so a second start of a node is refused as
//!   stale rather than taking the first one's place unseen.
//! - **Each start is later than the last.** A worktree is on the node started there last, so a
//!   start's time is after every other start in its project, even within one millisecond.
//! - **A capture adds to the start it was taken from.** It names that start by the commit its
//!   pin points at, so the SHAs of a capture taken before a restart never join the new start's.
//!   It is kept as the node's next [`revision`](super::revision), unless its base and tree are
//!   the latest revision's, and its SHAs are added naming the revision either way.
//! - **Writes take a lock per project; reads take none.** A start is one row's write, and a
//!   capture its revision's rows, then the record's.

use reef_api::{AddCapture, CaptureRead, Captured, StartWork, Started, WorkRecord, WorkStart};
use reef_model::{NodeName, PlanName};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;

use super::revision::check_files;
use super::{Store, Unkeepable};
use crate::backend::{Backend, Expect, Written};
use crate::schema::WorkRow;

/// What a work row keeps as JSON: everything of a record but its name and version
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    /// When the start was recorded, in milliseconds since the epoch
    started_at: u64,
    /// Where the work happens and what it started from
    #[serde(flatten)]
    start: WorkStart,
    /// What each capture since the start read, oldest first
    #[serde(default)]
    captures: Vec<CaptureRead>,
}

impl<B: Backend> Store<B> {
    /// Finds a project's work lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose work records' lock to find
    fn work_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.works
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(project.to_string())
            .or_default()
            .clone()
    }

    /// Lists the work records of a project's nodes, by node, across its plans
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::work", skip_all, err(Debug))]
    pub async fn work(&self, project: &str) -> anyhow::Result<Vec<WorkRecord>> {
        // every row of the project's partition, each checked against its full key
        let rows = self.backend.work(project, None).await?;
        rows.into_iter().map(|row| record(project, row)).collect()
    }

    /// Gets one node's work record, if it was started
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    #[instrument(name = "Store::work_of", skip_all, err(Debug))]
    pub async fn work_of(&self, node: &NodeName) -> anyhow::Result<Option<WorkRecord>> {
        // the one row under the node's key, which a collision could fill with another's
        let key = row_key(node);
        let rows = self
            .backend
            .work(&node.plan.project, Some(key.as_str()))
            .await?;
        let mut found = None;
        for row in rows {
            let read = record(&node.plan.project, row)?;
            // a row under another node's key is none of this node's
            if read.node == *node {
                found = Some(read);
            }
        }
        Ok(found)
    }

    /// Records a node's start: where its work happens and what it started from
    ///
    /// A node not started is recorded only if the start names no version, and one started
    /// already only if the start names the version of its record, which it replaces, captures
    /// and all. Otherwise nothing is written, and the start is [`Started::Stale`]. The store
    /// keeps rows; it doesn't check the node is in its plan. The service does.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `start` - What the start read, and the version of the record it replaces
    #[instrument(name = "Store::start_work", skip_all, err(Debug))]
    pub async fn start_work(&self, node: &NodeName, start: StartWork) -> anyhow::Result<Started> {
        // a worktree is named as its host sees it, so only an absolute path names one
        check_start(node, &start.start)?;
        let project = node.plan.project.as_str();
        // one write to a project's work records at a time
        let lock = self.work_lock(project);
        let _write = lock.lock().await;
        // the project's records, for the node's own and for the latest start
        let all = self.work(project).await?;
        let current = all.iter().find(|record| record.node == *node);
        // a new node is written where none is, and a restart only over the record it read
        let (expect, version) = match (current, start.replaces) {
            (None, None) => (Expect::Absent, 1),
            (Some(record), Some(read)) if record.version == read => {
                (Expect::Version(read), read + 1)
            }
            _ => {
                return Ok(Started::Stale {
                    record: current.cloned(),
                });
            }
        };
        // later than every start the project recorded, so the latest in a worktree is plain
        let latest = all.iter().map(|record| record.started_at).max();
        let started_at = latest.map_or(now_ms(), |latest| now_ms().max(latest + 1));
        let kept = Kept {
            started_at,
            start: start.start,
            captures: Vec::new(),
        };
        let row = work_row(node, version, &kept)?;
        match self.backend.write_work(row, expect).await? {
            Written::Applied => Ok(Started::Recorded {
                record: recorded(node, version, kept),
            }),
            // another writer got there between the read and the write
            Written::Refused(_) => Ok(Started::Stale {
                record: self.work_of(node).await?,
            }),
        }
    }

    /// Keeps a capture as its node's next revision, and adds the SHAs it read to its record
    ///
    /// The capture names the start it was taken from by its pinned commit, so a capture of a
    /// node started again since is [`Captured::Restarted`], and keeps nothing. One whose base and
    /// tree are the node's latest revision's is [`Captured::Unchanged`]: no revision is added,
    /// and its SHAs are added naming the latest. Otherwise its files are kept as the revision
    /// after the latest, whichever start that was taken from, and then its SHAs added.
    ///
    /// # Arguments
    ///
    /// * `node` - The node's full name
    /// * `add` - The start the capture was taken from, what it read, and its files
    #[instrument(name = "Store::add_capture", skip_all, err(Debug))]
    pub async fn add_capture(&self, node: &NodeName, add: AddCapture) -> anyhow::Result<Captured> {
        // every file must be readable back by a path of its own
        check_files(node, &add.files)?;
        // one write to a project's work records at a time
        let lock = self.work_lock(&node.plan.project);
        let _write = lock.lock().await;
        // a node with no record has no start to add to
        let Some(current) = self.work_of(node).await? else {
            return Ok(Captured::NotStarted { node: node.clone() });
        };
        // and a capture of an earlier start is none of this one's
        if current.start.pinned != add.pinned {
            return Ok(Captured::Restarted { record: current });
        }
        // the node's latest revision, which a capture of the same base and tree only matches
        let at = now_ms();
        let latest = self.revisions(node).await?.pop();
        let (revision, added) = match latest {
            Some(latest)
                if latest.shas.base == add.shas.base && latest.shas.tree == add.shas.tree =>
            {
                (latest, false)
            }
            latest => {
                // the next number after the latest, whatever start it was taken from
                let number = latest.map_or(1, |latest| latest.revision + 1);
                let shas = add.shas.clone();
                let kept = self
                    .keep_revision(node, number, at, shas, add.conflicts, add.files)
                    .await?;
                (kept, true)
            }
        };
        // the record with the capture's SHAs after the others, over the version read
        let read = current.version;
        let mut kept = Kept {
            started_at: current.started_at,
            start: current.start,
            captures: current.captures,
        };
        kept.captures.push(CaptureRead {
            at,
            revision: Some(revision.revision),
            shas: add.shas,
        });
        let row = work_row(node, read + 1, &kept)?;
        match self.backend.write_work(row, Expect::Version(read)).await? {
            Written::Applied => {
                let record = recorded(node, read + 1, kept);
                Ok(match added {
                    true => Captured::Added {
                        record,
                        revision,
                        trailers: Vec::new(),
                    },
                    false => Captured::Unchanged {
                        record,
                        revision,
                        trailers: Vec::new(),
                    },
                })
            }
            // under the lock, only a writer in another process could have moved it
            Written::Refused(refusal) => anyhow::bail!(
                "a capture of `{node}` wasn't added, since its work record was written by another \
                 writer meanwhile: {}",
                refusal.describe()
            ),
        }
    }
}

/// Refuses a start the store can't keep as it was given
///
/// # Arguments
///
/// * `node` - The node started
/// * `start` - What the start read
fn check_start(node: &NodeName, start: &WorkStart) -> anyhow::Result<()> {
    // a relative worktree names nothing anyone can find again
    if !start.worktree.is_absolute() {
        let message = format!(
            "the start of `{node}` names its worktree {}, which is not absolute",
            start.worktree.display()
        );
        return Err(Unkeepable(message).into());
    }
    // and one that isn't utf-8 can't be sent back as JSON
    if start.worktree.to_str().is_none() {
        let message = format!(
            "the start of `{node}` names its worktree {}, which is not a utf-8 path",
            start.worktree.display()
        );
        return Err(Unkeepable(message).into());
    }
    Ok(())
}

/// A node's key within its project's partition: `plan/id`
///
/// # Arguments
///
/// * `node` - The node's full name
fn row_key(node: &NodeName) -> String {
    format!("{}/{}", node.plan.plan, node.id)
}

/// A node's work row, its record as JSON
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `version` - The version the row is written at
/// * `kept` - The record but its name and version
fn work_row(node: &NodeName, version: u64, kept: &Kept) -> anyhow::Result<WorkRow> {
    Ok(WorkRow {
        project: node.plan.project.clone(),
        node: row_key(node),
        version,
        record: serde_json::to_string(kept)?,
    })
}

/// A record as a client reads it, from what was written
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `version` - The version it was written at
/// * `kept` - The record but its name and version
fn recorded(node: &NodeName, version: u64, kept: Kept) -> WorkRecord {
    WorkRecord {
        node: node.clone(),
        version,
        started_at: kept.started_at,
        start: kept.start,
        captures: kept.captures,
    }
}

/// A work row read back as its record, checked against the project it was read from
///
/// # Arguments
///
/// * `project` - The project whose partition was read
/// * `row` - The row
fn record(project: &str, row: WorkRow) -> anyhow::Result<WorkRecord> {
    // a row of another project whose key hashed the same is a collision, not this project's
    anyhow::ensure!(
        row.project == project,
        "work row `{}` of `{}` was stored where `{project}`'s work records are: their partition \
         keys' hashes collide",
        row.node,
        row.project
    );
    // the node's name is its project's, then the row's `plan/id`
    let (plan, id) = row.node.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "work row `{}` of `{project}` doesn't name its node as `plan/id`",
            row.node
        )
    })?;
    let node = PlanName::new(project, plan)?.node(id)?;
    let kept: Kept = serde_json::from_str(&row.record)
        .map_err(|error| anyhow::anyhow!("work record of `{node}` doesn't read as one: {error}"))?;
    Ok(recorded(&node, row.version, kept))
}

/// The milliseconds since the epoch
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Kept, Store, work_row};
    use crate::schema::WorkRow;
    use reef_api::WorkStart;
    use reef_model::NodeName;
    use std::path::PathBuf;

    /// A row stored in a project's partition under another project's key is an error, never a
    /// record
    #[tokio::test]
    async fn collided_work_row_is_an_error() -> anyhow::Result<()> {
        // a row of another project's, landing in reef's partition as a hash collision would
        let store = Store::memory();
        let node: NodeName = "other/central/C30".parse()?;
        let kept = Kept {
            started_at: 1,
            start: WorkStart {
                worktree: PathBuf::from("/src/other"),
                branch: None,
                base_branch: None,
                start: "a".repeat(40),
                start_tree: "b".repeat(40),
                fork: None,
                pin: "refs/reef/start/central/C30".to_string(),
                pinned: "c".repeat(40),
            },
            captures: Vec::new(),
        };
        let stray: WorkRow = work_row(&node, 1, &kept)?;
        store.backend().plant_work("reef", stray);
        // a listing of the project's records is caught, and so is a read of the one node
        let error = store
            .work("reef")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        let ours: NodeName = "reef/central/C30".parse()?;
        let error = store
            .work_of(&ours)
            .await
            .expect_err("a collided row was served");
        assert!(format!("{error:#}").contains("collide"), "{error:#}");
        Ok(())
    }
}
