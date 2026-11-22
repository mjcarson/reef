//! Times a node's diff through reef's git module, against git's CLI, and a whole capture's reads
//!
//! ```sh
//! cargo run --release -p reef --example diff_bench
//! cargo run --release -p reef --example diff_bench -- --commit 31155d8 --rounds 20 --clone <DIR>
//! ```
//!
//! The diff is one commit's own change, from its parent, read in this repo: X2's commit
//! `31155d8` by default, which X3 timed too. Before anything is timed, gix's diff is checked
//! against git's CLI, which must agree on every file, hunk and line. Then each figure is timed
//! over three warm-up rounds and `--rounds` timed ones (10 by default):
//!
//! - **gix**: the checkout opened and the diff read, each round, as one `reef node capture`
//!   process would;
//! - **gix, open**: the diff alone, the checkout opened once;
//! - **git CLI**: `git diff --raw -z -p` run and parsed, a process a round.
//!
//! With `--clone`, a capture's reads are timed too, in a clone of this repo checked out at the
//! commit with one tracked file edited and one untracked file written: its start the commit's
//! parent, its seal snapshot and its diff, as `reef node capture` takes them before it sends.
//! The clone is written to, so it must not be this repo.

#[path = "../tests/support/oracle.rs"]
mod oracle;

use reef::capture::take;
use reef::git::Checkout;
use reef_api::WorkStart;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// How many untimed rounds each step runs first
const WARMUP: usize = 3;

/// What the bench was asked to do
struct Args {
    /// The commit whose own change is read
    commit: String,
    /// How many timed rounds each step runs
    rounds: usize,
    /// A clone of this repo to time a capture's reads in, if one was named
    clone: Option<PathBuf>,
}

/// Reads the bench's arguments: `--commit SHA`, `--rounds N` and `--clone DIR`
fn args() -> anyhow::Result<Args> {
    let mut args = Args {
        commit: "31155d8".to_string(),
        rounds: 10,
        clone: None,
    };
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        // each flag takes the value after it
        let mut value = || {
            given
                .next()
                .ok_or_else(|| anyhow::anyhow!("`{arg}` needs a value"))
        };
        match arg.as_str() {
            "--commit" => args.commit = value()?,
            "--rounds" => args.rounds = value()?.parse()?,
            "--clone" => args.clone = Some(PathBuf::from(value()?)),
            other => anyhow::bail!("`{other}` is not one of --commit, --rounds or --clone"),
        }
    }
    Ok(args)
}

/// Times a step over the warm-up rounds and then the timed ones, returning each timed round's ms
///
/// # Arguments
///
/// * `rounds` - How many timed rounds
/// * `step` - The step
fn time<F: FnMut() -> anyhow::Result<()>>(rounds: usize, mut step: F) -> anyhow::Result<Vec<f64>> {
    // warm the page cache and git's own caches first
    for _ in 0..WARMUP {
        step()?;
    }
    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = Instant::now();
        step()?;
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(times)
}

/// Prints a step's median, lowest and highest round, in ms
///
/// # Arguments
///
/// * `name` - What was timed
/// * `times` - Each round's ms
fn report(name: &str, times: &[f64]) {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let len = sorted.len();
    // the middle round, or the mean of the two middle ones
    let median = match len % 2 {
        1 => sorted[len / 2],
        _ => (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0,
    };
    println!(
        "| {name} | {median:.3} | {:.3} | {:.3} |",
        sorted[0],
        sorted[len - 1]
    );
}

/// The full id a revision names in a checkout
///
/// # Arguments
///
/// * `checkout` - The checkout
/// * `rev` - The revision
fn full(checkout: &Checkout, rev: &str) -> anyhow::Result<String> {
    checkout
        .commit(rev)?
        .ok_or_else(|| anyhow::anyhow!("`{rev}` names no commit"))
}

/// Times a commit's diff through gix and the CLI, and with a clone a capture's reads
fn main() -> anyhow::Result<()> {
    let args = args()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    // the commit's own change, which gix and the CLI must agree on before either is timed
    let checkout = Checkout::open(&repo)?;
    let head = full(&checkout, &args.commit)?;
    let base = full(&checkout, &format!("{head}^"))?;
    let ours = checkout.diff(&base, &head)?;
    anyhow::ensure!(
        ours == oracle::git_diff(&repo, &base, &head),
        "gix and git's CLI disagree on {}",
        args.commit
    );
    let (added, removed) = ours.iter().fold((0, 0), |(added, removed), file| {
        let (a, r) = file.counts();
        (added + a, removed + r)
    });
    println!(
        "{} from its parent: {} files, +{added} -{removed}, {} timed rounds after {WARMUP}\n",
        &head[..7],
        ours.len(),
        args.rounds
    );
    println!("| Step | Median, ms | Lowest | Highest |\n|---|---:|---:|---:|");
    // the diff as one capture's process reads it, then with the checkout open, then the CLI's
    let cold = time(args.rounds, || {
        Checkout::open(&repo)?.diff(&base, &head).map(|_| ())
    })?;
    report("gix", &cold);
    let warm = time(args.rounds, || checkout.diff(&base, &head).map(|_| ()))?;
    report("gix, open", &warm);
    let cli = time(args.rounds, || {
        oracle::git_diff(&repo, &base, &head);
        Ok(())
    })?;
    report("git CLI", &cli);
    // a capture's reads, in a clone checked out at the commit with one edit and one new file
    if let Some(clone) = &args.clone {
        let clone = clone.canonicalize()?;
        anyhow::ensure!(clone != repo, "--clone must name a clone, not this repo");
        let checkout = Checkout::open(&clone)?;
        let start = checkout.tree_of(&base)?;
        let read = WorkStart {
            worktree: clone.clone(),
            branch: checkout.branch()?,
            base_branch: None,
            start: base.clone(),
            start_tree: start,
            fork: None,
            pin: "refs/reef/start/bench/B1".to_string(),
            pinned: String::new(),
        };
        let taken = take(&checkout, &read)?;
        let files = taken.files.len();
        let times = time(args.rounds, || {
            take(&Checkout::open(&clone)?, &read).map(|_| ())
        })?;
        report(&format!("capture's reads, {files} files"), &times);
    }
    Ok(())
}
