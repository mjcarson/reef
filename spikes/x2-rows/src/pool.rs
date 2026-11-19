//! An in-process shoal pool and a client connected to it, adapted from X1's store
//!
//! X1 settled that an external node is reached through the same client, so X2 runs shoal
//! in-process alone.

use shoal::server::conf::{DefaultStorageSettings, Networking, Resources, Storage};
use shoal::server::tables::storage::fs::conf::{
    FileSystemLatencyWriterConf, FileSystemTableConf, FileSystemThroughputWriterConf,
};
use shoal::{Conf, Shoal, ShoalPool};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tracing::instrument;

use crate::tables::{X2, X2Client};

/// A pool running on a thread of its own, stopped when this is closed or dropped
pub struct Pool {
    /// Tells the pool's thread to stop the pool
    stop: Sender<()>,
    /// Hears how stopping the pool went
    stopped: tokio::sync::oneshot::Receiver<anyhow::Result<()>>,
    /// The address the pool bound
    pub addr: SocketAddr,
    /// How long the pool took from start to ready
    pub ready_in: Duration,
}

impl Pool {
    /// Starts a pool over a data directory and waits for it to be ready
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory to keep the pool's data in, off tmpfs
    #[instrument(name = "Pool::start", skip_all, err(Debug))]
    pub async fn start(dir: &Path) -> anyhow::Result<Pool> {
        // shoal's data paths must be absolute, so make the directory and resolve it first
        std::fs::create_dir_all(dir)?;
        let dir = dir.canonicalize()?;
        let conf = conf(&dir)?;
        // the pool reports its address, or why it could not start, through this
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        // and its thread hears when to stop through this, and says how stopping went
        let (stop, stop_rx) = std::sync::mpsc::channel();
        let (stopped_tx, stopped) = tokio::sync::oneshot::channel();
        let started = Instant::now();
        std::thread::Builder::new()
            .name("shoal-pool".to_string())
            .spawn(move || hold(conf, ready_tx, stop_rx, stopped_tx))?;
        // wait for the thread to say how starting went
        let addr = ready_rx.await??;
        Ok(Pool {
            stop,
            stopped,
            addr,
            ready_in: started.elapsed(),
        })
    }

    /// Stops the pool and waits for every shard to finish
    #[instrument(name = "Pool::close", skip_all, err(Debug))]
    pub async fn close(self) -> anyhow::Result<()> {
        // ask the thread to stop the pool; a thread already gone has nothing left to stop
        if self.stop.send(()).is_err() {
            return Ok(());
        }
        // and wait to hear that it did
        self.stopped.await?
    }
}

/// Starts a pool, reports its address, and holds it until told to stop or a shard dies
///
/// # Arguments
///
/// * `conf` - The pool's config
/// * `ready` - Where to report the pool's address, or why it could not start
/// * `stop` - Where a request to stop arrives
/// * `stopped` - Where to report how stopping went
fn hold(
    conf: Conf,
    ready: tokio::sync::oneshot::Sender<anyhow::Result<SocketAddr>>,
    stop: Receiver<()>,
    stopped: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
) {
    // start every shard, and wait for them all to answer
    let started = ShoalPool::<X2>::start(conf).and_then(|mut pool| {
        let addr = pool.ready(Duration::from_secs(30))?;
        Ok((pool, addr))
    });
    let pool = match started {
        Ok((pool, addr)) => {
            // tell the caller where to connect
            let _ = ready.send(Ok(addr));
            pool
        }
        Err(error) => {
            // shoal's error can carry a thread's panic payload, which isn't `Sync`, so it goes
            // as its message
            let _ = ready.send(Err(anyhow::anyhow!("shoal failed to start: {error:?}")));
            return;
        }
    };
    // hold the pool up until told to stop, reporting a shard that dies
    loop {
        match stop.recv_timeout(Duration::from_millis(250)) {
            // a stop request, or the pool's owner gone: stop every shard
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            // nothing yet: check the shards are still running
            Err(RecvTimeoutError::Timeout) => {
                if let Some((shard, error)) = pool.failure() {
                    tracing::error!(shard, %error, "shoal: a shard died");
                    return;
                }
            }
        }
    }
    // stop the shards and say how it went
    let result = pool
        .exit()
        .map_err(|error| anyhow::anyhow!("shoal failed to stop: {error:?}"));
    let _ = stopped.send(result);
}

/// The pool's config: two shards, a small budget, and an ephemeral loopback port
///
/// # Arguments
///
/// * `dir` - The absolute directory to store the pool's data in
fn conf(dir: &Path) -> anyhow::Result<Conf> {
    // two shards leave the rest of the machine to tokio; a budget must be set, since with no
    // `resources` it is 0 and eviction never stops
    let resources = Resources::default()
        .cores(2)
        .memory("256MiB")
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    // an ephemeral port on loopback, since only this process connects to it
    let networking = Networking::default().interface("127.0.0.1").port(0);
    // both writers in the one directory
    let storage = Storage::default().default_settings(
        DefaultStorageSettings::default().filesystem(
            FileSystemTableConf::default()
                .latency_sensitive(FileSystemLatencyWriterConf::default().path(dir))
                .throughput_sensitive(FileSystemThroughputWriterConf::default().path(dir)),
        ),
    );
    Ok(Conf::default()
        .resources(resources)
        .networking(networking)
        .storage(storage))
}

/// Where a run keeps its data: the directory given, else the spike's `data/<name>`
///
/// # Arguments
///
/// * `given` - A directory named on the command line, if one was
/// * `name` - The directory under the spike's `data/` to use when none was
pub fn data_dir(given: Option<PathBuf>, name: &str) -> PathBuf {
    // an explicit directory wins
    if let Some(dir) = given {
        return dir;
    }
    // else beside the spike's sources, on the checkout's disk rather than a tmpfs `/tmp`
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data").join(name)
}

/// Connects to shoal, retrying while a just-started pool finishes binding
///
/// # Arguments
///
/// * `addr` - The address to connect to
pub async fn connect(addr: SocketAddr) -> anyhow::Result<Shoal<X2Client>> {
    // a loaded machine can refuse the very first connection, so try a few times
    let mut last = None;
    for _ in 0..40 {
        match Shoal::<X2Client>::new(addr).await {
            Ok(client) => return Ok(client),
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(anyhow::anyhow!(
        "could not connect to shoal at {addr}: {:?}",
        last.expect("the loop ran at least once")
    ))
}
