//! `reef test` against a served twin: a crate's tests run through it, read as they go and told
//! to central reef, and its output and exit code what `cargo test` gives
//!
//! Each case writes a small crate into its project's repo and runs `reef test` in-process, as
//! the command line runs it. A `hold` file in the crate keeps the first test it picks waiting,
//! so the case can read the run while that test runs, and `fail` makes one test fail. Each case
//! builds into a target dir of its own beside the repo, so the repo stays small to start a node
//! in.

// with `embedded`, the shared fixtures' pool open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef::test_run::{Mode, Tests};
use reef_api::paths;
use reef_api::test_run::{BinaryState, RunState, TestRun};
use reef_service::MemoryBackend;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::http::{Ax, TESTER};

/// How long a case waits to read a run held mid-way before it gives up
const HELD_WAIT: Duration = Duration::from_secs(120);

/// The crate's manifest, a workspace of its own so no enclosing one claims it
const MANIFEST: &str =
    "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n";

/// The crate's lib: a function its doc test checks, one that waits while the case holds the run,
/// and tests that hold, pass, fail when asked, and are ignored
const LIB: &str = r#"//! A crate whose tests a case runs through `reef test`

use std::path::Path;
use std::time::{Duration, Instant};

/// Adds two numbers
///
/// ```
/// assert_eq!(fixture::add(1, 2), 3);
/// ```
pub fn add(a: u64, b: u64) -> u64 {
    a + b
}

/// Waits while the case holds the run, so it can read the run mid-way
pub fn hold() {
    let held = Path::new(env!("CARGO_MANIFEST_DIR")).join("hold");
    let since = Instant::now();
    while held.exists() && since.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn a_holds() {
        super::hold();
    }

    #[test]
    fn adds() {
        assert_eq!(super::add(1, 2), 3);
    }

    #[test]
    fn fails_when_asked() {
        println!("asked to fail?");
        let asked = Path::new(env!("CARGO_MANIFEST_DIR")).join("fail").exists();
        assert!(!asked, "asked to fail");
    }

    #[test]
    #[ignore]
    fn ignored_holds() {
        super::hold();
    }
}
"#;

/// The crate's integration test: one that holds, and one that passes
const MORE: &str = r#"#[test]
fn more_a_holds() {
    fixture::hold();
}

#[test]
fn more_b() {}
"#;

/// A served twin with a crate in its project's repo, and a config naming the twin
struct Tx {
    /// The served twin, and the case's own directory
    ax: Ax<MemoryBackend>,
    /// The config naming the twin's address
    conf: PathBuf,
    /// The credentials file every command is named, so no token of the developer's is sent
    credentials: PathBuf,
    /// Where the crate builds, beside the repo
    target: PathBuf,
}

impl Tx {
    /// Serves a fresh twin, and writes the crate into its project's repo
    ///
    /// # Arguments
    ///
    /// * `case` - The case's name, which names its directory
    async fn new(case: &str) -> anyhow::Result<Tx> {
        let ax = common::Fx::memory(&format!("test-runs-{case}"))
            .await?
            .api()
            .await?;
        // the crate, at the repo's root
        std::fs::write(ax.repo.join("Cargo.toml"), MANIFEST)?;
        std::fs::create_dir_all(ax.repo.join("src"))?;
        std::fs::create_dir_all(ax.repo.join("tests"))?;
        std::fs::write(ax.repo.join("src/lib.rs"), LIB)?;
        std::fs::write(ax.repo.join("tests/more.rs"), MORE)?;
        // and the config the CLI calls the twin by
        let conf = ax.dir.join("reef.toml");
        std::fs::write(&conf, ax.cli_config())?;
        let credentials = ax.dir.join("home/reef/credentials.toml");
        let target = ax.dir.join("target");
        Ok(Tx {
            ax,
            conf,
            credentials,
            target,
        })
    }

    /// Runs one of reef's other commands from the repo, as the command line runs it
    ///
    /// # Arguments
    ///
    /// * `args` - The command line after `reef --conf <config> --credentials <file>`
    async fn reef(&self, args: &[&str]) -> anyhow::Result<String> {
        let conf = self.conf.to_string_lossy().into_owned();
        let credentials = self.credentials.to_string_lossy().into_owned();
        let line = ["reef", "--conf", &conf, "--credentials", &credentials]
            .into_iter()
            .chain(args.iter().copied());
        let cli = reef::Cli::from_args(line)?;
        let mut out = Vec::new();
        reef::run(cli, &self.ax.repo, &mut out).await?;
        Ok(String::from_utf8(out)?)
    }

    /// What `reef test` is asked, from the repo, building into the case's target dir
    ///
    /// # Arguments
    ///
    /// * `libtest` - libtest's arguments, after `--`
    /// * `mode` - How cargo's output reaches reef's
    fn tests(&self, libtest: &[&str], mode: Mode) -> Tests {
        let target = self.target.to_string_lossy().into_owned();
        let args = ["--target-dir", target.as_str(), "--"]
            .into_iter()
            .chain(libtest.iter().copied())
            .map(Into::into)
            .collect();
        Tests {
            conf: Some(self.conf.clone()),
            credentials: Some(self.credentials.clone()),
            cwd: self.ax.repo.clone(),
            args,
            mode,
        }
    }

    /// The project's test runs, newest first
    async fn runs(&self) -> anyhow::Result<Vec<TestRun>> {
        let (status, runs) = self
            .ax
            .client
            .get(&paths::project_test_runs("reef"))
            .await?;
        anyhow::ensure!(status.is_success(), "listing runs answered {status}");
        Ok(runs)
    }

    /// Runs `reef test` while the crate holds its first picked test, reading the run once it is
    /// held, then letting it go
    ///
    /// # Arguments
    ///
    /// * `tests` - What `reef test` is asked
    /// * `held` - Whether a run read is the one held, its first picked test running
    async fn held(
        &self,
        tests: Tests,
        held: impl Fn(&TestRun) -> bool,
    ) -> anyhow::Result<(TestRun, anyhow::Result<()>, String)> {
        let hold = self.ax.repo.join("hold");
        std::fs::write(&hold, "")?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        // the run, and the case reading it beside it until it is held
        let run = reef::test_run::run(tests, &mut out, &mut err);
        let read = async {
            let since = Instant::now();
            let found = loop {
                if let Some(run) = self.runs().await?.into_iter().find(|run| held(run)) {
                    break run;
                }
                if since.elapsed() > HELD_WAIT {
                    let _ = std::fs::remove_file(&hold);
                    anyhow::bail!(
                        "no run was held within {HELD_WAIT:?}: {:#?}",
                        self.runs().await?
                    );
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            // let the test go
            std::fs::remove_file(&hold)?;
            Ok::<_, anyhow::Error>(found)
        };
        let (ran, read) = tokio::join!(run, read);
        let found = read?;
        let said = String::from_utf8_lossy(&out).into_owned() + &String::from_utf8_lossy(&err);
        Ok((found, ran, said))
    }
}

/// Whether a run is held at its first test: running, a binary running, and no test ended
///
/// # Arguments
///
/// * `run` - The run
fn held_at_first(run: &TestRun) -> bool {
    run.state == RunState::Running
        && run.done() == 0
        && run
            .binaries
            .iter()
            .any(|binary| binary.state == BinaryState::Running && binary.total > 0)
}

/// Each binary of a run as its target, total and whether that is a guess
///
/// # Arguments
///
/// * `run` - The run
fn totals(run: &TestRun) -> Vec<(String, u64, bool)> {
    run.binaries
        .iter()
        .map(|binary| (binary.target.clone(), binary.total, binary.guessed))
        .collect()
}

/// A run's total is known before its first test runs, through a pty, and the run names the node
/// the checkout is on and the user who started it; its doc tests are guessed, from the command's
/// last run once there is one
#[tokio::test]
async fn a_runs_total_is_known_before_its_first_test() -> anyhow::Result<()> {
    let tx = Tx::new("total").await?;
    // a node started in the repo, which the run is told on
    let node = tx.ax.dir.join("t1.yml");
    std::fs::write(
        &node,
        "id: T1\ntitle: T1 Tests told\ndescription:\n  why: Without it, no run.\n  what: With it, a run.\n",
    )?;
    tx.reef(&["node", "put", &node.to_string_lossy()]).await?;
    tx.reef(&["start", "T1"]).await?;
    // held at its first test, every binary counted, the doc tests a guess of none
    let tests = tx.tests(&["--test-threads=1"], Mode::Pty);
    let (held, ran, said) = tx.held(tests.clone(), held_at_first).await?;
    ran.map_err(|error| anyhow::anyhow!("{error:#}: {said}"))?;
    assert_eq!(
        totals(&held),
        vec![
            ("unittests src/lib.rs".to_string(), 4, false),
            ("tests/more.rs".to_string(), 2, false),
            ("doc-tests".to_string(), 0, true),
        ]
    );
    assert_eq!(held.binaries[0].state, BinaryState::Running);
    assert_eq!(held.node.as_deref(), Some("T1"));
    assert_eq!(held.plan.as_deref(), Some(tx.ax.plan.plan.as_str()));
    assert_eq!(held.command[..2], ["cargo", "test"]);
    assert_eq!(held.started_by.describe().as_deref(), Some(TESTER));
    // it passed, the doc tests saying how many they were, and the pty's colours showing
    let done = tx.runs().await?.remove(0);
    assert_eq!(done.state, RunState::Passed);
    assert_eq!(done.binaries[2].total, 1);
    assert!(!done.binaries[2].guessed);
    assert_eq!((done.done(), done.passed(), done.ignored()), (7, 6, 1));
    assert!(said.contains("\x1b["), "no colour through the pty: {said}");
    // the same command again guesses the doc tests from the last run
    let (again, ran, said) = tx.held(tests, held_at_first).await?;
    ran.map_err(|error| anyhow::anyhow!("{error:#}: {said}"))?;
    assert_ne!(again.id, held.id);
    assert_eq!(totals(&again)[2], ("doc-tests".to_string(), 1, true));
    Ok(())
}

/// A filter, and `--ignored`, count only the tests they pick before any runs, through pipes
#[tokio::test]
async fn a_filter_and_ignored_count_only_their_tests() -> anyhow::Result<()> {
    let tx = Tx::new("filtered").await?;
    // a filter picks the integration test's two, the lib's none
    let tests = tx.tests(&["more", "--test-threads=1"], Mode::Pipes);
    let (held, ran, said) = tx.held(tests, held_at_first).await?;
    ran.map_err(|error| anyhow::anyhow!("{error:#}: {said}"))?;
    assert_eq!(held.total(), 2);
    assert_eq!(
        totals(&held)[..2],
        [
            ("unittests src/lib.rs".to_string(), 0, false),
            ("tests/more.rs".to_string(), 2, false),
        ]
    );
    // `--ignored` picks the lib's one ignored test alone
    let tests = tx.tests(&["--ignored", "--test-threads=1"], Mode::Pipes);
    let (held, ran, said) = tx.held(tests, held_at_first).await?;
    ran.map_err(|error| anyhow::anyhow!("{error:#}: {said}"))?;
    assert_eq!(held.total(), 1);
    assert_eq!(
        totals(&held)[0],
        ("unittests src/lib.rs".to_string(), 1, false)
    );
    let done = tx.runs().await?.remove(0);
    assert_eq!((done.state, done.passed()), (RunState::Passed, 1));
    Ok(())
}

/// With nothing answering, `reef test` says so once, runs the tests, and exits with cargo's code
#[tokio::test]
async fn with_nothing_answering_the_tests_run_and_exit_with_cargos_code() -> anyhow::Result<()> {
    let tx = Tx::new("unanswered").await?;
    // a config naming a port nothing listens on
    let free = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let dead = tx.ax.dir.join("dead.toml");
    std::fs::write(&dead, format!("[api]\naddress = \"{free}\"\n"))?;
    let run = |fail: bool| {
        let mut tests = tx.tests(&["--test-threads=1"], Mode::Pipes);
        tests.conf = Some(dead.clone());
        let repo = tx.ax.repo.clone();
        async move {
            let failing = repo.join("fail");
            match fail {
                true => std::fs::write(&failing, "")?,
                false => {
                    let _ = std::fs::remove_file(&failing);
                }
            }
            let mut out = Vec::new();
            let mut err = Vec::new();
            let ran = reef::test_run::run(tests, &mut out, &mut err).await;
            Ok::<_, anyhow::Error>((ran, String::from_utf8(out)?, String::from_utf8(err)?))
        }
    };
    // a failing test fails it with cargo's 101, said once on standard error
    let (ran, out, err) = run(true).await?;
    let error = ran.expect_err("a failing test fails the run");
    assert_eq!(
        error.downcast_ref::<reef::Exit>(),
        Some(&reef::Exit(101)),
        "{error:#}"
    );
    assert!(
        out.contains("test result: FAILED. 2 passed; 1 failed; 1 ignored"),
        "{out}"
    );
    assert!(out.contains("asked to fail"), "{out}");
    assert_eq!(
        err.matches("this run isn't told to central reef").count(),
        1,
        "{err}"
    );
    // and passing tests pass it
    let (ran, out, err) = run(false).await?;
    ran.map_err(|error| anyhow::anyhow!("{error:#}: {out}{err}"))?;
    assert_eq!(
        err.matches("this run isn't told to central reef").count(),
        1,
        "{err}"
    );
    // nothing reached the twin
    assert!(tx.runs().await?.is_empty());
    Ok(())
}

/// What `reef test` prints through pipes is what `cargo test` prints, on each stream
#[tokio::test]
async fn the_output_is_what_cargo_test_prints() -> anyhow::Result<()> {
    let tx = Tx::new("output").await?;
    let tests = tx.tests(&["--test-threads=1"], Mode::Pipes);
    // a first run builds, so both read a built crate
    let mut out = Vec::new();
    let mut err = Vec::new();
    reef::test_run::run(tests.clone(), &mut out, &mut err).await?;
    // cargo alone, then reef
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let alone = std::process::Command::new(cargo)
        .arg("test")
        .args(&tests.args)
        .current_dir(&tx.ax.repo)
        .output()?;
    assert!(alone.status.success());
    let mut out = Vec::new();
    let mut err = Vec::new();
    reef::test_run::run(tests, &mut out, &mut err).await?;
    // the same on each stream, but for how long things took
    assert_eq!(timeless(&out), timeless(&alone.stdout));
    assert_eq!(timeless(&err), timeless(&alone.stderr));
    // and the run told as it went
    assert_eq!(tx.runs().await?[0].state, RunState::Passed);
    Ok(())
}

/// Output with how long each thing took left out, which differs between two runs
///
/// # Arguments
///
/// * `output` - The output
fn timeless(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let mut kept = String::with_capacity(text.len());
    let mut rest: &str = &text;
    // each ` in <seconds>s` becomes ` in Ns`
    while let Some(at) = rest.find(" in ") {
        kept.push_str(&rest[..at + 4]);
        rest = &rest[at + 4..];
        let digits = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if digits > 0 && rest[digits..].starts_with('s') {
            kept.push('N');
            rest = &rest[digits..];
        }
    }
    kept.push_str(rest);
    kept
}
