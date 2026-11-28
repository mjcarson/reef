//! The JSON API suite's plan-summary cases: each of a project's plans summed up as the plan
//! explorer reads them, with who listed it, what it follows, its nodes by status, its last update
//! and nodes done today, who works on what, and its state of work (C120)
//!
//! Each case serves a fresh service whose project `reef` lists the plan `reef/central`, and
//! writes through the JSON API as the CLI, the hooks and a runner do.

use reef_api::{
    AddedPlan, AddedProject, ApiError, Attach, Branched, Checked, EditLoop, ErrorCode, LoopEdit,
    LoopEdited, NewPlan, NewProject, NodeCounts, NodePut, NodeWrite, OpenRun, Pane, PlanBranch,
    PlanState, PlanSummary, Puts, RunReport, RunState, Session, SessionState, StartWork, Started,
    StoredNode, TestRun, Updated, Working, paths,
};
use reef_model::{PlanName, Status};
use reef_service::Backend;
use reqwest::{Method, StatusCode};

use super::author_cases::added;
use super::http::{Ax, Client, TESTER};
use super::work_cases::start_in;
use super::{git, group, work};

/// A node to put: a piece of work at a status, a member of a group if it names one
///
/// # Arguments
///
/// * `id` - The node's id
/// * `group` - The group it is a member of, if one
/// * `status` - Its status, if it names one
/// * `order` - Where it falls in the order its plan was written in
fn node(id: &str, group: Option<&str>, status: Option<Status>, order: u64) -> NodeWrite {
    // a piece of work that says why and what, at its status and in its group
    let mut write = work(id, order);
    write.spec.group = group.map(str::to_string);
    write.spec.status = status;
    write
}

/// Puts nodes into a plan in one batch, each new, checking they were written
///
/// # Arguments
///
/// * `client` - The client to write with
/// * `plan` - The plan to put them in
/// * `nodes` - The nodes
async fn put_new(client: &Client, plan: &PlanName, nodes: Vec<NodeWrite>) -> anyhow::Result<()> {
    // each a new node, in one batch
    let puts = Puts::new(
        nodes
            .into_iter()
            .map(|node| NodePut::new(node, None))
            .collect(),
    );
    let (status, checked): (_, Checked) = client.post(&paths::nodes(plan), &puts).await?;
    anyhow::ensure!(
        status == StatusCode::OK && matches!(checked, Checked::Applied { .. }),
        "the put answered {status}: {checked:?}"
    );
    Ok(())
}

/// Sets a node of a plan to a status, over the version it is at now
///
/// # Arguments
///
/// * `client` - The client to write with
/// * `plan` - The plan it is in
/// * `id` - The node's id
/// * `status` - The status to set
async fn set_status(
    client: &Client,
    plan: &PlanName,
    id: &str,
    status: Status,
) -> anyhow::Result<()> {
    // the node as it is now, its status set, put back over its version
    let (_, stored): (_, StoredNode) = client.get(&paths::node(plan, id)).await?;
    let mut write = NodeWrite {
        spec: stored.spec,
        body: stored.body,
        order: stored.order,
    };
    write.spec.status = Some(status);
    let puts = Puts::new(vec![NodePut::new(write, Some(stored.version))]);
    let (code, checked): (_, Checked) = client.post(&paths::nodes(plan), &puts).await?;
    anyhow::ensure!(
        code == StatusCode::OK && matches!(checked, Checked::Applied { .. }),
        "setting `{id}` {status:?} answered {code}: {checked:?}"
    );
    Ok(())
}

/// Lists a plan in the project `reef`
///
/// # Arguments
///
/// * `ax` - The served service
/// * `name` - The plan's name within its project
async fn listed<B: Backend + 'static>(ax: &Ax<B>, name: &str) -> anyhow::Result<PlanName> {
    let (status, added): (_, AddedPlan) = ax
        .client
        .post(&paths::plans("reef"), &NewPlan::new(name))
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{added:?}");
    Ok(PlanName::new("reef", name)?)
}

/// Reads a project's plans summed up, by plan name within it
///
/// # Arguments
///
/// * `client` - The client to read with
/// * `project` - The project's name
async fn summed_up(client: &Client, project: &str) -> anyhow::Result<Vec<PlanSummary>> {
    let (status, summaries): (_, Vec<PlanSummary>) =
        client.get(&paths::plan_summaries(project)).await?;
    anyhow::ensure!(status == StatusCode::OK, "the summaries answered {status}");
    Ok(summaries)
}

/// One plan's summary among its project's
///
/// # Arguments
///
/// * `summaries` - The project's plans summed up
/// * `plan` - The plan's name within its project
fn of<'a>(summaries: &'a [PlanSummary], plan: &str) -> &'a PlanSummary {
    summaries
        .iter()
        .find(|summary| summary.plan.name.plan == plan)
        .unwrap_or_else(|| panic!("`{plan}` isn't summed up"))
}

/// Each plan's state of work and whether it is active, by plan name, in listing order
///
/// # Arguments
///
/// * `summaries` - The project's plans summed up
fn states(summaries: &[PlanSummary]) -> Vec<(&str, PlanState, bool)> {
    summaries
        .iter()
        .map(|summary| {
            (
                summary.plan.name.plan.as_str(),
                summary.state,
                summary.active,
            )
        })
        .collect()
}

/// A run of the project `reef`'s tests opening on a plan, and a node of it if one, building
///
/// # Arguments
///
/// * `id` - The run's id
/// * `plan` - The plan of the node its checkout is on
/// * `node` - The node its checkout is on, if one
fn opened(id: &str, plan: &str, node: Option<&str>) -> OpenRun {
    OpenRun {
        id: id.to_string(),
        project: "reef".to_string(),
        plan: Some(plan.to_string()),
        node: node.map(str::to_string),
        host: "box".to_string(),
        cwd: "/src/reef".into(),
        command: ["cargo", "test"].map(String::from).to_vec(),
        elapsed_ms: 0,
        state: RunState::Building,
        binaries: Vec::new(),
        failures: Vec::new(),
    }
}

/// Reports a run at a state, with no binaries listed yet
///
/// # Arguments
///
/// * `client` - The client to report with
/// * `id` - The run's id
/// * `state` - Where it stands now
async fn report(client: &Client, id: &str, state: RunState) -> anyhow::Result<()> {
    let report = RunReport {
        elapsed_ms: 1_000,
        state,
        binaries: Vec::new(),
        failures: Vec::new(),
    };
    let (status, run): (_, TestRun) = client.post(&paths::test_run(id), &report).await?;
    anyhow::ensure!(
        status == StatusCode::OK,
        "reporting `{id}` answered {status}"
    );
    anyhow::ensure!(run.state == state, "`{id}` reads {:?}", run.state);
    Ok(())
}

/// A session attaching to the project `reef` on a pane, working
///
/// # Arguments
///
/// * `id` - The session's own id, for a terminal's pane, or `None` for a runner's
/// * `pane` - The pane it attaches on
/// * `plan` - The plan it narrows to, if one
fn attaching(id: Option<&str>, pane: Pane, plan: Option<&str>) -> Attach {
    Attach {
        id: id.map(str::to_string),
        pane,
        project: "reef".to_string(),
        plan: plan.map(str::to_string),
        host: "box".to_string(),
        cwd: "/src/reef".into(),
        state: SessionState::Working,
        entries: Vec::new(),
        prompts: Vec::new(),
        claude: Some("c1".to_string()),
    }
}

/// Attaches a session, answering it as the service keeps it
///
/// # Arguments
///
/// * `client` - The client to attach with
/// * `attach` - The session attaching
async fn attach(client: &Client, attach: &Attach) -> anyhow::Result<Session> {
    let (status, session): (_, Session) = client.post(paths::SESSIONS, attach).await?;
    anyhow::ensure!(status == StatusCode::OK, "attaching answered {status}");
    Ok(session)
}

/// Detaches a session, as its runner letting go does
///
/// # Arguments
///
/// * `client` - The client to detach with
/// * `id` - The session's id
async fn detach(client: &Client, id: &str) -> anyhow::Result<()> {
    let (status, _): (_, Session) = client
        .send::<(), _>(Method::DELETE, &paths::session(id), None)
        .await?;
    anyhow::ensure!(
        status == StatusCode::OK,
        "detaching `{id}` answered {status}"
    );
    Ok(())
}

/// The summary route lists each plan with who listed it, its branch and commit, its nodes by
/// status with its groups left out, its last update, and its nodes done in the last 24 hours,
/// one set back since not counted; a project not registered is not found
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn plan_summaries_count_nodes_and_name_their_activity<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a repo published to a bare remote, `main` holding one doc, and `b2` a commit past it
    let (bare, checkout) = git::published(&ax.dir.join("sum"), &[("docs/main.md", "# Main\n")])?;
    let main_tip = git::run(&checkout, &["rev-parse", "HEAD"])?;
    git::run(&checkout, &["checkout", "-q", "-b", "b2"])?;
    let b2_tip = git::commit(&checkout, "docs/b2.md", "# B2\n", "b2's doc")?;
    git::run(&checkout, &["push", "-q", "origin", "b2"])?;
    // registered by its URL with two plans, `p2` following `b2`
    let url = format!("file://{}", bare.display());
    let (status, added): (_, AddedProject) = ax
        .client
        .post(paths::PROJECTS, &NewProject::remote("sum", url, None))
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{added:?}");
    for name in ["p1", "p2"] {
        let (status, added): (_, AddedPlan) = ax
            .client
            .post(&paths::plans("sum"), &NewPlan::new(name))
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{added:?}");
    }
    let p1 = PlanName::new("sum", "p1")?;
    let p2 = PlanName::new("sum", "p2")?;
    let follow = PlanBranch {
        branch: Some("b2".to_string()),
    };
    let (status, branched): (_, Branched) =
        ax.client.put(&paths::plan_branch(&p2), &follow).await?;
    assert_eq!(status, StatusCode::OK, "{branched:?}");
    // `p1` a group whose members stand at every status, two of them added done
    put_new(
        &ax.client,
        &p1,
        vec![
            group("G", 0),
            node("A", Some("G"), Some(Status::Done), 1),
            node("B", Some("G"), Some(Status::InProgress), 2),
            node("C", Some("G"), Some(Status::Blocked), 3),
            node("D", Some("G"), None, 4),
            node("E", Some("G"), None, 5),
            node("F", Some("G"), Some(Status::Done), 6),
        ],
    )
    .await?;
    // `E` set done since, and `F` set back to do
    set_status(&ax.client, &p1, "E", Status::Done).await?;
    set_status(&ax.client, &p1, "F", Status::Todo).await?;
    let summaries = summed_up(&ax.client, "sum").await?;
    let names: Vec<&str> = summaries
        .iter()
        .map(|summary| summary.plan.name.plan.as_str())
        .collect();
    assert_eq!(names, vec!["p1", "p2"]);
    // `p1` names who listed it, the commit `main` was fetched at, and its nodes, `G` left out
    let first = of(&summaries, "p1");
    let listed_by = |summary: &PlanSummary| {
        summary
            .plan
            .registered
            .as_ref()
            .and_then(|registered| registered.author.user.clone())
    };
    assert_eq!(listed_by(first).as_deref(), Some(TESTER));
    assert_eq!(first.plan.branch, None);
    assert_eq!(first.commit.as_deref(), Some(main_tip.as_str()));
    assert_eq!(
        first.nodes,
        NodeCounts {
            todo: 2,
            in_progress: 1,
            done: 2,
            blocked: 1,
        }
    );
    // its last update is the last write, tester's, and the nodes done now went done today
    let updated = first.updated.as_ref().expect("`p1` names its last update");
    assert_eq!(updated.author.user.as_deref(), Some(TESTER));
    let registered = first.plan.registered.as_ref().expect("`p1` was listed");
    assert!(
        updated.at >= registered.at,
        "{updated:?} before {registered:?}"
    );
    assert_eq!(first.done_today, vec!["A".to_string(), "E".to_string()]);
    assert_eq!(
        first.in_progress,
        vec![Working {
            node: "B".to_string(),
            started_by: Default::default(),
            started_at: None,
        }]
    );
    // `p2` follows `b2` at its tip, holds nothing, and answers its listing as its last update
    let second = of(&summaries, "p2");
    assert_eq!(listed_by(second).as_deref(), Some(TESTER));
    assert_eq!(second.plan.branch.as_deref(), Some("b2"));
    assert_eq!(second.commit.as_deref(), Some(b2_tip.as_str()));
    assert_eq!(second.nodes, NodeCounts::default());
    assert_eq!(
        second.updated,
        second.plan.registered.clone().map(Updated::from)
    );
    assert!(second.done_today.is_empty());
    assert_eq!(
        (second.state, second.active),
        (PlanState::NoSessions, false)
    );
    // a local project's plan reads its working tree, so it names no commit
    let local = summed_up(&ax.client, "reef").await?;
    assert_eq!(of(&local, "central").commit, None);
    // and a project not registered is not found
    let (status, error): (_, ApiError) = ax.client.get(&paths::plan_summaries("nope")).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotRegistered)
    );
    Ok(())
}

/// A plan with a test run building or running reads as testing, even beside a live session, one
/// with only a live session as sessions, and one with neither as no sessions; a run that ended,
/// and a session detached, leave their plan
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn a_plans_state_is_testing_sessions_or_no_sessions<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // three plans, `central` with a node for a run to be on
    let side = listed(&ax, "side").await?;
    listed(&ax, "quiet").await?;
    put_new(&ax.client, &ax.plan, vec![work("C1", 0)]).await?;
    // nothing goes on in any of them yet
    let summaries = summed_up(&ax.client, "reef").await?;
    assert_eq!(
        states(&summaries),
        vec![
            ("central", PlanState::NoSessions, false),
            ("quiet", PlanState::NoSessions, false),
            ("side", PlanState::NoSessions, false),
        ]
    );
    // a run building on `central`, and a runner's session on `side`
    let (status, _): (_, TestRun) = ax
        .client
        .post(paths::TEST_RUNS, &opened("r-1", "central", Some("C1")))
        .await?;
    assert_eq!(status, StatusCode::OK);
    let runner = attach(
        &ax.client,
        &attaching(None, "reef:k3x9-7".parse()?, Some(side.plan.as_str())),
    )
    .await?;
    let summaries = summed_up(&ax.client, "reef").await?;
    assert_eq!(
        states(&summaries),
        vec![
            ("central", PlanState::Testing, true),
            ("quiet", PlanState::NoSessions, false),
            ("side", PlanState::Sessions, true),
        ]
    );
    // each names its run or session, and who started or attached it
    let [run] = &of(&summaries, "central").runs[..] else {
        anyhow::bail!("`central` holds runs {:?}", of(&summaries, "central").runs);
    };
    assert_eq!(
        (run.id.as_str(), run.node.as_deref(), run.state),
        ("r-1", Some("C1"), RunState::Building)
    );
    assert_eq!(run.started_by.describe().as_deref(), Some(TESTER));
    let [on_side] = &of(&summaries, "side").sessions[..] else {
        anyhow::bail!(
            "`side` holds sessions {:?}",
            of(&summaries, "side").sessions
        );
    };
    assert_eq!(on_side.id, runner.id);
    assert_eq!(on_side.attached_by.describe().as_deref(), Some(TESTER));
    // running, with a session beside it, `central` still reads testing
    report(&ax.client, "r-1", RunState::Running).await?;
    let beside = attach(
        &ax.client,
        &attaching(None, "reef:m2p4-1".parse()?, Some("central")),
    )
    .await?;
    let summaries = summed_up(&ax.client, "reef").await?;
    assert_eq!(
        (
            of(&summaries, "central").state,
            of(&summaries, "central").runs[0].state
        ),
        (PlanState::Testing, RunState::Running)
    );
    // passed, it reads sessions, and with its session detached, no sessions
    report(&ax.client, "r-1", RunState::Passed).await?;
    let summaries = summed_up(&ax.client, "reef").await?;
    assert_eq!(of(&summaries, "central").state, PlanState::Sessions);
    assert!(of(&summaries, "central").runs.is_empty());
    detach(&ax.client, &beside.id).await?;
    detach(&ax.client, &runner.id).await?;
    let summaries = summed_up(&ax.client, "reef").await?;
    assert_eq!(
        states(&summaries),
        vec![
            ("central", PlanState::NoSessions, false),
            ("quiet", PlanState::NoSessions, false),
            ("side", PlanState::NoSessions, false),
        ]
    );
    Ok(())
}

/// A plan is active while a node is in progress, a session is live or a run is running: each node
/// in progress names who started its work, or nobody for one set in progress with no start, and
/// each live session names who attached it and its loop's node
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn a_plan_is_active_while_work_goes_on<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // bob, added by tester, and three nodes to do
    let admin = ax.user.clone().expect("the fixture signs tester in");
    let (_, bob) = added(&ax, &admin, "bob").await?;
    let plan = ax.plan.clone();
    put_new(
        &ax.client,
        &plan,
        vec![work("C2", 0), work("C3", 1), work("C4", 2)],
    )
    .await?;
    let central = |summaries: &[PlanSummary]| of(summaries, "central").clone();
    let quiet = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!((quiet.state, quiet.active), (PlanState::NoSessions, false));
    // bob starts C2's work and tester sets it in progress, and C4 is set in progress unstarted
    let start = StartWork {
        start: start_in(&ax.repo, "C2", 'a'),
        replaces: None,
    };
    let (_, started): (_, Started) = bob
        .post(&paths::node_work(&plan.node("C2")?), &start)
        .await?;
    let Started::Recorded { record, .. } = started else {
        anyhow::bail!("C2's start wasn't recorded: {started:?}");
    };
    set_status(&ax.client, &plan, "C2", Status::InProgress).await?;
    set_status(&ax.client, &plan, "C4", Status::InProgress).await?;
    // active, C2 naming bob, who started it, and C4 nobody
    let working = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!(
        (working.state, working.active),
        (PlanState::NoSessions, true)
    );
    assert_eq!(working.in_progress.len(), 2, "{:?}", working.in_progress);
    assert_eq!(working.in_progress[0].node, "C2");
    assert_eq!(
        working.in_progress[0].started_by.describe().as_deref(),
        Some("bob")
    );
    assert_eq!(working.in_progress[0].started_at, Some(record.started_at));
    assert_eq!(
        working.in_progress[1],
        Working {
            node: "C4".to_string(),
            started_by: Default::default(),
            started_at: None,
        }
    );
    // both done, it is idle again
    set_status(&ax.client, &plan, "C2", Status::Done).await?;
    set_status(&ax.client, &plan, "C4", Status::Done).await?;
    let idle = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!((idle.state, idle.active), (PlanState::NoSessions, false));
    // a loop in a terminal's pane holds C3, and bob's session there attaches, naming no plan
    let pane = Pane::Claude { pid: 4242 };
    let started = EditLoop {
        pane: pane.clone(),
        edit: LoopEdit::Start {
            project: "reef".to_string(),
            plan: None,
            session: Some("s-1".to_string()),
            width: None,
            opens: None,
            pid: None,
        },
    };
    let (status, edited): (_, LoopEdited) = ax.client.post(paths::LOOPS, &started).await?;
    assert_eq!(status, StatusCode::OK, "{edited:?}");
    let hold = EditLoop {
        pane: pane.clone(),
        edit: LoopEdit::Hold {
            node: Some(plan.node("C3")?),
            members: Vec::new(),
        },
    };
    let (status, edited): (_, LoopEdited) = ax.client.post(paths::LOOPS, &hold).await?;
    assert_eq!(status, StatusCode::OK, "{edited:?}");
    let session = attach(&bob, &attaching(Some("s-1"), pane, None)).await?;
    // active with sessions, the session naming bob and its loop's node
    let attached = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!(
        (attached.state, attached.active),
        (PlanState::Sessions, true)
    );
    assert!(
        attached.in_progress.is_empty(),
        "{:?}",
        attached.in_progress
    );
    let [live] = &attached.sessions[..] else {
        anyhow::bail!("{} sessions are live", attached.sessions.len());
    };
    assert_eq!(
        (live.id.as_str(), live.node.as_deref(), live.state),
        (session.id.as_str(), Some("C3"), SessionState::Working)
    );
    assert_eq!(live.attached_by.describe().as_deref(), Some("bob"));
    // detached, it is idle again
    detach(&bob, &session.id).await?;
    let detached = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!(
        (detached.state, detached.active),
        (PlanState::NoSessions, false)
    );
    // a run of bob's running on no node is active too, until it ends
    let (status, _): (_, TestRun) = bob
        .post(paths::TEST_RUNS, &opened("r-2", "central", None))
        .await?;
    assert_eq!(status, StatusCode::OK);
    report(&bob, "r-2", RunState::Running).await?;
    let testing = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!((testing.state, testing.active), (PlanState::Testing, true));
    assert_eq!(
        testing.runs[0].started_by.describe().as_deref(),
        Some("bob")
    );
    report(&bob, "r-2", RunState::Failed).await?;
    let ended = central(&summed_up(&ax.client, "reef").await?);
    assert_eq!((ended.state, ended.active), (PlanState::NoSessions, false));
    Ok(())
}
