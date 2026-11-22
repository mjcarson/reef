//! The store's in-memory twin: reef's rows kept the way shoal keeps them, with no shoal
//!
//! It builds on stable and starts at once, so the service's tests can run without a pool. It
//! answers every read and conditional write as shoal does, which the shared suite checks by
//! running each case against both.

use reef_model::PlanName;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use crate::schema::PROJECTS;
use crate::schema::{NodeKey, NodeRow, PageKey, PageRow, PlanRow, ProjectRow, QueueRow, WorkRow};

/// Every table's partitions, each sorted by its rows' sort keys
#[derive(Debug, Default)]
struct Tables {
    /// Every project, by the partition it is listed in and then by name
    projects: BTreeMap<String, BTreeMap<String, ProjectRow>>,
    /// Each project's plans, by plan name
    plans: BTreeMap<String, BTreeMap<String, PlanRow>>,
    /// Each revision's nodes, by id
    nodes: BTreeMap<RevisionKey, BTreeMap<String, NodeRow>>,
    /// Each project's queue, by rank
    queues: BTreeMap<String, BTreeMap<String, QueueRow>>,
    /// Each revision's pages, by path
    pages: BTreeMap<RevisionKey, BTreeMap<String, PageRow>>,
    /// Each project's work records, by node
    work: BTreeMap<String, BTreeMap<String, WorkRow>>,
}

/// reef's rows in memory, shared by every clone of this
///
/// A clone reads and writes the same rows, so a new store over a clone is a restart that kept
/// the data.
#[derive(Debug, Clone, Default)]
pub struct MemoryBackend {
    /// The rows, behind one lock
    tables: Arc<Mutex<Tables>>,
}

impl MemoryBackend {
    /// An empty twin
    pub fn new() -> MemoryBackend {
        MemoryBackend::default()
    }

    /// Locks the rows
    fn tables(&self) -> MutexGuard<'_, Tables> {
        // a panic while holding the lock leaves the rows as a test left them, which is fine
        self.tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Stores a node row in a partition other than its own, as a hash collision would
    ///
    /// Shoal hashes partition keys to a `u64`, and two keys that hash the same share a
    /// partition, silently. This puts a row where a collision would, so a test can show the
    /// store catches it.
    ///
    /// # Arguments
    ///
    /// * `partition` - The revision whose partition the row lands in
    /// * `row` - The row, keyed by its own fields as any other
    pub fn plant(&self, partition: &RevisionKey, row: NodeRow) {
        // under the partition named, at the row's own sort key
        self.tables()
            .nodes
            .entry(partition.clone())
            .or_default()
            .insert(row.id.clone(), row);
    }

    /// Stores a page row in a partition other than its own, as a hash collision would
    ///
    /// # Arguments
    ///
    /// * `partition` - The revision whose partition the row lands in
    /// * `row` - The row, keyed by its own path as any other
    pub fn plant_page(&self, partition: &RevisionKey, row: PageRow) {
        // under the partition named, at the row's own sort key
        self.tables()
            .pages
            .entry(partition.clone())
            .or_default()
            .insert(row.path.clone(), row);
    }

    /// Stores a queue row in a project's partition other than its own, as a hash collision would
    ///
    /// # Arguments
    ///
    /// * `partition` - The project whose partition the row lands in
    /// * `row` - The row, keyed by its own rank as any other
    pub fn plant_queue(&self, partition: &str, row: QueueRow) {
        // under the partition named, at the row's own sort key
        self.tables()
            .queues
            .entry(partition.to_string())
            .or_default()
            .insert(row.rank.clone(), row);
    }

    /// Stores a work row in a project's partition other than its own, as a hash collision would
    ///
    /// # Arguments
    ///
    /// * `partition` - The project whose partition the row lands in
    /// * `row` - The row, keyed by its own node as any other
    pub fn plant_work(&self, partition: &str, row: WorkRow) {
        // under the partition named, at the row's own sort key
        self.tables()
            .work
            .entry(partition.to_string())
            .or_default()
            .insert(row.node.clone(), row);
    }
}

/// Checks a write's condition against what is stored under its key
///
/// # Arguments
///
/// * `stored` - The version of the row stored under the key, if one is
/// * `expect` - What the write expects
fn holds(stored: Option<u64>, expect: Expect) -> Result<(), Refusal> {
    match (stored, expect) {
        // nothing stored, as expected
        (None, Expect::Absent) => Ok(()),
        // something stored where nothing was expected
        (Some(_), Expect::Absent) => Err(Refusal::Exists),
        // nothing stored where a row was expected
        (None, Expect::Version(_)) => Err(Refusal::Missing),
        // a row at the version expected, or at another
        (Some(version), Expect::Version(read)) if version == read => Ok(()),
        (Some(_), Expect::Version(_)) => Err(Refusal::Moved),
    }
}

impl Backend for MemoryBackend {
    /// Lists the project rows stored in the one partition every project is listed in
    async fn projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        // the listing partition, in name order
        Ok(self
            .tables()
            .projects
            .get(PROJECTS)
            .into_iter()
            .flat_map(|partition| partition.values().cloned())
            .collect())
    }

    /// Writes a project's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The project's row
    async fn insert_project(&self, row: ProjectRow) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        let partition = tables.projects.entry(row.list.clone()).or_default();
        // refused if a row is stored under its key, as `if_absent` is
        if partition.contains_key(&row.name) {
            return Ok(Written::Refused(Refusal::Exists));
        }
        partition.insert(row.name.clone(), row);
        Ok(Written::Applied)
    }

    /// Lists the plan rows stored in a project's partition, in name order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanRow>> {
        // the project's partition, in name order
        Ok(self
            .tables()
            .plans
            .get(project)
            .into_iter()
            .flat_map(|partition| partition.values().cloned())
            .collect())
    }

    /// Gets the row stored under a plan's key, if one is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    async fn plan(&self, plan: &PlanName) -> anyhow::Result<Option<PlanRow>> {
        // the row at the plan's name in its project's partition
        Ok(self
            .tables()
            .plans
            .get(&plan.project)
            .and_then(|partition| partition.get(&plan.plan))
            .cloned())
    }

    /// Writes a plan's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The plan's row
    async fn insert_plan(&self, row: PlanRow) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        let partition = tables.plans.entry(row.project.clone()).or_default();
        // refused if a row is stored under its key, as `if_absent` is
        if partition.contains_key(&row.name) {
            return Ok(Written::Refused(Refusal::Exists));
        }
        partition.insert(row.name.clone(), row);
        Ok(Written::Applied)
    }

    /// Reads the node rows stored in a revision's partition, in id order, or the one under an id
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `id` - The one node to read, or `None` for all of them
    async fn nodes(&self, key: &RevisionKey, id: Option<&str>) -> anyhow::Result<Vec<NodeRow>> {
        let tables = self.tables();
        // a partition that holds nothing reads as no rows
        let Some(partition) = tables.nodes.get(key) else {
            return Ok(Vec::new());
        };
        // the one row asked for, or every row in sort key order
        Ok(match id {
            Some(id) => partition.get(id).cloned().into_iter().collect(),
            None => partition.values().cloned().collect(),
        })
    }

    /// Writes one node row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    async fn write_node(&self, row: NodeRow, expect: Expect) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        // the row's own partition, as its keys hash
        let key = RevisionKey {
            project: row.project.clone(),
            plan: row.plan.clone(),
            revision: row.revision,
        };
        let partition = tables.nodes.entry(key).or_default();
        // judge the condition against the row stored under its sort key
        let stored = partition.get(&row.id).map(|stored| stored.version);
        if let Err(refusal) = holds(stored, expect) {
            return Ok(Written::Refused(refusal));
        }
        partition.insert(row.id.clone(), row);
        Ok(Written::Applied)
    }

    /// Changes a plan's pointers, if its row is still at the version read
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
        let mut tables = self.tables();
        let row = tables
            .plans
            .get_mut(&plan.project)
            .and_then(|partition| partition.get_mut(&plan.plan));
        // judge the condition against the row stored under the plan's key
        if let Err(refusal) = holds(row.as_ref().map(|row| row.version), Expect::Version(read)) {
            return Ok(Written::Refused(refusal));
        }
        let row = row.expect("a row holding the condition is stored");
        // change only what the update names, and move the version on
        if let Some(current) = pointers.current {
            row.current = current;
        }
        if let Some(pending) = pointers.pending {
            row.pending = pending;
        }
        if let Some(retired) = pointers.retired {
            row.retired = retired;
        }
        if let Some(pages_current) = pointers.pages_current {
            row.pages_current = pages_current;
        }
        if let Some(pages_retired) = pointers.pages_retired {
            row.pages_retired = pages_retired;
        }
        row.version = read + 1;
        Ok(Written::Applied)
    }

    /// Lists the keys and versions stored in a revision's partition, without their specs
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    async fn node_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<NodeKey>> {
        // every row in the partition, projected as shoal projects it
        let tables = self.tables();
        Ok(tables
            .nodes
            .get(key)
            .into_iter()
            .flat_map(|partition| partition.values())
            .map(|row| NodeKey {
                project: row.project.clone(),
                plan: row.plan.clone(),
                revision: row.revision,
                id: row.id.clone(),
                version: row.version,
            })
            .collect())
    }

    /// Writes node rows over whatever is stored under their keys
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_nodes(&self, rows: Vec<NodeRow>) -> anyhow::Result<()> {
        let mut tables = self.tables();
        // each into its own partition, as its keys hash
        for row in rows {
            let key = RevisionKey {
                project: row.project.clone(),
                plan: row.plan.clone(),
                revision: row.revision,
            };
            tables
                .nodes
                .entry(key)
                .or_default()
                .insert(row.id.clone(), row);
        }
        Ok(())
    }

    /// Deletes nodes from a revision's partition
    ///
    /// A delete of a row that isn't stored deletes nothing, as shoal's does.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `ids` - The ids of the nodes to delete
    async fn delete_nodes(&self, key: &RevisionKey, ids: Vec<String>) -> anyhow::Result<()> {
        let mut tables = self.tables();
        if let Some(partition) = tables.nodes.get_mut(key) {
            // each row by its sort key
            for id in ids {
                partition.remove(&id);
            }
            // and a partition left empty is gone, as it never existed
            if partition.is_empty() {
                tables.nodes.remove(key);
            }
        }
        Ok(())
    }

    /// Reads the page rows stored in a revision's partition, in path order, or the one at a path
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `path` - The one page to read, or `None` for all of them
    async fn pages(&self, key: &RevisionKey, path: Option<&str>) -> anyhow::Result<Vec<PageRow>> {
        let tables = self.tables();
        // a partition that holds nothing reads as no rows
        let Some(partition) = tables.pages.get(key) else {
            return Ok(Vec::new());
        };
        // the one row asked for, or every row in sort key order
        Ok(match path {
            Some(path) => partition.get(path).cloned().into_iter().collect(),
            None => partition.values().cloned().collect(),
        })
    }

    /// Writes one page row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    async fn write_page(&self, row: PageRow, expect: Expect) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        // the row's own partition, as its keys hash
        let key = RevisionKey {
            project: row.project.clone(),
            plan: row.plan.clone(),
            revision: row.revision,
        };
        let partition = tables.pages.entry(key).or_default();
        // judge the condition against the row stored under its sort key
        let stored = partition.get(&row.path).map(|stored| stored.version);
        if let Err(refusal) = holds(stored, expect) {
            return Ok(Written::Refused(refusal));
        }
        partition.insert(row.path.clone(), row);
        Ok(Written::Applied)
    }

    /// Lists the paths and versions stored in a revision's partition of pages, without their text
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    async fn page_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<PageKey>> {
        // every row in the partition, projected as shoal projects it
        let tables = self.tables();
        Ok(tables
            .pages
            .get(key)
            .into_iter()
            .flat_map(|partition| partition.values())
            .map(|row| PageKey {
                project: row.project.clone(),
                plan: row.plan.clone(),
                revision: row.revision,
                path: row.path.clone(),
                version: row.version,
            })
            .collect())
    }

    /// Writes page rows over whatever is stored under their keys
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    async fn write_pages(&self, rows: Vec<PageRow>) -> anyhow::Result<()> {
        let mut tables = self.tables();
        // each into its own partition, as its keys hash
        for row in rows {
            let key = RevisionKey {
                project: row.project.clone(),
                plan: row.plan.clone(),
                revision: row.revision,
            };
            tables
                .pages
                .entry(key)
                .or_default()
                .insert(row.path.clone(), row);
        }
        Ok(())
    }

    /// Deletes pages from a revision's partition
    ///
    /// A delete of a row that isn't stored deletes nothing, as shoal's does.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `paths` - The paths of the pages to delete
    async fn delete_pages(&self, key: &RevisionKey, paths: Vec<String>) -> anyhow::Result<()> {
        let mut tables = self.tables();
        if let Some(partition) = tables.pages.get_mut(key) {
            // each row by its sort key
            for path in paths {
                partition.remove(&path);
            }
            // and a partition left empty is gone, as it never existed
            if partition.is_empty() {
                tables.pages.remove(key);
            }
        }
        Ok(())
    }

    /// Lists the queue rows stored in a project's partition, in rank order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    async fn queue(&self, project: &str) -> anyhow::Result<Vec<QueueRow>> {
        // the project's partition, in rank order
        Ok(self
            .tables()
            .queues
            .get(project)
            .into_iter()
            .flat_map(|partition| partition.values().cloned())
            .collect())
    }

    /// Writes a queue row only if none is stored under its rank
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    async fn insert_queue(&self, row: QueueRow) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        let partition = tables.queues.entry(row.project.clone()).or_default();
        // refused if a row is stored under its rank, as `if_absent` is
        if partition.contains_key(&row.rank) {
            return Ok(Written::Refused(Refusal::Exists));
        }
        partition.insert(row.rank.clone(), row);
        Ok(Written::Applied)
    }

    /// Deletes a queue row, only if the row stored under its rank is still this one
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    async fn delete_queue(&self, row: &QueueRow) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        let Some(partition) = tables.queues.get_mut(&row.project) else {
            return Ok(Written::Refused(Refusal::Missing));
        };
        // judged as `if_matches` judges it: a row must be stored, and pass the filter
        match partition.get(&row.rank) {
            None => return Ok(Written::Refused(Refusal::Missing)),
            Some(stored)
                if (&stored.plan, &stored.id, stored.seq) != (&row.plan, &row.id, row.seq) =>
            {
                return Ok(Written::Refused(Refusal::Moved));
            }
            Some(_) => {}
        }
        partition.remove(&row.rank);
        // and a partition left empty is gone, as it never existed
        if partition.is_empty() {
            tables.queues.remove(&row.project);
        }
        Ok(Written::Applied)
    }

    /// Reads the work rows stored in a project's partition, in node order, or the one of a node
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The one node's name within its project, or `None` for all of them
    async fn work(&self, project: &str, node: Option<&str>) -> anyhow::Result<Vec<WorkRow>> {
        let tables = self.tables();
        // the project's partition, whole or the one row of it
        let Some(partition) = tables.work.get(project) else {
            return Ok(Vec::new());
        };
        Ok(match node {
            Some(node) => partition.get(node).cloned().into_iter().collect(),
            None => partition.values().cloned().collect(),
        })
    }

    /// Writes one work row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    async fn write_work(&self, row: WorkRow, expect: Expect) -> anyhow::Result<Written> {
        let mut tables = self.tables();
        // the row's own partition, as its key hashes
        let partition = tables.work.entry(row.project.clone()).or_default();
        // judge the condition against the row stored under its sort key
        let stored = partition.get(&row.node).map(|stored| stored.version);
        if let Err(refusal) = holds(stored, expect) {
            return Ok(Written::Refused(refusal));
        }
        partition.insert(row.node.clone(), row);
        Ok(Written::Applied)
    }
}
