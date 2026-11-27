//! The JSON API suite's test-run cases: runs opened, reported, read and listed as `reef test`
//! and the app call them, and told on the change stream
//!
//! Each case's plan is `reef/central`, of a project registered for a fresh git repo, and the
//! runs it opens are on its node `C1` unless they say otherwise.

use reef_api::test_run::OUTPUT_MAX;
use reef_api::{
    AddedPlan, ApiError, Binary, BinaryState, Change, Checked, ErrorCode, FailedTest, NewPlan,
    NodePut, OpenRun, Puts, RunReport, RunState, TestRun, paths,
};
use reef_model::PlanName;
use reef_service::Backend;
use reqwest::StatusCode;

use super::http::{Ax, Events};
use super::work;

/// The command every case's runs run
fn command() -> Vec<String> {
    ["cargo", "test", "--workspace"].map(String::from).to_vec()
}

/// A run opening on a node of a plan, or on none, building with nothing listed yet
///
/// # Arguments
///
/// * `id` - The run's id
/// * `plan` - The plan of the node its checkout is on, if one
/// * `node` - The node its checkout is on, if one
fn opened(id: &str, plan: Option<&str>, node: Option<&str>) -> OpenRun {
    OpenRun {
        id: id.to_string(),
        project: "reef".to_string(),
        plan: plan.map(str::to_string),
        node: node.map(str::to_string),
        host: "box".to_string(),
        cwd: "/src/reef".into(),
        command: command(),
        elapsed_ms: 0,
        state: RunState::Building,
        binaries: Vec::new(),
        failures: Vec::new(),
    }
}

/// A test binary standing somewhere, with its counts and times
///
/// # Arguments
///
/// * `target` - What cargo runs it as
/// * `state` - Where it stands
/// * `total` - How many tests it holds
/// * `counts` - How many passed, failed and were ignored
/// * `times` - When it started and ended, in milliseconds since the run started
fn binary(
    target: &str,
    state: BinaryState,
    total: u64,
    counts: [u64; 3],
    times: (Option<u64>, Option<u64>),
) -> Binary {
    Binary {
        package: "reef".to_string(),
        target: target.to_string(),
        total,
        guessed: false,
        passed: counts[0],
        failed: counts[1],
        ignored: counts[2],
        state,
        started_ms: times.0,
        ended_ms: times.1,
    }
}

/// The report of a run whose unit tests are done, one failing at length, and whose integration
/// tests are running
fn halfway() -> RunReport {
    RunReport {
        elapsed_ms: 1_000,
        state: RunState::Running,
        binaries: vec![
            binary(
                "unittests src/lib.rs",
                BinaryState::Done,
                3,
                [2, 1, 0],
                (Some(100), Some(900)),
            ),
            binary(
                "tests/scan.rs",
                BinaryState::Running,
                4,
                [1, 0, 0],
                (Some(900), None),
            ),
        ],
        failures: vec![FailedTest {
            binary: 0,
            test: "tests::fails".to_string(),
            output: format!("{}panicked at src/lib.rs:13:43", "x".repeat(9_000)),
        }],
    }
}

/// Puts a node of the case's own into a plan, for runs to be on
///
/// # Arguments
///
/// * `ax` - The served service
/// * `plan` - The plan to put it in
/// * `id` - The node's id
async fn put_node<B: Backend + 'static>(
    ax: &Ax<B>,
    plan: &PlanName,
    id: &str,
) -> anyhow::Result<()> {
    let puts = Puts::new(vec![NodePut::new(work(id, 0), None)]);
    let (status, checked): (_, Checked) = ax.client.post(&paths::nodes(plan), &puts).await?;
    assert_eq!(status, StatusCode::OK, "{checked:?}");
    Ok(())
}

/// Test runs' routes answer with their status: a run opened and reported reads back with its
/// done, left and failed tests and each failure's output, a listing leaves the outputs out, and a
/// report for a run the service doesn't hold is told so and opens it again
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn test_run_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    put_node(&ax, &ax.plan, "C1").await?;
    // an id a path can't hold, a node with no plan or not in it, and a run of no command are
    // bad requests, and a project or plan that isn't there is said so
    let mut no_command = opened("r-0", None, None);
    no_command.command.clear();
    let mut unregistered = opened("r-0", None, None);
    unregistered.project = "nobody".to_string();
    let refusals = [
        (opened("Bad Id", None, None), ErrorCode::BadRequest),
        (opened("r-0", None, Some("C1")), ErrorCode::BadRequest),
        (
            opened("r-0", Some("central"), Some("C9")),
            ErrorCode::BadRequest,
        ),
        (no_command, ErrorCode::BadRequest),
        (unregistered, ErrorCode::NotRegistered),
        (opened("r-0", Some("missing"), None), ErrorCode::NotListed),
    ];
    for (open, code) in refusals {
        let (status, error): (_, ApiError) = ax.client.post(paths::TEST_RUNS, &open).await?;
        assert_eq!(error.code, code, "{open:?}: {error:?}");
        assert_eq!(status.as_u16(), code.status(), "{open:?}");
    }
    // a run opens on C1, building, with nothing counted yet
    let open = opened("r-1", Some("central"), Some("C1"));
    let (status, run): (_, TestRun) = ax.client.post(paths::TEST_RUNS, &open).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((run.state, run.total()), (RunState::Building, 0));
    assert_eq!(run.node.as_deref(), Some("C1"));
    // halfway through, it counts what is done and left, and keeps the failure's end
    let path = paths::test_run("r-1");
    let (status, run): (_, TestRun) = ax.client.post(&path, &halfway()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run.state, RunState::Running);
    assert_eq!((run.total(), run.done(), run.left()), (7, 4, 3));
    assert_eq!((run.passed(), run.failed()), (3, 1));
    let output = &run.failures[0].output;
    assert!(output.ends_with("panicked at src/lib.rs:13:43"), "{output}");
    assert!(output.chars().count() < OUTPUT_MAX + 40, "{output}");
    // read back whole, as it was reported
    let (status, read): (_, TestRun) = ax.client.get(&path).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, run);
    // a project's listing names the failure but leaves its output out
    let (status, runs): (_, Vec<TestRun>) =
        ax.client.get(&paths::project_test_runs("reef")).await?;
    assert_eq!(status, StatusCode::OK);
    let [listed] = &runs[..] else {
        anyhow::bail!("{} runs were listed", runs.len());
    };
    assert_eq!(listed.failures[0].test, "tests::fails");
    assert_eq!(listed.failures[0].output, "");
    // a report, or a read, of a run this service doesn't hold is not found
    let lost = paths::test_run("r-lost");
    let (status, error): (_, ApiError) = ax.client.post(&lost, &halfway()).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    let (status, error): (_, ApiError) = ax.client.get(&lost).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    // so its client opens it again with what it holds, started as long ago as it says
    let report = halfway();
    let again = OpenRun {
        elapsed_ms: report.elapsed_ms,
        state: report.state,
        binaries: report.binaries,
        failures: report.failures,
        ..opened("r-lost", None, None)
    };
    let (status, reopened): (_, TestRun) = ax.client.post(paths::TEST_RUNS, &again).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((reopened.done(), reopened.failures.len()), (4, 1));
    assert!(reopened.started_at + 1_000 <= reopened.seen_at);
    // once its last binary ends, the run has failed, and a later report doesn't run it again
    let mut done = halfway();
    done.elapsed_ms = 2_000;
    done.state = RunState::Failed;
    done.binaries[1] = binary(
        "tests/scan.rs",
        BinaryState::Done,
        4,
        [4, 0, 0],
        (Some(900), Some(1_900)),
    );
    done.failures.clear();
    let (_, ended): (_, TestRun) = ax.client.post(&path, &done).await?;
    assert_eq!((ended.state, ended.left()), (RunState::Failed, 0));
    let mut late = done.clone();
    late.state = RunState::Running;
    let (_, still): (_, TestRun) = ax.client.post(&path, &late).await?;
    assert_eq!(still.state, RunState::Failed);
    assert_eq!(still.failures.len(), 1);
    // and its command's last finished run is that one
    let last = ax.service.last_test_run("reef", &command());
    assert_eq!(
        last.map(|run| (run.id, run.elapsed_ms)),
        Some(("r-1".to_string(), 2_000))
    );
    Ok(())
}

/// Each test run opened or reported is told on the change stream as `test-run-changed`: to its
/// node's plan's followers, or to every plan's for a run on no node
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn test_runs_are_told_on_the_change_stream<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a node of central's, and another plan with a node of its own
    put_node(&ax, &ax.plan, "C1").await?;
    let other = PlanName::new("reef", "other")?;
    let new = NewPlan {
        name: "other".to_string(),
    };
    let (_, _): (_, AddedPlan) = ax.client.post(&paths::plans("reef"), &new).await?;
    put_node(&ax, &other, "A").await?;
    // central's follower, subscribed once the answer's head is in
    let response = ax
        .client
        .open(&paths::changes(Some("reef"), Some("central")))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut events = Events::new(response);
    // a run on the other plan's node, which central's follower doesn't hear of
    let (_, _): (_, TestRun) = ax
        .client
        .post(
            paths::TEST_RUNS,
            &opened("r-other", Some("other"), Some("A")),
        )
        .await?;
    // a run on central's node, opened and reported, then a run on no node
    let (_, _): (_, TestRun) = ax
        .client
        .post(
            paths::TEST_RUNS,
            &opened("r-c1", Some("central"), Some("C1")),
        )
        .await?;
    let (_, _): (_, TestRun) = ax.client.post(&paths::test_run("r-c1"), &halfway()).await?;
    let (_, _): (_, TestRun) = ax
        .client
        .post(paths::TEST_RUNS, &opened("r-none", None, None))
        .await?;
    // central's run twice, named by its kind and its plan, then the run on no node
    let on_central = Change::TestRunChanged {
        project: "reef".to_string(),
        run: "r-c1".to_string(),
        plan: Some(ax.plan.clone()),
    };
    let on_none = Change::TestRunChanged {
        project: "reef".to_string(),
        run: "r-none".to_string(),
        plan: None,
    };
    for expected in [&on_central, &on_central, &on_none] {
        let sent = events.next().await?.expect("the stream sent the run");
        assert_eq!(sent.event, "test-run-changed");
        assert_eq!(&serde_json::from_str::<Change>(&sent.data)?, expected);
    }
    // and once the service ends the stream, the follower's ends too
    ax.service.end_changes();
    assert_eq!(events.next().await?, None);
    Ok(())
}
