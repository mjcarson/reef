//! The server's shoal: a pool started in this process, or an external node
//!
//! The client is the same either way. Only start-up differs: in-process starts a pool first and
//! connects to the address it bound, while external connects to the address it was given.

use deepsize2::DeepSizeOf;
use rkyv::{Archive, Deserialize, Serialize};
use shoal::{Shoal, ShoalUnsortedTable};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::instrument;

#[cfg(feature = "embedded")]
use shoal::server::conf::{DefaultStorageSettings, Networking, Resources, Storage};
#[cfg(feature = "embedded")]
use shoal::server::tables::storage::fs::conf::{
    FileSystemLatencyWriterConf, FileSystemTableConf, FileSystemThroughputWriterConf,
};
#[cfg(feature = "embedded")]
use shoal::{Conf, FileSystem, PersistentUnsortedTable, ShoalPool};

use crate::{BUILD, RoundTrip};

// the tables, shared with the node program
include!("../../tables.rs");

/// The spike's database
///
/// With `embedded` this is the full schema, which also generates the client. Without it only the
/// client half is generated, and the field types are read for their names alone.
#[cfg_attr(feature = "embedded", shoal::db)]
#[cfg_attr(not(feature = "embedded"), shoal::db(client))]
pub struct X1 {
    /// The notes the app round-trips
    pub notes: PersistentUnsortedTable<Note, FileSystem>,
}

/// The environment variable naming an external node; unset means in-process
const ADDR_VAR: &str = "X1_SHOAL_ADDR";

/// The one note every round trip reads and then rewrites
const NOTE_ID: u64 = 1;

/// The server's one store, opened on first use
static STORE: OnceCell<Store> = OnceCell::const_new();

/// How the server reaches shoal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A pool this process started, which only an `embedded` build can do
    #[cfg_attr(not(feature = "embedded"), allow(dead_code))]
    InProcess,
    /// A node some other process runs
    External,
}

impl Mode {
    /// The name a round trip reports this mode by
    pub fn name(self) -> &'static str {
        match self {
            Mode::InProcess => "in-process",
            Mode::External => "external",
        }
    }
}

/// The server's shoal: a client, however shoal was reached
pub struct Store {
    /// How this store reaches shoal
    pub mode: Mode,
    /// A client for the pool or the node
    client: Shoal<X1Client>,
}

impl Store {
    /// Gets the server's store, opening it on first use
    ///
    /// `dioxus::serve` runs its initializer again after every hot-patch, and a second pool would
    /// find the data directory locked, so this only ever opens one.
    pub async fn get() -> anyhow::Result<&'static Store> {
        // open it once, and hand every later caller the same one
        STORE.get_or_try_init(Store::open).await
    }

    /// Opens the store: an external node when one is configured, else an in-process pool
    #[instrument(name = "Store::open", skip_all, err(Debug))]
    async fn open() -> anyhow::Result<Store> {
        // an address configured means someone else runs shoal
        match std::env::var(ADDR_VAR) {
            Ok(addr) => Self::external(&addr).await,
            Err(_) => Self::in_process().await,
        }
    }

    /// Connects to an external node
    ///
    /// # Arguments
    ///
    /// * `addr` - The node's address
    #[instrument(name = "Store::external", skip_all, err(Debug))]
    async fn external(addr: &str) -> anyhow::Result<Store> {
        // resolve the address the node was configured with
        let addr: SocketAddr = addr.parse()?;
        // and connect to it the same way an in-process pool is connected to
        let client = connect(addr).await?;
        tracing::info!(%addr, "shoal: connected to an external node");
        Ok(Store {
            mode: Mode::External,
            client,
        })
    }

    /// Starts a pool in this process and connects to it
    #[cfg(feature = "embedded")]
    #[instrument(name = "Store::in_process", skip_all, err(Debug))]
    async fn in_process() -> anyhow::Result<Store> {
        // the pool's data lives off tmpfs, where glommio would silently lose O_DIRECT
        let dir = data_dir();
        std::fs::create_dir_all(&dir)?;
        let conf = conf(&dir)?;
        // start the pool on a thread of its own, since starting and waiting for it both block
        let started = std::time::Instant::now();
        let addr = start_pool(conf).await?;
        tracing::info!(
            ready_s = started.elapsed().as_secs_f64(),
            %addr,
            dir = %dir.display(),
            "shoal: in-process pool is ready"
        );
        // connect to it over loopback, exactly as to an external node
        let client = connect(addr).await?;
        Ok(Store {
            mode: Mode::InProcess,
            client,
        })
    }

    /// Refuses to start in-process in a build without the engine
    #[cfg(not(feature = "embedded"))]
    async fn in_process() -> anyhow::Result<Store> {
        // only `embedded` links shoal's engine, so this build can only reach a node
        anyhow::bail!(
            "this build has no in-process shoal: set {ADDR_VAR} to an external node's address, \
             or build with the `embedded` feature"
        )
    }

    /// Reads the last note, writes this build's over it, and reads that back
    #[instrument(name = "Store::round_trip", skip_all, err(Debug))]
    pub async fn round_trip(&self) -> anyhow::Result<RoundTrip> {
        // read what the last round trip, maybe in a server since restarted, left behind
        let prior = self.read().await?;
        // write this build's note over it
        let text = format!("written by build {BUILD} at {} ms", unix_ms());
        let mut batch = self.client.query();
        batch.add_mut(Note {
            id: NOTE_ID,
            text: text.clone(),
        });
        // drain the responses, which is what makes the write durable
        let mut responses = self.client.send(batch).await?;
        while responses.next().await?.is_some() {}
        // read it back, and refuse anything but what was just written
        let read = self.read().await?;
        anyhow::ensure!(
            read.as_deref() == Some(text.as_str()),
            "wrote {text:?} but read back {read:?}"
        );
        Ok(RoundTrip {
            mode: self.mode.name().to_string(),
            build: BUILD.to_string(),
            text,
            prior,
        })
    }

    /// Reads the round trip's note, if there is one
    #[instrument(name = "Store::read", skip_all, err(Debug))]
    async fn read(&self) -> anyhow::Result<Option<String>> {
        // get the one partition the note lives in; a get that found nothing is reported as a
        // query that did not succeed (shoal's known issue 55), so that error means no note
        let response = match self.client.send_one(NoteGet::new(vec![NOTE_ID])).await {
            Ok(response) => response,
            Err(shoal::Errors::QueryDidNotSucceed { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        // a response with no rows means no note too
        let Some(rows) = response.access::<Note>()? else {
            return Ok(None);
        };
        Ok(rows.iter().next().map(|note| note.text.as_str().to_owned()))
    }
}

/// Starts a pool on a thread of its own and waits for it to be ready
///
/// The thread keeps the pool for as long as the process lives, and logs a shard that dies.
///
/// # Arguments
///
/// * `conf` - The pool's config
#[cfg(feature = "embedded")]
async fn start_pool(conf: Conf) -> anyhow::Result<SocketAddr> {
    // the pool reports its address, or why it could not start, through this
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("shoal-pool".to_string())
        .spawn(move || {
            // start every shard, and wait for them all to answer
            let started = ShoalPool::<X1>::start(conf).and_then(|mut pool| {
                let addr = pool.ready(Duration::from_secs(30))?;
                Ok((pool, addr))
            });
            let pool = match started {
                Ok((pool, addr)) => {
                    // tell the server where to connect
                    let _ = tx.send(Ok(addr));
                    pool
                }
                Err(error) => {
                    // tell the server why there is nothing to connect to; shoal's error can carry
                    // a thread's panic payload, which isn't `Sync`, so it goes as its message
                    let _ = tx.send(Err(anyhow::anyhow!("shoal failed to start: {error:?}")));
                    return;
                }
            };
            // hold the pool up, reporting a shard that dies the way shoal's node program does
            loop {
                if let Some((shard, error)) = pool.failure() {
                    tracing::error!(shard, %error, "shoal: a shard died");
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })?;
    // wait for the thread to say how it went
    rx.await?
}

/// The in-process pool's config: two shards, a small budget, and an ephemeral loopback port
///
/// # Arguments
///
/// * `dir` - The directory to store the pool's data in
#[cfg(feature = "embedded")]
fn conf(dir: &std::path::Path) -> anyhow::Result<Conf> {
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

/// Where the in-process pool keeps its data: `X1_DATA_DIR`, else the spike's `data/embedded`
#[cfg(feature = "embedded")]
fn data_dir() -> std::path::PathBuf {
    // an explicit directory wins
    if let Some(dir) = std::env::var_os("X1_DATA_DIR") {
        return dir.into();
    }
    // else beside the spike's sources, which dx can't move out from under us
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/embedded")
}

/// Connects to shoal, retrying while a just-started server finishes binding
///
/// # Arguments
///
/// * `addr` - The address to connect to
async fn connect(addr: SocketAddr) -> anyhow::Result<Shoal<X1Client>> {
    // a loaded machine can refuse the very first connection, so try a few times
    let mut last = None;
    for _ in 0..40 {
        match Shoal::<X1Client>::new(addr).await {
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

/// Answers `GET /x1/roundtrip` with a round trip's JSON
pub async fn round_trip_route()
-> Result<axum::Json<RoundTrip>, (axum::http::StatusCode, String)> {
    // the store was opened before the router was served, so this is the same one
    let store = Store::get().await.map_err(internal)?;
    // round-trip a note, and report any failure as a 500 with its causes
    store.round_trip().await.map(axum::Json).map_err(internal)
}

/// Turns an error into a 500 that says what went wrong
///
/// # Arguments
///
/// * `error` - What went wrong
fn internal(error: anyhow::Error) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}"))
}

/// The time now, in milliseconds since the epoch
fn unix_ms() -> u128 {
    // a clock before the epoch is not worth failing a spike over
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}
