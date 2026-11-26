//! A backend that fails or waits where a test asks it to, to stand in for a write killed midway
//!
//! A bulk write is not atomic, so a process killed partway leaves some of its rows written. A
//! write that fails partway leaves the same rows, which the store neither cleans up nor hides,
//! so the next write must recover from both the same way.

use reef_model::PlanName;
use reef_service::schema::{
    CommentRow, DecisionRow, EditRow, LoopRow, NodeKey, NodeRow, PageKey, PageRow, PlanRow,
    ProjectRow, QueueRow, RemoteRow, RevisionFileRow, RevisionRow, TokenRow, UserRow, WorkRow,
};
use reef_service::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// A point in a write where it waits, so a test can act while the write holds its plan's lock
#[derive(Debug, Default)]
pub struct Pause {
    /// Notified when the write has reached the point: a flip with its rows in, or a node write
    pub reached: Notify,
    /// Notified to let the write go on
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
    /// Wait here before the next write of one node row
    node_pause: Mutex<Option<Arc<Pause>>>,
    /// Let this many writes of one node row through, then refuse the next as moved
    node_writes_left: Mutex<Option<usize>>,
    /// Fail the next delete of a queue row, before deleting it
    fail_queue_delete: AtomicBool,
    /// Let this many writes of a queue row through, then fail the next before writing it
    queue_inserts_left: Mutex<Option<usize>>,
    /// Fail the next bulk write of page rows after writing this many of them
    page_rows_left: Mutex<Option<usize>>,
    /// Fail the next delete of page rows, before deleting any
    fail_page_prune: AtomicBool,
    /// Refuse the next write of a work row as moved, as another writer having written would
    refuse_work_write: AtomicBool,
    /// Refuse the next write of one page row as moved, without writing it
    refuse_page_write: AtomicBool,
    /// Fail the next write of a revision row, before writing it, its files already written
    fail_revision_write: AtomicBool,
    /// Refuse the next write of a comment row as moved, as another writer having written would
    refuse_comment_write: AtomicBool,
    /// Refuse the next write of a loop row as moved, as another writer having written would
    refuse_loop_write: AtomicBool,
    /// Fail the next insert of a project row, before writing it, as a registration killed after
    /// its remote's row would be
    fail_project_insert: AtomicBool,
    /// Fail the next write of edit rows, before writing any, as a history that couldn't keep
    fail_edit_write: AtomicBool,
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

    /// Makes the next write of one node row wait, and returns where to hear it is waiting and
    /// to let it go
    pub fn pause_next_node_write(&self) -> Arc<Pause> {
        let pause = Arc::new(Pause::default());
        *self.node_pause.lock().unwrap() = Some(pause.clone());
        pause
    }

    /// Lets `writes` writes of one node row through, then refuses the next as another process's
    /// write would, without writing it
    ///
    /// # Arguments
    ///
    /// * `writes` - How many node writes to let through first
    pub fn refuse_node_writes_after(&self, writes: usize) {
        *self.node_writes_left.lock().unwrap() = Some(writes);
    }

    /// Fails the next delete of a queue row before deleting it, as a move killed between writing
    /// its new row and deleting its old one would
    pub fn fail_next_queue_delete(&self) {
        self.fail_queue_delete.store(true, Ordering::SeqCst);
    }

    /// Refuses the next write of a work row as moved, as a write from another process between
    /// the store's read and its write would be
    pub fn refuse_next_work_write(&self) {
        self.refuse_work_write.store(true, Ordering::SeqCst);
    }

    /// Fails the next write of a revision row before it is written, as a capture killed after
    /// its files would leave it
    pub fn fail_next_revision_write(&self) {
        self.fail_revision_write.store(true, Ordering::SeqCst);
    }

    /// Fails the next write of edit rows before any is written, as a history the store couldn't
    /// keep after the write it records landed
    pub fn fail_next_edit_write(&self) {
        self.fail_edit_write.store(true, Ordering::SeqCst);
    }

    /// Refuses the next write of a comment row as moved, as a write from another process between
    /// the store's read and its write would be
    pub fn refuse_next_comment_write(&self) {
        self.refuse_comment_write.store(true, Ordering::SeqCst);
    }

    /// Refuses the next write of a loop row as moved, as a write from another process between
    /// the store's read and its write would be
    pub fn refuse_next_loop_write(&self) {
        self.refuse_loop_write.store(true, Ordering::SeqCst);
    }

    /// Fails the next insert of a project row, before writing it, as a registration killed
    /// between its remote's row and its project's would leave it
    pub fn fail_next_project_insert(&self) {
        self.fail_project_insert.store(true, Ordering::SeqCst);
    }

    /// Lets `writes` writes of a queue row through, then fails the next before writing it
    ///
    /// # Arguments
    ///
    /// * `writes` - How many queue row writes to let through first
    pub fn fail_queue_inserts_after(&self, writes: usize) {
        *self.queue_inserts_left.lock().unwrap() = Some(writes);
    }

    /// Fails the next bulk write of page rows once it has written `rows` of them
    ///
    /// # Arguments
    ///
    /// * `rows` - How many page rows to write before failing
    pub fn fail_page_rows_after(&self, rows: usize) {
        *self.page_rows_left.lock().unwrap() = Some(rows);
    }

    /// Fails the next delete of page rows, before deleting any
    pub fn fail_next_page_prune(&self) {
        self.fail_page_prune.store(true, Ordering::SeqCst);
    }

    /// Refuses the next write of one page row as another process's write would, without writing
    /// it
    pub fn refuse_next_page_write(&self) {
        self.refuse_page_write.store(true, Ordering::SeqCst);
    }

    /// Whether the next write of a queue row fails, counting it against the fault armed
    fn fail_queue_insert(&self) -> bool {
        // only an armed fault fails, once its writes have been let through
        let mut left = self.queue_inserts_left.lock().unwrap();
        match left.as_mut() {
            Some(0) => {
                *left = None;
                true
            }
            Some(writes) => {
                *writes -= 1;
                false
            }
            None => false,
        }
    }

    /// Whether the next write of one node row is refused, counting it against the fault armed
    fn refuse_node_write(&self) -> bool {
        // only an armed fault refuses, once its writes have been let through
        let mut left = self.node_writes_left.lock().unwrap();
        match left.as_mut() {
            Some(0) => {
                *left = None;
                true
            }
            Some(writes) => {
                *writes -= 1;
                false
            }
            None => false,
        }
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
        // a fault armed for this write fails it before anything is written
        if self
            .faults
            .fail_project_insert
            .swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("killed before registering project `{}`", row.name);
        }
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

    /// Writes one node row, if its condition holds, waiting or refusing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_node(&self, row: NodeRow, expect: Expect) -> anyhow::Result<Written> {
        // say the write has come, and wait to be let go, with no lock held across the wait
        let pause = self.faults.node_pause.lock().unwrap().take();
        if let Some(pause) = pause {
            pause.reached.notify_one();
            pause.resume.notified().await;
        }
        // a fault armed for this write refuses it, as another process having written would
        if self.faults.refuse_node_write() {
            return Ok(Written::Refused(Refusal::Moved));
        }
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

    /// Reads a revision's page rows
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `path` - The one page to read, or `None` for all of them
    async fn pages(&self, key: &RevisionKey, path: Option<&str>) -> anyhow::Result<Vec<PageRow>> {
        self.inner.pages(key, path).await
    }

    /// Writes one page row, if its condition holds, refusing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_page(&self, row: PageRow, expect: Expect) -> anyhow::Result<Written> {
        // a fault armed for this write refuses it, as another process having written would
        if self.faults.refuse_page_write.swap(false, Ordering::SeqCst) {
            return Ok(Written::Refused(Refusal::Moved));
        }
        self.inner.write_page(row, expect).await
    }

    /// Lists a revision's page keys
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    async fn page_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<PageKey>> {
        self.inner.page_keys(key).await
    }

    /// Writes page rows, failing partway if asked to
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_pages(&self, mut rows: Vec<PageRow>) -> anyhow::Result<()> {
        // a fault armed for this write lets only so many rows through
        let Some(left) = self.faults.page_rows_left.lock().unwrap().take() else {
            return self.inner.write_pages(rows).await;
        };
        rows.truncate(left);
        self.inner.write_pages(rows).await?;
        anyhow::bail!("killed after writing {left} page rows")
    }

    /// Deletes page rows, failing before any if asked to
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `paths` - The paths of the pages to delete
    async fn delete_pages(&self, key: &RevisionKey, paths: Vec<String>) -> anyhow::Result<()> {
        // a fault armed for this delete stops it before anything is deleted
        if self.faults.fail_page_prune.swap(false, Ordering::SeqCst) {
            anyhow::bail!("killed before pruning pages");
        }
        self.inner.delete_pages(key, paths).await
    }

    /// Lists a project's queue rows
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    async fn queue(&self, project: &str) -> anyhow::Result<Vec<QueueRow>> {
        self.inner.queue(project).await
    }

    /// Writes a queue row only if none is stored under its rank, failing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    async fn insert_queue(&self, row: QueueRow) -> anyhow::Result<Written> {
        // a fault armed for this write stops it before it is written
        if self.faults.fail_queue_insert() {
            anyhow::bail!("killed before writing queue row `{}`", row.rank);
        }
        self.inner.insert_queue(row).await
    }

    /// Deletes a queue row if it is still the one read, failing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    async fn delete_queue(&self, row: &QueueRow) -> anyhow::Result<Written> {
        // a fault armed for this delete stops it before anything is deleted
        if self.faults.fail_queue_delete.swap(false, Ordering::SeqCst) {
            anyhow::bail!("killed before deleting queue row `{}`", row.rank);
        }
        self.inner.delete_queue(row).await
    }

    /// Reads a project's work rows, or one node's
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The one node's name within its project, or `None` for all of them
    async fn work(&self, project: &str, node: Option<&str>) -> anyhow::Result<Vec<WorkRow>> {
        self.inner.work(project, node).await
    }

    /// Writes one work row, if its condition holds, refusing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_work(&self, row: WorkRow, expect: Expect) -> anyhow::Result<Written> {
        // a fault armed for this write refuses it, as another process having written would
        if self.faults.refuse_work_write.swap(false, Ordering::SeqCst) {
            return Ok(Written::Refused(Refusal::Moved));
        }
        self.inner.write_work(row, expect).await
    }

    /// Reads a node's revision rows, or one of them
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's `plan/id`
    /// * `revision` - The one revision's sort key, or `None` for all of them
    async fn revisions(
        &self,
        project: &str,
        node: &str,
        revision: Option<&str>,
    ) -> anyhow::Result<Vec<RevisionRow>> {
        self.inner.revisions(project, node, revision).await
    }

    /// Writes one revision row, if its condition holds, failing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_revision(&self, row: RevisionRow, expect: Expect) -> anyhow::Result<Written> {
        // a fault armed for this write stops it before it is written
        if self
            .faults
            .fail_revision_write
            .swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("killed before writing revision `{}`", row.revision);
        }
        self.inner.write_revision(row, expect).await
    }

    /// Reads a revision's file rows, or one of them
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's `plan/id`
    /// * `revision` - The revision's number
    /// * `path` - The one file's path, or `None` for all of them
    async fn revision_files(
        &self,
        project: &str,
        node: &str,
        revision: u64,
        path: Option<&str>,
    ) -> anyhow::Result<Vec<RevisionFileRow>> {
        self.inner
            .revision_files(project, node, revision, path)
            .await
    }

    /// Writes revision file rows over whatever is stored under their keys
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_revision_files(&self, rows: Vec<RevisionFileRow>) -> anyhow::Result<()> {
        self.inner.write_revision_files(rows).await
    }

    /// Reads a plan's comment rows, or one of them
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `plan` - The plan's name
    /// * `number` - The one comment's sort key, or `None` for all of them
    async fn comments(
        &self,
        project: &str,
        plan: &str,
        number: Option<&str>,
    ) -> anyhow::Result<Vec<CommentRow>> {
        self.inner.comments(project, plan, number).await
    }

    /// Writes one comment row, if its condition holds, refusing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_comment(&self, row: CommentRow, expect: Expect) -> anyhow::Result<Written> {
        // a fault armed for this write refuses it, as another process having written would
        if self
            .faults
            .refuse_comment_write
            .swap(false, Ordering::SeqCst)
        {
            return Ok(Written::Refused(Refusal::Moved));
        }
        self.inner.write_comment(row, expect).await
    }

    /// Reads every loop row, or one pane's
    ///
    /// # Arguments
    ///
    /// * `pane` - The one pane's key, or `None` for every loop
    async fn loops(&self, pane: Option<&str>) -> anyhow::Result<Vec<LoopRow>> {
        self.inner.loops(pane).await
    }

    /// Writes one loop row, if its condition holds, refusing first if asked to
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_loop(&self, row: LoopRow, expect: Expect) -> anyhow::Result<Written> {
        // a fault armed for this write refuses it, as another process having written would
        if self.faults.refuse_loop_write.swap(false, Ordering::SeqCst) {
            return Ok(Written::Refused(Refusal::Moved));
        }
        self.inner.write_loop(row, expect).await
    }

    /// Deletes one loop row, if it is still at the version read
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    async fn delete_loop(&self, row: &LoopRow) -> anyhow::Result<Written> {
        self.inner.delete_loop(row).await
    }

    /// Reads a node's decision rows
    ///
    /// # Arguments
    ///
    /// * `project` - The project the node's plan belongs to
    /// * `node` - The node's name within its project, `plan/id`
    async fn decisions(&self, project: &str, node: &str) -> anyhow::Result<Vec<DecisionRow>> {
        self.inner.decisions(project, node).await
    }

    /// Writes one decision row, if its condition holds
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_decision(&self, row: DecisionRow, expect: Expect) -> anyhow::Result<Written> {
        self.inner.write_decision(row, expect).await
    }

    /// Reads every user row, or the one of a name
    ///
    /// # Arguments
    ///
    /// * `name` - The one user's name, or `None` for every user
    async fn users(&self, name: Option<&str>) -> anyhow::Result<Vec<UserRow>> {
        self.inner.users(name).await
    }

    /// Writes one user row, if its condition holds
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_user(&self, row: UserRow, expect: Expect) -> anyhow::Result<Written> {
        self.inner.write_user(row, expect).await
    }

    /// Deletes one user row, if it is still at the version read
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    async fn delete_user(&self, row: &UserRow) -> anyhow::Result<Written> {
        self.inner.delete_user(row).await
    }

    /// Reads every token row
    async fn tokens(&self) -> anyhow::Result<Vec<TokenRow>> {
        self.inner.tokens().await
    }

    /// Reads the token row kept under a hash
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's SHA-256
    async fn token(&self, hash: &str) -> anyhow::Result<Option<TokenRow>> {
        self.inner.token(hash).await
    }

    /// Writes one token row, if its condition holds
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_token(&self, row: TokenRow, expect: Expect) -> anyhow::Result<Written> {
        self.inner.write_token(row, expect).await
    }

    /// Deletes one token row, if it is still at the version read
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    async fn delete_token(&self, row: &TokenRow) -> anyhow::Result<Written> {
        self.inner.delete_token(row).await
    }

    /// Reads every remote row
    async fn remotes(&self) -> anyhow::Result<Vec<RemoteRow>> {
        self.inner.remotes().await
    }

    /// Writes one remote row, if its condition holds
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key
    async fn write_remote(&self, row: RemoteRow, expect: Expect) -> anyhow::Result<Written> {
        self.inner.write_remote(row, expect).await
    }

    /// Reads a subject's edit rows, newest first
    ///
    /// # Arguments
    ///
    /// * `project` - The project the subject belongs to
    /// * `subject` - What was edited
    /// * `limit` - The most rows to read
    async fn edits(
        &self,
        project: &str,
        subject: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<EditRow>> {
        self.inner.edits(project, subject, limit).await
    }

    /// Writes edit rows, failing first if asked to
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_edits(&self, rows: Vec<EditRow>) -> anyhow::Result<()> {
        // a fault armed for this write stops it before any row is written
        if self.faults.fail_edit_write.swap(false, Ordering::SeqCst) {
            anyhow::bail!("killed before writing {} edit rows", rows.len());
        }
        self.inner.write_edits(rows).await
    }
}
