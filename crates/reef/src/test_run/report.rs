//! Telling central reef how far a run has got: the run opened on its project, and the node
//! the checkout is on, then reported as it goes, beside the tests and never in their way
//!
//! - **Best-effort.** A config that can't be read, a checkout of no registered project, or a
//!   service that doesn't answer is said once, and the tests run as they would have.
//! - **A report a change,** coalesced to one each [`GAP`], and one each [`HEARTBEAT`] while
//!   nothing changes, so a long quiet test doesn't read as gone.
//! - **A failed report is dropped,** never retried: the next carries every binary whole and
//!   each failure it missed. A run the service doesn't hold, after it restarted, is opened again
//!   with everything this run holds.

use reef_api::Project;
use reef_api::test_run::{BinaryState, OpenRun, RunReport};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::Shared;
use super::cargo::Args;
use super::libtest::DOC_TESTS;
use crate::cli;
use crate::client::Client;
use crate::conf::Conf;
use crate::git::Checkout;
use crate::run::{host_name, runner_id};

/// The shortest time between two reports, so a run of many quick tests sends few
const GAP: Duration = Duration::from_millis(500);

/// The longest time between two reports, well within the minute a silent run reads as gone in
const HEARTBEAT: Duration = Duration::from_secs(15);

/// What a run tells central reef, and how to reach it
#[derive(Debug, Clone)]
pub struct Telling<'a> {
    /// The config named on the command line, if one was
    pub conf: Option<&'a Path>,
    /// The credentials file named, if one was
    pub credentials: Option<&'a Path>,
    /// The working directory, which picks the project and the node
    pub cwd: &'a Path,
    /// The command line
    pub args: &'a Args,
}

/// A run central reef holds: its client, its id, and what reopening it names
struct Opened {
    /// The service's client
    client: Client,
    /// What the run was opened with, its binaries and failures left to fill
    open: OpenRun,
}

/// Tells central reef of a run until it ends, saying once if it can't
///
/// # Arguments
///
/// * `telling` - What to tell, and how to reach central reef
/// * `shared` - The run's tally, and the signals between it and the tests
/// * `started` - When the run started
pub async fn report(telling: Telling<'_>, shared: &Shared, started: Instant) {
    // the run opened, else said once why not
    let mut opened = match open(&telling, shared, started).await {
        Ok(opened) => opened,
        Err(error) => {
            shared.say(format!(
                "reef: this run isn't told to central reef: {error:#}"
            ));
            return;
        }
    };
    // the failures it held when it opened went with it
    let mut sent = shared.tally().failures().len();
    loop {
        // wait for news, else the heartbeat, unless the run has ended
        if !shared.tally().state().ended() {
            tokio::select! {
                () = shared.news.notified() => {}
                () = tokio::time::sleep(HEARTBEAT) => {}
            }
        }
        // the run as it stands, with the failures since the last report that went
        let (report, upto) = {
            let tally = shared.tally();
            let failures = tally.failures();
            let report = RunReport {
                elapsed_ms: elapsed_ms(started),
                state: tally.state(),
                binaries: tally.binaries(),
                failures: failures.get(sent..).unwrap_or_default().to_vec(),
            };
            (report, failures.len())
        };
        let ended = report.state.ended();
        match opened
            .client
            .report_test_run(&opened.open.id, &report)
            .await
        {
            Ok(Some(_)) => sent = upto,
            // a service that restarted forgot the run, so it opens again with everything held
            Ok(None) => {
                let again = reopened(&opened.open, shared, started);
                if opened.client.open_test_run(&again).await.is_ok() {
                    sent = upto;
                }
            }
            // a report that fails is dropped, and the next carries what it missed
            Err(_) => {}
        }
        if ended {
            return;
        }
        tokio::time::sleep(GAP).await;
        opened.open.elapsed_ms = elapsed_ms(started);
    }
}

/// Opens the run on the working directory's project, and the node its checkout is on
///
/// # Arguments
///
/// * `telling` - What to tell, and how to reach central reef
/// * `shared` - The run's tally
/// * `started` - When the run started
async fn open(telling: &Telling<'_>, shared: &Shared, started: Instant) -> anyhow::Result<Opened> {
    // the config named, else the one the service reads, and its credentials
    let mut conf = Conf::load(telling.conf)?;
    conf.credentials = telling.credentials.map(Path::to_path_buf);
    let client = Client::new(&conf)?;
    // the project, which must be registered
    let project = client.find_project(telling.cwd).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "no project is registered for {}: `reef project add <name>` registers one",
            telling.cwd.display()
        )
    })?;
    // the node `reef work` finds here, if any
    let node = match Checkout::open(telling.cwd) {
        Ok(checkout) => cli::worktree_record(&client, &project, checkout.root())
            .await
            .ok()
            .flatten()
            .map(|record| record.node),
        Err(_) => None,
    };
    // each package's doc tests in the command's last run, guessed to still be as many
    let command = telling.args.command();
    let guesses = guesses(&client, &project, &command).await;
    shared.tally().guess_doc_tests(guesses);
    // opened with whatever the run has found so far
    let mut open = OpenRun {
        id: runner_id(),
        project: project.name.clone(),
        plan: node.as_ref().map(|node| node.plan.plan.clone()),
        node: node.map(|node| node.id),
        host: host_name(),
        cwd: telling.cwd.to_path_buf(),
        command,
        elapsed_ms: 0,
        state: shared.tally().state(),
        binaries: Vec::new(),
        failures: Vec::new(),
    };
    let first = reopened(&open, shared, started);
    client.open_test_run(&first).await?;
    open.elapsed_ms = first.elapsed_ms;
    Ok(Opened { client, open })
}

/// The run as it opens again, with everything it holds
///
/// # Arguments
///
/// * `open` - What it opened with first
/// * `shared` - The run's tally
/// * `started` - When the run started
fn reopened(open: &OpenRun, shared: &Shared, started: Instant) -> OpenRun {
    let tally = shared.tally();
    OpenRun {
        elapsed_ms: elapsed_ms(started),
        state: tally.state(),
        binaries: tally.binaries(),
        failures: tally.failures().to_vec(),
        ..open.clone()
    }
}

/// How many doc tests each package had in the last run of a command that finished, which a
/// run guesses until its doc tests say
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project
/// * `command` - The command, its program first
async fn guesses(client: &Client, project: &Project, command: &[String]) -> HashMap<String, u64> {
    // the project's runs, newest first; a service that can't list them guesses nothing
    let Ok(runs) = client.project_test_runs(&project.name).await else {
        return HashMap::new();
    };
    let Some(last) = runs
        .into_iter()
        .find(|run| run.command == command && run.state.finished())
    else {
        return HashMap::new();
    };
    // the doc tests that said how many they were
    last.binaries
        .into_iter()
        .filter(|binary| {
            binary.target == DOC_TESTS && !binary.guessed && binary.state == BinaryState::Done
        })
        .map(|binary| (binary.package, binary.total))
        .collect()
}

/// How long the run has gone, in milliseconds
///
/// # Arguments
///
/// * `started` - When it started
pub fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}
