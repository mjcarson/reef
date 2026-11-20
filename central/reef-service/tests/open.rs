//! Opening the store: a pool in this process with no external node configured, and a refusal
//! that says what to set in a build that can't start one

use reef_service::{Conf, Store};

#[cfg(feature = "embedded")]
use reef_service::Mode;

/// A fresh data directory for one test, under cargo's target dir rather than a tmpfs `/tmp`
///
/// # Arguments
///
/// * `name` - The test's name
#[cfg(feature = "embedded")]
fn fresh(name: &str) -> std::path::PathBuf {
    // wipe what an earlier run left
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("an old test directory is removed");
    }
    dir
}

/// Lists a plan, puts a node in it, and gets the node back through the store
///
/// # Arguments
///
/// * `store` - The store to round-trip through
#[cfg(feature = "embedded")]
async fn round_trip(store: &Store) -> anyhow::Result<()> {
    use reef_service::{NodeWrite, Put};
    // a plan to put into
    let plan = reef_model::PlanName::new("reef", "round-trip")?;
    anyhow::ensure!(
        store.add_plan(&plan).await?,
        "a fresh store already listed the plan"
    );
    // a new node goes in at version 1
    let mut node = NodeWrite::new("C57", 0);
    node.body = "Round-tripped.".to_string();
    let put = store.put(&plan, node.clone(), None).await?;
    anyhow::ensure!(
        put == Put::Applied { version: 1 },
        "the put came back {put:?}"
    );
    // and comes back as it was written
    let got = store
        .get(&plan, "C57")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the node put was not got"))?;
    anyhow::ensure!(
        got.spec == node.spec && got.body == node.body,
        "read back {got:?}"
    );
    Ok(())
}

/// With no external node configured, a put and a get round-trip through an in-process pool
#[cfg(feature = "embedded")]
#[tokio::test(flavor = "multi_thread")]
async fn in_process_round_trip() -> anyhow::Result<()> {
    // no address, so the store starts its own pool, with one shard over a fresh directory
    let mut conf = Conf::default();
    conf.shoal.data = Some(fresh("open-in-process"));
    conf.shoal.cores = 1;
    let store = Store::open(&conf).await?;
    assert_eq!(store.mode(), Mode::InProcess);
    // a row goes in and comes back out
    round_trip(&store).await?;
    // and the pool stops cleanly
    store.close().await
}

/// Built without `embedded`, the store refuses to start with no external node, saying what to set
#[cfg(not(feature = "embedded"))]
#[tokio::test]
async fn without_embedded_names_what_to_set() {
    // no address, and no engine to start a pool with
    let refused = match Store::open(&Conf::default()).await {
        Ok(_) => panic!("a build without the engine opened a store with no node to reach"),
        Err(error) => format!("{error:#}"),
    };
    // the refusal names the key to set and the feature that would start shoal in-process
    assert!(refused.contains("`shoal.address`"), "{refused}");
    assert!(refused.contains("`embedded`"), "{refused}");
}
