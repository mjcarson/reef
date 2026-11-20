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

/// Writes a project's row and reads it back through the store's client
///
/// # Arguments
///
/// * `store` - The store to round-trip through
#[cfg(feature = "embedded")]
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
