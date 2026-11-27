//! Test runs: how far a run of a repo's tests has got, as the client running them reports it,
//! kept by central reef for the app to draw beside the node its checkout is on
//!
//! - **The service runs nothing.** `reef test` runs `cargo test` on the user's own machine,
//!   reads its output, and reports each binary's counts and each failure here.
//! - **Kept in memory,** as a runner's session is. A run lives as long as the service, and a
//!   client whose service restarted opens its run again with everything it holds.
//! - **Times are the client's, from the run's start.** A binary's start and end, and how long
//!   the run has gone, are milliseconds since the run started by the client's clock, so a host
//!   whose clock disagrees with the service's still times its binaries right.
//! - **Each binary is counted before it runs** where its tests could be listed, and guessed
//!   until its `running N tests` line where they couldn't, as doc tests can't.
//! - **A listing leaves out each failure's output,** which reading the run gives, so a project's
//!   runs read light.

use crate::user::Author;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How long a run can go without a report before it reads as gone, in milliseconds
pub const RUN_GONE_AFTER_MS: u64 = 60_000;

/// The most characters of a failed test's output kept, its end kept since its panic is last
pub const OUTPUT_MAX: usize = 8_000;

/// The most failures a run keeps, the first ones, though every failure is counted
pub const KEPT_FAILURES: usize = 200;

/// The most characters a run's own id holds
pub const RUN_ID_MAX: usize = 64;

/// Where a test run stands, as its client last said, or gone if it stopped saying
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// cargo is building the tests, or listing them
    Building,
    /// The tests are running
    Running,
    /// Every test that ran passed
    Passed,
    /// A test failed
    Failed,
    /// It ended before its tests could say: the build failed, or the run was stopped
    Stopped,
    /// Its client hasn't reported in a minute
    Gone,
}

impl RunState {
    /// The state as a person reads it
    pub fn describe(self) -> &'static str {
        match self {
            RunState::Building => "building",
            RunState::Running => "running",
            RunState::Passed => "passed",
            RunState::Failed => "failed",
            RunState::Stopped => "stopped",
            RunState::Gone => "gone",
        }
    }

    /// Whether the run has ended, whatever its tests found
    pub fn ended(self) -> bool {
        matches!(
            self,
            RunState::Passed | RunState::Failed | RunState::Stopped
        )
    }

    /// Whether the run ended with its tests' verdict, so its binaries' times are whole
    pub fn finished(self) -> bool {
        matches!(self, RunState::Passed | RunState::Failed)
    }
}

/// Where one test binary of a run stands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BinaryState {
    /// cargo hasn't run it yet
    Waiting,
    /// Its tests are running
    Running,
    /// Its tests have all ended
    Done,
    /// cargo won't run it, since a binary before it failed without `--no-fail-fast`, or the run
    /// was stopped
    NotRun,
}

/// One test binary of a run: a crate's unit tests, an integration test, or its doc tests
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binary {
    /// The package it tests, as cargo names it
    pub package: String,
    /// What cargo runs it as, such as `unittests src/lib.rs`, `tests/scan.rs` or `doc-tests`
    pub target: String,
    /// How many tests it holds
    pub total: u64,
    /// Whether its total is only a guess, from the last run, until its tests say how many
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub guessed: bool,
    /// How many of its tests passed
    #[serde(default)]
    pub passed: u64,
    /// How many failed
    #[serde(default)]
    pub failed: u64,
    /// How many were ignored
    #[serde(default)]
    pub ignored: u64,
    /// Where it stands
    pub state: BinaryState,
    /// When it started, in milliseconds since the run started
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_ms: Option<u64>,
    /// When it ended, in milliseconds since the run started
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
}

impl Binary {
    /// How many of its tests have ended, passed, failed or ignored
    pub fn done(&self) -> u64 {
        self.passed + self.failed + self.ignored
    }

    /// How many of its tests are still to end, none once it is done or won't run
    pub fn left(&self) -> u64 {
        match self.state {
            BinaryState::Waiting | BinaryState::Running => self.total.saturating_sub(self.done()),
            BinaryState::Done | BinaryState::NotRun => 0,
        }
    }

    /// How many of its tests won't run, since cargo won't run it
    pub fn not_run(&self) -> u64 {
        match self.state {
            BinaryState::NotRun => self.total.saturating_sub(self.done()),
            _ => 0,
        }
    }
}

/// A test that failed, and what it said
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedTest {
    /// The binary it is in, by its place in the run's binaries
    pub binary: usize,
    /// The test's name, as libtest names it, such as `store::tests::a_key_reads_back`
    pub test: String,
    /// What it printed and how it panicked, its end kept past [`OUTPUT_MAX`] characters; empty
    /// in a listing
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
}

/// One run of a repo's tests, as central reef keeps it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRun {
    /// Its id, which its client named
    pub id: String,
    /// The project whose tests it runs
    pub project: String,
    /// The plan of the node its checkout is on, if one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The node its checkout is on, if one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The host it runs on
    pub host: String,
    /// The checkout it runs in
    pub cwd: PathBuf,
    /// The command it runs, its program first, such as `["cargo", "test", "--workspace"]`
    pub command: Vec<String>,
    /// When it started, in milliseconds since the epoch by the service's clock
    pub started_at: u64,
    /// When its client last reported, in milliseconds since the epoch by the service's clock
    pub seen_at: u64,
    /// How long it had gone at its client's last report, in milliseconds: its length, once it
    /// ended
    pub elapsed_ms: u64,
    /// Where it stands
    pub state: RunState,
    /// Its test binaries, in the order cargo runs them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<Binary>,
    /// Its failed tests, the first [`KEPT_FAILURES`] of them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailedTest>,
    /// Who started it: the user its client's token signs in
    #[serde(default, skip_serializing_if = "Author::is_unknown")]
    pub started_by: Author,
}

impl TestRun {
    /// The run as it reads at a time: gone once its client hasn't reported in a minute, unless
    /// it ended first
    ///
    /// # Arguments
    ///
    /// * `now` - The time, in milliseconds since the epoch
    pub fn at(mut self, now: u64) -> TestRun {
        // a run that ended said so, and one unheard from is gone
        if !self.state.ended() && now.saturating_sub(self.seen_at) > RUN_GONE_AFTER_MS {
            self.state = RunState::Gone;
        }
        self
    }

    /// Whether its client is still running it
    pub fn live(&self) -> bool {
        !self.state.ended() && self.state != RunState::Gone
    }

    /// How many tests it holds, across its binaries
    pub fn total(&self) -> u64 {
        self.binaries.iter().map(|binary| binary.total).sum()
    }

    /// How many of its tests have ended
    pub fn done(&self) -> u64 {
        self.binaries.iter().map(Binary::done).sum()
    }

    /// How many of its tests passed
    pub fn passed(&self) -> u64 {
        self.binaries.iter().map(|binary| binary.passed).sum()
    }

    /// How many of its tests failed, those whose failure isn't kept among them
    pub fn failed(&self) -> u64 {
        self.binaries.iter().map(|binary| binary.failed).sum()
    }

    /// How many of its tests were ignored
    pub fn ignored(&self) -> u64 {
        self.binaries.iter().map(|binary| binary.ignored).sum()
    }

    /// How many of its tests are still to end
    pub fn left(&self) -> u64 {
        self.binaries.iter().map(Binary::left).sum()
    }

    /// How many of its tests won't run, since cargo won't run their binaries
    pub fn not_run(&self) -> u64 {
        self.binaries.iter().map(Binary::not_run).sum()
    }

    /// The command as a shell would read it, an argument with a space or a quote in it quoted
    pub fn command_line(&self) -> String {
        command_line(&self.command)
    }

    /// The run as a listing gives it, each failure's output left out
    pub fn listed(mut self) -> TestRun {
        // the outputs are what make a run heavy, and reading the run gives them
        for failure in &mut self.failures {
            failure.output.clear();
        }
        self
    }
}

/// A client opening a run, or opening it again with what it holds after its service restarted
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRun {
    /// The run's id, which its client names: lowercase letters, digits and dashes, at most
    /// [`RUN_ID_MAX`]
    pub id: String,
    /// The project whose tests it runs
    pub project: String,
    /// The plan of the node its checkout is on, if one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The node its checkout is on, if one, which needs its plan
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The host it runs on
    pub host: String,
    /// The checkout it runs in
    pub cwd: PathBuf,
    /// The command it runs, its program first
    pub command: Vec<String>,
    /// How long it has gone, in milliseconds: 0 on its first open
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Where it stands
    pub state: RunState,
    /// Its test binaries as they stand, empty until they are listed
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<Binary>,
    /// The failures it holds, empty on its first open
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailedTest>,
}

/// What a run's client reports: where the run and each binary stand, and the failures since its
/// last report
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// How long the run has gone, in milliseconds
    pub elapsed_ms: u64,
    /// Where it stands
    pub state: RunState,
    /// Its test binaries as they stand, every one, a binary found since added at the end
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<Binary>,
    /// The failures since its last report
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailedTest>,
}

/// A command as a shell would read it, an argument with a space or a quote in it quoted
///
/// # Arguments
///
/// * `command` - The command, its program first
///
/// # Examples
///
/// ```
/// use reef_api::test_run::command_line;
///
/// let command = ["cargo", "test", "it's"].map(String::from);
/// assert_eq!(command_line(&command), r"cargo test 'it'\''s'");
/// ```
pub fn command_line(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            // a plain argument reads as it is, and any other is single-quoted
            let plain = !arg.is_empty()
                && arg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
            match plain {
                true => arg.clone(),
                false => format!("'{}'", arg.replace('\'', r"'\''")),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Cuts a failed test's output to at most [`OUTPUT_MAX`] characters, keeping its end, where its
/// panic is, and saying how much was left out
///
/// # Arguments
///
/// * `output` - The output
///
/// # Examples
///
/// ```
/// use reef_api::test_run::cut_output;
///
/// assert_eq!(cut_output("short"), "short");
/// let cut = cut_output(&format!("{}panicked", "x".repeat(9000)));
/// assert!(cut.starts_with("(1008 characters before) …") && cut.ends_with("panicked"));
/// ```
pub fn cut_output(output: &str) -> String {
    // most outputs are short, and kept whole
    let count = output.chars().count();
    if count <= OUTPUT_MAX {
        return output.to_string();
    }
    let kept: String = output.chars().skip(count - OUTPUT_MAX).collect();
    format!("({} characters before) …{kept}", count - OUTPUT_MAX)
}

#[cfg(test)]
mod tests {
    use super::{Binary, BinaryState, FailedTest, RunState, TestRun};
    use serde_json::json;

    /// A binary standing somewhere, with its counts
    ///
    /// # Arguments
    ///
    /// * `state` - Where it stands
    /// * `total` - How many tests it holds
    /// * `counts` - How many passed, failed and were ignored
    fn binary(state: BinaryState, total: u64, counts: [u64; 3]) -> Binary {
        Binary {
            package: "reef".to_string(),
            target: "unittests src/lib.rs".to_string(),
            total,
            guessed: false,
            passed: counts[0],
            failed: counts[1],
            ignored: counts[2],
            state,
            started_ms: None,
            ended_ms: None,
        }
    }

    /// A run of three binaries, one done, one running and one cargo won't run, last heard from
    /// at a time
    ///
    /// # Arguments
    ///
    /// * `seen_at` - When its client last reported
    /// * `state` - Where it stands
    fn run(seen_at: u64, state: RunState) -> TestRun {
        TestRun {
            id: "r1".to_string(),
            project: "reef".to_string(),
            plan: Some("central".to_string()),
            node: Some("C102".to_string()),
            host: "box".to_string(),
            cwd: "/src/reef".into(),
            command: vec!["cargo".to_string(), "test".to_string()],
            started_at: 0,
            seen_at,
            elapsed_ms: 0,
            state,
            binaries: vec![
                binary(BinaryState::Done, 4, [3, 1, 0]),
                binary(BinaryState::Running, 10, [5, 0, 1]),
                binary(BinaryState::NotRun, 7, [0, 0, 0]),
            ],
            failures: vec![FailedTest {
                binary: 0,
                test: "tests::fails".to_string(),
                output: "panicked".to_string(),
            }],
            started_by: crate::user::Author::default(),
        }
    }

    /// A run counts its binaries' tests: done, left, and those cargo won't run
    #[test]
    fn a_run_counts_its_binaries() {
        let run = run(0, RunState::Running);
        assert_eq!(run.total(), 21);
        assert_eq!(run.done(), 10);
        assert_eq!((run.passed(), run.failed(), run.ignored()), (8, 1, 1));
        // the running binary's four are left, and the last binary's seven won't run
        assert_eq!(run.left(), 4);
        assert_eq!(run.not_run(), 7);
    }

    /// A run unheard from for a minute reads as gone, unless it ended
    #[test]
    fn a_silent_run_is_gone() {
        // heard from within the minute, it stands as it said
        let running = run(1_000, RunState::Running);
        assert_eq!(running.clone().at(61_000).state, RunState::Running);
        assert!(running.clone().at(61_000).live());
        // a minute and a moment later, it is gone
        assert_eq!(running.at(61_001).state, RunState::Gone);
        // and one that ended stays as it ended
        let failed = run(1_000, RunState::Failed);
        assert_eq!(failed.clone().at(99_000).state, RunState::Failed);
        assert!(!failed.live());
    }

    /// A listing leaves each failure's output out, and the wire takes the shape written
    #[test]
    fn a_listing_reads_light() {
        let listed = run(0, RunState::Failed).listed();
        assert_eq!(listed.failures[0].output, "");
        let wire = serde_json::to_value(&listed).unwrap();
        assert_eq!(
            wire["failures"],
            json!([{"binary": 0, "test": "tests::fails"}])
        );
        assert_eq!(wire["state"], "failed");
        assert_eq!(wire["binaries"][2]["state"], "not-run");
        assert_eq!(serde_json::from_value::<TestRun>(wire).unwrap(), listed);
    }
}
