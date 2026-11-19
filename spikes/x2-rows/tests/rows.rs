//! X2's acceptance tests, each run against the spike's binary over a data directory of its own
//!
//! Every command starts its own in-process pool, so a run killed midway is followed by one that
//! opens what it left on disk. Only one pool opens a data directory at a time, and the tests run
//! one at a time so their shards don't contend.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Mutex;

/// Held by each test, so only one runs its pools at a time
static SERIAL: Mutex<()> = Mutex::new(());

/// A fresh data directory for one test, on the checkout's disk rather than a tmpfs `/tmp`
///
/// # Arguments
///
/// * `name` - The test's name
fn fresh(name: &str) -> PathBuf {
    // wipe what an earlier run left
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data").join(format!("test-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("an old test directory is removed");
    }
    dir
}

/// Runs one of the spike's commands over a data directory
///
/// # Arguments
///
/// * `dir` - The data directory
/// * `args` - The command and its flags
fn x2(dir: &PathBuf, args: &[&str]) -> Output {
    // the binary cargo built for these tests
    let output = Command::new(env!("CARGO_BIN_EXE_x2"))
        .args(args)
        .arg("--data")
        .arg(dir)
        .output()
        .expect("the spike runs");
    // show what it said, which a failing assertion's message leaves out
    eprintln!(
        "x2 {}: {}\n{}{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// What a command printed
///
/// # Arguments
///
/// * `output` - The command's output
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn rows_build_the_books_graph() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = fresh("graph");
    // the book's 202 nodes go in as one revision
    assert!(x2(&dir, &["import"]).status.success());
    // and read back as the graph the book builds, listed under their project
    let verify = x2(&dir, &["verify"]);
    assert!(verify.status.success());
    assert!(stdout(&verify).contains("verified: 202 nodes"));
}

#[test]
fn stale_put_is_refused() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = fresh("stale");
    // two writers from one version: the first is written and the second refused
    let stale = x2(&dir, &["stale"]);
    assert!(stale.status.success());
    assert!(stdout(&stale).contains("second put Stale(RowMismatch)"));
}

#[test]
fn killed_import_keeps_old_revision() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = fresh("killed");
    // a first revision, whole
    assert!(x2(&dir, &["import"]).status.success());
    // a second import dies after 100 of its 202 rows are written
    let killed = x2(&dir, &["import", "--chunk", "25", "--die-after", "100"]);
    assert!(!killed.status.success());
    assert!(String::from_utf8_lossy(&killed.stderr).contains("dying after 100 of 202 rows"));
    // the first revision is still current and whole
    assert!(x2(&dir, &["verify"]).status.success());
    // the next import finds the dead one's 100 rows by the plan's pending revision and prunes them
    let next = x2(&dir, &["import"]);
    assert!(next.status.success());
    assert!(stdout(&next).contains("recovered 100 rows"));
    assert!(x2(&dir, &["verify"]).status.success());
}

#[test]
fn put_during_import_is_refused_not_lost() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = fresh("race");
    // with the plan's lock, a put made mid-import waits for the flip and is refused as stale
    let locked = x2(&dir, &["race"]);
    assert!(locked.status.success());
    assert!(stdout(&locked).contains("after the flip; outcome: refused"));
    // without it, the put is written into the revision the import retires, and lost
    let unlocked = x2(&dir, &["race", "--no-lock"]);
    assert!(unlocked.status.success());
    assert!(stdout(&unlocked).contains("before the flip; outcome: lost"));
}
