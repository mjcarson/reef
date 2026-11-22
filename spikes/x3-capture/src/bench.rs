//! Timing the CLI against gix on real commits, and the steps a capture adds around the diff
//!
//! Every timed step runs in a fresh state each round, as a hook's capture would: the CLI starts
//! a git process, and gix opens the repo again, unless the warm figure says otherwise.

use std::path::Path;
use std::time::Instant;

use crate::git::Git;

/// A step's times over every round, in ms
#[derive(Debug, Clone)]
pub struct Timed {
    /// What was timed
    pub name: String,
    /// Each round's time, in ms
    pub rounds: Vec<f64>,
}

impl Timed {
    /// The median, the lowest and the highest, in ms
    pub fn summary(&self) -> (f64, f64, f64) {
        let mut sorted = self.rounds.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let len = sorted.len();
        // the middle round, or the mean of the two middle ones
        let median = if len % 2 == 1 {
            sorted[len / 2]
        } else {
            (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
        };
        (median, sorted[0], sorted[len - 1])
    }
}

/// Times a step for some rounds after some warm-up rounds
///
/// # Arguments
///
/// * `name` - What is timed
/// * `warmup` - Untimed rounds first
/// * `rounds` - Timed rounds
/// * `step` - The step
fn time<F: FnMut() -> anyhow::Result<()>>(
    name: String,
    warmup: usize,
    rounds: usize,
    mut step: F,
) -> anyhow::Result<Timed> {
    // warm the page cache and git's own caches first
    for _ in 0..warmup {
        step()?;
    }
    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = Instant::now();
        step()?;
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(Timed {
        name,
        rounds: times,
    })
}

/// Times reading one commit's diff from its parent, through the CLI and through gix
///
/// Fails if the two readers disagree on the diff.
///
/// # Arguments
///
/// * `repo` - The checkout to read in
/// * `commit` - The commit whose own change is read
/// * `warmup` - Untimed rounds first
/// * `rounds` - Timed rounds
pub fn diff(repo: &Path, commit: &str, warmup: usize, rounds: usize) -> anyhow::Result<Vec<Timed>> {
    let git = Git::new(repo);
    // full ids, so gix reads the same objects
    let head = git.run(&["rev-parse", &format!("{commit}^{{commit}}")])?;
    let base = git.run(&["rev-parse", &format!("{commit}^")])?;
    // the two readers must agree before either is timed
    let cli = git.diff(&base, &head)?;
    let gix = crate::gix::diff(&crate::gix::open(repo)?, &base, &head)?;
    if cli.sorted() != gix.sorted() {
        anyhow::bail!("gix and the CLI disagree on {commit}");
    }
    let (added, removed) = cli.counts();
    let label = format!(
        "{} ({} files, +{added} -{removed})",
        &commit[..commit.len().min(7)],
        cli.files.len()
    );
    // a process a round for the CLI, an open repo a round for gix, and one open for warm gix
    let cli = time(format!("{label}: git CLI"), warmup, rounds, || {
        git.diff(&base, &head).map(|_| ())
    })?;
    let cold = time(format!("{label}: gix"), warmup, rounds, || {
        crate::gix::diff(&crate::gix::open(repo)?, &base, &head).map(|_| ())
    })?;
    let opened = crate::gix::open(repo)?;
    let warm = time(format!("{label}: gix, repo open"), warmup, rounds, || {
        crate::gix::diff(&opened, &base, &head).map(|_| ())
    })?;
    Ok(vec![cli, cold, warm])
}

/// Times writing the worktree as one tree, its uncommitted changes included
///
/// # Arguments
///
/// * `repo` - The checkout, with whatever is uncommitted in it
/// * `warmup` - Untimed rounds first
/// * `rounds` - Timed rounds
pub fn snapshot(repo: &Path, warmup: usize, rounds: usize) -> anyhow::Result<Timed> {
    let git = Git::new(repo);
    time("snapshot".to_owned(), warmup, rounds, || {
        git.snapshot().map(|_| ())
    })
}

/// Writes the times as a table, in ms
///
/// # Arguments
///
/// * `timed` - Each step's times
pub fn table(timed: &[Timed]) -> String {
    let mut out = String::from("| Step | Median, ms | Lowest | Highest |\n|---|---:|---:|---:|\n");
    for step in timed {
        let (median, lo, hi) = step.summary();
        out.push_str(&format!(
            "| {} | {median:.2} | {lo:.2} | {hi:.2} |\n",
            step.name
        ));
    }
    out
}
