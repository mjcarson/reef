//! reef's store, over one client of shoal
//!
//! A process keeps one store: reef-service is shoal's only writer, and the store is what will
//! serialize its writes. `Store::shared` opens it once however often a caller asks, since
//! `dioxus::serve` runs its setup again after every hot-patch, and a second pool would find the
//! data directory locked.

use tokio::sync::OnceCell;
use tracing::instrument;

use crate::client::{Mode, ShoalBackend};
use crate::conf::Conf;

/// The process's one store, opened on first use
static SHARED: OnceCell<Store> = OnceCell::const_new();

/// reef's store
pub struct Store {
    /// How the store reaches shoal
    backend: ShoalBackend,
}

impl Store {
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
        Ok(Store { backend })
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

    /// The client of shoal under this store
    pub fn backend(&self) -> &ShoalBackend {
        &self.backend
    }

    /// Closes the store, stopping its in-process pool if it started one
    #[instrument(name = "Store::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // the backend holds the client and the pool
        self.backend.close().await
    }
}
