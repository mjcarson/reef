//! reef's store over shoal's rows: listings, versioned puts, and revisions behind a pointer row
//!
//! Shoal has no transactions and a bundle is not atomic, so a whole-plan write goes to a new
//! revision's partition and is made current by one conditional write to the plan's row. A put
//! of one node names the version it read and is refused if the row has moved on. Writes to one
//! plan are serialized by a lock in this process, the only writer, so a put never lands in a
//! revision an import is about to retire.

use reef_model::{Diagnostic, Graph, NodeSpec, Origin, PlanName, Record, nodes_from_records};
use shoal::shared::queries::{ConditionRefusal, ConditionalInsert, ConditionalWrite};
use shoal::{Errors, FromShoal, Shoal};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, OwnedMutexGuard};
use tracing::instrument;

use crate::book::Flat;
use crate::tables::{
    NodeKey, NodeRow, NodeRowFilter, NodeRowGet, PROJECTS, PlanRow, PlanRowFilter, PlanRowGet,
    PlanRowUpdate, ProjectRow, ProjectRowGet, X2Client, X2QueryKinds,
};

/// How a node's spec is serialized into its row
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFormat {
    /// `serde_json`
    Json,
    /// reef's canonical frontmatter, through `NodeSpec::to_yaml`
    Yaml,
}

impl SpecFormat {
    /// Reads a format from its name on the command line
    ///
    /// # Arguments
    ///
    /// * `name` - `json` or `yaml`
    pub fn parse(name: &str) -> anyhow::Result<SpecFormat> {
        match name {
            "json" => Ok(SpecFormat::Json),
            "yaml" => Ok(SpecFormat::Yaml),
            other => anyhow::bail!("unknown spec format `{other}`: use json or yaml"),
        }
    }

    /// The format's name
    pub fn name(self) -> &'static str {
        match self {
            SpecFormat::Json => "json",
            SpecFormat::Yaml => "yaml",
        }
    }

    /// Serializes a spec
    ///
    /// # Arguments
    ///
    /// * `spec` - The spec to serialize
    pub fn encode(self, spec: &NodeSpec) -> anyhow::Result<String> {
        match self {
            SpecFormat::Json => Ok(serde_json::to_string(spec)?),
            SpecFormat::Yaml => Ok(spec.to_yaml()),
        }
    }

    /// Reads a spec back
    ///
    /// # Arguments
    ///
    /// * `text` - The serialized spec
    pub fn decode(self, text: &str) -> anyhow::Result<NodeSpec> {
        match self {
            SpecFormat::Json => Ok(serde_json::from_str(text)?),
            SpecFormat::Yaml => {
                NodeSpec::from_yaml(text).map_err(|error| anyhow::anyhow!("{}", error.message))
            }
        }
    }
}

/// A node as a reader got it from the store
#[derive(Debug, Clone, PartialEq)]
pub struct StoredNode {
    /// Its spec
    pub spec: NodeSpec,
    /// Its body
    pub body: String,
    /// Where it falls in the order its plan was written in
    pub order: u64,
    /// The version it was read at, which a put of it names
    pub version: u64,
}

/// A plan's current revision, as a reader got it
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The revision that was current
    pub revision: u64,
    /// Its nodes, in id order
    pub nodes: Vec<StoredNode>,
}

impl Loaded {
    /// Builds the graph these nodes make, each named in its plan
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan these nodes were loaded from
    pub fn graph(self, plan: &PlanName) -> anyhow::Result<(Graph, Vec<Diagnostic>)> {
        // each node becomes a stored record, named by its plan and id
        let mut records = Vec::with_capacity(self.nodes.len());
        for node in self.nodes {
            let name = plan.node(node.spec.id.as_str())?;
            records.push(Record {
                spec: node.spec,
                body: node.body,
                order: node.order,
                origin: Origin::Stored { name },
            });
        }
        // put them in written order and check them as one graph
        Ok(Graph::build(nodes_from_records(records)))
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
    /// The node was not at the version the put named, so nothing was written
    Stale(ConditionRefusal),
}

/// A point in an import where it waits, so a test can act between its rows and its flip
#[derive(Debug, Default)]
pub struct Pause {
    /// Notified when the import has written its rows and not yet flipped
    pub reached: Notify,
    /// Notified to let the import flip
    pub resume: Notify,
}

/// How an import writes its rows
#[derive(Debug, Clone, Copy)]
pub struct ImportOptions<'a> {
    /// How many rows to send in one bundle
    pub chunk: usize,
    /// Abort the process once this many rows are written, to try a killed import
    pub die_after: Option<usize>,
    /// Wait here between the rows and the flip
    pub pause: Option<&'a Pause>,
}

impl Default for ImportOptions<'_> {
    /// Every row in one bundle, never dying and never pausing
    fn default() -> Self {
        ImportOptions {
            chunk: usize::MAX,
            die_after: None,
            pause: None,
        }
    }
}

/// What an import did and what it cost
#[derive(Debug, Clone, Copy)]
pub struct Imported {
    /// The revision it made current
    pub revision: u64,
    /// How many rows that a killed write left behind it pruned first
    pub recovered: usize,
    /// From reading the plan's row to the flip: the cost of a new revision
    pub rewrite: Duration,
    /// Pruning the revision it replaced
    pub prune: Duration,
}

/// reef's store, as one client of shoal
pub struct Store {
    /// The client every read and write goes through
    client: Shoal<X2Client>,
    /// Whether writes to one plan are serialized, which only the race turns off
    locking: bool,
    /// Each plan's write lock
    locks: Mutex<HashMap<PlanName, Arc<tokio::sync::Mutex<()>>>>,
}

impl Store {
    /// Wraps a client in a store
    ///
    /// # Arguments
    ///
    /// * `client` - A client of the pool or node
    /// * `locking` - Whether to serialize writes to each plan
    pub fn new(client: Shoal<X2Client>, locking: bool) -> Store {
        Store {
            client,
            locking,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Takes a plan's write lock, or nothing when locking is off
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to lock
    async fn lock(&self, plan: &PlanName) -> Option<OwnedMutexGuard<()>> {
        // the race turns locking off to show what it guards against
        if !self.locking {
            return None;
        }
        // find this plan's lock, making it on first use
        let lock = self
            .locks
            .lock()
            .expect("the lock map is never poisoned")
            .entry(plan.clone())
            .or_default()
            .clone();
        // and wait for it
        Some(lock.lock_owned().await)
    }

    /// Sends one get and reads each row it found, archived
    ///
    /// A get that finds nothing is an answer with no rows here, never shoal's known issue 55.
    ///
    /// # Arguments
    ///
    /// * `query` - The get to send
    /// * `each` - Reads one archived row
    async fn read<Q, T, R>(
        &self,
        query: Q,
        mut each: impl FnMut(&<T as rkyv::Archive>::Archived) -> anyhow::Result<R>,
    ) -> anyhow::Result<Vec<R>>
    where
        Q: Into<X2QueryKinds>,
        T: FromShoal<X2Client> + rkyv::Archive,
    {
        // send a bundle of this one get, so its answer is read unjudged
        let mut bundle = self.client.query();
        bundle.add_mut(query);
        let mut stream = self.client.send(bundle).await?;
        let response = stream
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("a get went unanswered"))?;
        // a get the server could not run is a failure, but one that found nothing is not
        if let Some(error) = response.error() {
            anyhow::bail!("a get failed: {}", error.msg());
        }
        let Some(rows) = response.access::<T>()? else {
            return Ok(Vec::new());
        };
        // read each row straight out of the archive
        rows.iter().map(&mut each).collect()
    }

    /// Lists a project, refusing one that is already listed
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - The path of its repo
    #[instrument(name = "Store::add_project", skip_all, err(Debug))]
    pub async fn add_project(&self, name: &str, repo: &str) -> anyhow::Result<bool> {
        // insert it only if no project of that name is listed
        let row = ProjectRow {
            list: PROJECTS.to_string(),
            name: name.to_string(),
            repo: repo.to_string(),
        };
        refused_as_false(self.client.send_one(row.if_absent()).await)
    }

    /// Lists every project, by name
    #[instrument(name = "Store::list_projects", skip_all, err(Debug))]
    pub async fn list_projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        // every project sits in the one listing partition
        let get = ProjectRowGet::new(vec![PROJECTS.to_string()]);
        self.read::<_, ProjectRow, _>(get, |row| {
            // a row from another partition that hashed the same is a collision, not a project
            anyhow::ensure!(row.list.as_str() == PROJECTS, "a project row from `{}`", row.list);
            Ok(rkyv::deserialize::<ProjectRow, rkyv::rancor::Error>(row)?)
        })
        .await
    }

    /// Lists a plan in its project, with no revision yet, refusing one already listed
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<bool> {
        // insert it only if its project has no plan of that name
        let row = PlanRow {
            project: plan.project.clone(),
            name: plan.plan.clone(),
            current: 0,
            pending: 0,
            retired: 0,
            version: 1,
        };
        refused_as_false(self.client.send_one(row.if_absent()).await)
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project whose plans to list
    #[instrument(name = "Store::list_plans", skip_all, err(Debug))]
    pub async fn list_plans(&self, project: &str) -> anyhow::Result<Vec<PlanRow>> {
        // a project's plans sit in that project's partition
        let get = PlanRowGet::new(vec![project.to_string()]);
        self.read::<_, PlanRow, _>(get, |row| {
            // check the row against the key it was asked for
            anyhow::ensure!(
                row.project.as_str() == project,
                "a plan row of `{}` read as one of `{project}`",
                row.project
            );
            Ok(rkyv::deserialize::<PlanRow, rkyv::rancor::Error>(row)?)
        })
        .await
    }

    /// Gets a plan's row, if it is listed
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::plan_row", skip_all, err(Debug))]
    pub async fn plan_row(&self, plan: &PlanName) -> anyhow::Result<Option<PlanRow>> {
        // the one row named by the plan's name, in its project's partition
        let get = PlanRowGet::new(vec![plan.project.clone()]).sort_keys(vec![plan.plan.clone()]);
        let rows = self
            .read::<_, PlanRow, _>(get, |row| {
                // check the row against its full key
                anyhow::ensure!(
                    row.project.as_str() == plan.project && row.name.as_str() == plan.plan,
                    "plan row `{}/{}` read as `{plan}`",
                    row.project,
                    row.name
                );
                Ok(rkyv::deserialize::<PlanRow, rkyv::rancor::Error>(row)?)
            })
            .await?;
        Ok(rows.into_iter().next())
    }

    /// Changes a plan's row if it is still at the version read, and returns its new version
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the change was made from
    /// * `current` - A new current revision, if it changes
    /// * `pending` - A new pending revision, if it changes
    /// * `retired` - A new retired revision, if it changes
    #[instrument(name = "Store::update_plan", skip_all, err(Debug))]
    async fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        current: Option<u64>,
        pending: Option<u64>,
        retired: Option<u64>,
    ) -> anyhow::Result<u64> {
        // write only the fields that change, and the next version
        let update = PlanRowUpdate {
            partition_key: plan.project.clone(),
            sort_key: plan.plan.clone(),
            current,
            pending,
            retired,
            version: Some(read + 1),
        };
        // and only if no one else changed the row since it was read
        let filter = PlanRowFilter {
            version: Some(vec![read]),
        };
        match self.client.send_one(update.if_matches(filter)).await {
            Ok(_) => Ok(read + 1),
            Err(Errors::Refused { reason, .. }) => {
                anyhow::bail!("plan `{plan}` changed under this write ({reason:?} at version {read})")
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Lists a revision's node ids and versions, without their specs
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to list
    #[instrument(name = "Store::keys", skip_all, err(Debug))]
    pub async fn keys(&self, plan: &PlanName, revision: u64) -> anyhow::Result<Vec<(String, u64)>> {
        // the whole partition, projected to its keys
        let key = (plan.project.clone(), plan.plan.clone(), revision);
        let get = NodeRowGet::new(vec![key]).projection::<NodeKey>();
        self.read::<_, NodeKey, _>(get, |row| Ok((row.id.to_string(), row.version.to_native())))
            .await
    }

    /// Deletes every row of a revision, and returns how many there were
    ///
    /// Shoal can't delete a partition, so this lists the revision's keys and deletes each row,
    /// all in one bundle.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to delete
    #[instrument(name = "Store::prune", skip_all, err(Debug))]
    pub async fn prune(&self, plan: &PlanName, revision: u64) -> anyhow::Result<usize> {
        // find the rows to delete
        let keys = self.keys(plan, revision).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        // and delete them all in one bundle, each one checked
        let mut bundle = self.client.query();
        for (id, _) in &keys {
            bundle.add_mut(crate::tables::NodeRowDelete::new(
                plan.project.clone(),
                plan.plan.clone(),
                revision,
                id.clone(),
            ));
        }
        self.client
            .exec(bundle)
            .await
            .map_err(|error| anyhow::anyhow!("pruning revision {revision} failed: {error:?}"))?;
        Ok(keys.len())
    }

    /// Writes a whole plan as a new revision and makes it current
    ///
    /// The steps, under the plan's lock:
    ///
    /// 1. Prune any revision a killed write left pending or retired.
    /// 2. Read the current revision's versions, so each node's carries forward.
    /// 3. Reserve the new revision: name it pending on the plan's row.
    /// 4. Write its rows, each bundle checked.
    /// 5. Flip: make it current and the old one retired, in one conditional write.
    /// 6. Prune the old revision and clear `retired`.
    ///
    /// A process killed before step 5 leaves the old revision current and whole.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name, already listed
    /// * `nodes` - The plan's nodes, flattened
    /// * `format` - How to serialize their specs
    /// * `options` - How to write the rows
    #[instrument(name = "Store::import", skip_all, err(Debug))]
    pub async fn import(
        &self,
        plan: &PlanName,
        nodes: &[Flat],
        format: SpecFormat,
        options: ImportOptions<'_>,
    ) -> anyhow::Result<Imported> {
        // one write to this plan at a time
        let _guard = self.lock(plan).await;
        let row = self
            .plan_row(plan)
            .await?
            .ok_or_else(|| anyhow::anyhow!("plan `{plan}` is not listed"))?;
        // 1. prune whatever a killed write left behind
        let mut recovered = 0;
        for revision in [row.pending, row.retired] {
            if revision != 0 && revision != row.current {
                recovered += self.prune(plan, revision).await?;
            }
        }
        let started = Instant::now();
        // 2. carry each node's version forward, so a put read before this import is refused
        let old: HashMap<String, u64> = if row.current == 0 {
            HashMap::new()
        } else {
            self.keys(plan, row.current).await?.into_iter().collect()
        };
        // 3. reserve the new revision, so a write killed from here on can be found
        let revision = fresh_revision();
        let version = self
            .update_plan(plan, row.version, None, Some(revision), Some(0))
            .await?;
        // 4. write its rows, a bundle at a time, dying midway if asked to
        let chunk = options.die_after.map_or(options.chunk, |n| n.min(options.chunk)).max(1);
        let mut written = 0;
        for part in nodes.chunks(chunk) {
            let mut bundle = self.client.query();
            for node in part {
                bundle.add_mut(node_row(plan, revision, node, &old, format)?);
            }
            // every row's answer is checked, which draining the stream would not do
            self.client
                .exec(bundle)
                .await
                .map_err(|error| anyhow::anyhow!("writing revision {revision} failed: {error:?}"))?;
            written += part.len();
            if options.die_after.is_some_and(|n| written >= n) {
                eprintln!("dying after {written} of {} rows of revision {revision:016x}", nodes.len());
                std::process::abort();
            }
        }
        // wait here if a test asked to act between the rows and the flip
        if let Some(pause) = options.pause {
            pause.reached.notify_one();
            pause.resume.notified().await;
        }
        // 5. flip: the new revision current, the old one retired, in one conditional write
        let version = self
            .update_plan(plan, version, Some(revision), Some(0), Some(row.current))
            .await?;
        let rewrite = started.elapsed();
        // 6. prune the old revision, then say there is nothing left to prune
        let pruning = Instant::now();
        if row.current != 0 {
            self.prune(plan, row.current).await?;
            self.update_plan(plan, version, None, None, Some(0)).await?;
        }
        Ok(Imported {
            revision,
            recovered,
            rewrite,
            prune: pruning.elapsed(),
        })
    }

    /// Loads a plan's current revision
    ///
    /// Every row is checked against the full key it was asked for, so two partitions whose keys
    /// hash the same are caught rather than read as one.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `format` - How its specs were serialized
    #[instrument(name = "Store::load", skip_all, err(Debug))]
    pub async fn load(&self, plan: &PlanName, format: SpecFormat) -> anyhow::Result<Loaded> {
        // find the current revision
        let row = self
            .plan_row(plan)
            .await?
            .ok_or_else(|| anyhow::anyhow!("plan `{plan}` is not listed"))?;
        anyhow::ensure!(row.current != 0, "plan `{plan}` has no revision yet");
        // and read all of its rows
        let nodes = self.revision(plan, row.current, None, format).await?;
        Ok(Loaded {
            revision: row.current,
            nodes,
        })
    }

    /// Gets one node of a plan's current revision
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    /// * `format` - How its spec was serialized
    #[instrument(name = "Store::get_node", skip_all, err(Debug))]
    pub async fn get_node(
        &self,
        plan: &PlanName,
        id: &str,
        format: SpecFormat,
    ) -> anyhow::Result<Option<StoredNode>> {
        // find the current revision
        let row = self
            .plan_row(plan)
            .await?
            .ok_or_else(|| anyhow::anyhow!("plan `{plan}` is not listed"))?;
        // and read the one row
        let nodes = self.revision(plan, row.current, Some(id), format).await?;
        Ok(nodes.into_iter().next())
    }

    /// Reads a revision's rows, or one of them, checking each against its full key
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `revision` - The revision to read
    /// * `id` - The one node to read, or all of them
    /// * `format` - How the specs were serialized
    async fn revision(
        &self,
        plan: &PlanName,
        revision: u64,
        id: Option<&str>,
        format: SpecFormat,
    ) -> anyhow::Result<Vec<StoredNode>> {
        // the revision's partition, whole or one row of it
        let key = (plan.project.clone(), plan.plan.clone(), revision);
        let mut get = NodeRowGet::new(vec![key]);
        if let Some(id) = id {
            get = get.sort_keys(vec![id.to_string()]);
        }
        self.read::<_, NodeRow, _>(get, |row| {
            // a row of another partition that hashed the same is a collision, not a node
            anyhow::ensure!(
                row.project.as_str() == plan.project
                    && row.plan.as_str() == plan.plan
                    && row.revision.to_native() == revision,
                "node row `{}/{}/{}` at revision {} read as one of `{plan}` at revision {revision}",
                row.project,
                row.plan,
                row.id,
                row.revision.to_native()
            );
            // decode the spec straight out of the archive
            Ok(StoredNode {
                spec: format.decode(row.spec.as_str())?,
                body: row.body.to_string(),
                order: row.order.to_native(),
                version: row.version.to_native(),
            })
        })
        .await
    }

    /// Writes one node into a plan's current revision, if it is still at the version read
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `node` - The node as it should be written; its version is the one it was read at
    /// * `read` - The version it was read at, or `None` for a node that must not exist yet
    /// * `format` - How to serialize its spec
    #[instrument(name = "Store::put_node", skip_all, err(Debug))]
    pub async fn put_node(
        &self,
        plan: &PlanName,
        node: &StoredNode,
        read: Option<u64>,
        format: SpecFormat,
    ) -> anyhow::Result<Put> {
        // one write to this plan at a time, so this never lands in a revision being retired
        let _guard = self.lock(plan).await;
        let row = self
            .plan_row(plan)
            .await?
            .ok_or_else(|| anyhow::anyhow!("plan `{plan}` is not listed"))?;
        anyhow::ensure!(row.current != 0, "plan `{plan}` has no revision yet");
        // the row as it should be, one version on from the one read
        let version = read.map_or(1, |read| read + 1);
        let new = NodeRow {
            project: plan.project.clone(),
            plan: plan.plan.clone(),
            revision: row.current,
            id: node.spec.id.clone(),
            version,
            status: node.spec.status.map(|s| s.as_str().to_string()).unwrap_or_default(),
            group: node.spec.group.clone().unwrap_or_default(),
            order: node.order,
            spec: format.encode(&node.spec)?,
            body: node.body.clone(),
        };
        // written only over the version read, or only where there is no row
        let sent = match read {
            Some(read) => {
                let filter = NodeRowFilter {
                    version: Some(vec![read]),
                    ..NodeRowFilter::default()
                };
                self.client.send_one(new.if_matches(filter)).await
            }
            None => self.client.send_one(new.if_absent()).await,
        };
        match sent {
            Ok(_) => Ok(Put::Applied { version }),
            Err(Errors::Refused { reason, .. }) => Ok(Put::Stale(reason)),
            Err(error) => Err(error.into()),
        }
    }
}

/// Builds a node's row in a revision, its version one on from the one it had
///
/// # Arguments
///
/// * `plan` - The plan's full name
/// * `revision` - The revision the row is part of
/// * `node` - The node
/// * `old` - Each node's version in the revision this replaces
/// * `format` - How to serialize its spec
fn node_row(
    plan: &PlanName,
    revision: u64,
    node: &Flat,
    old: &HashMap<String, u64>,
    format: SpecFormat,
) -> anyhow::Result<NodeRow> {
    // a node already in the plan moves on a version, and a new one starts at 1
    let version = old.get(&node.spec.id).map_or(1, |version| version + 1);
    Ok(NodeRow {
        project: plan.project.clone(),
        plan: plan.plan.clone(),
        revision,
        id: node.spec.id.clone(),
        version,
        status: node.spec.status.map(|s| s.as_str().to_string()).unwrap_or_default(),
        group: node.spec.group.clone().unwrap_or_default(),
        order: node.order,
        spec: format.encode(&node.spec)?,
        body: node.body.clone(),
    })
}

/// A random revision id, never 0, which means none
fn fresh_revision() -> u64 {
    // std's hasher state is randomly seeded, which is all a spike needs
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

/// Reads a conditional insert's answer: written, refused, or failed
///
/// # Arguments
///
/// * `sent` - The answer to the insert
fn refused_as_false<T>(sent: Result<T, Errors>) -> anyhow::Result<bool> {
    match sent {
        // written
        Ok(_) => Ok(true),
        // a row was already there
        Err(Errors::Refused { .. }) => Ok(false),
        // anything else is a failure
        Err(error) => Err(error.into()),
    }
}
