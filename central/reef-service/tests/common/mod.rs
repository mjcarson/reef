//! The store suite's fixtures: a store over the twin or over shoal, and a plan in it
//!
//! Every case is written once, generic over the backend, and `tests/store.rs` runs each against
//! both. Shoal's cases run one at a time, each over a fresh data directory of its own. Each store
//! sits over a [`Faulty`] backend, so a case can kill a write midway, then reopen the store over
//! what it left.

#![allow(dead_code)]

pub mod cases;
pub mod faulty;

use reef_model::{NodeSpec, PlanName};
use reef_service::{Backend, Conf, MemoryBackend, NodeWrite, Store};
use std::future::Future;
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

/// A store, the faults its backend can be armed with, and the plan each case works in
pub struct Fx<B> {
    /// The store under test
    pub store: Store<Faulty<B>>,
    /// The faults its backend fails or waits on
    pub faults: Arc<Faults>,
    /// The plan the case works in, listed before the case starts
    pub plan: PlanName,
    /// The config the store was opened with
    conf: Conf,
}

impl Fx<MemoryBackend> {
    /// A store over a fresh twin, with the case's plan listed
    pub async fn memory() -> anyhow::Result<Fx<MemoryBackend>> {
        Fx::listed(MemoryBackend::new(), Conf::default()).await
    }
}

#[cfg(feature = "embedded")]
impl Fx<reef_service::ShoalBackend> {
    /// A store over a fresh in-process pool, with the case's plan listed
    ///
    /// The pool's data sits under cargo's target dir, never a tmpfs `/tmp`. Dropping the
    /// fixture stops the pool in the background, which takes up to 3 s, so the next case need
    /// not wait for it.
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its data directory
    pub async fn shoal(case: &str) -> anyhow::Result<Fx<reef_service::ShoalBackend>> {
        // a fresh directory for this case alone
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("store")
            .join(case);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        // one shard over it
        let mut conf = Conf::default();
        conf.shoal.data = Some(dir);
        conf.shoal.cores = 1;
        let backend = reef_service::ShoalBackend::open(&conf).await?;
        Fx::listed(backend, conf).await
    }
}

impl<B: Backend> Fx<B> {
    /// Lists the case's plan in a store over a backend
    ///
    /// # Arguments
    ///
    /// * `backend` - The backend under the store
    /// * `conf` - The config it was opened with
    async fn listed(backend: B, conf: Conf) -> anyhow::Result<Fx<B>> {
        // nothing armed yet
        let faults = Arc::new(Faults::default());
        let store = Store::new(Faulty::new(backend, faults.clone()));
        let plan = PlanName::new("reef", "central")?;
        anyhow::ensure!(
            store.add_plan(&plan).await?,
            "a fresh store already listed the plan"
        );
        Ok(Fx {
            store,
            faults,
            plan,
            conf,
        })
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
