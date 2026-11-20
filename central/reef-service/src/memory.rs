//! The store's in-memory twin: reef's rows kept the way shoal keeps them, with no shoal
//!
//! It builds on stable and starts at once, so the service's tests can run without a pool. It
//! answers every read and conditional write as shoal does, which the shared suite checks by
//! running each case against both.

use reef_model::PlanName;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{Backend, Expect, Refusal, RevisionKey, Written};
use crate::schema::{NodeRow, PlanRow};

/// Every table's partitions, each sorted by its rows' sort keys
#[derive(Debug, Default)]
struct Tables {
    /// Each project's plans, by plan name
    plans: BTreeMap<String, BTreeMap<String, PlanRow>>,
    /// Each revision's nodes, by id
    nodes: BTreeMap<RevisionKey, BTreeMap<String, NodeRow>>,
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
}
