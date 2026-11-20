//! reef's store: plans and their nodes, read and written by version
//!
//! A store keeps the logic, over a [`Backend`] that reads and writes rows the way shoal does:
//! [`ShoalBackend`] in service, [`MemoryBackend`] in tests and on stable.
//!
//! - **Versions by conditional write.** Each node row carries a version, and a put names the one
//!   it read. A stale put is refused as [`Put::Stale`], an answer and not a failure.
//! - **Writes to a plan serialized.** reef-service is shoal's only writer, so a lock per plan in
//!   this process keeps a write out of a revision another write is about to retire. Reads take
//!   the same lock shared, so a read never sees a revision part-pruned.
//! - **Every row checked against its full key.** Shoal hashes partition keys to a `u64`, and a
//!   collision is silent, so a row from another partition is caught rather than served.
//! - **Whole-plan writes as revisions.** Shoal has no transactions, so a bulk write goes to a new
//!   revision's partition, named `pending` on the plan's row first, and is made current by one
//!   conditional write to that row. A write killed midway leaves the old revision current and
//!   whole, and the next write prunes what it left.
//!
//! A process keeps one store over shoal. `Store::shared` opens it once however often a caller
//! asks, since `dioxus::serve` runs its setup again after every hot-patch, and a second pool
//! would find the data directory locked.

use reef_model::{NodeSpec, Origin, PlanName, Record};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::{Arc, Mutex};
use tokio::sync::{OnceCell, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::instrument;

use crate::backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;
use crate::memory::MemoryBackend;
use crate::schema::{NodeRow, PlanRow};

/// The process's one store over shoal, opened on first use
static SHARED: OnceCell<Store> = OnceCell::const_new();

/// How many node rows a bulk write sends in one bundle
const BUNDLE_ROWS: usize = 256;

/// A node as a reader got it from the store
#[derive(Debug, Clone, PartialEq)]
pub struct StoredNode {
    /// Its spec, with no inline members
    pub spec: NodeSpec,
    /// Its body as markdown: a file node's page, empty for one that was listed inline
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
    /// The version it was read at, which a put of it names
    pub version: u64,
}

/// A node as a writer gives it to the store
#[derive(Debug, Clone, PartialEq)]
pub struct NodeWrite {
    /// Its spec, with no inline members: each member is a node of its own
    pub spec: NodeSpec,
    /// Its body as markdown
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
}

impl NodeWrite {
    /// A node with only an id, an empty body and an order
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    /// * `order` - Where it falls in the order its plan was written in
    pub fn new<T: Into<String>>(id: T, order: u64) -> NodeWrite {
        NodeWrite {
            spec: NodeSpec {
                id: id.into(),
                ..NodeSpec::default()
            },
            body: String::new(),
            order,
        }
    }
}

/// A plan's current revision, as a reader got it
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    /// The revision that was current
    pub revision: u64,
    /// Its nodes, in the order the plan was written in
    pub nodes: Vec<StoredNode>,
}

impl Listing {
    /// The records these nodes make, each named in its plan, for `Graph::build`
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan these nodes were read from
    pub fn records(self, plan: &PlanName) -> anyhow::Result<Vec<Record>> {
        // each node becomes a stored record, named by its plan and id
        self.nodes
            .into_iter()
            .map(|node| {
                let name = plan.node(node.spec.id.as_str())?;
                Ok(Record {
                    spec: node.spec,
                    body: node.body,
                    order: node.order,
                    origin: Origin::Stored { name },
                })
            })
            .collect()
    }
}

/// What came of a put
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Put {
    /// The put was written, and the node is now at this version
    Applied {
        /// The node's new version
        version: u64,
    },
    /// The node was not as the put expected, so nothing was written
    Stale(Refusal),
}

/// What a whole-plan write did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rewritten {
    /// The revision it made current
    pub revision: u64,
    /// How many rows a killed or failed write had left behind, which it pruned first
    pub recovered: usize,
    /// Whether pruning the revision it replaced failed, leaving that to the next write
    pub left_retired: bool,
}

/// A plan the store was asked about that isn't listed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotListed(pub PlanName);

impl std::fmt::Display for NotListed {
    /// Names the plan
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plan `{}` is not listed", self.0)
    }
}

impl std::error::Error for NotListed {}

/// reef's store, over shoal or its in-memory twin
pub struct Store<B = ShoalBackend> {
    /// The rows under the store
    backend: B,
    /// Each plan's lock: shared by reads, held alone by writes
    locks: Mutex<HashMap<PlanName, Arc<RwLock<()>>>>,
}

impl Store<ShoalBackend> {
    /// Opens a store: the external node reef's config names, else a pool in this process
    ///
    /// Only one process can open a data directory, so a process opens one store; most callers
    /// want `shared`.
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config
    #[instrument(name = "Store::open", skip_all, err(Debug))]
    pub async fn open(conf: &Conf) -> anyhow::Result<Store> {
        // reach shoal however the config says to
        let backend = ShoalBackend::open(conf).await?;
        Ok(Store::new(backend))
    }

    /// Gets the process's store, opening it on first use
    ///
    /// Every later caller gets the store the first opened, whatever config it passes.
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config, used only by the first call
    #[instrument(name = "Store::shared", skip_all, err(Debug))]
    pub async fn shared(conf: &Conf) -> anyhow::Result<&'static Store> {
        // open it once, and hand every later caller the same one
        SHARED.get_or_try_init(|| Store::open(conf)).await
    }

    /// How this store reaches shoal
    pub fn mode(&self) -> Mode {
        self.backend.mode()
    }

    /// Closes the store, stopping its in-process pool if it started one
    #[instrument(name = "Store::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // the backend holds the client and the pool
        self.backend.close().await
    }
}

impl Store<MemoryBackend> {
    /// A store over an empty in-memory twin
    pub fn memory() -> Store<MemoryBackend> {
        Store::new(MemoryBackend::new())
    }
}

impl<B: Backend> Store<B> {
    /// A store over a backend
    ///
    /// # Arguments
    ///
    /// * `backend` - The rows under the store
    pub fn new(backend: B) -> Store<B> {
        Store {
            backend,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// The rows under this store
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Gives up the store, keeping the rows under it
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Finds a plan's lock, making it on first use
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan whose lock to find
    fn plan_lock(&self, plan: &PlanName) -> Arc<RwLock<()>> {
        // the map is only held to find or add the lock, never across a wait
        self.locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(plan.clone())
            .or_default()
            .clone()
    }

    /// Waits to read a plan: alongside other reads, never during a write
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to read
    async fn reading(&self, plan: &PlanName) -> OwnedRwLockReadGuard<()> {
        self.plan_lock(plan).read_owned().await
    }

    /// Waits to write a plan: alone
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to write
    async fn writing(&self, plan: &PlanName) -> OwnedRwLockWriteGuard<()> {
        self.plan_lock(plan).write_owned().await
    }

    /// Gets a plan's row, checked against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_row", skip_all, err(Debug))]
    async fn plan_row(&self, plan: &PlanName) -> anyhow::Result<PlanRow> {
        // a plan with no row is not listed, which a caller can tell from a failure
        let row = self
            .backend
            .plan(plan)
            .await?
            .ok_or_else(|| NotListed(plan.clone()))?;
        // a row of another project whose key hashed the same is a collision, not this plan
        anyhow::ensure!(
            row.project == plan.project && row.name == plan.plan,
            "plan row `{}/{}` was stored where `{plan}` is: their partition keys' hashes collide",
            row.project,
            row.name
        );
        Ok(row)
    }

    /// Lists a plan in its project, with an empty revision to put nodes into
    ///
    /// Returns whether it was added; a plan already listed is left as it is.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<bool> {
        // its first revision is current and empty, so a put has somewhere to go
        let row = PlanRow {
            project: plan.project.clone(),
            name: plan.plan.clone(),
            current: fresh_revision(),
            pending: 0,
            retired: 0,
            version: 1,
        };
        // written only if its project lists no plan of that name
        let _write = self.writing(plan).await;
        Ok(self.backend.insert_plan(row).await? == Written::Applied)
    }

    /// Gets one node of a plan's current revision
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    #[instrument(name = "Store::get", skip_all, err(Debug))]
    pub async fn get(&self, plan: &PlanName, id: &str) -> anyhow::Result<Option<StoredNode>> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // the one row of the current revision
        let nodes = self.revision(plan, row.current, Some(id)).await?;
        Ok(nodes.into_iter().next())
    }

    /// Lists a plan's current revision, in the order the plan was written in
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::list", skip_all, err(Debug))]
    pub async fn list(&self, plan: &PlanName) -> anyhow::Result<Listing> {
        // never while a write is retiring the revision this reads
        let _read = self.reading(plan).await;
        let row = self.plan_row(plan).await?;
        // every row of the current revision, put in written order
        let mut nodes = self.revision(plan, row.current, None).await?;
        nodes.sort_by(|a, b| (a.order, &a.spec.id).cmp(&(b.order, &b.spec.id)));
        Ok(Listing {
            revision: row.current,
            nodes,
        })
    }

    /// Reads a revision's rows, or one of them, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to read
    /// * `id` - The one node to read, or `None` for all of them
    #[instrument(name = "Store::revision", skip_all, err(Debug))]
    async fn revision(
        &self,
        plan: &PlanName,
        revision: u64,
        id: Option<&str>,
    ) -> anyhow::Result<Vec<StoredNode>> {
        // the revision's partition, whole or one row of it
        let key = RevisionKey::new(plan, revision);
        let rows = self.backend.nodes(&key, id).await?;
        rows.into_iter()
            .map(|row| {
                // a row of another partition that hashed the same is a collision, not a node
                anyhow::ensure!(
                    key.holds(&row) && id.is_none_or(|id| row.id == id),
                    "node row `{}/{}/{}` at revision {} was stored where `{plan}` at revision \
                     {revision} is: their partition keys' hashes collide",
                    row.project,
                    row.plan,
                    row.id,
                    row.revision
                );
                stored(row)
            })
            .collect()
    }

    /// Writes one node into a plan's current revision, if it is as the writer read it
    ///
    /// A put names the version it read the node at, or `None` for a node that must not exist
    /// yet. A node that has moved on, or isn't as expected, comes back as [`Put::Stale`].
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `node` - The node as it should be written
    /// * `read` - The version it was read at, or `None` for a new node
    #[instrument(name = "Store::put", skip_all, err(Debug))]
    pub async fn put(
        &self,
        plan: &PlanName,
        node: NodeWrite,
        read: Option<u64>,
    ) -> anyhow::Result<Put> {
        // a node with members inline would hide them from every read by id
        check_node(plan, &node)?;
        // one write to this plan at a time, so this never lands in a revision being retired
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // prune whatever a killed write left, so it never outlives the next write
        let (row, _) = self.recover(plan, row).await?;
        // the row as it should be, one version on from the one read
        let (version, expect) = match read {
            Some(read) => (read + 1, Expect::Version(read)),
            None => (1, Expect::Absent),
        };
        let new = node_row(plan, row.current, node, version)?;
        // written only over the version read, or only where there is no row
        Ok(match self.backend.write_node(new, expect).await? {
            Written::Applied => Put::Applied { version },
            Written::Refused(refusal) => Put::Stale(refusal),
        })
    }

    /// Writes a whole plan as a new revision, and makes it current in one write
    ///
    /// The steps, under the plan's lock:
    ///
    /// 1. Prune any revision a killed write left pending or retired.
    /// 2. Read the current revision's versions, so each node's moves on.
    /// 3. Reserve the new revision: name it `pending` on the plan's row.
    /// 4. Write its rows, each bundle's answers checked.
    /// 5. Flip: make it current and the old one retired, in one conditional write.
    /// 6. Prune the old revision, and clear `retired`.
    ///
    /// A write killed or failed before step 5 leaves the old revision current and whole, with
    /// the new one named pending for the next write to prune. One that fails pruning after step
    /// 5 has still made its revision current, and says so with `left_retired`.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `nodes` - Every node of the plan, each a node of its own
    #[instrument(name = "Store::write_plan", skip_all, err(Debug))]
    pub async fn write_plan(
        &self,
        plan: &PlanName,
        nodes: Vec<NodeWrite>,
    ) -> anyhow::Result<Rewritten> {
        // every node must be one the store can keep, under an id no other node has
        let mut ids = HashSet::with_capacity(nodes.len());
        for node in &nodes {
            check_node(plan, node)?;
            anyhow::ensure!(
                ids.insert(node.spec.id.as_str()),
                "node `{}` is written twice",
                node.spec.id
            );
        }
        // one write to this plan at a time
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left behind
        let (row, recovered) = self.recover(plan, row).await?;
        // 2. carry each node's version on, so a put read before this write is refused after it
        let old: HashMap<String, u64> = self.keys(plan, row.current).await?.into_iter().collect();
        // 3. reserve the new revision, so a write killed from here on can be found
        let revision = fresh_revision();
        let reserve = Pointers {
            pending: Some(revision),
            ..Pointers::default()
        };
        let version = self.update_plan(plan, row.version, reserve).await?;
        // 4. write its rows, a bundle at a time
        let mut rows = Vec::with_capacity(nodes.len());
        for node in nodes {
            let version = old.get(&node.spec.id).map_or(1, |version| version + 1);
            rows.push(node_row(plan, revision, node, version)?);
        }
        while !rows.is_empty() {
            let rest = rows.split_off(rows.len().min(BUNDLE_ROWS));
            self.backend
                .write_nodes(std::mem::replace(&mut rows, rest))
                .await?;
        }
        // 5. flip: the new revision current, the old one retired, in one conditional write
        let flip = Pointers {
            current: Some(revision),
            pending: Some(0),
            retired: Some(row.current),
        };
        let version = self.update_plan(plan, version, flip).await?;
        // 6. prune the old revision, then say there is nothing left to prune; the new revision
        // is current either way, so a failure here is left for the next write
        let left_retired = match self.prune(plan, row.current).await {
            Ok(_) => {
                let cleared = Pointers {
                    retired: Some(0),
                    ..Pointers::default()
                };
                self.update_plan(plan, version, cleared).await.is_err()
            }
            Err(error) => {
                tracing::warn!(%plan, error = %format!("{error:#}"), "pruning a retired revision failed");
                true
            }
        };
        Ok(Rewritten {
            revision,
            recovered,
            left_retired,
        })
    }

    /// Prunes any revision a killed or failed write left named on a plan's row
    ///
    /// Returns the plan's row as it now stands, and how many rows were pruned. The caller holds
    /// the plan's write lock.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `row` - The plan's row as it was read
    #[instrument(name = "Store::recover", skip_all, err(Debug))]
    async fn recover(&self, plan: &PlanName, row: PlanRow) -> anyhow::Result<(PlanRow, usize)> {
        // only a revision that is named and isn't the current one is left over
        let leftovers: Vec<u64> = [row.pending, row.retired]
            .into_iter()
            .filter(|revision| *revision != 0 && *revision != row.current)
            .collect();
        if leftovers.is_empty() && row.pending == 0 && row.retired == 0 {
            return Ok((row, 0));
        }
        // prune each, then say there is nothing left to prune
        let mut pruned = 0;
        for revision in leftovers {
            pruned += self.prune(plan, revision).await?;
        }
        let cleared = Pointers {
            pending: Some(0),
            retired: Some(0),
            ..Pointers::default()
        };
        let version = self.update_plan(plan, row.version, cleared).await?;
        tracing::info!(%plan, pruned, "pruned what a killed write left");
        Ok((
            PlanRow {
                pending: 0,
                retired: 0,
                version,
                ..row
            },
            pruned,
        ))
    }

    /// Changes a plan's pointers, if its row is at the version read, and returns its new version
    ///
    /// The caller holds the plan's write lock, so a refusal means another process wrote it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the row was read at
    /// * `pointers` - The revisions that change
    #[instrument(name = "Store::update_plan", skip_all, err(Debug))]
    async fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        pointers: Pointers,
    ) -> anyhow::Result<u64> {
        // written only over the version read
        match self.backend.update_plan(plan, read, pointers).await? {
            Written::Applied => Ok(read + 1),
            Written::Refused(refusal) => anyhow::bail!(
                "plan `{plan}` changed under this write, from version {read}: {refusal}"
            ),
        }
    }

    /// Lists a revision's node ids and versions, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to list
    #[instrument(name = "Store::keys", skip_all, err(Debug))]
    async fn keys(&self, plan: &PlanName, revision: u64) -> anyhow::Result<Vec<(String, u64)>> {
        // the whole partition, projected to its keys
        let key = RevisionKey::new(plan, revision);
        let keys = self.backend.node_keys(&key).await?;
        keys.into_iter()
            .map(|node| {
                // a key of another partition that hashed the same is never this revision's
                anyhow::ensure!(
                    key.holds_key(&node),
                    "node key `{}/{}/{}` at revision {} was stored where `{plan}` at revision \
                     {revision} is: their partition keys' hashes collide",
                    node.project,
                    node.plan,
                    node.id,
                    node.revision
                );
                Ok((node.id, node.version))
            })
            .collect()
    }

    /// Deletes every row of a revision, and returns how many there were
    ///
    /// Shoal can't delete a partition, so this lists the revision's keys and deletes each row,
    /// a bundle at a time. A key of another partition is refused before anything is deleted,
    /// since deleting by this revision's key would delete the other's row.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to delete
    #[instrument(name = "Store::prune", skip_all, err(Debug))]
    async fn prune(&self, plan: &PlanName, revision: u64) -> anyhow::Result<usize> {
        // find the rows to delete
        let key = RevisionKey::new(plan, revision);
        let mut ids: Vec<String> = self
            .keys(plan, revision)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let count = ids.len();
        // and delete them, each answer checked
        while !ids.is_empty() {
            let rest = ids.split_off(ids.len().min(BUNDLE_ROWS));
            self.backend
                .delete_nodes(&key, std::mem::replace(&mut ids, rest))
                .await?;
        }
        Ok(count)
    }
}

/// Refuses a node the store can't keep as one row
///
/// # Arguments
///
/// * `plan` - The plan it is written to
/// * `node` - The node
fn check_node(plan: &PlanName, node: &NodeWrite) -> anyhow::Result<()> {
    // its id must name it within its plan
    plan.node(node.spec.id.as_str())?;
    // and each member is a node of its own, its `group` naming this one
    anyhow::ensure!(
        node.spec.nodes.is_empty(),
        "node `{}` lists {} members inline: put each as a node of its own, its `group` naming \
         `{}`",
        node.spec.id,
        node.spec.nodes.len(),
        node.spec.id
    );
    Ok(())
}

/// Builds a node's row in a revision
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `revision` - The revision the row is part of
/// * `node` - The node
/// * `version` - The version the row is written at
fn node_row(
    plan: &PlanName,
    revision: u64,
    node: NodeWrite,
    version: u64,
) -> anyhow::Result<NodeRow> {
    Ok(NodeRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        revision,
        id: node.spec.id.clone(),
        version,
        // the fields reef filters on, beside the spec they come from
        status: node
            .spec
            .status
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        group: node.spec.group.clone().unwrap_or_default(),
        order: node.order,
        spec: serde_json::to_string(&node.spec)?,
        body: node.body,
    })
}

/// Reads a node out of its row
///
/// # Arguments
///
/// * `row` - The row
fn stored(row: NodeRow) -> anyhow::Result<StoredNode> {
    // the spec is JSON, which reads back equal
    let spec = serde_json::from_str(&row.spec)
        .map_err(|error| anyhow::anyhow!("node `{}`'s spec: {error}", row.id))?;
    Ok(StoredNode {
        spec,
        body: row.body,
        order: row.order,
        version: row.version,
    })
}

/// A random revision id, never 0, which means none
fn fresh_revision() -> u64 {
    // std's hasher state is randomly seeded per process, and the clock separates calls
    loop {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        let revision = hasher.finish();
        if revision != 0 {
            return revision;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeWrite, Store};
    use crate::backend::RevisionKey;
    use crate::schema::NodeRow;
    use reef_model::PlanName;

    /// A row stored in a plan's partition under another plan's key is an error, never a node
    #[tokio::test]
    async fn collided_row_is_an_error() -> anyhow::Result<()> {
        // a plan with one node of its own
        let store = Store::memory();
        let plan = PlanName::new("reef", "central")?;
        store.add_plan(&plan).await?;
        store.put(&plan, NodeWrite::new("C7", 0), None).await?;
        // and a row of another project's, landing in its partition as a hash collision would
        let current = store.plan_row(&plan).await?.current;
        let stray = NodeRow {
            project: "other".to_string(),
            plan: "central".to_string(),
            revision: current,
            id: "C8".to_string(),
            version: 1,
            status: String::new(),
            group: String::new(),
            order: 1,
            spec: "{\"id\":\"C8\"}".to_string(),
            body: String::new(),
        };
        store
            .backend()
            .plant(&RevisionKey::new(&plan, current), stray);
        // a get of it is caught, and so is a list of the whole revision
        let got = store
            .get(&plan, "C8")
            .await
            .expect_err("a collided row was served");
        assert!(format!("{got:#}").contains("collide"), "{got:#}");
        let listed = store
            .list(&plan)
            .await
            .expect_err("a collided row was listed");
        assert!(format!("{listed:#}").contains("collide"), "{listed:#}");
        // while the plan's own node still reads back
        assert!(store.get(&plan, "C7").await?.is_some());
        Ok(())
    }
}
