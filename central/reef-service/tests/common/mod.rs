//! The store suite's fixtures: a store over the twin or over shoal, and a project and plan in it
//!
//! Every case is written once, generic over the backend, and `tests/store.rs` runs each against
//! both. Shoal's cases run one at a time, each over a fresh data directory of its own. Each store
//! sits over a [`Faulty`] backend, so a case can kill a write midway, then reopen the store over
//! what it left. Each case's project is registered for a fresh git repo of its own.

#![allow(dead_code)]

pub mod api_cases;
pub mod cases;
pub mod faulty;
pub mod git;
pub mod http;
pub mod queue_cases;
pub mod service_cases;

use reef_model::{Description, NodeSpec, PlanName};
use reef_service::{AddedProject, Backend, Conf, MemoryBackend, NodeWrite, Service, Store};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use faulty::{Faults, Faulty};

/// Held by each shoal case, so only one runs its pool at a time
#[cfg(feature = "embedded")]
pub static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A backend a store can be reopened over, keeping its rows, as a restarted process would
pub trait Reopen: Backend + Sized {
    /// Closes this backend and opens it again over the same rows
    ///
    /// # Arguments
    ///
    /// * `conf` - The config it was opened with
    fn reopen(self, conf: &Conf) -> impl Future<Output = anyhow::Result<Self>>;
}

impl Reopen for MemoryBackend {
    /// The same rows, under a store with none of the last one's locks
    ///
    /// # Arguments
    ///
    /// * `conf` - Unused: the twin has no config
    async fn reopen(self, _conf: &Conf) -> anyhow::Result<MemoryBackend> {
        Ok(self)
    }
}

#[cfg(feature = "embedded")]
impl Reopen for reef_service::ShoalBackend {
    /// Stops the pool, waiting for every shard, and starts a new one over its directory
    ///
    /// # Arguments
    ///
    /// * `conf` - The config the pool was started with
    async fn reopen(self, conf: &Conf) -> anyhow::Result<reef_service::ShoalBackend> {
        // the data directory is locked until the last pool has stopped
        self.close().await?;
        reef_service::ShoalBackend::open(conf).await
    }
}

/// A store, the faults its backend can be armed with, and the project and plan a case works in
pub struct Fx<B> {
    /// The store under test
    pub store: Store<Faulty<B>>,
    /// The faults its backend fails or waits on
    pub faults: Arc<Faults>,
    /// The plan the case works in, listed before the case starts
    pub plan: PlanName,
    /// The repo of the plan's project, `reef`, registered before the case starts
    pub repo: PathBuf,
    /// A directory of the case's own, under cargo's target dir, which holds the repo
    pub dir: PathBuf,
    /// The config the store was opened with
    conf: Conf,
}

/// A service over a store, the faults its backend can be armed with, and the project and plan a
/// case works in
pub struct Sx<B> {
    /// The service under test
    pub service: Service<Faulty<B>>,
    /// The faults its backend fails or waits on
    pub faults: Arc<Faults>,
    /// The plan the case works in, listed before the case starts
    pub plan: PlanName,
    /// The repo of the plan's project, `reef`, whose working tree holds the plan's docs
    pub repo: PathBuf,
    /// A directory of the case's own, under cargo's target dir, which holds the repo
    pub dir: PathBuf,
}

/// A fresh directory for one case, under cargo's target dir rather than a tmpfs `/tmp`
///
/// # Arguments
///
/// * `kind` - What the directory is for, such as `repos`
/// * `name` - The case's name, and its backend's
fn fresh(kind: &str, name: &str) -> anyhow::Result<PathBuf> {
    // wipe what an earlier run left
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(kind).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl Fx<MemoryBackend> {
    /// A store over a fresh twin, with the case's project registered and its plan listed
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    pub async fn memory(case: &str) -> anyhow::Result<Fx<MemoryBackend>> {
        let dir = fresh("repos", &format!("memory-{case}"))?;
        Fx::listed(MemoryBackend::new(), Conf::default(), dir).await
    }
}

#[cfg(feature = "embedded")]
impl Fx<reef_service::ShoalBackend> {
    /// A store over a fresh in-process pool, with the case's project registered and plan listed
    ///
    /// The pool's data sits under cargo's target dir, never a tmpfs `/tmp`. Dropping the
    /// fixture stops the pool in the background, which takes up to 3 s, so the next case need
    /// not wait for it.
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directories
    pub async fn shoal(case: &str) -> anyhow::Result<Fx<reef_service::ShoalBackend>> {
        // a fresh data directory for this case alone, with one shard over it
        let mut conf = Conf::default();
        conf.shoal.data = Some(fresh("store", case)?);
        conf.shoal.cores = 1;
        let backend = reef_service::ShoalBackend::open(&conf).await?;
        Fx::listed(backend, conf, fresh("repos", &format!("shoal-{case}"))?).await
    }
}

impl<B: Backend> Fx<B> {
    /// Registers the case's project and lists its plan, in a store over a backend
    ///
    /// # Arguments
    ///
    /// * `backend` - The backend under the store
    /// * `conf` - The config it was opened with
    /// * `dir` - The case's own directory, to make the project's repo in
    async fn listed(backend: B, conf: Conf, dir: PathBuf) -> anyhow::Result<Fx<B>> {
        // nothing armed yet
        let faults = Arc::new(Faults::default());
        let store = Store::new(Faulty::new(backend, faults.clone()));
        // a project for a repo of the case's own
        let repo = git::repo(&dir.join("reef"))?;
        let added = store.add_project("reef", &repo).await?;
        anyhow::ensure!(
            matches!(added, AddedProject::Added(_)),
            "registering reef: {added:?}"
        );
        // and a plan of it
        let plan = PlanName::new("reef", "central")?;
        anyhow::ensure!(
            store.add_plan(&plan).await?,
            "a fresh store already listed the plan"
        );
        Ok(Fx {
            store,
            faults,
            plan,
            repo,
            dir,
            conf,
        })
    }
}

impl<B: Backend> Fx<B> {
    /// A service over this fixture's store, its project and plan kept
    pub fn serve(self) -> Sx<B> {
        Sx {
            service: Service::new(self.store),
            faults: self.faults,
            plan: self.plan,
            repo: self.repo,
            dir: self.dir,
        }
    }
}

impl<B: Reopen> Fx<B> {
    /// Reopens the store over the rows it left, as a restarted process would
    pub async fn reopen(self) -> anyhow::Result<Fx<B>> {
        // the backend alone survives, without the last store's locks or faults
        let backend = self.store.into_backend().into_inner();
        let backend = backend.reopen(&self.conf).await?;
        let faults = Arc::new(Faults::default());
        Ok(Fx {
            store: Store::new(Faulty::new(backend, faults.clone())),
            faults,
            plan: self.plan,
            repo: self.repo,
            dir: self.dir,
            conf: self.conf,
        })
    }
}

/// A node with a spec worth reading back: a title, a status, a group and needs
///
/// # Arguments
///
/// * `id` - The node's id
/// * `order` - Where it falls in the order its plan was written in
pub fn node(id: &str, order: u64) -> NodeWrite {
    NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} A node")),
            status: Some(reef_model::Status::InProgress),
            group: Some("M2".to_string()),
            needs: vec!["C6".to_string()],
            ..NodeSpec::default()
        },
        body: format!("# {id}\n\nIts body.\n"),
        order,
    }
}

/// The ids a plan lists, in written order
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn ids<B: Backend>(fx: &Fx<B>) -> anyhow::Result<Vec<String>> {
    let listing = fx.store.list(&fx.plan).await?;
    Ok(listing.nodes.into_iter().map(|node| node.spec.id).collect())
}

/// A piece of work that says why it is needed and what it does, with nothing it needs
///
/// # Arguments
///
/// * `id` - The node's id
/// * `order` - Where it falls in the order its plan was written in
pub fn work(id: &str, order: u64) -> NodeWrite {
    NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} Some work")),
            description: Some(Description {
                why: Some(format!("Without {id}, something is missing.")),
                what: Some(format!("With {id}, it is there.")),
                details: None,
            }),
            ..NodeSpec::default()
        },
        body: format!("# {id}\n\nIts body.\n"),
        order,
    }
}

/// A group, which says neither why nor what, since a group's members do
///
/// # Arguments
///
/// * `id` - The group's id
/// * `order` - Where it falls in the order its plan was written in
pub fn group(id: &str, order: u64) -> NodeWrite {
    NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} A group")),
            ..NodeSpec::default()
        },
        body: String::new(),
        order,
    }
}

/// A piece of work in a group
///
/// # Arguments
///
/// * `id` - The node's id
/// * `group` - The group it belongs to
/// * `order` - Where it falls in the order its plan was written in
pub fn member(id: &str, group: &str, order: u64) -> NodeWrite {
    let mut node = work(id, order);
    node.spec.group = Some(group.to_string());
    node
}

/// Writes a doc into a repo's working tree, making its directories
///
/// # Arguments
///
/// * `repo` - The repo's root
/// * `path` - The doc's path from the root
/// * `text` - The doc's markdown
pub fn doc(repo: &Path, path: &str, text: &str) -> anyhow::Result<()> {
    // its directories first, then the doc itself
    let file = repo.join(path);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)?;
    Ok(())
}

/// A design doc with a why, a what and design choices under one of its headings
pub const DESIGN: &str = "# Design\n\n## Rows\n\n### Context\n\nRows are slow.\n\n### What it does\n\nRows go out together.\n\n### Design choices\n\n- **One write.** Atomic.\n";
