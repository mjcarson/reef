//! reef's client of shoal: a pool started in this process, or an external node
//!
//! The client is the same either way. Only start-up differs: in-process starts a pool first and
//! connects to the address it bound, while external connects to the address it was given.

use reef_model::PlanName;
use shoal::shared::queries::{ConditionRefusal, ConditionalInsert, ConditionalWrite};
use shoal::{Errors, FromShoal, Shoal};
use std::net::SocketAddr;
use std::time::Duration;
use tracing::instrument;

use crate::backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use crate::conf::Conf;
use crate::schema::{
    CommentRow, CommentRowFilter, CommentRowGet, DecisionRow, DecisionRowFilter, DecisionRowGet,
    LOOPS, LoopRow, LoopRowDelete, LoopRowFilter, LoopRowGet, NodeKey, NodeRow, NodeRowDelete,
    NodeRowFilter, NodeRowGet, PROJECTS, PageKey, PageRow, PageRowDelete, PageRowFilter,
    PageRowGet, PlanRow, PlanRowFilter, PlanRowGet, PlanRowUpdate, ProjectRow, ProjectRowGet,
    QueueRow, QueueRowDelete, QueueRowFilter, QueueRowGet, REMOTES, ReefClient, ReefQueryKinds,
    RemoteRow, RemoteRowFilter, RemoteRowGet, RevisionFileRow, RevisionFileRowGet, RevisionRow,
    RevisionRowFilter, RevisionRowGet, TOKENS, TokenRow, TokenRowDelete, TokenRowFilter,
    TokenRowGet, USERS, UserRow, UserRowDelete, UserRowFilter, UserRowGet, WorkRow, WorkRowFilter,
    WorkRowGet,
};

#[cfg(feature = "embedded")]
use crate::checks::{self, Keys, Problem, Refused};
#[cfg(feature = "embedded")]
use crate::pool::Pool;

/// How many times to try connecting to a server that just started
const CONNECT_TRIES: usize = 40;

/// How long to wait between those tries
const CONNECT_PAUSE: Duration = Duration::from_millis(25);

/// How reef reaches shoal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A pool this process started, which only an `embedded` build can do
    InProcess,
    /// A node some other process runs, such as `reef-shoal`
    External,
}

impl Mode {
    /// The name this mode is reported by
    pub fn name(self) -> &'static str {
        match self {
            Mode::InProcess => "in-process",
            Mode::External => "external",
        }
    }
}

/// A client of shoal, however shoal was reached
pub struct ShoalBackend {
    /// A client for the pool or the node
    client: Shoal<ReefClient>,
    /// How this reaches shoal
    mode: Mode,
    /// The pool this process started, kept up for as long as this lives
    #[cfg(feature = "embedded")]
    pool: Option<Pool>,
}

impl ShoalBackend {
    /// Reaches shoal: the external node reef's config names, else a pool in this process
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config
    #[instrument(name = "ShoalBackend::open", skip_all, err(Debug))]
    pub async fn open(conf: &Conf) -> anyhow::Result<ShoalBackend> {
        // an address configured means someone else runs shoal
        match &conf.shoal.address {
            Some(addr) => Self::external(addr).await,
            None => Self::in_process(conf).await,
        }
    }

    /// Connects to an external node
    ///
    /// # Arguments
    ///
    /// * `addr` - The node's address
    #[instrument(name = "ShoalBackend::external", skip_all, err(Debug))]
    pub async fn external(addr: &str) -> anyhow::Result<ShoalBackend> {
        // resolve the address the node was configured with
        let addr: SocketAddr = addr
            .parse()
            .map_err(|error| anyhow::anyhow!("shoal's address `{addr}`: {error}"))?;
        // and connect to it the same way an in-process pool is connected to
        let client = connect(addr).await?;
        tracing::info!(%addr, "shoal: connected to an external node");
        Ok(ShoalBackend {
            client,
            mode: Mode::External,
            #[cfg(feature = "embedded")]
            pool: None,
        })
    }

    /// Starts a pool in this process and connects to it
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config, which names the pool's data directory, cores and budget
    #[cfg(feature = "embedded")]
    #[instrument(name = "ShoalBackend::in_process", skip_all, err(Debug))]
    pub async fn in_process(conf: &Conf) -> anyhow::Result<ShoalBackend> {
        // check the pool's config before anything is made, naming the fix for each problem
        let dir = conf.data_dir()?;
        let problems = checks::check(
            &crate::pool::shoal_conf(&conf.shoal, &dir)?,
            Keys::Reef(conf),
        );
        let (fatal, warnings): (Vec<Problem>, Vec<Problem>) =
            problems.into_iter().partition(|problem| problem.fatal);
        for warning in warnings {
            tracing::warn!(what = %warning.what, fix = %warning.fix, "shoal: a config problem");
        }
        if !fatal.is_empty() {
            return Err(Refused(fatal).into());
        }
        // make the directory, and resolve it through any symlinks
        std::fs::create_dir_all(&dir)
            .map_err(|error| anyhow::anyhow!("creating {}: {error}", dir.display()))?;
        let dir = dir.canonicalize()?;
        let shoal_conf = crate::pool::shoal_conf(&conf.shoal, &dir)?;
        // start the pool on a thread of its own
        let started = std::time::Instant::now();
        let pool = Pool::start(shoal_conf).await?;
        tracing::info!(
            ready_s = started.elapsed().as_secs_f64(),
            addr = %pool.addr,
            dir = %dir.display(),
            "shoal: in-process pool is ready"
        );
        // connect to it over loopback, exactly as to an external node
        let client = connect(pool.addr).await?;
        Ok(ShoalBackend {
            client,
            mode: Mode::InProcess,
            pool: Some(pool),
        })
    }

    /// Refuses to start in-process in a build without shoal's engine
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config, named in the message so it says what to set
    #[cfg(not(feature = "embedded"))]
    pub async fn in_process(conf: &Conf) -> anyhow::Result<ShoalBackend> {
        // only `embedded` links shoal's engine, so this build can only reach a node
        anyhow::bail!(
            "this build of reef has no in-process shoal: set {} to the address of an external \
             node (`reef-shoal` serves one), or build reef-service with the `embedded` feature",
            conf.key("shoal.address")
        )
    }

    /// How this reaches shoal
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The client every read and write goes through
    pub fn client(&self) -> &Shoal<ReefClient> {
        &self.client
    }

    /// Closes the client, then stops the in-process pool if there is one
    ///
    /// Stopping a pool takes up to 3 s. An external node is left running.
    #[instrument(name = "ShoalBackend::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // nothing more goes to shoal through this client
        drop(self.client);
        // then stop the pool this process started, if it started one
        #[cfg(feature = "embedded")]
        if let Some(pool) = self.pool {
            pool.close().await?;
        }
        Ok(())
    }
}

impl ShoalBackend {
    /// Sends one get and deserializes each row it found
    ///
    /// The get goes as a bundle of one, whose answer is read unjudged: a get that found nothing
    /// is then an answer with no rows, never shoal's known issue 55 (`QueryDidNotSucceed`).
    ///
    /// # Arguments
    ///
    /// * `query` - The get to send
    async fn read<Q, T>(&self, query: Q) -> anyhow::Result<Vec<T>>
    where
        Q: Into<ReefQueryKinds>,
        T: FromShoal<ReefClient> + rkyv::Archive,
        T::Archived: rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
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
        // deserialize each row out of the archive
        rows.iter()
            .map(|row| Ok(rkyv::deserialize::<T, rkyv::rancor::Error>(row)?))
            .collect()
    }
}

/// Reads a conditional write's answer: written, refused by its condition, or failed
///
/// # Arguments
///
/// * `sent` - The answer to the write
fn written<T>(sent: Result<T, Errors>) -> anyhow::Result<Written> {
    match sent {
        // written
        Ok(_) => Ok(Written::Applied),
        // its condition did not hold, which is an answer rather than a failure
        Err(Errors::Refused { reason, .. }) => Ok(Written::Refused(match reason {
            ConditionRefusal::RowExists => Refusal::Exists,
            ConditionRefusal::RowMissing => Refusal::Missing,
            ConditionRefusal::RowMismatch => Refusal::Moved,
            // reef keeps no objects, so a refusal of a stripe's placement is a failure
            ConditionRefusal::Placement => {
                anyhow::bail!("a row write was refused for a stripe's placement")
            }
            // nor does it declare an order on its tables, so an order index's refusal is too
            ConditionRefusal::KeyCollision | ConditionRefusal::OrderEntry => {
                anyhow::bail!("a row write was refused by an order index: {reason:?}")
            }
        })),
        // anything else is a failure
        Err(error) => Err(error.into()),
    }
}

impl Backend for ShoalBackend {
    /// Lists the project rows stored in the one partition every project is listed in
    #[instrument(name = "ShoalBackend::projects", skip_all, err(Debug))]
    async fn projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        // every project sits in the one listing partition
        let get = ProjectRowGet::new(vec![PROJECTS.to_string()]);
        self.read::<_, ProjectRow>(get).await
    }

    /// Writes a project's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The project's row
    #[instrument(name = "ShoalBackend::insert_project", skip_all, err(Debug))]
    async fn insert_project(&self, row: ProjectRow) -> anyhow::Result<Written> {
        // `if_absent` refuses it where a row is stored
        written(self.client.send_one(row.if_absent()).await)
    }

    /// Lists the plan rows stored in a project's partition, in name order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "ShoalBackend::plans", skip_all, err(Debug))]
    async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanRow>> {
        // a project's plans sit in that project's partition
        let get = PlanRowGet::new(vec![project.to_string()]);
        self.read::<_, PlanRow>(get).await
    }

    /// Gets the row stored under a plan's key, if one is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "ShoalBackend::plan", skip_all, err(Debug))]
    async fn plan(&self, plan: &PlanName) -> anyhow::Result<Option<PlanRow>> {
        // the one row named by the plan's name, in its project's partition
        let get = PlanRowGet::new(vec![plan.project.clone()]).sort_keys(vec![plan.plan.clone()]);
        Ok(self.read::<_, PlanRow>(get).await?.into_iter().next())
    }

    /// Writes a plan's row only if none is stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The plan's row
    #[instrument(name = "ShoalBackend::insert_plan", skip_all, err(Debug))]
    async fn insert_plan(&self, row: PlanRow) -> anyhow::Result<Written> {
        // `if_absent` refuses it where a row is stored
        written(self.client.send_one(row.if_absent()).await)
    }

    /// Reads the node rows stored in a revision's partition, in id order, or the one under an id
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `id` - The one node to read, or `None` for all of them
    #[instrument(name = "ShoalBackend::nodes", skip_all, err(Debug))]
    async fn nodes(&self, key: &RevisionKey, id: Option<&str>) -> anyhow::Result<Vec<NodeRow>> {
        // the revision's partition, whole or one row of it
        let partition = (key.project.clone(), key.plan.clone(), key.revision);
        let mut get = NodeRowGet::new(vec![partition]);
        if let Some(id) = id {
            get = get.sort_keys(vec![id.to_string()]);
        }
        self.read::<_, NodeRow>(get).await
    }

    /// Writes one node row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_node", skip_all, err(Debug))]
    async fn write_node(&self, row: NodeRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = NodeRowFilter {
                    version: Some(vec![read]),
                    ..NodeRowFilter::default()
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Changes a plan's pointers, if its row is still at the version read
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `read` - The version the row was read at
    /// * `pointers` - The revisions that change
    #[instrument(name = "ShoalBackend::update_plan", skip_all, err(Debug))]
    async fn update_plan(
        &self,
        plan: &PlanName,
        read: u64,
        pointers: Pointers,
    ) -> anyhow::Result<Written> {
        // write only the fields that change, and the next version
        let update = PlanRowUpdate {
            partition_key: plan.project.clone(),
            sort_key: plan.plan.clone(),
            current: pointers.current,
            pending: pointers.pending,
            retired: pointers.retired,
            pages_current: pointers.pages_current,
            pages_retired: pointers.pages_retired,
            version: Some(read + 1),
        };
        // and only if no one changed the row since it was read
        let filter = PlanRowFilter {
            version: Some(vec![read]),
        };
        written(self.client.send_one(update.if_matches(filter)).await)
    }

    /// Lists the keys and versions stored in a revision's partition, without their specs
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    #[instrument(name = "ShoalBackend::node_keys", skip_all, err(Debug))]
    async fn node_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<NodeKey>> {
        // the whole partition, projected to its keys
        let partition = (key.project.clone(), key.plan.clone(), key.revision);
        let get = NodeRowGet::new(vec![partition]).projection::<NodeKey>();
        self.read::<_, NodeKey>(get).await
    }

    /// Writes node rows over whatever is stored under their keys, every answer checked
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    #[instrument(name = "ShoalBackend::write_nodes", skip_all, err(Debug))]
    async fn write_nodes(&self, rows: Vec<NodeRow>) -> anyhow::Result<()> {
        // nothing to send is nothing to check
        if rows.is_empty() {
            return Ok(());
        }
        // one bundle, whose every answer `exec` checks, which draining its stream would not
        let mut bundle = self.client.query();
        for row in rows {
            bundle.add_mut(row);
        }
        self.client
            .exec(bundle)
            .await
            .map_err(|error| anyhow::anyhow!("writing node rows failed: {error:?}"))?;
        Ok(())
    }

    /// Deletes nodes from a revision's partition, every answer checked
    ///
    /// Shoal answers a delete of a row that isn't stored as `QueryDidNotSucceed`: it worked and
    /// found nothing. That deletes nothing, so it is no failure here.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `ids` - The ids of the nodes to delete
    #[instrument(name = "ShoalBackend::delete_nodes", skip_all, err(Debug))]
    async fn delete_nodes(&self, key: &RevisionKey, ids: Vec<String>) -> anyhow::Result<()> {
        // nothing to send is nothing to check
        if ids.is_empty() {
            return Ok(());
        }
        // one bundle of deletes, shoal having no way to delete a partition
        let mut bundle = self.client.query();
        for id in ids {
            bundle.add_mut(NodeRowDelete::new(
                key.project.clone(),
                key.plan.clone(),
                key.revision,
                id,
            ));
        }
        match self.client.exec(bundle).await {
            Ok(_) => Ok(()),
            // a delete that found nothing to delete worked, as shoal's own docs read it, which
            // makes a prune that is tried again harmless
            Err(Errors::QueryDidNotSucceed { .. }) => Ok(()),
            Err(Errors::BulkErrors(errors))
                if errors
                    .iter()
                    .all(|error| matches!(error, Errors::QueryDidNotSucceed { .. })) =>
            {
                Ok(())
            }
            Err(error) => anyhow::bail!("deleting node rows failed: {error:?}"),
        }
    }

    /// Reads the page rows stored in a revision's partition, in path order, or the one at a path
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `path` - The one page to read, or `None` for all of them
    #[instrument(name = "ShoalBackend::pages", skip_all, err(Debug))]
    async fn pages(&self, key: &RevisionKey, path: Option<&str>) -> anyhow::Result<Vec<PageRow>> {
        // the revision's partition, whole or one row of it
        let partition = (key.project.clone(), key.plan.clone(), key.revision);
        let mut get = PageRowGet::new(vec![partition]);
        if let Some(path) = path {
            get = get.sort_keys(vec![path.to_string()]);
        }
        self.read::<_, PageRow>(get).await
    }

    /// Writes one page row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_page", skip_all, err(Debug))]
    async fn write_page(&self, row: PageRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = PageRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Lists the paths and versions stored in a revision's partition of pages, without their text
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    #[instrument(name = "ShoalBackend::page_keys", skip_all, err(Debug))]
    async fn page_keys(&self, key: &RevisionKey) -> anyhow::Result<Vec<PageKey>> {
        // the whole partition, projected to its keys
        let partition = (key.project.clone(), key.plan.clone(), key.revision);
        let get = PageRowGet::new(vec![partition]).projection::<PageKey>();
        self.read::<_, PageKey>(get).await
    }

    /// Writes page rows over whatever is stored under their keys, every answer checked
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    #[instrument(name = "ShoalBackend::write_pages", skip_all, err(Debug))]
    async fn write_pages(&self, rows: Vec<PageRow>) -> anyhow::Result<()> {
        // nothing to send is nothing to check
        if rows.is_empty() {
            return Ok(());
        }
        // one bundle, whose every answer `exec` checks, which draining its stream would not
        let mut bundle = self.client.query();
        for row in rows {
            bundle.add_mut(row);
        }
        self.client
            .exec(bundle)
            .await
            .map_err(|error| anyhow::anyhow!("writing page rows failed: {error:?}"))?;
        Ok(())
    }

    /// Deletes pages from a revision's partition, every answer checked
    ///
    /// A delete of a row that isn't stored is answered as `QueryDidNotSucceed`, as a node's is,
    /// and is no failure here.
    ///
    /// # Arguments
    ///
    /// * `key` - The revision's key
    /// * `paths` - The paths of the pages to delete
    #[instrument(name = "ShoalBackend::delete_pages", skip_all, err(Debug))]
    async fn delete_pages(&self, key: &RevisionKey, paths: Vec<String>) -> anyhow::Result<()> {
        // nothing to send is nothing to check
        if paths.is_empty() {
            return Ok(());
        }
        // one bundle of deletes, shoal having no way to delete a partition
        let mut bundle = self.client.query();
        for path in paths {
            bundle.add_mut(PageRowDelete::new(
                key.project.clone(),
                key.plan.clone(),
                key.revision,
                path,
            ));
        }
        match self.client.exec(bundle).await {
            Ok(_) => Ok(()),
            // a delete that found nothing to delete worked, which makes a prune tried again
            // harmless
            Err(Errors::QueryDidNotSucceed { .. }) => Ok(()),
            Err(Errors::BulkErrors(errors))
                if errors
                    .iter()
                    .all(|error| matches!(error, Errors::QueryDidNotSucceed { .. })) =>
            {
                Ok(())
            }
            Err(error) => anyhow::bail!("deleting page rows failed: {error:?}"),
        }
    }

    /// Lists the queue rows stored in a project's partition, in rank order
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "ShoalBackend::queue", skip_all, err(Debug))]
    async fn queue(&self, project: &str) -> anyhow::Result<Vec<QueueRow>> {
        // a project's queue sits in that project's partition
        let get = QueueRowGet::new(vec![project.to_string()]);
        self.read::<_, QueueRow>(get).await
    }

    /// Writes a queue row only if none is stored under its rank
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    #[instrument(name = "ShoalBackend::insert_queue", skip_all, err(Debug))]
    async fn insert_queue(&self, row: QueueRow) -> anyhow::Result<Written> {
        // `if_absent` refuses it where a row is stored
        written(self.client.send_one(row.if_absent()).await)
    }

    /// Deletes a queue row, only if the row stored under its rank is still this one
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    #[instrument(name = "ShoalBackend::delete_queue", skip_all, err(Debug))]
    async fn delete_queue(&self, row: &QueueRow) -> anyhow::Result<Written> {
        // the row under its rank, only if it still names the node and write it was read with
        let delete = QueueRowDelete::new(row.project.clone(), row.rank.clone());
        let filter = QueueRowFilter {
            plan: Some(vec![row.plan.clone()]),
            id: Some(vec![row.id.clone()]),
            seq: Some(vec![row.seq]),
        };
        written(self.client.send_one(delete.if_matches(filter)).await)
    }

    /// Reads the work rows stored in a project's partition, in node order, or the one of a node
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The one node's name within its project, or `None` for all of them
    #[instrument(name = "ShoalBackend::work", skip_all, err(Debug))]
    async fn work(&self, project: &str, node: Option<&str>) -> anyhow::Result<Vec<WorkRow>> {
        // a project's work records sit in that project's partition
        let mut get = WorkRowGet::new(vec![project.to_string()]);
        if let Some(node) = node {
            get = get.sort_keys(vec![node.to_string()]);
        }
        self.read::<_, WorkRow>(get).await
    }

    /// Writes one work row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_work", skip_all, err(Debug))]
    async fn write_work(&self, row: WorkRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = WorkRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Reads the revision rows stored in a node's partition, oldest first, or the one of a number
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's name within its project, `plan/id`
    /// * `revision` - The one revision's sort key, or `None` for all of them
    #[instrument(name = "ShoalBackend::revisions", skip_all, err(Debug))]
    async fn revisions(
        &self,
        project: &str,
        node: &str,
        revision: Option<&str>,
    ) -> anyhow::Result<Vec<RevisionRow>> {
        // a node's revisions sit in the node's partition
        let mut get = RevisionRowGet::new(vec![(project.to_string(), node.to_string())]);
        if let Some(revision) = revision {
            get = get.sort_keys(vec![revision.to_string()]);
        }
        self.read::<_, RevisionRow>(get).await
    }

    /// Writes one revision row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_revision", skip_all, err(Debug))]
    async fn write_revision(&self, row: RevisionRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = RevisionRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Reads the file rows stored in a revision's partition, in path order, or the one at a path
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `node` - The node's name within its project, `plan/id`
    /// * `revision` - The revision's number
    /// * `path` - The one file's path, or `None` for all of them
    #[instrument(name = "ShoalBackend::revision_files", skip_all, err(Debug))]
    async fn revision_files(
        &self,
        project: &str,
        node: &str,
        revision: u64,
        path: Option<&str>,
    ) -> anyhow::Result<Vec<RevisionFileRow>> {
        // a revision's files sit in the revision's partition
        let partition = (project.to_string(), node.to_string(), revision);
        let mut get = RevisionFileRowGet::new(vec![partition]);
        if let Some(path) = path {
            get = get.sort_keys(vec![path.to_string()]);
        }
        self.read::<_, RevisionFileRow>(get).await
    }

    /// Writes revision file rows over whatever is stored under their keys, every answer checked
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows to write
    #[instrument(name = "ShoalBackend::write_revision_files", skip_all, err(Debug))]
    async fn write_revision_files(&self, rows: Vec<RevisionFileRow>) -> anyhow::Result<()> {
        // nothing to send is nothing to check
        if rows.is_empty() {
            return Ok(());
        }
        // one bundle, whose every answer `exec` checks, which draining its stream would not
        let mut bundle = self.client.query();
        for row in rows {
            bundle.add_mut(row);
        }
        self.client
            .exec(bundle)
            .await
            .map_err(|error| anyhow::anyhow!("writing revision file rows failed: {error:?}"))?;
        Ok(())
    }

    /// Reads the comment rows stored in a plan's partition, in number order, or the one of a
    /// number
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `plan` - The plan's name within its project
    /// * `number` - The one comment's sort key, or `None` for all of them
    #[instrument(name = "ShoalBackend::comments", skip_all, err(Debug))]
    async fn comments(
        &self,
        project: &str,
        plan: &str,
        number: Option<&str>,
    ) -> anyhow::Result<Vec<CommentRow>> {
        // a plan's comments sit in the plan's partition
        let mut get = CommentRowGet::new(vec![(project.to_string(), plan.to_string())]);
        if let Some(number) = number {
            get = get.sort_keys(vec![number.to_string()]);
        }
        self.read::<_, CommentRow>(get).await
    }

    /// Writes one comment row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_comment", skip_all, err(Debug))]
    async fn write_comment(&self, row: CommentRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = CommentRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Reads every loop row, in pane order, or the one of a pane
    ///
    /// # Arguments
    ///
    /// * `pane` - The one pane's key, or `None` for every loop
    #[instrument(name = "ShoalBackend::loops", skip_all, err(Debug))]
    async fn loops(&self, pane: Option<&str>) -> anyhow::Result<Vec<LoopRow>> {
        // every loop sits in the one listing partition
        let mut get = LoopRowGet::new(vec![LOOPS.to_string()]);
        if let Some(pane) = pane {
            get = get.sort_keys(vec![pane.to_string()]);
        }
        self.read::<_, LoopRow>(get).await
    }

    /// Writes one loop row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_loop", skip_all, err(Debug))]
    async fn write_loop(&self, row: LoopRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = LoopRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Deletes one loop row, if it is still at the version it was read at
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    #[instrument(name = "ShoalBackend::delete_loop", skip_all, err(Debug))]
    async fn delete_loop(&self, row: &LoopRow) -> anyhow::Result<Written> {
        // the row under its pane, only if no write moved it on since the read
        let delete = LoopRowDelete::new(row.list.clone(), row.pane.clone());
        let filter = LoopRowFilter {
            version: Some(vec![row.version]),
        };
        written(self.client.send_one(delete.if_matches(filter)).await)
    }

    /// Reads a node's decision rows, in number order
    ///
    /// # Arguments
    ///
    /// * `project` - The project the node's plan belongs to
    /// * `node` - The node's name within its project, `plan/id`
    #[instrument(name = "ShoalBackend::decisions", skip_all, err(Debug))]
    async fn decisions(&self, project: &str, node: &str) -> anyhow::Result<Vec<DecisionRow>> {
        // a node's decisions sit in the node's partition
        let get = DecisionRowGet::new(vec![(project.to_string(), node.to_string())]);
        self.read::<_, DecisionRow>(get).await
    }

    /// Writes one decision row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_decision", skip_all, err(Debug))]
    async fn write_decision(&self, row: DecisionRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = DecisionRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Reads every user row, in name order, or the one of a name
    ///
    /// # Arguments
    ///
    /// * `name` - The one user's name, or `None` for every user
    #[instrument(name = "ShoalBackend::users", skip_all, err(Debug))]
    async fn users(&self, name: Option<&str>) -> anyhow::Result<Vec<UserRow>> {
        // every user sits in the one listing partition
        let mut get = UserRowGet::new(vec![USERS.to_string()]);
        if let Some(name) = name {
            get = get.sort_keys(vec![name.to_string()]);
        }
        self.read::<_, UserRow>(get).await
    }

    /// Writes one user row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_user", skip_all, err(Debug))]
    async fn write_user(&self, row: UserRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = UserRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Deletes one user row, if it is still at the version it was read at
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    #[instrument(name = "ShoalBackend::delete_user", skip_all, err(Debug))]
    async fn delete_user(&self, row: &UserRow) -> anyhow::Result<Written> {
        // the row under its name, only if no write moved it on since the read
        let delete = UserRowDelete::new(row.list.clone(), row.name.clone());
        let filter = UserRowFilter {
            version: Some(vec![row.version]),
        };
        written(self.client.send_one(delete.if_matches(filter)).await)
    }

    /// Reads every token row, in hash order
    #[instrument(name = "ShoalBackend::tokens", skip_all, err(Debug))]
    async fn tokens(&self) -> anyhow::Result<Vec<TokenRow>> {
        // every token sits in the one listing partition
        let get = TokenRowGet::new(vec![TOKENS.to_string()]);
        self.read::<_, TokenRow>(get).await
    }

    /// Reads the token row kept under a token's hash, if one is
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's SHA-256, as 64 lowercase hex digits
    #[instrument(name = "ShoalBackend::token", skip_all, err(Debug))]
    async fn token(&self, hash: &str) -> anyhow::Result<Option<TokenRow>> {
        // every token sits in the one listing partition, under its hash
        let get = TokenRowGet::new(vec![TOKENS.to_string()]).sort_keys(vec![hash.to_string()]);
        let rows = self.read::<_, TokenRow>(get).await?;
        Ok(rows.into_iter().next())
    }

    /// Writes one token row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_token", skip_all, err(Debug))]
    async fn write_token(&self, row: TokenRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = TokenRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }

    /// Deletes one token row, if it is still at the version it was read at
    ///
    /// # Arguments
    ///
    /// * `row` - The row to delete, as it was read
    #[instrument(name = "ShoalBackend::delete_token", skip_all, err(Debug))]
    async fn delete_token(&self, row: &TokenRow) -> anyhow::Result<Written> {
        // the row under its hash, only if no write moved it on since the read
        let delete = TokenRowDelete::new(row.list.clone(), row.hash.clone());
        let filter = TokenRowFilter {
            version: Some(vec![row.version]),
        };
        written(self.client.send_one(delete.if_matches(filter)).await)
    }

    /// Reads every remote row, in project order
    #[instrument(name = "ShoalBackend::remotes", skip_all, err(Debug))]
    async fn remotes(&self) -> anyhow::Result<Vec<RemoteRow>> {
        // every remote sits in the one listing partition
        let get = RemoteRowGet::new(vec![REMOTES.to_string()]);
        self.read::<_, RemoteRow>(get).await
    }

    /// Writes one remote row, if the condition holds of the row stored under its key
    ///
    /// # Arguments
    ///
    /// * `row` - The row to write
    /// * `expect` - What must be stored under its key for it to be written
    #[instrument(name = "ShoalBackend::write_remote", skip_all, err(Debug))]
    async fn write_remote(&self, row: RemoteRow, expect: Expect) -> anyhow::Result<Written> {
        // each write is sent alone and its answer checked, never drained from a stream
        let sent = match expect {
            Expect::Absent => self.client.send_one(row.if_absent()).await,
            Expect::Version(read) => {
                // only over the version read
                let filter = RemoteRowFilter {
                    version: Some(vec![read]),
                };
                self.client.send_one(row.if_matches(filter)).await
            }
        };
        written(sent)
    }
}

/// Connects to shoal, retrying while a just-started server finishes binding
///
/// # Arguments
///
/// * `addr` - The address to connect to
#[instrument(name = "client::connect", skip_all, err(Debug))]
async fn connect(addr: SocketAddr) -> anyhow::Result<Shoal<ReefClient>> {
    // a loaded machine can refuse the very first connection, so try a few times
    let mut last = None;
    for _ in 0..CONNECT_TRIES {
        match Shoal::<ReefClient>::new(addr).await {
            Ok(client) => return Ok(client),
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(CONNECT_PAUSE).await;
    }
    Err(anyhow::anyhow!(
        "could not connect to shoal at {addr}: {:?}",
        last.expect("the loop ran at least once")
    ))
}
