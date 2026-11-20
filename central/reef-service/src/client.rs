//! reef's client of shoal: a pool started in this process, or an external node
//!
//! The client is the same either way. Only start-up differs: in-process starts a pool first and
//! connects to the address it bound, while external connects to the address it was given.

use shoal::Shoal;
use std::net::SocketAddr;
use std::time::Duration;
use tracing::instrument;

use crate::conf::Conf;
use crate::schema::ReefClient;

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
