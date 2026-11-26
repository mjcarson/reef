//! The authorship suite's cases, each generic over the backend so it runs against shoal and the
//! twin
//!
//! Each case serves its fixture's service on a loopback port of its own and writes through the
//! JSON API as signed-in users do, as Claude does for them with the `reef-writer` header, and with
//! no token at all, then reads back who each write names. A write's author is always the user its
//! token signs in, whatever its body says.

use reef_api::{
    Author, Checked, Commented, Decided, DecisionKind, DecisionsRecorded, Edited, HistoryPage,
    Logged, MarkReview, MarkThread, Marked, NewReply, NewThread, NodePut, NodeWrite, PageEdit,
    PageEdited, PageWrite, Place, Puts, QueueEdit, QueueListing, Queued, RecordDecisions,
    ResolveThread, Resolved, Resolves, ResolvingCommit, Review, ReviewState, Reviewed, Revision,
    StartWork, Started, Target, Thread, ThreadState, User, UserAdded, Via, WorkRecord, Writer,
    paths,
};
use reef_model::{Description, NodeSpec, PlanName, Status};
use reef_service::{Asker, Backend};
use reqwest::StatusCode;
use serde_json::json;

use super::Fx;
use super::http::{Ax, Client};
use super::sign_in_cases::{first_admin, new_user, signed_in};
use super::work_cases::{added_file, capture, start_in};

/// A node that says why and what, to put
///
/// # Arguments
///
/// * `id` - The node's id
/// * `status` - Its status, if it says one
fn work(id: &str, status: Option<Status>) -> NodeWrite {
    NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} Some work")),
            status,
            description: Some(Description {
                why: Some(format!("Without {id}, something is missing.")),
                what: Some(format!("With {id}, it is there.")),
                details: None,
            }),
            ..NodeSpec::default()
        },
        body: String::new(),
        order: 0,
    }
}

/// Puts one node over the version read, through a client, checking it was written
///
/// # Arguments
///
/// * `client` - The client to write with
/// * `plan` - The plan to put it in
/// * `node` - The node
/// * `read` - The version read, or `None` for a new node
async fn put(
    client: &Client,
    plan: &PlanName,
    node: NodeWrite,
    read: Option<u64>,
) -> anyhow::Result<()> {
    let puts = Puts::new(vec![NodePut::new(node, read)]);
    let (status, checked): (_, Checked) = client.post(&paths::nodes(plan), &puts).await?;
    anyhow::ensure!(
        status == StatusCode::OK && matches!(checked, Checked::Applied { .. }),
        "the put answered {status}: {checked:?}"
    );
    Ok(())
}

/// Adds a user, as the admin who adds them, and signs them in
///
/// # Arguments
///
/// * `ax` - The served service
/// * `admin` - The admin who adds them
/// * `name` - Their name
async fn added<B: Backend + 'static>(
    ax: &Ax<B>,
    admin: &User,
    name: &str,
) -> anyhow::Result<(User, Client)> {
    // added by the admin, from the service's own host
    let asker = Asker {
        user: Some(admin),
        local: true,
        ..Default::default()
    };
    let UserAdded::Added(user) = ax.service.add_user(new_user(name, false), asker).await? else {
        anyhow::bail!("`{name}` was taken in a fresh service");
    };
    Ok((user, signed_in(ax, name).await?))
}

/// Who a history's entries name, newest first
///
/// # Arguments
///
/// * `history` - The history
fn authors(history: &[Logged]) -> Vec<Option<String>> {
    history
        .iter()
        .map(|logged| logged.author.describe())
        .collect()
}

/// A comment shows who wrote it: the user its token signs in, and Claude, for them, where the
/// request says it writes as Claude; and Claude's reply still leaves its thread waiting on the user
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn a_comment_shows_its_author<B: Backend + 'static>(fx: Fx<B>) -> anyhow::Result<()> {
    // ada signed in, and a node to comment on
    let ax = fx.api_signing_in().await?;
    let ada_user = first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    let plan = ax.plan.clone();
    put(&ada, &plan, work("C50", None), None).await?;
    let node = plan.node("C50")?;
    // ada starts a thread, and Claude replies for her
    let new = NewThread {
        target: Target::Node,
        body: "Who wrote this?".to_string(),
        by: Writer::User,
        review: false,
    };
    let (status, commented): (_, Commented) = ada.post(&paths::node_comments(&node), &new).await?;
    assert_eq!(status, StatusCode::OK);
    let Commented::Added { thread, .. } = commented else {
        anyhow::bail!("the thread wasn't kept: {commented:?}");
    };
    let reply = NewReply {
        body: "I did.".to_string(),
        by: Writer::User,
        review: false,
    };
    let (status, _): (_, Commented) = ada
        .as_claude()
        .post(&paths::replies(&plan, thread.number), &reply)
        .await?;
    assert_eq!(status, StatusCode::OK);
    // each comment names who wrote it, ada by when she was added too
    let (_, thread): (_, Thread) = ada.get(&paths::thread(&plan, thread.number)).await?;
    let said: Vec<Option<String>> = thread
        .comments
        .iter()
        .map(|comment| comment.author.describe())
        .collect();
    assert_eq!(
        said,
        vec![Some("ada".to_string()), Some("Claude, for ada".to_string())]
    );
    assert_eq!(thread.comments[0].author.since, Some(ada_user.created_at));
    // and Claude's word last leaves the thread waiting on the user, as before reef knew them
    assert!(thread.is_open() && !thread.wants_answer());
    Ok(())
}

/// Every write names the user its token signs in, whatever its body says: a node's put, a
/// page's edit, a queue's edit, a comment and a decision
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn every_write_names_the_tokens_user<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // ada, an admin, and bob, each signed in
    let ax = fx.api_signing_in().await?;
    let ada_user = first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    let (_, bob) = added(&ax, &ada_user, "bob").await?;
    let plan = ax.plan.clone();
    let node = plan.node("C1")?;
    // ada puts C1, and bob sets it in progress
    put(&ada, &plan, work("C1", None), None).await?;
    put(&bob, &plan, work("C1", Some(Status::InProgress)), Some(1)).await?;
    let (_, HistoryPage { edits: history, .. }) = ada.get(&paths::node_history(&node)).await?;
    assert_eq!(
        authors(&history),
        vec![Some("bob".to_string()), Some("ada".to_string())]
    );
    assert_eq!(
        history[0].edits,
        vec![Edited::Status {
            from: None,
            to: Some(Status::InProgress)
        }]
    );
    // ada adds a page, and bob queues C1
    let edit = PageEdit::Put {
        page: PageWrite::new("design/a.md", "# A\n"),
        read: None,
    };
    let (status, edited): (_, PageEdited) = ada.post(&paths::pages(&plan), &edit).await?;
    assert!(
        status == StatusCode::OK && matches!(edited, PageEdited::Applied { .. }),
        "{edited:?}"
    );
    let (_, HistoryPage { edits: history, .. }) =
        ada.get(&paths::page_history(&plan, "design/a.md")).await?;
    assert_eq!(authors(&history), vec![Some("ada".to_string())]);
    let queue = QueueEdit::Add {
        node: node.clone(),
        at: Place::Top,
    };
    let (status, queued): (_, Queued) = bob.post(&paths::queue("reef"), &queue).await?;
    assert!(
        status == StatusCode::OK && matches!(queued, Queued::Applied { .. }),
        "{queued:?}"
    );
    let (_, HistoryPage { edits: history, .. }) = ada.get(&paths::queue_history("reef")).await?;
    assert_eq!(authors(&history), vec![Some("bob".to_string())]);
    let (_, listing): (_, QueueListing) = ada.get(&paths::queue("reef")).await?;
    let placed = listing.entries[0].placed.as_ref().expect("bob placed C1");
    assert_eq!(placed.describe().as_deref(), Some("queued by bob"));
    // a comment whose body names another writer is still the token's user's
    let forged = json!({
        "target": {"on": "node"},
        "body": "Not by mallory",
        "user": "mallory",
        "since": 1,
    });
    let (_, commented): (_, Commented) = bob.post(&paths::node_comments(&node), &forged).await?;
    let Commented::Added { thread, .. } = commented else {
        anyhow::bail!("the thread wasn't kept: {commented:?}");
    };
    assert_eq!(thread.comments[0].author.describe().as_deref(), Some("bob"));
    // and so is a decision whose body names another author
    let decided = Decided {
        call: "toolu_1".to_string(),
        session: "s1".to_string(),
        via: Via::Terminal,
        asked_at: 1,
        answered_at: None,
        kind: DecisionKind::Plan {
            plan: "# Plan".to_string(),
            html: String::new(),
        },
        outcome: None,
        author: Author {
            user: Some("mallory".to_string()),
            ..Author::default()
        },
    };
    let record = RecordDecisions {
        decisions: vec![decided],
    };
    let (status, recorded): (_, DecisionsRecorded) =
        ada.post(&paths::node_decisions(&node), &record).await?;
    assert!(
        status == StatusCode::OK && matches!(recorded, DecisionsRecorded::Recorded { .. }),
        "{recorded:?}"
    );
    let (_, decisions): (_, Vec<reef_api::Decision>) =
        ada.get(&paths::node_decisions(&node)).await?;
    assert_eq!(
        decisions[0].decided.author.describe().as_deref(),
        Some("ada")
    );
    Ok(())
}

/// With sign-in off, a write sent with a token still names its user, and one sent with none
/// names nobody
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn writes_with_sign_in_off_name_a_token_or_nobody<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // ada added from this host and signed in, though nobody need sign in
    let ax = fx.api().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    let plan = ax.plan.clone();
    let node = plan.node("C1")?;
    // ada puts C1, and a client with no token sets it done
    put(&ada, &plan, work("C1", None), None).await?;
    put(&ax.client, &plan, work("C1", Some(Status::Done)), Some(1)).await?;
    let (_, HistoryPage { edits: history, .. }) =
        ax.client.get(&paths::node_history(&node)).await?;
    assert_eq!(authors(&history), vec![None, Some("ada".to_string())]);
    assert!(history[0].author.is_unknown());
    // and Claude with no token is Claude for nobody known
    put(
        &ax.client.as_claude(),
        &plan,
        work("C1", Some(Status::Todo)),
        Some(2),
    )
    .await?;
    let (_, HistoryPage { edits: history, .. }) =
        ax.client.get(&paths::node_history(&node)).await?;
    assert_eq!(history[0].author.describe().as_deref(), Some("Claude"));
    Ok(())
}

/// A thread's mark, a resolve by hand, a review started and submitted, a start, a capture and its
/// revision, and a trailer's resolve each name who made them
///
/// # Arguments
///
/// * `fx` - The store and its plan
pub async fn marks_starts_and_captures_name_their_authors<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    // ada, an admin, and bob, each signed in, and C2 to work on
    let ax = fx.api_signing_in().await?;
    let ada_user = first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    let (_, bob) = added(&ax, &ada_user, "bob").await?;
    let plan = ax.plan.clone();
    put(&ada, &plan, work("C2", None), None).await?;
    let node = plan.node("C2")?;
    // bob starts it, and ada captures it
    let start = StartWork {
        start: start_in(&ax.repo, "C2", 'a'),
        replaces: None,
    };
    let (_, started): (_, Started) = bob.post(&paths::node_work(&node), &start).await?;
    let Started::Recorded { record, .. } = started else {
        anyhow::bail!("the start wasn't recorded: {started:?}");
    };
    assert_eq!(record.started_by.describe().as_deref(), Some("bob"));
    let first = capture(
        &record.start.pinned,
        'e',
        vec![added_file("src/a.rs", "one\n")],
    );
    let (status, _): (_, reef_api::Captured) = ada.post(&paths::captures(&node), &first).await?;
    assert_eq!(status, StatusCode::OK);
    let (_, read): (_, WorkRecord) = ada.get(&paths::node_work(&node)).await?;
    assert_eq!(read.started_by.describe().as_deref(), Some("bob"));
    assert_eq!(read.captures[0].author.describe().as_deref(), Some("ada"));
    let (_, revisions): (_, Vec<Revision>) = ada.get(&paths::revisions(&node)).await?;
    assert_eq!(revisions[0].author.describe().as_deref(), Some("ada"));
    // three general threads of ada's
    let mut numbers = Vec::new();
    for body in ["One", "Two", "Three"] {
        let new = NewThread {
            target: Target::Node,
            body: body.to_string(),
            by: Writer::User,
            review: false,
        };
        let (_, commented): (_, Commented) = ada.post(&paths::node_comments(&node), &new).await?;
        let Commented::Added { thread, .. } = commented else {
            anyhow::bail!("the thread wasn't kept: {commented:?}");
        };
        numbers.push(thread.number);
    }
    // bob resolves the first by hand, as a mark
    let mark = MarkThread {
        state: ThreadState::Resolved,
        read: 1,
    };
    let (_, marked): (_, Marked) = bob
        .put(&paths::thread_state(&plan, numbers[0]), &mark)
        .await?;
    let Marked::Marked { thread } = marked else {
        anyhow::bail!("the thread wasn't marked: {marked:?}");
    };
    let last = thread.last_mark().expect("bob marked it");
    assert_eq!(
        (last.state, last.author.describe().as_deref()),
        (ThreadState::Resolved, Some("bob"))
    );
    // Claude, for bob, resolves the second by hand, linking revision 1
    let resolve = ResolveThread {
        node: node.clone(),
        revision: 1,
        read: 1,
    };
    let (_, resolved): (_, Resolved) = bob
        .as_claude()
        .post(&paths::thread_resolution(&plan, numbers[1]), &resolve)
        .await?;
    let Resolved::Resolved { thread } = resolved else {
        anyhow::bail!("the thread wasn't resolved: {resolved:?}");
    };
    let linked = thread.resolved_by().expect("Claude linked a revision");
    assert_eq!(linked.author.describe().as_deref(), Some("Claude, for bob"));
    // ada's capture whose trailer names the third resolves it as hers
    let commit = ResolvingCommit {
        commit: "c".repeat(40),
        summary: "Answer three".to_string(),
        author: "ada@example.invalid".to_string(),
        authored: 100,
    };
    let mut second = capture(
        &record.start.pinned,
        'f',
        vec![added_file("src/a.rs", "one\ntwo\n")],
    );
    second.resolves = vec![Resolves {
        thread: numbers[2],
        commit,
    }];
    let (_, _): (_, reef_api::Captured) = ada.post(&paths::captures(&node), &second).await?;
    let (_, third): (_, Thread) = ada.get(&paths::thread(&plan, numbers[2])).await?;
    assert_eq!(third.state, ThreadState::Resolved);
    assert_eq!(
        third
            .resolved_by()
            .and_then(|linked| linked.author.describe())
            .as_deref(),
        Some("ada")
    );
    // ada starts a review, and bob submits it
    let pending = NewThread {
        target: Target::Node,
        body: "Later".to_string(),
        by: Writer::User,
        review: true,
    };
    let (_, _): (_, Commented) = ada.post(&paths::node_comments(&node), &pending).await?;
    let (_, review): (_, Option<Review>) = ada.get(&paths::node_review(&node)).await?;
    let review = review.expect("ada's comment opened a review");
    assert_eq!(review.author.describe().as_deref(), Some("ada"));
    assert!(review.marked_by.is_unknown() && review.marked_at.is_none());
    let submit = MarkReview {
        state: ReviewState::Submitted,
        read: review.version,
    };
    let (_, reviewed): (_, Reviewed) = bob.put(&paths::node_review(&node), &submit).await?;
    let Reviewed::Marked { review } = reviewed else {
        anyhow::bail!("the review wasn't submitted: {reviewed:?}");
    };
    assert_eq!(review.author.describe().as_deref(), Some("ada"));
    assert_eq!(review.marked_by.describe().as_deref(), Some("bob"));
    assert!(review.marked_at.is_some());
    Ok(())
}
