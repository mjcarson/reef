//! A shoal pool started in this process, on a thread of its own
//!
//! Starting a pool and waiting for it to be ready both block, and shoal's start error can carry a
//! thread's panic payload, which isn't `Sync`, so the pool lives on a thread that reports its
//! address, or why it could not start, as a message.

use shoal::server::conf::{DefaultStorageSettings, Networking, Resources, Storage};
use shoal::server::tables::storage::fs::conf::{
    FileSystemLatencyWriterConf, FileSystemTableConf, FileSystemThroughputWriterConf,
};
use shoal::{Conf, ShoalPool};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tracing::instrument;

use crate::conf::ShoalSettings;
use crate::schema::Reef;

/// How long a pool may take to have every shard answer
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// The stack each thread gets unless `RUST_MIN_STACK` says otherwise: 8 MiB, which shoal's
/// shards need for reef's schema in an unoptimised build
pub const SHARD_STACK: usize = 8 << 20;

/// Gives every thread this process starts at least the stack shoal's shards need, unless
/// `RUST_MIN_STACK` says otherwise
///
/// glommio starts each of shoal's shards on a thread with std's default stack, 2 MiB, which an
/// unoptimised build of reef's thirteen tables overflows. std reads `RUST_MIN_STACK` when it
/// starts its first thread, so a server calls this first in `main`, before any thread starts.
/// `cargo test` and `cargo run` get the same from `central/.cargo/config.toml`.
///
/// # Safety
///
/// No other thread may be running, since it sets an environment variable.
pub unsafe fn widen_stacks() {
    // a stack size set already is the caller's to keep
    if std::env::var_os("RUST_MIN_STACK").is_none() {
        // SAFETY: the caller has started no other thread to read the environment meanwhile
        unsafe { std::env::set_var("RUST_MIN_STACK", SHARD_STACK.to_string()) };
    }
}

/// How often the pool's thread checks for a stop request or a dead shard
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A pool running on a thread of its own, stopped when this is closed or dropped
///
/// Dropping it stops the pool in the background. Closing it waits for every shard to finish,
/// which takes up to 3 s, since shoal's shards poll their shutdown flag that often.
pub struct Pool {
    /// Tells the pool's thread to stop the pool
    stop: Sender<()>,
    /// Hears how stopping the pool went
    stopped: tokio::sync::oneshot::Receiver<anyhow::Result<()>>,
    /// The address the pool bound
    pub addr: SocketAddr,
}

impl Pool {
    /// Starts a pool and waits for it to be ready
    ///
    /// # Arguments
    ///
    /// * `conf` - The pool's config, its data paths absolute
    #[instrument(name = "Pool::start", skip_all, err(Debug))]
    pub async fn start(conf: Conf) -> anyhow::Result<Pool> {
        // the pool reports its address, or why it could not start, through this
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        // and its thread hears when to stop through this, and says how stopping went
        let (stop, stop_rx) = std::sync::mpsc::channel();
        let (stopped_tx, stopped) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("shoal-pool".to_string())
            .spawn(move || hold(conf, ready_tx, stop_rx, stopped_tx))?;
        // wait for the thread to say how starting went
        let addr = ready_rx.await??;
        Ok(Pool {
            stop,
            stopped,
            addr,
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
    let started = ShoalPool::<Reef>::start(conf).and_then(|mut pool| {
        let addr = pool.ready(READY_TIMEOUT)?;
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
        match stop.recv_timeout(POLL_INTERVAL) {
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

/// The in-process pool's config: reef's cores and budget, on an ephemeral loopback port
///
/// # Arguments
///
/// * `settings` - reef's shoal settings
/// * `dir` - The absolute directory to keep the pool's data in
pub fn shoal_conf(settings: &ShoalSettings, dir: &Path) -> anyhow::Result<Conf> {
    // cores must be set, or a shard is pinned to every cpu but cpu 0; and a budget must be,
    // since with no `resources` it is 0 and eviction never stops
    let resources = Resources::default()
        .cores(settings.cores)
        // kept off the physical cores something else on this machine owns, such as a benchmark
        .exclude_cores(settings.exclude_cores.clone())
        .memory(settings.memory.as_str())
        .map_err(|error| {
            anyhow::anyhow!("shoal's memory budget `{}`: {error:?}", settings.memory)
        })?;
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

#[cfg(test)]
mod tests {
    use super::shoal_conf;
    use crate::conf::ShoalSettings;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// The pool's shards keep off every physical core the config excludes
    #[test]
    fn shards_keep_off_excluded_cores() {
        // a pool asked for more shards than any machine has cpus takes every one it may
        let settings = ShoalSettings {
            cores: 4096,
            ..ShoalSettings::default()
        };
        let every = shoal_conf(&settings, Path::new("/var/tmp"))
            .expect("the config builds")
            .resources
            .cpus()
            .expect("the cpus are read");
        // the physical cores those cpus are on
        let cores = every
            .iter()
            .map(|location| location.core)
            .collect::<BTreeSet<_>>();
        // keep the last core alone, excluding every other one
        let kept = *cores
            .last()
            .expect("this machine has a core besides cpu 0's");
        let settings = ShoalSettings {
            cores: 1,
            exclude_cores: cores.iter().copied().filter(|core| *core != kept).collect(),
            ..ShoalSettings::default()
        };
        let chosen = shoal_conf(&settings, Path::new("/var/tmp"))
            .expect("the config builds")
            .resources
            .cpus()
            .expect("the cpus are read");
        // the one shard is on the core that was kept
        let chosen = chosen
            .iter()
            .map(|location| location.core)
            .collect::<Vec<_>>();
        assert_eq!(chosen, [kept]);
    }
}
