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
//! - **Checks under the plan's lock.** The store keeps rows; it doesn't judge what they make.
//!   `put_checked` and `write_plan_checked` hand a caller's check the plan as it stands and as
//!   the write would leave it, under the plan's write lock, and write only if it finds no
//!   problem. [`Service`](crate::Service) checks every write this way; `put` and `write_plan`
//!   are the unchecked layer under it.

// the store's data types are the JSON API's, so the service and its clients share them
pub use reef_api::{
    AddedProject, Checked, Listing, NodePut, NodeWrite, PlanInfo, Project, Rewrite, Rewritten,
    StalePut, StoredNode, records,
};
use reef_model::{Diagnostic, PlanName, Record};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::instrument;

use crate::backend::{Backend, Expect, Pointers, Refusal, RevisionKey, Written};
use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;
use crate::memory::MemoryBackend;
use crate::repo::{self, Located};
use crate::schema::{NodeRow, PROJECTS, PlanRow, ProjectRow};

/// How many node rows a bulk write sends in one bundle
const BUNDLE_ROWS: usize = 256;

/// A plan as it stands and as a write would leave it, for a check to judge
#[derive(Debug, Clone)]
pub struct Proposal {
    /// The plan's nodes as they stand, in written order
    pub before: Vec<Record>,
    /// The plan's nodes as the write would leave them, in written order
    pub after: Vec<Record>,
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

/// A project the store was asked about that isn't registered
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotRegistered(pub String);

impl std::fmt::Display for NotRegistered {
    /// Names the project
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "project `{}` is not registered", self.0)
    }
}

impl std::error::Error for NotRegistered {}

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

/// A write the store can't keep as it was given, such as a node with members inline
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unkeepable(pub String);

impl std::fmt::Display for Unkeepable {
    /// Says what the store can't keep, and how to write it instead
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unkeepable {}

/// reef's store, over shoal or its in-memory twin
pub struct Store<B = ShoalBackend> {
    /// The rows under the store
    backend: B,
    /// Each plan's lock: shared by reads, held alone by writes
    locks: Mutex<HashMap<PlanName, Arc<RwLock<()>>>>,
    /// Held to register a project, so two can't take one repo between a check and a write
    projects: tokio::sync::Mutex<()>,
}

impl Store<ShoalBackend> {
    /// Opens a store: the external node reef's config names, else a pool in this process
    ///
    /// Only one process can open a data directory, so a process opens one store; most callers
    /// want [`Service::shared`](crate::Service::shared).
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
            projects: tokio::sync::Mutex::new(()),
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

    /// Registers a project for a repo, refusing a name or a repo another project has
    ///
    /// The repo is located first: a directory in a git repo, a linked worktree's too, names
    /// its main worktree's root, so one repo is one path however it was reached. Shoal has no
    /// secondary index, so the repo is checked by listing every project, under a lock this
    /// process holds, since it is shoal's only writer.
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in the repo its plans describe
    #[instrument(name = "Store::add_project", skip_all, err(Debug))]
    pub async fn add_project(&self, name: &str, repo: &Path) -> anyhow::Result<AddedProject> {
        // a project's name keeps the rule every plan's full name holds it to
        PlanName::new(name, "plan")?;
        // find the repo's root, which asks git, off the async threads
        let dir = repo.to_path_buf();
        let repo = tokio::task::spawn_blocking(move || repo::locate(&dir))
            .await??
            .into_path();
        let repo_text = repo
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", repo.display()))?
            .to_string();
        // one registration at a time, so the repo can't be taken between the check and the write
        let _registering = self.projects.lock().await;
        if let Some(taken) = self.projects().await?.into_iter().find(|p| p.repo == repo) {
            return Ok(AddedProject::RepoTaken {
                project: taken.name,
            });
        }
        // written only if no project of that name is
        let row = ProjectRow {
            list: PROJECTS.to_string(),
            name: name.to_string(),
            repo: repo_text,
        };
        Ok(match self.backend.insert_project(row).await? {
            Written::Applied => AddedProject::Added(Project {
                name: name.to_string(),
                repo,
            }),
            Written::Refused(_) => AddedProject::NameTaken,
        })
    }

    /// Lists every project, by name
    #[instrument(name = "Store::projects", skip_all, err(Debug))]
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        // every project sits in the one listing partition
        let rows = self.backend.projects().await?;
        rows.into_iter()
            .map(|row| {
                // a row of another partition that hashed the same is a collision, not a project
                anyhow::ensure!(
                    row.list == PROJECTS,
                    "project row `{}` from `{}` was stored where the projects are listed: their \
                     partition keys' hashes collide",
                    row.name,
                    row.list
                );
                Ok(Project {
                    name: row.name,
                    repo: PathBuf::from(row.repo),
                })
            })
            .collect()
    }

    /// Finds a registered project by name
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    #[instrument(name = "Store::project", skip_all, err(Debug))]
    pub async fn project(&self, name: &str) -> anyhow::Result<Option<Project>> {
        // every project sits in the one listing partition, which is small
        let projects = self.projects().await?;
        Ok(projects.into_iter().find(|project| project.name == name))
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Store::plans", skip_all, err(Debug))]
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
        // a project's plans sit in that project's partition
        let rows = self.backend.plans(project).await?;
        rows.into_iter()
            .map(|row| {
                // a row of another project whose key hashed the same is a collision
                anyhow::ensure!(
                    row.project == project,
                    "plan row `{}/{}` was stored where `{project}`'s plans are listed: their \
                     partition keys' hashes collide",
                    row.project,
                    row.name
                );
                Ok(PlanInfo {
                    name: PlanName::new(row.project, row.name)?,
                    revision: row.current,
                })
            })
            .collect()
    }

    /// Finds the project a directory belongs to
    ///
    /// A directory in a git repo belongs to the project registered for that repo's main
    /// worktree, so a linked worktree finds its main repo's project. One outside git belongs to
    /// the project registered for its nearest registered ancestor.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, such as the working directory
    #[instrument(name = "Store::find_project", skip_all, err(Debug))]
    pub async fn find_project(&self, dir: &Path) -> anyhow::Result<Option<Project>> {
        // locate the directory, which asks git, off the async threads
        let dir = dir.to_path_buf();
        let located = tokio::task::spawn_blocking(move || repo::locate(&dir)).await??;
        let projects = self.projects().await?;
        Ok(match located {
            // git draws a repo's bounds, so only the project for its root
            Located::Repo(root) => projects.into_iter().find(|project| project.repo == root),
            // outside git, the deepest project that holds the directory
            Located::Dir(dir) => projects
                .into_iter()
                .filter(|project| dir.starts_with(&project.repo))
                .max_by_key(|project| project.repo.components().count()),
        })
    }

    /// Lists a plan in its project, with an empty revision to put nodes into
    ///
    /// Returns whether it was added; a plan already listed is left as it is. Its project must
    /// be registered.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    #[instrument(name = "Store::add_plan", skip_all, err(Debug))]
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<bool> {
        // a plan belongs to a project that is registered
        let projects = self.projects().await?;
        if !projects.iter().any(|project| project.name == plan.project) {
            return Err(NotRegistered(plan.project.clone()).into());
        }
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
        listed_order(&mut nodes);
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
    /// This doesn't check what the node does to its plan; `put_checked` does.
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
    /// This doesn't check what the nodes make of the plan; `write_plan_checked` does.
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
        check_plan_nodes(plan, &nodes)?;
        // one write to this plan at a time
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left behind
        let (row, recovered) = self.recover(plan, row).await?;
        // 2. carry each node's version on, so a put read before this write is refused after it
        let old: HashMap<String, u64> = self.keys(plan, row.current).await?.into_iter().collect();
        // 3 to 6. write the new revision, make it current and prune the old one
        self.rewrite(plan, row, old, recovered, nodes).await
    }

    /// Writes a whole plan as a new revision, if a check finds no problem it would add
    ///
    /// Under the plan's write lock, this prunes what a killed write left, reads the current
    /// revision, and hands `check` the plan as it stands and as these nodes would make it. A
    /// check that finds problems refuses the write before a revision is reserved, so nothing
    /// is written. Otherwise the nodes are written as `write_plan` writes them.
    ///
    /// The check runs under the plan's write lock, so it must not read or write this plan
    /// through the store, or it waits forever.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `nodes` - Every node of the plan, each a node of its own
    /// * `check` - Finds the problems the write would add, or none
    #[instrument(name = "Store::write_plan_checked", skip_all, err(Debug))]
    pub async fn write_plan_checked<F, Fut>(
        &self,
        plan: &PlanName,
        mut nodes: Vec<NodeWrite>,
        check: F,
    ) -> anyhow::Result<Rewrite>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // every node must be one the store can keep, under an id no other node has
        check_plan_nodes(plan, &nodes)?;
        // one write to this plan at a time, so the check judges the plan this lands on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // prune whatever a killed write left behind
        let (row, recovered) = self.recover(plan, row).await?;
        // the revision as it stands, whose versions each node carries on
        let mut current = self.revision(plan, row.current, None).await?;
        listed_order(&mut current);
        let old: HashMap<String, u64> = current
            .iter()
            .map(|node| (node.spec.id.clone(), node.version))
            .collect();
        // judge the plan as these nodes would make it, and write nothing if it adds a problem
        in_written_order(&mut nodes);
        let before = records(plan, current.into_iter().map(NodeWrite::from).collect())?;
        let proposal = Proposal {
            before,
            after: records(plan, nodes.clone())?,
        };
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(Rewrite::Refused(problems));
        }
        // write the new revision, make it current and prune the old one
        let rewritten = self.rewrite(plan, row, old, recovered, nodes).await?;
        Ok(Rewrite::Applied(rewritten))
    }

    /// Writes a plan's nodes as a new revision, makes it current, and prunes the old one
    ///
    /// These are steps 3 to 6 of `write_plan`. The caller holds the plan's write lock, and has
    /// pruned what a killed write left.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `row` - The plan's row as it now stands
    /// * `old` - The version of each node of the current revision, by id
    /// * `recovered` - How many rows a killed write had left, which the caller pruned
    /// * `nodes` - Every node of the plan, each a node of its own
    #[instrument(name = "Store::rewrite", skip_all, err(Debug))]
    async fn rewrite(
        &self,
        plan: &PlanName,
        row: PlanRow,
        old: HashMap<String, u64>,
        recovered: usize,
        nodes: Vec<NodeWrite>,
    ) -> anyhow::Result<Rewritten> {
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

    /// Writes a batch of nodes into a plan's current revision, if each is as its writer read it
    /// and a check finds no problem they would add
    ///
    /// The steps, under the plan's write lock:
    ///
    /// 1. Prune whatever a killed write left.
    /// 2. Read the current revision whole.
    /// 3. Refuse the batch as [`Checked::Stale`] if any put finds its node other than its
    ///    writer read it, naming every such put.
    /// 4. Hand `check` the plan as it stands and as the puts would leave it, each replacing
    ///    its node in place or joining the plan, and refuse the batch as [`Checked::Refused`]
    ///    if it finds problems.
    /// 5. Write each put in the order given, each only over the version its writer read.
    ///
    /// A refused batch writes nothing. A batch isn't atomic, though: reef-service is shoal's
    /// only writer, so a write refused in step 5 means another process wrote the plan, and a
    /// write that fails there leaves the puts before it written. Either is an error that names
    /// what was written.
    ///
    /// The check runs under the plan's write lock, so it must not read or write this plan
    /// through the store, or it waits forever.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `puts` - The nodes to write, each with the version its writer read
    /// * `check` - Finds the problems the puts would add, or none
    #[instrument(name = "Store::put_checked", skip_all, err(Debug))]
    pub async fn put_checked<F, Fut>(
        &self,
        plan: &PlanName,
        puts: Vec<NodePut>,
        check: F,
    ) -> anyhow::Result<Checked>
    where
        F: FnOnce(Proposal) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<Vec<Diagnostic>>> + Send,
    {
        // every node must be one the store can keep, each put once
        let mut ids = HashSet::with_capacity(puts.len());
        for put in &puts {
            check_node(plan, &put.node)?;
            anyhow::ensure!(
                ids.insert(put.node.spec.id.as_str()),
                "node `{}` is put twice in one batch",
                put.node.spec.id
            );
        }
        // one write to this plan at a time, so the check judges the plan the puts land on
        let _write = self.writing(plan).await;
        let row = self.plan_row(plan).await?;
        // 1. prune whatever a killed write left
        let (row, _) = self.recover(plan, row).await?;
        // 2. the revision as it stands, in written order
        let mut current = self.revision(plan, row.current, None).await?;
        listed_order(&mut current);
        // 3. every put must find its node as its writer read it, before anything is judged
        let stale = stale_puts(&current, &puts);
        if !stale.is_empty() {
            return Ok(Checked::Stale(stale));
        }
        // an empty batch changes nothing
        if puts.is_empty() {
            return Ok(Checked::Applied {
                versions: Vec::new(),
            });
        }
        // 4. the plan as the puts would leave it: each replaces its node in place, or joins it
        let mut after: Vec<NodeWrite> = current.into_iter().map(NodeWrite::from).collect();
        let before = records(plan, after.clone())?;
        let at: HashMap<String, usize> = after
            .iter()
            .enumerate()
            .map(|(i, node)| (node.spec.id.clone(), i))
            .collect();
        for put in &puts {
            match at.get(&put.node.spec.id) {
                Some(&i) => after[i] = put.node.clone(),
                None => after.push(put.node.clone()),
            }
        }
        in_written_order(&mut after);
        // judge it, and write nothing if it adds a problem
        let proposal = Proposal {
            before,
            after: records(plan, after)?,
        };
        let problems = check(proposal).await?;
        if !problems.is_empty() {
            return Ok(Checked::Refused(problems));
        }
        // 5. write each put in the order given, each only over the version its writer read
        let mut written: Vec<(String, u64)> = Vec::with_capacity(puts.len());
        for put in puts {
            let id = put.node.spec.id.clone();
            let (version, expect) = match put.read {
                Some(read) => (read + 1, Expect::Version(read)),
                None => (1, Expect::Absent),
            };
            let new = node_row(plan, row.current, put.node, version)?;
            match self.backend.write_node(new, expect).await {
                Ok(Written::Applied) => written.push((id, version)),
                // this process holds the plan's lock and saw the node as read, so another did
                Ok(Written::Refused(refusal)) => anyhow::bail!(
                    "node `{id}` of plan `{plan}` changed under this batch ({refusal}): another \
                     process wrote the plan, after {} were written",
                    names(&written)
                ),
                Err(error) => {
                    return Err(error.context(format!(
                        "writing node `{id}` of plan `{plan}`, after {} were written",
                        names(&written)
                    )));
                }
            }
        }
        Ok(Checked::Applied {
            versions: written.into_iter().map(|(_, version)| version).collect(),
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

/// Refuses a whole plan the store can't keep: a node it can't keep as one row, or an id twice
///
/// # Arguments
///
/// * `plan` - The plan it is written to
/// * `nodes` - Every node of the plan
fn check_plan_nodes(plan: &PlanName, nodes: &[NodeWrite]) -> anyhow::Result<()> {
    // every node must be one the store can keep, under an id no other node has
    let mut ids = HashSet::with_capacity(nodes.len());
    for node in nodes {
        check_node(plan, node)?;
        if !ids.insert(node.spec.id.as_str()) {
            let message = format!("node `{}` is written twice", node.spec.id);
            return Err(Unkeepable(message).into());
        }
    }
    Ok(())
}

/// The puts of a batch that find their nodes other than their writers read them
///
/// # Arguments
///
/// * `current` - The plan's current revision, as read under its write lock
/// * `puts` - The batch
fn stale_puts(current: &[StoredNode], puts: &[NodePut]) -> Vec<StalePut> {
    // each node's version as it stands, by id
    let versions: HashMap<&str, u64> = current
        .iter()
        .map(|node| (node.spec.id.as_str(), node.version))
        .collect();
    puts.iter()
        .filter_map(|put| {
            // as a conditional write would judge it
            let refusal = match (versions.get(put.node.spec.id.as_str()), put.read) {
                (None, None) => return None,
                (Some(stored), Some(read)) if *stored == read => return None,
                (Some(_), None) => Refusal::Exists,
                (None, Some(_)) => Refusal::Missing,
                (Some(_), Some(_)) => Refusal::Moved,
            };
            Some(StalePut {
                id: put.node.spec.id.clone(),
                refusal,
            })
        })
        .collect()
}

/// Puts stored nodes in the order a listing reads in: by written order, then by id
///
/// # Arguments
///
/// * `nodes` - The nodes to sort
fn listed_order(nodes: &mut [StoredNode]) {
    nodes.sort_by(|a, b| (a.order, &a.spec.id).cmp(&(b.order, &b.spec.id)));
}

/// Puts nodes to be written in the order a listing of them will read in: by written order,
/// then by id
///
/// # Arguments
///
/// * `nodes` - The nodes to sort
fn in_written_order(nodes: &mut [NodeWrite]) {
    nodes.sort_by(|a, b| (a.order, &a.spec.id).cmp(&(b.order, &b.spec.id)));
}

/// Names the nodes a batch wrote before it stopped, for its error
///
/// # Arguments
///
/// * `written` - Each node written, with its new version
fn names(written: &[(String, u64)]) -> String {
    // none, or each id in backticks
    if written.is_empty() {
        return "none".to_string();
    }
    written
        .iter()
        .map(|(id, _)| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ")
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
    if !node.spec.nodes.is_empty() {
        let message = format!(
            "node `{}` lists {} members inline: put each as a node of its own, its `group` \
             naming `{}`",
            node.spec.id,
            node.spec.nodes.len(),
            node.spec.id
        );
        return Err(Unkeepable(message).into());
    }
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
    use super::{AddedProject, NodeWrite, Store};
    use crate::backend::RevisionKey;
    use crate::schema::NodeRow;
    use reef_model::PlanName;
    use std::path::Path;

    /// A second project for a repo, however its path is spelled, or under a taken name, is refused
    #[tokio::test]
    async fn duplicate_name_or_repo_refused() -> anyhow::Result<()> {
        // a project for the repo this crate is in, located at its root
        let store = Store::memory();
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let AddedProject::Added(reef) = store.add_project("reef", here).await? else {
            panic!("the first project for a repo was refused");
        };
        assert!(
            reef.repo.join(".git").exists(),
            "{} is no repo's root",
            reef.repo.display()
        );
        // another for the same repo, spelled through a subdirectory and `..`, is refused
        let respelled = here.join("src").join("..");
        let taken = AddedProject::RepoTaken {
            project: "reef".to_string(),
        };
        assert_eq!(store.add_project("again", &respelled).await?, taken);
        // and so is the name, for another directory
        assert_eq!(
            store.add_project("reef", Path::new("/")).await?,
            AddedProject::NameTaken
        );
        // a name that breaks the rule names keep is an error
        assert!(
            store
                .add_project("no spaces", Path::new("/"))
                .await
                .is_err()
        );
        // and only the first is registered
        assert_eq!(store.projects().await?, vec![reef]);
        Ok(())
    }

    /// A row stored in a plan's partition under another plan's key is an error, never a node
    #[tokio::test]
    async fn collided_row_is_an_error() -> anyhow::Result<()> {
        // a plan with one node of its own
        let store = Store::memory();
        store
            .add_project("reef", Path::new(env!("CARGO_MANIFEST_DIR")))
            .await?;
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
