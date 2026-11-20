//! reef-shoal as an external node: started the way a user would, and reached by reef-service
//!
//! Each test runs the binary cargo built for these tests over a data directory of its own, under
//! cargo's target dir rather than a tmpfs `/tmp`.

use reef_service::{Conf, Mode, Store};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The line a node prints, followed by its address, once every shard answers
const READY_LINE: &str = "SHOAL_NODE_SERVING";

/// How long a node may take to say it is serving
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// A fresh directory for one test
///
/// # Arguments
///
/// * `name` - The test's name
fn fresh(name: &str) -> PathBuf {
    // wipe what an earlier run left
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("an old test directory is removed");
    }
    std::fs::create_dir_all(&dir).expect("the test directory is made");
    dir
}

/// Writes a node's config: one shard on an ephemeral loopback port, its data in `data`
///
/// # Arguments
///
/// * `dir` - The directory to write `shoal.yml` in
/// * `data` - The node's data directory
fn write_conf(dir: &Path, data: &Path) -> PathBuf {
    // shoal's data paths are written out in full, since `SHOAL_*` can't reach a nested key
    let yml = format!(
        "resources:\n  cores: 1\n  memory: \"256MiB\"\n\
         networking:\n  interface: \"127.0.0.1\"\n  port: 0\n\
         storage:\n  default:\n    filesystem:\n\
         \x20     latency_sensitive:\n        path: \"{data}\"\n\
         \x20     throughput_sensitive:\n        path: \"{data}\"\n",
        data = data.display()
    );
    let path = dir.join("shoal.yml");
    std::fs::write(&path, yml).expect("the node's config is written");
    path
}

/// A node this test started, stopped with SIGTERM when it is done with
struct Node {
    /// The node's process
    child: Child,
    /// The address it serves on
    addr: String,
}

impl Node {
    /// Starts reef-shoal over a config and waits for it to say it is serving
    ///
    /// # Arguments
    ///
    /// * `conf` - The node's `shoal.yml`
    fn start(conf: &Path) -> Node {
        // the binary cargo built for these tests, its output read for the ready line
        let mut child = Command::new(env!("CARGO_BIN_EXE_reef-shoal"))
            .arg("serve")
            .arg("--conf")
            .arg(conf)
            .stdout(Stdio::piped())
            .spawn()
            .expect("reef-shoal starts");
        let stdout = child.stdout.take().expect("the node's stdout is piped");
        // read its output on a thread of its own, which keeps draining it after the ready line
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(addr) = line.strip_prefix(READY_LINE) {
                    let _ = tx.send(addr.trim().to_string());
                }
            }
        });
        // and wait for the address it serves on
        let addr = match rx.recv_timeout(READY_TIMEOUT) {
            Ok(addr) => addr,
            Err(error) => {
                let _ = child.kill();
                panic!("reef-shoal never said it was serving: {error}");
            }
        };
        Node { child, addr }
    }

    /// Stops the node as a supervisor would, and says whether it stopped cleanly
    fn stop(mut self) -> bool {
        // SIGTERM, which shoal's node program answers by stopping every shard
        let pid = libc::pid_t::try_from(self.child.id()).expect("a pid fits a pid_t");
        // SAFETY: signalling a child process this test started and has not yet reaped
        unsafe { libc::kill(pid, libc::SIGTERM) };
        self.child.wait().expect("the node is reaped").success()
    }
}

/// Writes a project's row and reads it back through the store's client
///
/// # Arguments
///
/// * `store` - The store to round-trip through
async fn round_trip(store: &Store) -> anyhow::Result<()> {
    use reef_service::schema::{PROJECTS, ProjectRow, ProjectRowGet};
    let client = store.backend().client();
    // write one row, its answer checked
    let row = ProjectRow {
        list: PROJECTS.to_string(),
        name: "round-trip".to_string(),
        repo: "/repos/round-trip".to_string(),
    };
    let mut bundle = client.query();
    bundle.add_mut(row);
    client.exec(bundle).await?;
    // and read it back by its full key, as a one-query bundle whose answer is read unjudged
    let get =
        ProjectRowGet::new(vec![PROJECTS.to_string()]).sort_keys(vec!["round-trip".to_string()]);
    let mut bundle = client.query();
    bundle.add_mut(get);
    let mut stream = client.send(bundle).await?;
    let response = stream
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("the get went unanswered"))?;
    anyhow::ensure!(response.error().is_none(), "the get failed");
    let rows = response
        .access::<ProjectRow>()?
        .ok_or_else(|| anyhow::anyhow!("the get found no rows"))?;
    // exactly the row written
    let repos: Vec<&str> = rows.iter().map(|row| row.repo.as_str()).collect();
    anyhow::ensure!(repos == ["/repos/round-trip"], "read back {repos:?}");
    Ok(())
}

/// With an external node configured, the same build round-trips through `reef-shoal`
#[tokio::test(flavor = "multi_thread")]
async fn external_round_trip() -> anyhow::Result<()> {
    // a node over a fresh directory, started as a user would start one
    let dir = fresh("node-external");
    let data = dir.join("data");
    std::fs::create_dir_all(&data)?;
    let node = Node::start(&write_conf(&dir, &data));
    // reef's config names it, so the store connects rather than starting a pool
    let mut conf = Conf::default();
    conf.shoal.address = Some(node.addr.clone());
    let store = Store::open(&conf).await?;
    assert_eq!(store.mode(), Mode::External);
    // a row goes in and comes back out
    let tripped = round_trip(&store).await;
    // closing the store leaves the node up, so it is stopped either way
    store.close().await?;
    assert!(node.stop(), "reef-shoal did not stop cleanly on SIGTERM");
    tripped
}
