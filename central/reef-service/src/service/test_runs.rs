//! Test runs: what each `reef test` reports of the tests it runs, kept in memory for the app
//!
//! - **The service runs nothing.** A client runs `cargo test` on the user's own machine and
//!   reports each binary's counts and each failure here, and the app reads them.
//! - **Kept in memory, not in shoal,** as runners' sessions are. A run lives as long as this
//!   process, and a client told its run is unknown opens it again with everything it holds.
//! - **Each change is a [`Change::TestRunChanged`]** to the run's project, naming the plan of the
//!   node its checkout is on, so the app reads the run again.
//! - **A project keeps its latest runs,** at most [`KEPT_RUNS`], letting go of the oldest that
//!   ended or went silent first, and the last finished run of each command, at most
//!   [`KEPT_COMMANDS`], which a later run of that command is estimated from.
//! - **A run that ended stays ended.** A report after its end keeps what it adds, but not a state
//!   that says it runs again.

use reef_api::test_run::{KEPT_FAILURES, RUN_ID_MAX};
use reef_api::{Author, Change, FailedTest, OpenRun, RunReport, TestRun, cut_output};
use reef_model::PlanName;
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use tracing::instrument;

use super::Service;
use super::sessions::now_ms;
use crate::backend::Backend;
use crate::store::{NotRegistered, Unkeepable};

/// The most runs a project keeps, the oldest that ended let go past it
pub const KEPT_RUNS: usize = 32;

/// The most commands a project keeps a last finished run of, the oldest let go past it
pub const KEPT_COMMANDS: usize = 64;

/// A run this service doesn't hold: never opened, let go of, or opened before it restarted
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoTestRun(pub String);

impl std::fmt::Display for NoTestRun {
    /// Says no run goes by the id, and how one opens
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no test run `{}` is held: `reef test` opens one", self.0)
    }
}

impl std::error::Error for NoTestRun {}

/// The runs a service keeps: each by its id, and each project's last finished run of a command
#[derive(Debug, Default)]
struct Held {
    /// Each run kept, by its id
    runs: BTreeMap<String, TestRun>,
    /// The last finished run of each command, by its project and its command
    last: BTreeMap<(String, Vec<String>), TestRun>,
}

impl Held {
    /// Keeps a run, as the last of its command too if it finished, then lets go of its project's
    /// oldest runs and commands past what a project keeps
    ///
    /// # Arguments
    ///
    /// * `run` - The run as it now stands
    fn keep(&mut self, run: TestRun) {
        // a finished run's binaries' times are whole, and estimate its command's next run
        if run.state.finished() {
            let key = (run.project.clone(), run.command.clone());
            self.last.insert(key, run.clone());
            self.trim_last(&run.project);
        }
        let project = run.project.clone();
        self.runs.insert(run.id.clone(), run);
        self.trim_runs(&project);
    }

    /// Lets go of a project's oldest runs that ended or went silent while it keeps more than
    /// [`KEPT_RUNS`], never one that is live
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    fn trim_runs(&mut self, project: &str) {
        let now = now_ms();
        loop {
            // the project's runs, and the oldest of them that no client still runs
            let mine = self.runs.values().filter(|run| run.project == project);
            if mine.clone().count() <= KEPT_RUNS {
                return;
            }
            let oldest = mine
                .filter(|run| !TestRun::clone(run).at(now).live())
                .min_by_key(|run| (run.started_at, run.id.clone()))
                .map(|run| run.id.clone());
            match oldest {
                Some(id) => self.runs.remove(&id),
                None => return,
            };
        }
    }

    /// Lets go of the oldest finished runs a project keeps for its commands while it keeps more
    /// than [`KEPT_COMMANDS`]
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    fn trim_last(&mut self, project: &str) {
        loop {
            // the project's commands, and the one whose last run ended longest ago
            let mine = self.last.iter().filter(|((owner, _), _)| owner == project);
            if mine.clone().count() <= KEPT_COMMANDS {
                return;
            }
            let oldest = mine
                .min_by_key(|(_, run)| run.started_at + run.elapsed_ms)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.last.remove(&key),
                None => return,
            };
        }
    }
}

/// The test runs clients report, kept in memory
#[derive(Debug, Default)]
pub struct TestRuns {
    /// The runs kept, and each project's last finished run of a command
    held: Mutex<Held>,
}

impl TestRuns {
    /// Runs a closure on the runs held, under their lock
    ///
    /// # Arguments
    ///
    /// * `f` - What to do with them
    fn with<T>(&self, f: impl FnOnce(&mut Held) -> T) -> T {
        // a lock poisoned by a panic still holds the runs, which stay as they were
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut held)
    }
}

/// Refuses an id that can't name a run: one that isn't 1 to [`RUN_ID_MAX`] lowercase letters,
/// digits and dashes, which a path segment holds as it is
///
/// # Arguments
///
/// * `id` - The id the client named
fn check_run_id(id: &str) -> Result<(), Unkeepable> {
    // lowercase letters, digits and dashes alone
    let safe = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if id.is_empty() || id.len() > RUN_ID_MAX || !safe {
        return Err(Unkeepable(format!(
            "`{id}` can't name a test run: use 1 to {RUN_ID_MAX} lowercase letters, digits and \
             dashes"
        )));
    }
    Ok(())
}

/// Adds failures to those a run keeps, each output cut, up to [`KEPT_FAILURES`]
///
/// # Arguments
///
/// * `kept` - The failures the run keeps
/// * `failures` - The failures to add, oldest first
fn add_failures(kept: &mut Vec<FailedTest>, failures: Vec<FailedTest>) {
    // the first failures are kept, and the rest only counted by their binaries
    let room = KEPT_FAILURES.saturating_sub(kept.len());
    kept.extend(failures.into_iter().take(room).map(|mut failure| {
        failure.output = cut_output(&failure.output);
        failure
    }));
}

/// The plan a run names as a change names it, if it names one
///
/// # Arguments
///
/// * `run` - The run
fn run_plan(run: &TestRun) -> Option<PlanName> {
    // a plan that was checked on its open has a name that parses
    run.plan
        .as_deref()
        .and_then(|plan| PlanName::new(&run.project, plan).ok())
}

impl<B: Backend> Service<B> {
    /// Opens a test run, or opens it again with what its client holds after this service
    /// restarted, replacing whatever was held under its id
    ///
    /// # Arguments
    ///
    /// * `open` - The run's id, project, node, host, checkout and command, and what it holds
    /// * `author` - Who opens it, from the caller's token
    #[instrument(name = "Service::open_test_run", skip_all, err(Debug))]
    pub async fn open_test_run(&self, open: OpenRun, author: &Author) -> anyhow::Result<TestRun> {
        // an id a path segment holds, and a command to run
        check_run_id(&open.id)?;
        if open.command.is_empty() {
            return Err(Unkeepable("a test run names the command it runs".to_string()).into());
        }
        // a registered project, a plan it lists if one is named, and a node in that plan
        match (&open.plan, &open.node) {
            (Some(plan), node) => {
                let plan = PlanName::new(&open.project, plan)?;
                self.listed(&plan).await?;
                if let Some(node) = node
                    && self.store.get(&plan, node).await?.is_none()
                {
                    return Err(Unkeepable(format!("`{node}` is not a node of `{plan}`")).into());
                }
            }
            (None, Some(node)) => {
                return Err(Unkeepable(format!("node `{node}` is named with no plan")).into());
            }
            (None, None) if self.store.project(&open.project).await?.is_none() => {
                return Err(NotRegistered(open.project.clone()).into());
            }
            (None, None) => {}
        }
        // started as long ago as its client says, by this service's clock
        let now = now_ms();
        let mut run = TestRun {
            id: open.id,
            project: open.project,
            plan: open.plan,
            node: open.node,
            host: open.host,
            cwd: open.cwd,
            command: open.command,
            started_at: now.saturating_sub(open.elapsed_ms),
            seen_at: now,
            elapsed_ms: open.elapsed_ms,
            state: open.state,
            binaries: open.binaries,
            failures: Vec::new(),
            started_by: author.clone(),
        };
        add_failures(&mut run.failures, open.failures);
        // kept afresh, whatever was held under its id before
        self.test_runs.with(|held| held.keep(run.clone()));
        self.publish(Change::TestRunChanged {
            project: run.project.clone(),
            run: run.id.clone(),
            plan: run_plan(&run),
        });
        Ok(run)
    }

    /// Takes a run's report: where it and each binary stand, and the failures since its last
    ///
    /// # Arguments
    ///
    /// * `id` - The run's id
    /// * `report` - What its client reports
    #[instrument(name = "Service::report_test_run", skip_all, err(Debug))]
    pub async fn report_test_run(&self, id: &str, report: RunReport) -> anyhow::Result<TestRun> {
        // the run as its client now says, an end it reached kept
        let run = self.test_runs.with(|held| {
            let mut run = held
                .runs
                .get(id)
                .cloned()
                .ok_or_else(|| NoTestRun(id.to_string()))?;
            if !run.state.ended() {
                run.state = report.state;
            }
            run.elapsed_ms = run.elapsed_ms.max(report.elapsed_ms);
            run.binaries = report.binaries;
            add_failures(&mut run.failures, report.failures);
            run.seen_at = now_ms();
            held.keep(run.clone());
            Ok::<_, NoTestRun>(run)
        })?;
        self.publish(Change::TestRunChanged {
            project: run.project.clone(),
            run: run.id.clone(),
            plan: run_plan(&run),
        });
        Ok(run)
    }

    /// A test run as it stands, gone if its client went silent
    ///
    /// # Arguments
    ///
    /// * `id` - The run's id
    #[instrument(name = "Service::test_run", skip_all, err(Debug))]
    pub async fn test_run(&self, id: &str) -> anyhow::Result<TestRun> {
        let now = now_ms();
        let run = self.test_runs.with(|held| {
            held.runs
                .get(id)
                .map(|run| run.clone().at(now))
                .ok_or_else(|| NoTestRun(id.to_string()))
        })?;
        Ok(run)
    }

    /// Lists a project's test runs, newest first, each gone if its client went silent and each
    /// failure's output left out
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    #[instrument(name = "Service::project_test_runs", skip_all, err(Debug))]
    pub async fn project_test_runs(&self, project: &str) -> anyhow::Result<Vec<TestRun>> {
        // a project that isn't registered has no runs
        if self.store.project(project).await?.is_none() {
            return Err(NotRegistered(project.to_string()).into());
        }
        let now = now_ms();
        let mut runs: Vec<TestRun> = self.test_runs.with(|held| {
            held.runs
                .values()
                .filter(|run| run.project == project)
                .map(|run| run.clone().at(now).listed())
                .collect()
        });
        runs.sort_by(|a, b| (b.started_at, &b.id).cmp(&(a.started_at, &a.id)));
        Ok(runs)
    }

    /// The last run of a command in a project that finished with its tests' verdict, if this
    /// service holds one
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `command` - The command, its program first
    pub fn last_test_run(&self, project: &str, command: &[String]) -> Option<TestRun> {
        // kept apart from the runs, so letting go of a run keeps what it estimates
        let key = (project.to_string(), command.to_vec());
        self.test_runs.with(|held| held.last.get(&key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Held, KEPT_COMMANDS, KEPT_RUNS};
    use reef_api::{Author, RunState, TestRun};

    /// A run of a command, started at a time and standing somewhere
    ///
    /// # Arguments
    ///
    /// * `id` - Its id
    /// * `command` - The command's last argument
    /// * `started_at` - When it started
    /// * `state` - Where it stands
    fn run(id: &str, command: &str, started_at: u64, state: RunState) -> TestRun {
        TestRun {
            id: id.to_string(),
            project: "reef".to_string(),
            plan: None,
            node: None,
            host: "box".to_string(),
            cwd: "/src/reef".into(),
            command: vec!["cargo".to_string(), "test".to_string(), command.to_string()],
            started_at,
            seen_at: super::now_ms(),
            elapsed_ms: 10,
            state,
            binaries: Vec::new(),
            failures: Vec::new(),
            started_by: Author::default(),
        }
    }

    /// A project keeps the last finished run of each command, a stopped run kept apart from it
    #[test]
    fn a_project_keeps_each_commands_last_finished_run() {
        let mut held = Held::default();
        // two runs of one command finish, and a third of it is stopped
        held.keep(run("a1", "-p=a", 1, RunState::Passed));
        held.keep(run("a2", "-p=a", 2, RunState::Failed));
        held.keep(run("a3", "-p=a", 3, RunState::Stopped));
        // a run of another command is still running
        held.keep(run("b1", "-p=b", 4, RunState::Running));
        let key = |arg: &str| {
            let command = ["cargo", "test", arg].map(String::from).to_vec();
            ("reef".to_string(), command)
        };
        // the command's last finished run is the failed one, and the other has none yet
        assert_eq!(
            held.last.get(&key("-p=a")).map(|r| r.id.as_str()),
            Some("a2")
        );
        assert!(!held.last.contains_key(&key("-p=b")));
        assert_eq!(held.runs.len(), 4);
    }

    /// A project lets go of its oldest ended runs past what it keeps, never a live one, and of
    /// its oldest commands
    #[test]
    fn a_project_lets_go_of_its_oldest_ended_runs() {
        let mut held = Held::default();
        // the oldest run is still running, and the rest ended
        held.keep(run("live", "-p=0", 0, RunState::Running));
        for n in 1..=KEPT_RUNS as u64 + 1 {
            held.keep(run(
                &format!("r{n}"),
                &format!("-p={n}"),
                n,
                RunState::Passed,
            ));
        }
        // the live one stays, and the oldest that ended is let go
        assert_eq!(held.runs.len(), KEPT_RUNS);
        assert!(held.runs.contains_key("live"));
        assert!(!held.runs.contains_key("r1") && !held.runs.contains_key("r2"));
        // every command finished once, and the oldest past what a project keeps are let go
        for n in KEPT_RUNS as u64 + 2..=KEPT_COMMANDS as u64 + 2 {
            held.keep(run(
                &format!("r{n}"),
                &format!("-p={n}"),
                n,
                RunState::Passed,
            ));
        }
        assert_eq!(held.last.len(), KEPT_COMMANDS);
        let oldest = held.last.values().map(|run| run.started_at).min();
        assert_eq!(oldest, Some(3));
    }
}
