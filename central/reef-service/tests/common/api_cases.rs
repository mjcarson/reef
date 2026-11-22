//! The JSON API suite's cases, each generic over the backend so it runs against shoal and the
//! twin
//!
//! Each case talks to a served service through a client that knows only `reef-api`'s types and
//! the reef-model types they are built on, as the CLI will. Its plan is `reef/central`, of a
//! project registered for a fresh git repo.

use reef_api::{
    AddCapture, AddedPlan, AddedProject, ApiError, CaptureShas, Captured, Change, Checked,
    ErrorCode, Listing, NewPlan, NewProject, NodePut, NodeWrite, PageEdit, PageEdited, PageListing,
    PageWrite, Place, PlanInfo, PlanWrite, Project, PutNode, Puts, QueueEdit, QueueListing, Queued,
    Refusal, Rewrite, StalePut, StartWork, Started, StoredNode, StoredPage, WorkRecord, paths,
};
use reef_model::{
    Description, Diagnostic, EntryState, Next, NodeSpec, Payload, PlanName, Status, WaitsOn,
};
use reef_service::Backend;
use reqwest::StatusCode;

use super::http::{Ax, Events};
use super::work_cases::start_in;
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

/// The work routes start a node, read its record and add a capture's SHAs, each outcome said
/// by its status, and each record written is a change to the node's plan
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
    // a capture of its start is added, 200, one of another start is a conflict, 409, and one of
    // a node never started, 422
    let shas = CaptureShas {
        head: "d".repeat(40),
        tree: "e".repeat(40),
        merge_base: None,
        base: record.start.start_tree.clone(),
    };
    let add = AddCapture {
        pinned: record.start.pinned.clone(),
        shas: shas.clone(),
    };
    let (status, added): (_, Captured) = ax.client.post(&paths::captures(&node), &add).await?;
    assert_eq!(status, StatusCode::OK);
    let Captured::Added { record: captured } = added else {
        anyhow::bail!("the capture wasn't added: {added:?}");
    };
    assert_eq!(captured.captures.len(), 1);
    assert_eq!(captured.captures[0].shas, shas);
    let other = AddCapture {
        pinned: "f".repeat(40),
        shas: shas.clone(),
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
    // and the plan's followers were told of the start and the capture, each at its version
    for version in [1, 2] {
        let event = events
            .next()
            .await?
            .expect("the stream sent the record written");
        assert_eq!(event.event, "work-recorded");
        let change: Change = serde_json::from_str(&event.data)?;
        let expected = Change::WorkRecorded {
            node: node.clone(),
            version,
        };
        assert_eq!(change, expected);
    }
    Ok(())
}
