//! The JSON API suite's cases, each generic over the backend so it runs against shoal and the
//! twin
//!
//! Each case talks to a served service through a client that knows only `reef-api`'s types and
//! the reef-model types they are built on, as the CLI will. Its plan is `reef/central`, of a
//! project registered for a fresh git repo.

use reef_api::{
    AddCapture, AddedPlan, AddedProject, Anchor, ApiError, CaptureShas, Captured, Change, Checked,
    Commented, EditLoop, ErrorCode, FileDiff, FollowedThread, Listing, Loop, LoopEdit, LoopEdited,
    LoopState, MarkThread, Marked, NewPlan, NewProject, NewReply, NewThread, NodePut, NodeWrite,
    PageEdit, PageEdited, PageListing, PageWrite, Pane, Place, PlanInfo, PlanWrite, Project,
    PutNode, Puts, QueueEdit, QueueListing, Queued, Refusal, ResolveThread, Resolved, Resolves,
    ResolvingCommit, Revision, Rewrite, Side, StalePut, StartWork, Started, StoredNode, StoredPage,
    Target, Thread, ThreadState, TrailerOutcome, TrailerRead, WorkRecord, paths,
};
use reef_model::{
    Description, Diagnostic, EntryState, Next, NodeSpec, Payload, PlanName, Status, WaitsOn,
};
use reef_service::Backend;
use reqwest::StatusCode;

use super::http::{Ax, Events};
use super::work_cases::{added_file, start_in};
use super::{doc, git};

/// A piece of work that says why it is needed and what it does, needing what it names
///
/// # Arguments
///
/// * `id` - The node's id
/// * `needs` - The ids it needs
/// * `order` - Where it falls in the order its plan was written in
fn work(id: &str, needs: &[&str], order: u64) -> NodeWrite {
    NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} Some work")),
            needs: needs.iter().map(|need| need.to_string()).collect(),
            description: Some(Description {
                why: Some(format!("Without {id}, something is missing.")),
                what: Some(format!("With {id}, it is there.")),
                details: None,
            }),
            ..NodeSpec::default()
        },
        body: format!("# {id}\n\nIts body.\n"),
        order,
    }
}

/// A batch of one put
///
/// # Arguments
///
/// * `node` - The node to put
/// * `read` - The version its writer read, or `None` for a new node
fn one(node: NodeWrite, read: Option<u64>) -> Puts {
    Puts::new(vec![NodePut::new(node, read)])
}

/// A client using only reef-api's types registers a project, lists a plan in it, puts a node,
/// and lists plans and nodes back
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn client_puts_a_node_and_lists_plans<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a project of its own, for a fresh repo, registered through the API
    let repo = git::repo(&ax.dir.join("api"))?;
    let new = NewProject {
        name: "api".to_string(),
        repo: repo.clone(),
    };
    let (status, added): (_, AddedProject) = ax.client.post(paths::PROJECTS, &new).await?;
    let project = Project {
        name: "api".to_string(),
        repo: repo.clone(),
    };
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(added, AddedProject::Added(project.clone()));
    // the name again, for another repo, is a conflict
    let other = NewProject {
        name: "api".to_string(),
        repo: git::repo(&ax.dir.join("other"))?,
    };
    let (status, taken): (_, AddedProject) = ax.client.post(paths::PROJECTS, &other).await?;
    assert_eq!(
        (status, taken),
        (StatusCode::CONFLICT, AddedProject::NameTaken)
    );
    // and so is the repo again, under another name, naming the project that has it
    let again = NewProject {
        name: "again".to_string(),
        repo: repo.clone(),
    };
    let (status, taken): (_, AddedProject) = ax.client.post(paths::PROJECTS, &again).await?;
    let expected = AddedProject::RepoTaken {
        project: "api".to_string(),
    };
    assert_eq!((status, taken), (StatusCode::CONFLICT, expected));
    // both projects are listed, by name, and the repo finds its project
    let (_, projects): (_, Vec<Project>) = ax.client.get(paths::PROJECTS).await?;
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["api", "reef"]);
    let dir = repo.to_str().expect("the repo is a utf-8 path");
    let (status, found): (_, Project) = ax.client.get(&paths::find_project(dir)).await?;
    assert_eq!((status, found), (StatusCode::OK, project));
    // a plan listed in it is created once, and left as it is after that
    let plan = PlanName::new("api", "main")?;
    let request = NewPlan {
        name: "main".to_string(),
    };
    let (status, added): (_, AddedPlan) = ax.client.post(&paths::plans("api"), &request).await?;
    assert_eq!(
        (status, added),
        (StatusCode::CREATED, AddedPlan { added: true })
    );
    let (status, added): (_, AddedPlan) = ax.client.post(&paths::plans("api"), &request).await?;
    assert_eq!(
        (status, added),
        (StatusCode::OK, AddedPlan { added: false })
    );
    // the project lists that plan, at a revision to put into
    let (_, plans): (_, Vec<PlanInfo>) = ax.client.get(&paths::plans("api")).await?;
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].name, plan);
    assert_ne!(plans[0].revision, 0);
    // a node put into it is written at version 1
    let node = work("C53", &[], 0);
    let (status, checked): (_, Checked) = ax
        .client
        .post(&paths::nodes(&plan), &one(node.clone(), None))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(checked, Checked::Applied { versions: vec![1] });
    // and reads back as it was written, at that version
    let (_, got): (_, StoredNode) = ax.client.get(&paths::node(&plan, "C53")).await?;
    assert_eq!((got.spec, got.body, got.version), (node.spec, node.body, 1));
    // the plan's listing holds it, at the revision the plan names
    let (_, listing): (_, Listing) = ax.client.get(&paths::nodes(&plan)).await?;
    let (_, info): (_, PlanInfo) = ax.client.get(&paths::plan(&plan)).await?;
    assert_eq!(listing.revision, info.revision);
    assert_eq!(listing.nodes.len(), 1);
    // the plan builds into a payload of that node, with no problems
    let (_, payload): (_, Payload) = ax.client.get(&paths::payload(&plan)).await?;
    let ids: Vec<&str> = payload.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["C53"]);
    assert_eq!(payload.nodes[0].origin.to_string(), "api/main/C53");
    let (_, problems): (_, Vec<Diagnostic>) = ax.client.get(&paths::check(&plan)).await?;
    assert!(problems.is_empty(), "{problems:?}");
    Ok(())
}

/// A put that would add a problem is 422 with check's message, and a stale one is 409 naming
/// the node and how it moved
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn refused_and_stale_puts_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    let nodes = paths::nodes(&ax.plan);
    // a node needing one that isn't there is refused, with check's message
    let (status, refused): (_, Checked) = ax
        .client
        .post(&nodes, &one(work("C11", &["X"], 0), None))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let problem = Diagnostic {
        source: "reef/central/C11".to_string(),
        line: None,
        message: "needs `X`, which is not a node".to_string(),
    };
    assert_eq!(refused, Checked::Refused(vec![problem]));
    // the same node needing nothing goes in
    let (status, _): (_, Checked) = ax
        .client
        .post(&nodes, &one(work("C11", &[], 0), None))
        .await?;
    assert_eq!(status, StatusCode::OK);
    // a second put of it as new finds it there, and one over a version never read finds it moved
    for (read, refusal) in [(None, Refusal::Exists), (Some(5), Refusal::Moved)] {
        let (status, stale): (_, Checked) = ax
            .client
            .post(&nodes, &one(work("C11", &[], 0), read))
            .await?;
        assert_eq!(status, StatusCode::CONFLICT);
        let expected = StalePut {
            id: "C11".to_string(),
            refusal,
        };
        assert_eq!(stale, Checked::Stale(vec![expected]));
    }
    // and a put over the version it was read at goes in, one version on
    let (status, checked): (_, Checked) = ax
        .client
        .post(&nodes, &one(work("C11", &[], 0), Some(1)))
        .await?;
    assert_eq!(
        (status, checked),
        (StatusCode::OK, Checked::Applied { versions: vec![2] })
    );
    Ok(())
}

/// Each error a client can cause names its kind, with the status that kind takes
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn errors_say_what_is_missing<B: Backend + 'static>(ax: Ax<B>) -> anyhow::Result<()> {
    // a project that isn't registered, a plan that isn't listed, and a node that isn't there
    let unlisted = PlanName::new("reef", "missing")?;
    let cases = [
        (paths::plans("nobody"), ErrorCode::NotRegistered),
        (
            paths::payload(&PlanName::new("nobody", "central")?),
            ErrorCode::NotRegistered,
        ),
        (paths::nodes(&unlisted), ErrorCode::NotListed),
        (paths::plan(&unlisted), ErrorCode::NotListed),
        (paths::node(&ax.plan, "nope"), ErrorCode::NotFound),
        // a project name can't start with `-`
        (
            "/api/v1/projects/-bad/plans/central/nodes".to_string(),
            ErrorCode::BadName,
        ),
        // a plan is followed within its project
        (
            "/api/v1/changes?plan=central".to_string(),
            ErrorCode::BadRequest,
        ),
    ];
    for (path, code) in cases {
        let (status, error): (_, ApiError) = ax.client.get(&path).await?;
        assert_eq!(error.code, code, "{path}: {error:?}");
        assert_eq!(status.as_u16(), code.status(), "{path}");
    }
    // a directory no project's repo holds finds no project
    let lonely = git::repo(&ax.dir.join("lonely"))?;
    let lonely = lonely.to_str().expect("the repo is a utf-8 path");
    let (status, error): (_, ApiError) = ax.client.get(&paths::find_project(lonely)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    // a body that isn't JSON, and a node that lists its members inline, are bad requests
    let nodes = paths::nodes(&ax.plan);
    let (status, error) = ax.client.post_text(&nodes, "not json").await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    let mut group = work("M3", &[], 0);
    group.spec.nodes = vec![NodeSpec {
        id: "C11".to_string(),
        ..NodeSpec::default()
    }];
    let (status, error): (_, ApiError) = ax.client.post(&nodes, &one(group, None)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    assert!(
        error.message.contains("members inline"),
        "{}",
        error.message
    );
    // a repo named relative to wherever the service started is refused
    let relative = NewProject {
        name: "rel".to_string(),
        repo: "some/repo".into(),
    };
    let (status, error): (_, ApiError) = ax.client.post(paths::PROJECTS, &relative).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    Ok(())
}

/// A whole plan written through the API becomes the revision the plan names, and one that
/// would break it leaves the plan as it was
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn whole_plan_written_as_a_revision<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    let plan = paths::plan(&ax.plan);
    let revision = paths::revision(&ax.plan);
    // the revision the plan was listed with
    let (_, before): (_, PlanInfo) = ax.client.get(&plan).await?;
    // two nodes, one needing the other, written as a new revision
    let write = PlanWrite::from(vec![work("C11", &[], 0), work("C53", &["C11"], 1)]);
    let (status, rewrite): (_, Rewrite) = ax.client.put(&revision, &write).await?;
    assert_eq!(status, StatusCode::OK);
    let Rewrite::Applied(rewritten) = rewrite else {
        anyhow::bail!("the write was refused: {rewrite:?}");
    };
    assert_ne!(rewritten.revision, before.revision);
    // the plan now names it, and lists both nodes in it
    let (_, after): (_, PlanInfo) = ax.client.get(&plan).await?;
    assert_eq!(after.revision, rewritten.revision);
    let (_, listing): (_, Listing) = ax.client.get(&paths::nodes(&ax.plan)).await?;
    let ids: Vec<&str> = listing.nodes.iter().map(|n| n.spec.id.as_str()).collect();
    assert_eq!(ids, ["C11", "C53"]);
    // a plan that needs a node it doesn't have is refused, and the revision stays
    let broken = PlanWrite::from(vec![work("C53", &["C11"], 0)]);
    let (status, refused): (_, Rewrite) = ax.client.put(&revision, &broken).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(matches!(refused, Rewrite::Refused(_)), "{refused:?}");
    let (_, kept): (_, PlanInfo) = ax.client.get(&plan).await?;
    assert_eq!(kept.revision, rewritten.revision);
    Ok(())
}

/// A follower of one plan's change stream is sent each write to it, and none to another plan,
/// and its stream ends when the service ends the stream
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn change_stream_tells_of_each_write<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // follow reef/central alone, subscribed once the answer's head is in
    let path = paths::changes(Some("reef"), Some("central"));
    let response = ax.client.open(&path).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let kind = response.headers().get("content-type").cloned();
    assert_eq!(
        kind.as_ref().and_then(|k| k.to_str().ok()),
        Some("text/event-stream")
    );
    let mut events = Events::new(response);
    // a sibling plan listed and put into, which the follower doesn't follow
    let other = PlanName::new("reef", "other")?;
    let request = NewPlan {
        name: "other".to_string(),
    };
    let (_, _): (_, AddedPlan) = ax.client.post(&paths::plans("reef"), &request).await?;
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&other), &one(work("A", &[], 0), None))
        .await?;
    // then a put and a whole-plan write to the plan it follows
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C11", &[], 0), None))
        .await?;
    let write = PlanWrite::from(vec![work("C11", &[], 0), work("C53", &["C11"], 1)]);
    let (_, rewrite): (_, Rewrite) = ax.client.put(&paths::revision(&ax.plan), &write).await?;
    let Rewrite::Applied(rewritten) = rewrite else {
        anyhow::bail!("the write was refused: {rewrite:?}");
    };
    // the first event is the put, named by its kind, each node at its new version
    let first = events.next().await?.expect("the stream sent the put");
    assert_eq!(first.event, "nodes-put");
    let put: Change = serde_json::from_str(&first.data)?;
    let expected = Change::NodesPut {
        plan: ax.plan.clone(),
        nodes: vec![PutNode {
            id: "C11".to_string(),
            version: 1,
        }],
        removed: Vec::new(),
    };
    assert_eq!(put, expected);
    // then the revision the write made current
    let second = events.next().await?.expect("the stream sent the write");
    assert_eq!(second.event, "plan-written");
    let written: Change = serde_json::from_str(&second.data)?;
    let expected = Change::PlanWritten {
        plan: ax.plan.clone(),
        revision: rewritten.revision,
    };
    assert_eq!(written, expected);
    // and once the service ends the stream, the follower's ends too
    ax.service.end_changes();
    assert_eq!(events.next().await?, None);
    Ok(())
}

/// A design doc, written as a node file is, whose `rows` section links within it and out of it,
/// with a section after it
const SECTIONED: &str = "---\nid: D1\n---\n# Design\n\n## Rows\n\n### Context\n\nRows are slow: see [columns](#columns) and [the other](other.md#x).\n\n## Columns\n\nColumns are fast.\n";

/// A payload's anchored doc link leads to a page holding its section where the viewer's *Read
/// here* cuts it: the heading with the anchor's id in the page's `<main>`, and what follows it up
/// to the next heading at its level
///
/// No browser drives the viewer here, so the link is followed as `docs.rs` follows it: its
/// anchor taken off, the page fetched, the section found by the anchor's id.
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn anchored_doc_link_opens_its_section<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a node linking a section of a doc in the repo
    doc(&ax.repo, "docs/design.md", SECTIONED)?;
    let mut node = work("C14", &[], 0);
    node.spec.docs = vec!["docs/design.md#rows".to_string()];
    let (status, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(node, None))
        .await?;
    assert_eq!(status, StatusCode::OK);
    // the payload links it by an absolute path under the plan's docs, titled by its heading
    let docs = paths::docs(&ax.plan);
    let (_, payload): (_, Payload) = ax.client.get(&paths::payload(&ax.plan)).await?;
    let link = &payload.nodes[0].docs[0];
    assert_eq!(link.label, "Rows");
    assert_eq!(link.href, format!("{docs}docs/design.html#rows"));
    // the page, without the anchor, is HTML that runs no script
    let (page, anchor) = link.href.split_once('#').expect("the link names a heading");
    let response = ax.client.open(page).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    assert!(
        header("content-type").is_some_and(|kind| kind.starts_with("text/html")),
        "{:?}",
        header("content-type")
    );
    assert_eq!(
        header("content-security-policy").as_deref(),
        Some("script-src 'none'")
    );
    let html = response.text().await?;
    // its `<main>` holds the doc without its frontmatter, the anchor's heading in it
    let main = html
        .split_once("<main>")
        .and_then(|(_, rest)| rest.split_once("</main>"))
        .map(|(main, _)| main)
        .expect("the page has a <main>");
    assert!(!main.contains("id: D1"), "{main}");
    let heading = format!("<h2 id=\"{anchor}\">Rows</h2>");
    let (_, after) = main
        .split_once(&heading)
        .unwrap_or_else(|| panic!("no {heading} in {main}"));
    // its section runs to the next heading at its level, its own subsections in it
    let section = after.split("<h2").next().unwrap_or(after);
    assert!(
        section.contains("<h3 id=\"context\">Context</h3>"),
        "{section}"
    );
    assert!(section.contains("Rows are slow"), "{section}");
    assert!(!section.contains("Columns are fast"), "{section}");
    // and its links lead where they did from the doc, under the plan's docs
    for href in ["docs/design.html#columns", "docs/other.html#x"] {
        assert!(
            section.contains(&format!("href=\"{docs}{href}\"")),
            "{href}: {section}"
        );
    }
    // a page no doc is written to is not found, and neither is a doc of an unlisted plan
    let unlisted = PlanName::new("reef", "missing")?;
    let cases = [
        (format!("{docs}docs/gone.html"), ErrorCode::NotFound),
        (
            format!("{}docs/design.html", paths::docs(&unlisted)),
            ErrorCode::NotListed,
        ),
    ];
    for (path, code) in cases {
        let (status, error): (_, ApiError) = ax.client.get(&path).await?;
        assert_eq!(
            (status.as_u16(), error.code),
            (code.status(), code),
            "{path}"
        );
    }
    // nor is a doc beside the repo, reached by a path that climbs out or a symlink that leads out
    doc(&ax.dir, "outside.md", SECTIONED)?;
    std::os::unix::fs::symlink(ax.dir.join("outside.md"), ax.repo.join("docs/out.md"))?;
    for path in [
        format!("{docs}..%2Foutside.html"),
        format!("{docs}docs%2F..%2F..%2Foutside.html"),
        format!("{docs}docs/out.html"),
    ] {
        let (status, error): (_, ApiError) = ax.client.get(&path).await?;
        assert_eq!(
            (status, error.code),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            "{path}"
        );
    }
    Ok(())
}

/// The plan's ready work and what the rest waits on come back as `mdbook-reef next` says them
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn next_says_what_is_ready_and_what_waits<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // two pieces of work, the second needing the first
    let batch = Puts::new(vec![
        NodePut::new(work("C1", &[], 0), None),
        NodePut::new(work("C2", &["C1"], 1), None),
    ]);
    let (_, checked): (_, Checked) = ax.client.post(&paths::nodes(&ax.plan), &batch).await?;
    assert_eq!(
        checked,
        Checked::Applied {
            versions: vec![1, 1]
        }
    );
    // the first is ready, and the second waits on it
    let (status, next): (_, Next) = ax.client.get(&paths::next(&ax.plan)).await?;
    assert_eq!(status, StatusCode::OK);
    let ready: Vec<&str> = next.ready.iter().map(|ready| ready.id.as_str()).collect();
    assert_eq!(ready, ["C1"]);
    assert_eq!(next.waiting.len(), 1);
    assert_eq!(next.waiting[0].id, "C2");
    let needs = vec![WaitsOn {
        id: "C1".to_string(),
        status: Status::Todo,
        through: None,
    }];
    assert_eq!(next.waiting[0].needs, needs);
    assert_eq!(next.problems, 0);
    // a plan that isn't listed is an error naming it
    let unlisted = PlanName::new("reef", "nope")?;
    let (status, error): (_, ApiError) = ax.client.get(&paths::next(&unlisted)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotListed)
    );
    Ok(())
}

/// The queue's routes edit an entry at a time, answering what came of it with a status that
/// says which, list each entry with its state, and the project's next follows them
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn queue_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // two pieces of work, the second needing the first
    let batch = Puts::new(vec![
        NodePut::new(work("C1", &[], 0), None),
        NodePut::new(work("C2", &["C1"], 1), None),
    ]);
    let (_, _): (_, Checked) = ax.client.post(&paths::nodes(&ax.plan), &batch).await?;
    let queue = paths::queue("reef");
    // the second queued is made, 200
    let add = QueueEdit::Add {
        node: ax.plan.node("C2")?,
        at: Place::Bottom,
    };
    let (status, queued): (_, Queued) = ax.client.post(&queue, &add).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(queued, Queued::Applied { queue } if queue.len() == 1));
    // a move of a node not queued is a conflict, 409, and an add of one not in its plan, 422
    let stray = QueueEdit::Move {
        node: ax.plan.node("C9")?,
        at: Place::Top,
    };
    let (status, queued): (_, Queued) = ax.client.post(&queue, &stray).await?;
    assert_eq!(
        (status, queued),
        (
            StatusCode::CONFLICT,
            Queued::NotQueued {
                node: ax.plan.node("C9")?
            }
        )
    );
    let missing = QueueEdit::Add {
        node: ax.plan.node("C9")?,
        at: Place::Top,
    };
    let (status, queued): (_, Queued) = ax.client.post(&queue, &missing).await?;
    assert_eq!(
        (status, queued),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Queued::NotInPlan {
                node: ax.plan.node("C9")?
            }
        )
    );
    // the listing names the entry's rank and that it waits on the first
    let (status, listing): (_, QueueListing) = ax.client.get(&queue).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].rank, "a0");
    let needs = vec![WaitsOn {
        id: "C1".to_string(),
        status: Status::Todo,
        through: None,
    }];
    assert_eq!(listing.entries[0].entry.state, EntryState::Waiting(needs));
    // the project's next takes the first, and says it skipped the second
    let (status, next): (_, Next) = ax.client.get(&paths::project_next("reef")).await?;
    assert_eq!(status, StatusCode::OK);
    let ready: Vec<&str> = next.ready.iter().map(|ready| ready.id.as_str()).collect();
    assert_eq!(ready, ["C1"]);
    assert_eq!(next.skipped.len(), 1);
    assert_eq!(next.skipped[0].node, ax.plan.node("C2")?);
    // and a project that isn't registered has no queue
    let (status, error): (_, ApiError) = ax.client.get(&paths::queue("nobody")).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotRegistered)
    );
    Ok(())
}

/// The page routes list, get and edit a plan's pages, each edit's outcome said by its status,
/// and a page they keep is served as a doc where the payload links it
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn page_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    let pages = paths::pages(&ax.plan);
    // a page is a doc without a node's frontmatter
    let text = SECTIONED.trim_start_matches("---\nid: D1\n---\n");
    // a plan's pages start empty
    let (status, listing): (_, PageListing) = ax.client.get(&pages).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(listing.pages.is_empty(), "{listing:?}");
    // a new page is put, at version 1, and read back by its path
    let put = |text: &str, read: Option<u64>| PageEdit::Put {
        page: PageWrite::new("design/rows.md", text),
        read,
    };
    let (status, edited): (_, PageEdited) = ax.client.post(&pages, &put(text, None)).await?;
    assert_eq!(
        (status, edited),
        (StatusCode::OK, PageEdited::Applied { version: Some(1) })
    );
    let (status, page): (_, StoredPage) = ax
        .client
        .get(&paths::page(&ax.plan, "design/rows.md"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((page.text.as_str(), page.version), (text, 1));
    // a node links its heading, and the payload's link opens the page's section
    let mut node = work("D1", &[], 0);
    node.spec.docs = vec!["design/rows.md#rows".to_string()];
    let (status, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(node, None))
        .await?;
    assert_eq!(status, StatusCode::OK);
    let (_, payload): (_, Payload) = ax.client.get(&paths::payload(&ax.plan)).await?;
    let href = &payload.nodes[0].docs[0].href;
    let docs = paths::docs(&ax.plan);
    assert_eq!(href, &format!("{docs}design/rows.html#rows"));
    let response = ax
        .client
        .open(href.split('#').next().unwrap_or(href))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let html = response.text().await?;
    assert!(html.contains("<h2 id=\"rows\">Rows</h2>"), "{html}");
    // a stale edit is a conflict, and one that breaks the node's link unprocessable
    let (status, edited): (_, PageEdited) = ax.client.post(&pages, &put("# Rows\n", None)).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        matches!(
            edited,
            PageEdited::Stale {
                refusal: Refusal::Exists,
                ..
            }
        ),
        "{edited:?}"
    );
    let remove = PageEdit::Remove {
        path: "design/rows.md".to_string(),
        read: 1,
    };
    let (status, edited): (_, PageEdited) = ax.client.post(&pages, &remove).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(matches!(edited, PageEdited::Refused(_)), "{edited:?}");
    // a page it doesn't keep is not found, a path no page can have is a bad request, and the
    // pages of a plan no one listed are not listed
    let unlisted = PlanName::new("reef", "missing")?;
    let cases = [
        (paths::page(&ax.plan, "gone.md"), ErrorCode::NotFound),
        (paths::pages(&unlisted), ErrorCode::NotListed),
    ];
    for (path, code) in cases {
        let (status, error): (_, ApiError) = ax.client.get(&path).await?;
        assert_eq!(
            (status.as_u16(), error.code),
            (code.status(), code),
            "{path}"
        );
    }
    let bad = PageEdit::Put {
        page: PageWrite::new("../out.md", "# Out\n"),
        read: None,
    };
    let (status, error): (_, ApiError) = ax.client.post(&pages, &bad).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    // and the listing holds the one page, at its version
    let (_, listing): (_, PageListing) = ax.client.get(&pages).await?;
    let listed: Vec<(&str, u64)> = listing
        .pages
        .iter()
        .map(|page| (page.path.as_str(), page.version))
        .collect();
    assert_eq!(listed, [("design/rows.md", 1)]);
    Ok(())
}

/// A whole-plan write carrying more than axum's default body of 2 MiB, as a book's pages do, is
/// taken
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn whole_plan_write_over_two_mebibytes_is_taken<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // three pages of a mebibyte each, beside a node
    let big = format!("# Big\n\n{}\n", "x".repeat(1 << 20));
    let write = PlanWrite {
        nodes: vec![work("C1", &[], 0)],
        pages: Some(
            ["a.md", "b.md", "c.md"]
                .into_iter()
                .map(|path| PageWrite::new(path, big.as_str()))
                .collect(),
        ),
    };
    let (status, rewrite): (_, Rewrite) = ax.client.put(&paths::revision(&ax.plan), &write).await?;
    assert_eq!(status, StatusCode::OK, "{rewrite:?}");
    // every page kept whole
    let (_, listing): (_, PageListing) = ax.client.get(&paths::pages(&ax.plan)).await?;
    assert_eq!(listing.pages.len(), 3);
    assert!(listing.pages.iter().all(|page| page.text == big));
    Ok(())
}

/// The work routes start a node, read its record, keep a capture as a revision and serve it a
/// file at a time, each outcome said by its status, and each record and revision written is a
/// change to the node's plan
///
/// # Arguments
///
/// * `ax` - The served service and its plan
pub async fn work_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a node to start, and the plan's changes followed
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C30", &[], 0), None))
        .await?;
    let response = ax
        .client
        .open(&paths::changes(Some("reef"), Some("central")))
        .await?;
    let mut events = Events::new(response);
    let node = ax.plan.node("C30")?;
    // nothing started, so no records, and a node's is not found
    let (status, records): (_, Vec<WorkRecord>) = ax.client.get(&paths::work("reef")).await?;
    assert_eq!((status, records), (StatusCode::OK, Vec::new()));
    let (status, error): (_, ApiError) = ax.client.get(&paths::node_work(&node)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    // a start is recorded, 200, and reads back from both routes
    let start = StartWork {
        start: start_in(&ax.repo, "C30", 'a'),
        replaces: None,
    };
    let (status, started): (_, Started) = ax.client.post(&paths::node_work(&node), &start).await?;
    assert_eq!(status, StatusCode::OK);
    let Started::Recorded { record } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    let (_, read): (_, WorkRecord) = ax.client.get(&paths::node_work(&node)).await?;
    assert_eq!(read, record);
    let (_, records): (_, Vec<WorkRecord>) = ax.client.get(&paths::work("reef")).await?;
    assert_eq!(records, [record.clone()]);
    // the same start again is stale, 409, and one of a node not in the plan, 422
    let (status, again): (_, Started) = ax.client.post(&paths::node_work(&node), &start).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        again,
        Started::Stale {
            record: Some(record.clone())
        }
    );
    let stray = ax.plan.node("C99")?;
    let (status, missing): (_, Started) = ax.client.post(&paths::node_work(&stray), &start).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        missing,
        Started::NotInPlan {
            node: stray.clone()
        }
    );
    // a capture of its start is kept as revision 1, 200, one of another start is a conflict,
    // 409, and one of a node never started, 422
    let shas = CaptureShas {
        head: "d".repeat(40),
        tree: "e".repeat(40),
        merge_base: None,
        base: record.start.start_tree.clone(),
    };
    let file = added_file("src/a file.rs", "fn main() {}\n");
    let add = AddCapture {
        pinned: record.start.pinned.clone(),
        shas: shas.clone(),
        conflicts: Vec::new(),
        files: vec![file.clone()],
        resolves: Vec::new(),
        draft: false,
    };
    let (status, added): (_, Captured) = ax.client.post(&paths::captures(&node), &add).await?;
    assert_eq!(status, StatusCode::OK);
    let Captured::Added {
        record: captured,
        revision,
        ..
    } = added
    else {
        anyhow::bail!("the capture wasn't added: {added:?}");
    };
    assert_eq!(captured.captures.len(), 1);
    assert_eq!(captured.captures[0].shas, shas);
    assert_eq!(captured.captures[0].revision, Some(1));
    assert_eq!((revision.revision, revision.files.len()), (1, 1));
    // the revision reads back listed, by its number, and its file by its path
    let (status, listed): (_, Vec<Revision>) = ax.client.get(&paths::revisions(&node)).await?;
    assert_eq!((status, listed), (StatusCode::OK, vec![revision.clone()]));
    let (_, one): (_, Revision) = ax.client.get(&paths::node_revision(&node, 1)).await?;
    assert_eq!(one, revision);
    let path = paths::revision_file(&node, 1, "src/a file.rs");
    let (status, read): (_, FileDiff) = ax.client.get(&path).await?;
    assert_eq!((status, read), (StatusCode::OK, file));
    // a revision it doesn't have, or a file its revision doesn't list, is not found
    for path in [
        paths::node_revision(&node, 2),
        paths::revision_file(&node, 1, "src/other.rs"),
        paths::revision_file(&node, 2, "src/a file.rs"),
    ] {
        let (status, error): (_, ApiError) = ax.client.get(&path).await?;
        assert_eq!(
            (status, error.code),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            "{path}"
        );
    }
    // the same capture again is unchanged, 200, and adds its SHAs naming revision 1
    let (status, again): (_, Captured) = ax.client.post(&paths::captures(&node), &add).await?;
    assert_eq!(status, StatusCode::OK);
    let Captured::Unchanged {
        record: captured,
        revision: same,
        ..
    } = again
    else {
        anyhow::bail!("the same capture wasn't unchanged: {again:?}");
    };
    assert_eq!(same, revision);
    assert_eq!(captured.captures.len(), 2);
    assert_eq!(captured.captures[1].revision, Some(1));
    let other = AddCapture {
        pinned: "f".repeat(40),
        shas: shas.clone(),
        conflicts: Vec::new(),
        files: Vec::new(),
        resolves: Vec::new(),
        draft: false,
    };
    let (status, late): (_, Captured) = ax.client.post(&paths::captures(&node), &other).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        late,
        Captured::Restarted {
            record: captured.clone()
        }
    );
    let (status, none): (_, Captured) = ax.client.post(&paths::captures(&stray), &add).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(none, Captured::NotStarted { node: stray });
    // a start naming a relative worktree is a bad request, and an unregistered project's
    // records, or a plan not listed, are said as such
    let relative = StartWork {
        start: start_in(std::path::Path::new("reef"), "C30", 'b'),
        replaces: Some(captured.version),
    };
    let (status, error): (_, ApiError) =
        ax.client.post(&paths::node_work(&node), &relative).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    let (status, error): (_, ApiError) = ax.client.get(&paths::work("nobody")).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotRegistered)
    );
    let unlisted = PlanName::new("reef", "unlisted")?.node("C30")?;
    let (status, error): (_, ApiError) = ax.client.get(&paths::node_work(&unlisted)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotListed)
    );
    // and the plan's followers were told of the start, the revision and each capture's SHAs
    let recorded = |version| Change::WorkRecorded {
        node: node.clone(),
        version,
    };
    let expected = [
        recorded(1),
        Change::RevisionCaptured {
            node: node.clone(),
            revision: 1,
        },
        recorded(2),
        recorded(3),
    ];
    for expected in expected {
        let event = events.next().await?.expect("the stream sent the write");
        assert_eq!(event.event, expected.kind());
        let change: Change = serde_json::from_str(&event.data)?;
        assert_eq!(change, expected);
    }
    Ok(())
}

/// The comment routes start threads on a node and its lines, reply in them and mark them, each
/// outcome with its status, serve a node's and a plan's threads rendered, and tell the plan's
/// followers of each comment and mark
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn comment_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // a node started and captured once, and the plan's changes followed from then on
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C34", &[], 0), None))
        .await?;
    let node = ax.plan.node("C34")?;
    let start = StartWork {
        start: start_in(&ax.repo, "C34", 'a'),
        replaces: None,
    };
    let (_, started): (_, Started) = ax.client.post(&paths::node_work(&node), &start).await?;
    let Started::Recorded { record } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    let add = AddCapture {
        pinned: record.start.pinned.clone(),
        shas: CaptureShas {
            head: "d".repeat(40),
            tree: "e".repeat(40),
            merge_base: None,
            base: record.start.start_tree.clone(),
        },
        conflicts: Vec::new(),
        files: vec![added_file("src/a.rs", "one\ntwo\n")],
        resolves: Vec::new(),
        draft: false,
    };
    let (status, _): (_, Captured) = ax.client.post(&paths::captures(&node), &add).await?;
    assert_eq!(status, StatusCode::OK);
    let response = ax
        .client
        .open(&paths::changes(Some("reef"), Some("central")))
        .await?;
    let mut events = Events::new(response);
    // no thread yet, on the plan or the node
    let (status, threads): (_, Vec<Thread>) =
        ax.client.get(&paths::plan_comments(&ax.plan)).await?;
    assert_eq!((status, threads), (StatusCode::OK, Vec::new()));
    // a thread on a line of revision 1 is kept, 200, its comment rendered with raw HTML escaped
    let line = Target::Line(Anchor {
        revision: 1,
        path: "src/a.rs".to_string(),
        side: Side::New,
        line: 2,
    });
    let new = NewThread {
        target: line.clone(),
        body: "Why **two** <b>here</b>?".to_string(),
    };
    let (status, added): (_, Commented) =
        ax.client.post(&paths::node_comments(&node), &new).await?;
    assert_eq!(status, StatusCode::OK);
    let Commented::Added { thread, comment } = added else {
        anyhow::bail!("the thread wasn't kept: {added:?}");
    };
    assert_eq!(
        (thread.number, comment, thread.target.clone()),
        (1, 1, line)
    );
    assert_eq!(
        thread.comments[0].html,
        "<p>Why <strong>two</strong> &lt;b&gt;here&lt;/b&gt;?</p>\n"
    );
    // a general thread, and a reply in the first, each 200
    let general = NewThread {
        target: Target::Node,
        body: "Looks good".to_string(),
    };
    let (status, _): (_, Commented) = ax
        .client
        .post(&paths::node_comments(&node), &general)
        .await?;
    assert_eq!(status, StatusCode::OK);
    let reply = NewReply {
        body: "Because.".to_string(),
    };
    let (status, replied): (_, Commented) =
        ax.client.post(&paths::replies(&ax.plan, 1), &reply).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        matches!(replied, Commented::Added { comment: 3, .. }),
        "{replied:?}"
    );
    // the plan's threads and the node's are both, and one thread reads by its number
    let (_, threads): (_, Vec<Thread>) = ax.client.get(&paths::plan_comments(&ax.plan)).await?;
    let numbers: Vec<u32> = threads.iter().map(|thread| thread.number).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(threads[0].comments.len(), 2);
    assert_eq!(threads[0].comments[1].html, "<p>Because.</p>\n");
    let (_, on_node): (_, Vec<Thread>) = ax.client.get(&paths::node_comments(&node)).await?;
    assert_eq!(on_node, threads);
    let (status, one): (_, Thread) = ax.client.get(&paths::thread(&ax.plan, 1)).await?;
    assert_eq!((status, &one), (StatusCode::OK, &threads[0]));
    // resolved over its version, 200, then stale over the same version, 409
    let mark = MarkThread {
        state: ThreadState::Resolved,
        read: 1,
    };
    let (status, marked): (_, Marked) = ax
        .client
        .put(&paths::thread_state(&ax.plan, 1), &mark)
        .await?;
    assert_eq!(status, StatusCode::OK);
    let Marked::Marked { thread: resolved } = marked else {
        anyhow::bail!("the mark was refused: {marked:?}");
    };
    assert_eq!(
        (resolved.state, resolved.version),
        (ThreadState::Resolved, 2)
    );
    assert!(!resolved.comments[0].html.is_empty());
    let (status, stale): (_, Marked) = ax
        .client
        .put(&paths::thread_state(&ax.plan, 1), &mark)
        .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(matches!(stale, Marked::Stale { .. }), "{stale:?}");
    // a line its revisions don't hold, a node not in the plan, a reply or a mark in no thread,
    // each 422, and nothing written
    let off = NewThread {
        target: Target::Line(Anchor {
            revision: 1,
            path: "src/a.rs".to_string(),
            side: Side::New,
            line: 3,
        }),
        body: "Here?".to_string(),
    };
    let (status, refused): (_, Commented) =
        ax.client.post(&paths::node_comments(&node), &off).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, Commented::NoLine { target: off.target });
    let stray = ax.plan.node("C99")?;
    let (status, refused): (_, Commented) = ax
        .client
        .post(&paths::node_comments(&stray), &general)
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, Commented::NotInPlan { node: stray });
    let (status, refused): (_, Commented) =
        ax.client.post(&paths::replies(&ax.plan, 3), &reply).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, Commented::NoThread { thread: 3 });
    let (status, refused): (_, Marked) = ax
        .client
        .put(&paths::thread_state(&ax.plan, 9), &mark)
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, Marked::NoThread { thread: 9 });
    // an empty body is a bad request, a thread not there is not found, and a plan not listed
    // is said as such
    let empty = NewReply {
        body: " ".to_string(),
    };
    let (status, error): (_, ApiError) =
        ax.client.post(&paths::replies(&ax.plan, 1), &empty).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    let (status, error): (_, ApiError) = ax.client.get(&paths::thread(&ax.plan, 3)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    let unlisted = PlanName::new("reef", "unlisted")?;
    let (status, error): (_, ApiError) = ax.client.get(&paths::plan_comments(&unlisted)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotListed)
    );
    let (_, threads): (_, Vec<Thread>) = ax.client.get(&paths::plan_comments(&ax.plan)).await?;
    assert_eq!(threads.len(), 2);
    // and the plan's followers were told of each comment kept, and the one mark
    let added = |thread, comment| Change::CommentAdded {
        node: node.clone(),
        thread,
        comment,
    };
    let expected = [
        added(1, 1),
        added(2, 2),
        added(1, 3),
        Change::ThreadMarked {
            node: node.clone(),
            thread: 1,
            state: ThreadState::Resolved,
            version: 2,
        },
    ];
    for expected in expected {
        let event = events.next().await?.expect("the stream sent the write");
        assert_eq!(event.event, expected.kind());
        let change: Change = serde_json::from_str(&event.data)?;
        assert_eq!(change, expected);
    }
    Ok(())
}

/// A capture that moves or loses commented lines follows their threads before it is told, and
/// the stream says which threads it wrote; a capture that moves none says nothing of threads
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn a_capture_announces_the_threads_it_followed<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // C36 started, and captured once with a file of two lines
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C36", &[], 0), None))
        .await?;
    let node = ax.plan.node("C36")?;
    let start = StartWork {
        start: start_in(&ax.repo, "C36", 'a'),
        replaces: None,
    };
    let (_, started): (_, Started) = ax.client.post(&paths::node_work(&node), &start).await?;
    let Started::Recorded { record } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    // a capture of a tree, holding `src/a.rs` as it is in a blob of the same letter
    let capture = |tree: char, text: &str| AddCapture {
        pinned: record.start.pinned.clone(),
        shas: CaptureShas {
            head: "d".repeat(40),
            tree: tree.to_string().repeat(40),
            merge_base: None,
            base: record.start.start_tree.clone(),
        },
        conflicts: Vec::new(),
        files: vec![FileDiff {
            new_id: Some(tree.to_string().repeat(40)),
            ..added_file("src/a.rs", text)
        }],
        resolves: Vec::new(),
        draft: false,
    };
    let (status, _): (_, Captured) = ax
        .client
        .post(&paths::captures(&node), &capture('1', "one\ntwo\n"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    let response = ax
        .client
        .open(&paths::changes(Some("reef"), Some("central")))
        .await?;
    let mut events = Events::new(response);
    // a thread on each line, and one on the node
    let on_line = |line| NewThread {
        target: Target::Line(Anchor {
            revision: 1,
            path: "src/a.rs".to_string(),
            side: Side::New,
            line,
        }),
        body: format!("Line {line}?"),
    };
    let general = NewThread {
        target: Target::Node,
        body: "Overall?".to_string(),
    };
    for new in [on_line(2), on_line(1), general] {
        let (status, _): (_, Commented) =
            ax.client.post(&paths::node_comments(&node), &new).await?;
        assert_eq!(status, StatusCode::OK);
    }
    // the next capture deletes `two` and adds a line above `one`
    let (status, captured): (_, Captured) = ax
        .client
        .post(&paths::captures(&node), &capture('2', "zero\none\n"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(captured, Captured::Added { .. }), "{captured:?}");
    // so the thread on `two` is outdated, the one on `one` moved down, and the general one alone
    let (_, threads): (_, Vec<Thread>) = ax.client.get(&paths::node_comments(&node)).await?;
    assert_eq!(threads[0].outdated, Some(2));
    assert!(threads[0].followed.is_empty());
    let moved = Anchor {
        revision: 2,
        path: "src/a.rs".to_string(),
        side: Side::New,
        line: 2,
    };
    assert_eq!(threads[1].followed, [moved]);
    assert_eq!((threads[1].outdated, threads[1].version), (None, 2));
    assert_eq!((threads[2].followed.len(), threads[2].version), (0, 1));
    // the same tree again keeps nothing new and moves no thread
    let (status, again): (_, Captured) = ax
        .client
        .post(&paths::captures(&node), &capture('2', "zero\none\n"))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(again, Captured::Unchanged { .. }), "{again:?}");
    // and the stream told of the comments, the revision, then the threads it followed
    let added = |thread| Change::CommentAdded {
        node: node.clone(),
        thread,
        comment: thread,
    };
    let recorded = |version| Change::WorkRecorded {
        node: node.clone(),
        version,
    };
    let expected = [
        added(1),
        added(2),
        added(3),
        Change::RevisionCaptured {
            node: node.clone(),
            revision: 2,
        },
        recorded(3),
        Change::ThreadsFollowed {
            node: node.clone(),
            revision: 2,
            threads: vec![
                FollowedThread {
                    thread: 1,
                    version: 2,
                    outdated: true,
                },
                FollowedThread {
                    thread: 2,
                    version: 2,
                    outdated: false,
                },
            ],
        },
        recorded(4),
    ];
    for expected in expected {
        let event = events.next().await?.expect("the stream sent the write");
        assert_eq!(event.event, expected.kind());
        let change: Change = serde_json::from_str(&event.data)?;
        assert_eq!(change, expected);
    }
    Ok(())
}

/// A capture resolves the threads its trailers name and says what came of each, a thread is
/// resolved by hand at its resolution route, and each thread written is told to the stream
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn resolves_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // C37 started and captured once, with two general threads on it
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C37", &[], 0), None))
        .await?;
    let node = ax.plan.node("C37")?;
    let start = StartWork {
        start: start_in(&ax.repo, "C37", 'a'),
        replaces: None,
    };
    let (_, started): (_, Started) = ax.client.post(&paths::node_work(&node), &start).await?;
    let Started::Recorded { record } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    let capture = |tree: char, resolves: Vec<Resolves>| AddCapture {
        pinned: record.start.pinned.clone(),
        shas: CaptureShas {
            head: "d".repeat(40),
            tree: tree.to_string().repeat(40),
            merge_base: None,
            base: record.start.start_tree.clone(),
        },
        conflicts: Vec::new(),
        files: vec![FileDiff {
            new_id: Some(tree.to_string().repeat(40)),
            ..added_file("src/a.rs", "one\n")
        }],
        resolves,
        draft: false,
    };
    let (status, _): (_, Captured) = ax
        .client
        .post(&paths::captures(&node), &capture('e', Vec::new()))
        .await?;
    assert_eq!(status, StatusCode::OK);
    for body in ["Take the lock first", "And the docs?"] {
        let new = NewThread {
            target: Target::Node,
            body: body.to_string(),
        };
        let (status, _): (_, Commented) =
            ax.client.post(&paths::node_comments(&node), &new).await?;
        assert_eq!(status, StatusCode::OK);
    }
    let response = ax
        .client
        .open(&paths::changes(Some("reef"), Some("central")))
        .await?;
    let mut events = Events::new(response);
    // a capture whose commit names thread 1 and one no comment has resolves the first, 200
    let commit = ResolvingCommit {
        commit: "f".repeat(40),
        summary: "Take the lock first".to_string(),
        author: "dev@example.invalid".to_string(),
        authored: 100,
    };
    let named = |thread| Resolves {
        thread,
        commit: commit.clone(),
    };
    let (status, captured): (_, Captured) = ax
        .client
        .post(
            &paths::captures(&node),
            &capture('g', vec![named(1), named(7)]),
        )
        .await?;
    assert_eq!(status, StatusCode::OK);
    let Captured::Added {
        revision, trailers, ..
    } = captured
    else {
        anyhow::bail!("the capture wasn't added: {captured:?}");
    };
    let read = |thread, outcome| TrailerRead {
        thread,
        commit: commit.commit.clone(),
        outcome,
    };
    assert_eq!(
        trailers,
        [
            read(1, TrailerOutcome::Resolved),
            read(7, TrailerOutcome::NoThread)
        ]
    );
    // the thread is resolved, the capture's revision linked with its commit
    let (_, one): (_, Thread) = ax.client.get(&paths::thread(&ax.plan, 1)).await?;
    assert_eq!((one.state, one.version), (ThreadState::Resolved, 2));
    let linked = one.resolved_by().expect("the thread was linked");
    assert_eq!((&linked.node, linked.revision), (&node, revision.revision));
    assert_eq!(linked.commit.as_ref(), Some(&commit));
    // the same capture again is unchanged, and its trailer, read again, writes nothing
    let (status, again): (_, Captured) = ax
        .client
        .post(&paths::captures(&node), &capture('g', vec![named(1)]))
        .await?;
    assert_eq!(status, StatusCode::OK);
    let Captured::Unchanged { trailers, .. } = again else {
        anyhow::bail!("the same capture wasn't unchanged: {again:?}");
    };
    assert_eq!(trailers, [read(1, TrailerOutcome::Seen)]);
    // thread 2 resolved by hand: a revision the node lacks is 422, one it has is 200
    let resolve = |revision, read| ResolveThread {
        node: node.clone(),
        revision,
        read,
    };
    let path = paths::thread_resolution(&ax.plan, 2);
    let (status, refused): (_, Resolved) = ax.client.post(&path, &resolve(9, 1)).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused,
        Resolved::NoRevision {
            node: node.clone(),
            revision: 9
        }
    );
    let (status, resolved): (_, Resolved) = ax.client.post(&path, &resolve(1, 1)).await?;
    assert_eq!(status, StatusCode::OK);
    let Resolved::Resolved { thread: two } = resolved else {
        anyhow::bail!("the resolve was refused: {resolved:?}");
    };
    assert_eq!((two.state, two.version), (ThreadState::Resolved, 2));
    assert_eq!(two.resolutions[0].commit, None);
    assert!(!two.comments[0].html.is_empty());
    // over the same version again it is stale, 409, and a thread not there is 422
    let (status, stale): (_, Resolved) = ax.client.post(&path, &resolve(1, 1)).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(matches!(stale, Resolved::Stale { .. }), "{stale:?}");
    let missing = paths::thread_resolution(&ax.plan, 9);
    let (status, refused): (_, Resolved) = ax.client.post(&missing, &resolve(1, 1)).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, Resolved::NoThread { thread: 9 });
    // a revision of another project's node answers none of this one's threads
    let elsewhere = ResolveThread {
        node: "other/central/C37".parse()?,
        revision: 1,
        read: 2,
    };
    let (status, refused): (_, Resolved) = ax.client.post(&path, &elsewhere).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        matches!(refused, Resolved::NoRevision { .. }),
        "{refused:?}"
    );
    // and the stream heard the capture, then each thread resolved, as it now stands
    let marked = |thread| Change::ThreadMarked {
        node: node.clone(),
        thread,
        state: ThreadState::Resolved,
        version: 2,
    };
    let expected = [
        Change::RevisionCaptured {
            node: node.clone(),
            revision: 2,
        },
        Change::WorkRecorded {
            node: node.clone(),
            version: 3,
        },
        marked(1),
        Change::WorkRecorded {
            node: node.clone(),
            version: 4,
        },
        marked(2),
    ];
    for expected in expected {
        let event = events.next().await?.expect("the stream sent the write");
        assert_eq!(event.event, expected.kind());
        let change: Change = serde_json::from_str(&event.data)?;
        assert_eq!(change, expected);
    }
    Ok(())
}

/// Each loop route answers with its status: a start, a hold, a hand to a session and a stop are
/// 200, a busy pane and a node another loop holds are 409, a pane with no loop and a node not in
/// its plan are 422, an unregistered project or an unlisted plan is 404, and each loop written
/// is told to its project's followers
///
/// # Arguments
///
/// * `ax` - The served service and a client of it
pub async fn loop_routes_answer_with_their_status<B: Backend + 'static>(
    ax: Ax<B>,
) -> anyhow::Result<()> {
    // C39 in the plan, the project's changes followed, and no loop yet
    let (_, _): (_, Checked) = ax
        .client
        .post(&paths::nodes(&ax.plan), &one(work("C39", &[], 0), None))
        .await?;
    let response = ax.client.open(&paths::changes(Some("reef"), None)).await?;
    let mut events = Events::new(response);
    let (status, loops): (_, Vec<Loop>) = ax.client.get(paths::LOOPS).await?;
    assert_eq!((status, loops), (StatusCode::OK, Vec::new()));
    // an edit of a pane's loop
    let edit = |pid, edit| EditLoop {
        pane: Pane::Claude { pid },
        edit,
    };
    let start = |project: &str, plan: Option<&str>| LoopEdit::Start {
        project: project.to_string(),
        plan: plan.map(str::to_string),
        session: Some("s1".to_string()),
        width: None,
        opens: None,
        pid: None,
    };
    // started in a pane, 200, then busy for another plan, 409
    let (status, started): (_, LoopEdited) = ax
        .client
        .post(paths::LOOPS, &edit(1, start("reef", Some("central"))))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(started, LoopEdited::Started { .. }), "{started:?}");
    let (status, busy): (_, LoopEdited) = ax
        .client
        .post(paths::LOOPS, &edit(1, start("reef", None)))
        .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(matches!(busy, LoopEdited::Busy { .. }), "{busy:?}");
    // a project not registered, or a plan it doesn't list, is not found
    for (project, plan, code) in [
        ("nope", None, ErrorCode::NotRegistered),
        ("reef", Some("unlisted"), ErrorCode::NotListed),
    ] {
        let (status, error): (_, ApiError) = ax
            .client
            .post(paths::LOOPS, &edit(2, start(project, plan)))
            .await?;
        assert_eq!((status, error.code), (StatusCode::NOT_FOUND, code));
    }
    // C39 held by the first pane, 200, then refused to a second, 409, naming the first
    let node = ax.plan.node("C39")?;
    let hold = |node: &reef_model::NodeName| LoopEdit::Hold {
        node: Some(node.clone()),
        members: Vec::new(),
    };
    let (status, held): (_, LoopEdited) =
        ax.client.post(paths::LOOPS, &edit(1, hold(&node))).await?;
    assert_eq!(status, StatusCode::OK);
    let LoopEdited::Edited { r#loop: holding } = held else {
        anyhow::bail!("the hold wasn't written: {held:?}");
    };
    assert_eq!(holding.node.as_ref(), Some(&node));
    let (status, _): (_, LoopEdited) = ax
        .client
        .post(paths::LOOPS, &edit(2, start("reef", None)))
        .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, refused): (_, LoopEdited) =
        ax.client.post(paths::LOOPS, &edit(2, hold(&node))).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        refused,
        LoopEdited::Held {
            node: node.clone(),
            by: holding.clone()
        }
    );
    // a node its plan lacks, or a pane with no loop, is 422
    let stray = ax.plan.node("C99")?;
    let (status, refused): (_, LoopEdited) =
        ax.client.post(paths::LOOPS, &edit(2, hold(&stray))).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused, LoopEdited::NotInPlan { node: stray });
    let (status, refused): (_, LoopEdited) = ax
        .client
        .post(paths::LOOPS, &edit(3, LoopEdit::Pause))
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused,
        LoopEdited::NoLoop {
            pane: Pane::Claude { pid: 3 }
        }
    );
    // the first handed to a clear's new session keeps its node, 200
    let session = LoopEdit::Session {
        session: "s2".to_string(),
        pid: None,
    };
    let (status, handed): (_, LoopEdited) = ax.client.post(paths::LOOPS, &edit(1, session)).await?;
    assert_eq!(status, StatusCode::OK);
    let LoopEdited::Edited { r#loop: handed } = handed else {
        anyhow::bail!("the session wasn't handed the loop: {handed:?}");
    };
    assert_eq!(handed.session.as_deref(), Some("s2"));
    assert_eq!(handed.node.as_ref(), Some(&node));
    assert_eq!(handed.state, LoopState::Running);
    // the project's loops are both, by pane, and an unregistered project's are not found
    let (status, loops): (_, Vec<Loop>) = ax.client.get(&paths::project_loops("reef")).await?;
    assert_eq!(status, StatusCode::OK);
    let panes: Vec<String> = loops
        .iter()
        .map(|feeding| feeding.pane.to_string())
        .collect();
    assert_eq!(panes, ["claude:1", "claude:2"]);
    let (status, error): (_, ApiError) = ax.client.get(&paths::project_loops("nope")).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotRegistered)
    );
    // the second stopped, 200, as it was
    let (status, stopped): (_, LoopEdited) = ax
        .client
        .post(paths::LOOPS, &edit(2, LoopEdit::Stop))
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(stopped, LoopEdited::Stopped { .. }), "{stopped:?}");
    // and the project's followers heard of each loop written, and of nothing refused
    let changed = |pid| Change::LoopChanged {
        project: "reef".to_string(),
        pane: Pane::Claude { pid },
    };
    for expected in [changed(1), changed(1), changed(2), changed(1), changed(2)] {
        let event = events.next().await?.expect("the stream sent the write");
        assert_eq!(event.event, expected.kind());
        let change: Change = serde_json::from_str(&event.data)?;
        assert_eq!(change, expected);
    }
    Ok(())
}
