//! A backend that fails or waits where a test asks it to, to stand in for a write killed midway
//!
//! A bulk write is not atomic, so a process killed partway leaves some of its rows written. A
//! write that fails partway leaves the same rows, which the store neither cleans up nor hides,
//! so the next write must recover from both the same way.

use reef_model::PlanName;
use reef_service::schema::{NodeKey, NodeRow, PlanRow, ProjectRow};
use reef_service::{Backend, Expect, Pointers, RevisionKey, Written};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// A point in a whole-plan write where it waits, so a test can act between its rows and its flip
#[derive(Debug, Default)]
pub struct Pause {
    /// Notified when the write has its rows in and is about to flip
    pub reached: Notify,
    /// Notified to let the write flip
    pub resume: Notify,
}

/// The faults armed for the next writes
#[derive(Debug, Default)]
pub struct Faults {
    /// Fail the next bulk write of rows after writing this many of them
    rows_left: Mutex<Option<usize>>,
    /// Fail the next delete of rows, before deleting any
    fail_prune: AtomicBool,
    /// Wait here before the next write that makes a revision current
    pause: Mutex<Option<Arc<Pause>>>,
}

impl Faults {
    /// Fails the next bulk write of rows once it has written `rows` of them
    ///
    /// # Arguments
    ///
    /// * `rows` - How many rows to write before failing
    pub fn fail_rows_after(&self, rows: usize) {
        *self.rows_left.lock().unwrap() = Some(rows);
    }

    /// Fails the next delete of rows, before deleting any
    pub fn fail_next_prune(&self) {
        self.fail_prune.store(true, Ordering::SeqCst);
    }

    /// Makes the next flip wait, and returns where to hear it is waiting and to let it go
    pub fn pause_next_flip(&self) -> Arc<Pause> {
        let pause = Arc::new(Pause::default());
        *self.pause.lock().unwrap() = Some(pause.clone());
        pause
    }
}

/// A backend whose writes fail or wait where its faults say
pub struct Faulty<B> {
    /// The backend that does the work
    inner: B,
    /// The faults armed for the next writes
    faults: Arc<Faults>,
}

impl<B> Faulty<B> {
    /// Wraps a backend, with nothing armed
    ///
    /// # Arguments
    ///
    /// * `inner` - The backend that does the work
    /// * `faults` - The faults a test arms
    pub fn new(inner: B, faults: Arc<Faults>) -> Faulty<B> {
        Faulty { inner, faults }
    }

    /// Gives up the wrapper, keeping the backend
    pub fn into_inner(self) -> B {
        self.inner
    }
}

impl<B: Backend> Backend for Faulty<B> {
    /// Lists every project's row
    async fn projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        self.inner.projects().await
    }

    /// Writes a project's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The project's row
    async fn insert_project(&self, row: ProjectRow) -> anyhow::Result<Written> {
        self.inner.insert_project(row).await
    }

    /// Lists a project's plan rows
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanRow>> {
        self.inner.plans(project).await
    }

    /// Gets a plan's row
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    async fn plan(&self, plan: &PlanName) -> anyhow::Result<Option<PlanRow>> {
        self.inner.plan(plan).await
    }

    /// Writes a plan's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The plan's row
    async fn insert_plan(&self, row: PlanRow) -> anyhow::Result<Written> {
        self.inner.insert_plan(row).await
    }

    /// Reads a revision's node rows
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `id` - The one node to read, or `None` for all of them
    async fn nodes(&self, key: &RevisionKey, id: Option<&str>) -> anyhow::Result<Vec<NodeRow>> {
        self.inner.nodes(key, id).await
    }

    /// Writes one node row, if its condition holds
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_node(&self, row: NodeRow, expect: Expect) -> anyhow::Result<Written> {
        self.inner.write_node(row, expect).await
    }

    /// Changes a plan's pointers, waiting first if this flip was asked to
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the row was read at
    /// * `pointers` - The revisions that change
    async fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        pointers: Pointers,
    ) -> anyhow::Result<Written> {
        // only a write that makes a revision current is a flip
        let pause = match pointers.current {
            Some(_) => self.faults.pause.lock().unwrap().take(),
            None => None,
        };
        // say the rows are in, and wait to be let go
        if let Some(pause) = pause {
            pause.reached.notify_one();
            pause.resume.notified().await;
        }
        self.inner.update_plan(plan, read, pointers).await
    }

    /// Lists a revision's keys
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    async fn node_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<NodeKey>> {
        self.inner.node_keys(key).await
    }

    /// Writes node rows, failing partway if asked to
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_nodes(&self, mut rows: Vec<NodeRow>) -> anyhow::Result<()> {
        // a fault armed for this write lets only so many rows through
        let Some(left) = self.faults.rows_left.lock().unwrap().take() else {
            return self.inner.write_nodes(rows).await;
        };
        rows.truncate(left);
        self.inner.write_nodes(rows).await?;
        anyhow::bail!("killed after writing {left} rows")
    }

    /// Deletes node rows, failing before any if asked to
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `ids` - The ids of the nodes to delete
    async fn delete_nodes(&self, key: &RevisionKey, ids: Vec<String>) -> anyhow::Result<()> {
        // a fault armed for this delete stops it before anything is deleted
        if self.faults.fail_prune.swap(false, Ordering::SeqCst) {
            anyhow::bail!("killed before pruning");
        }
        self.inner.delete_nodes(key, ids).await
    }
}
