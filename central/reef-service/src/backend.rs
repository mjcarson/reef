//! What a store needs of the rows under it: reads by key, and writes that hold a condition
//!
//! A [`Store`](crate::Store) keeps the logic: the locks, versions, revisions and full-key checks.
//! A backend only reads and writes rows the way shoal does, so the store's logic runs the same
//! over shoal and over [`MemoryBackend`](crate::MemoryBackend), its in-memory twin, which builds
//! on stable. One test suite runs against both, so shoal is the twin's oracle.

use reef_model::PlanName;
use std::future::Future;

use crate::schema::{NodeRow, PlanRow};

/// Why a conditional write was refused, as shoal says it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The write expected no row, and one is stored
    Exists,
    /// The write expected a row, and none is stored
    Missing,
    /// The write expected a row at a version, and the stored row is at another
    Moved,
}

impl Refusal {
    /// Name this refusal the way a person reads it
    pub fn describe(self) -> &'static str {
        match self {
            Refusal::Exists => "it already exists",
            Refusal::Missing => "it does not exist",
            Refusal::Moved => "it changed since it was read",
        }
    }
}

impl std::fmt::Display for Refusal {
    /// Shows the refusal as the phrase that describes it
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

/// What came of a conditional write
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// It was written
    Applied,
    /// Its condition did not hold, so nothing was written
    Refused(Refusal),
}

/// The condition a write holds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// No row is stored under its key
    Absent,
    /// A row is stored under its key, at this version
    Version(u64),
}

/// The key of one revision of a plan: the partition its nodes sit in
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RevisionKey {
    /// The project the plan belongs to
    pub project: String,
    /// The plan's name within its project
    pub plan: String,
    /// The revision
    pub revision: u64,
}

impl RevisionKey {
    /// The key of one revision of a plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision
    pub fn new(plan: &PlanName, revision: u64) -> RevisionKey {
        RevisionKey {
            project: plan.project.clone(),
            plan: plan.plan.clone(),
            revision,
        }
    }

    /// Whether a node row is part of this revision
    ///
    /// # Arguments
    ///
    /// * `row` - The row to check
    pub fn holds(&self, row: &NodeRow) -> bool {
        row.project == self.project && row.plan == self.plan && row.revision == self.revision
    }
}

/// Reads and writes reef's rows the way shoal does
///
/// Reads return what is stored under a key's partition, unchecked: a partition key whose hash
/// collides with another's can hold the other's rows, so the store checks every row against
/// the full key it asked for. A read of a partition that holds nothing is empty, never an error.
pub trait Backend: Send + Sync {
    /// Gets the row stored under a plan's key, if one is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    fn plan(&self, plan: &PlanName)
    -> impl Future<Output = anyhow::Result<Option<PlanRow>>> + Send;

    /// Writes a plan's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The plan's row
    fn insert_plan(&self, row: PlanRow) -> impl Future<Output = anyhow::Result<Written>> + Send;

    /// Reads the node rows stored in a revision's partition, in id order, or the one under an id
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `id` - The one node to read, or `None` for all of them
    fn nodes(
        &self,
        key: &RevisionKey,
        id: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<Vec<NodeRow>>> + Send;

    /// Writes one node row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    fn write_node(
        &self,
        row: NodeRow,
        expect: Expect,
    ) -> impl Future<Output = anyhow::Result<Written>> + Send;
}
