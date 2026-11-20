//! The store suite's fixtures: a store over the twin or over shoal, and a plan in it
//!
//! Every case is written once, generic over the backend, and `tests/store.rs` runs each against
//! both. Shoal's cases run one at a time, each over a fresh data directory of its own.

#![allow(dead_code)]

pub mod cases;

use reef_model::{NodeSpec, PlanName};
use reef_service::{Backend, MemoryBackend, NodeWrite, Store};

/// Held by each shoal case, so only one runs its pool at a time
#[cfg(feature = "embedded")]
pub static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A store, and the plan each case works in
pub struct Fx<B> {
    /// The store under test
    pub store: Store<B>,
    /// The plan the case works in, listed before the case starts
    pub plan: PlanName,
}

impl Fx<MemoryBackend> {
    /// A store over a fresh twin, with the case's plan listed
    pub async fn memory() -> anyhow::Result<Fx<MemoryBackend>> {
        Fx::listed(Store::memory()).await
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
        let mut conf = reef_service::Conf::default();
        conf.shoal.data = Some(dir);
        conf.shoal.cores = 1;
        Fx::listed(Store::open(&conf).await?).await
    }
}

impl<B: Backend> Fx<B> {
    /// Lists the case's plan in a store
    ///
    /// # Arguments
    ///
    /// * `store` - The store to list it in
    async fn listed(store: Store<B>) -> anyhow::Result<Fx<B>> {
        let plan = PlanName::new("reef", "central")?;
        anyhow::ensure!(
            store.add_plan(&plan).await?,
            "a fresh store already listed the plan"
        );
        Ok(Fx { store, plan })
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
