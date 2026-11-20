//! The process's one store: opened once however often it is asked for
//!
//! `dioxus::serve` runs its setup again after every hot-patch, and a second pool would find the
//! data directory locked. This is a test binary of its own, since the shared store lives for the
//! whole process.

#![cfg(feature = "embedded")]

use reef_service::{Conf, Mode, Store};

/// Asking for the shared store twice opens one store and one pool
#[tokio::test(flavor = "multi_thread")]
async fn shared_store_opens_once() -> anyhow::Result<()> {
    // a fresh directory under cargo's target dir, rather than a tmpfs `/tmp`
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-once");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let mut conf = Conf::default();
    conf.shoal.data = Some(dir);
    conf.shoal.cores = 1;
    // the first ask opens a store over its own pool
    let first = Store::shared(&conf).await?;
    assert_eq!(first.mode(), Mode::InProcess);
    // and the second gets that same store, where a second pool would find the directory locked
    let second = Store::shared(&conf).await?;
    assert!(
        std::ptr::eq(first, second),
        "the shared store was opened twice"
    );
    Ok(())
}
