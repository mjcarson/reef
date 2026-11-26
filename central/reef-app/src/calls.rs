//! The server functions the app's pages call, each a call on the service
//!
//! They sit under `/app/`, apart from the JSON API's `/api/v1`, whose wire format the CLI and
//! the hooks depend on. Their answers are the service's own types, from `reef-api`.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use reef_api::{Checked, Puts, lift, puts_for};
use reef_api::{
    Commented, Decision, FileDiff, HISTORY_MAX, HistoryPage, Loop, Marked, Me, PasswordChange,
    PasswordChanged, PlanInfo, Project, QueueEdit, QueueListing, Queued, Review, ReviewState,
    Reviewed, Revision, Session, SessionAct, SessionActed, SignedOut, Target, Thread, ThreadState,
    TokenInfo, Transcript, User, UserRemoved,
};
#[cfg(feature = "server")]
use reef_api::{HistoryQuery, MarkReview, MarkThread, NewReply, NewThread, Writer};
use reef_model::Payload;
#[cfg(feature = "server")]
use reef_model::{NodeSpec, PlanName};
use serde::{Deserialize, Serialize};

use crate::side::timeline::NodeTimeline;

/// A project and its plans, as the picker lists them
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPlans {
    /// The project: its name and repo
    pub project: Project,
    /// Its plans, by name
    pub plans: Vec<PlanInfo>,
}

/// Whom the server takes this browser for, and whether it asks callers to sign in
#[get("/app/me")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::me", skip_all, err(Debug))
)]
pub async fn me() -> Result<Me, ServerFnError> {
    // the caller the guard found for this request
    let caller = caller();
    Ok(Me {
        user: caller.user,
        required: crate::server::service().sign_in_required(),
    })
}

/// The caller the guard found for this request, or nobody outside one
#[cfg(feature = "server")]
fn caller() -> reef_service::auth::Caller {
    dioxus::fullstack::FullstackContext::current()
        .and_then(|context| context.extension::<reef_service::auth::Caller>())
        .unwrap_or_default()
}

/// What the account page shows: who this browser is signed in as, their tokens, and every user
/// to an admin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// Whom the server takes this browser for, and whether it asks callers to sign in
    pub me: Me,
    /// Their tokens, oldest first, this browser's marked
    pub tokens: Vec<TokenInfo>,
    /// Every user, if they are an admin, else none
    pub users: Vec<User>,
    /// The newest page of the log of acts on users, if they are an admin, else empty
    #[serde(default)]
    pub log: HistoryPage,
}

/// Who this browser is signed in as, their tokens, and every user and the log of acts on them if
/// they are an admin
#[get("/app/account")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::account", skip_all, err(Debug))
)]
pub async fn account() -> Result<Account, ServerFnError> {
    let caller = caller();
    let service = crate::server::service();
    let me = Me {
        user: caller.user.clone(),
        required: service.sign_in_required(),
    };
    // nobody signed in has no tokens to show
    let Some(user) = &caller.user else {
        return Ok(Account {
            me,
            tokens: Vec::new(),
            users: Vec::new(),
            log: HistoryPage::default(),
        });
    };
    let tokens = service.tokens(caller.asker(), None).await.map_err(failed)?;
    // every user, and the log of acts on them, only to an admin
    let (users, log) = match user.admin {
        true => {
            let users = service.users(caller.asker()).await.map_err(failed)?;
            let log = service
                .user_history(caller.asker(), &HistoryQuery::default())
                .await
                .map_err(failed)?;
            (users, log)
        }
        false => (Vec::new(), HistoryPage::default()),
    };
    Ok(Account {
        me,
        tokens,
        users,
        log,
    })
}

/// Changes a password: this browser's user's own with their current one, or another user's by
/// an admin, which revokes that user's tokens
///
/// # Arguments
///
/// * `name` - The user whose password it is
/// * `change` - The current password, for one's own, and the new one
#[post("/app/account/password")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::change_password", skip_all, err(Debug))
)]
pub async fn change_password(
    name: String,
    change: PasswordChange,
) -> Result<PasswordChanged, ServerFnError> {
    crate::server::service()
        .change_password(caller().asker(), &name, change)
        .await
        .map_err(failed)
}

/// Revokes a token by its id: one of this browser's user's, or anyone's by an admin
///
/// # Arguments
///
/// * `id` - The token's id, as the account page lists it
#[post("/app/account/revoke")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::revoke_token", skip_all, err(Debug))
)]
pub async fn revoke_token(id: String) -> Result<SignedOut, ServerFnError> {
    let revoked = crate::server::service()
        .revoke_token(caller().asker(), &id)
        .await
        .map_err(failed)?;
    Ok(SignedOut { revoked })
}

/// Removes a user and their tokens, which only an admin may, and never the last admin
///
/// # Arguments
///
/// * `name` - The user's name
#[post("/app/account/remove")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::remove_user", skip_all, err(Debug))
)]
pub async fn remove_user(name: String) -> Result<UserRemoved, ServerFnError> {
    crate::server::service()
        .remove_user(caller().asker(), &name)
        .await
        .map_err(failed)
}

/// Whether a call failed because the server asks this browser to sign in, and it isn't
///
/// # Arguments
///
/// * `error` - What the call failed with
pub fn signed_out(error: &ServerFnError) -> bool {
    // the guard answers 401 before the call runs
    matches!(error, ServerFnError::ServerError { code: 401, .. })
}

/// Every registered project, by name, with its plans, each saying who registered or listed it
#[get("/app/projects")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::projects", skip_all, err(Debug))
)]
pub async fn projects() -> Result<Vec<ProjectPlans>, ServerFnError> {
    // each project, then the plans it lists
    let service = crate::server::service();
    let projects = service.projects().await.map_err(failed)?;
    let mut listed = Vec::with_capacity(projects.len());
    for project in projects {
        let plans = service.plan_listing(&project.name).await.map_err(failed)?;
        listed.push(ProjectPlans { project, plans });
    }
    Ok(listed)
}

/// One plan's name and the revision its readers load
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[get("/app/plan?project&plan")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::plan_info", skip_all, err(Debug))
)]
pub async fn plan_info(project: String, plan: String) -> Result<PlanInfo, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    // the plan among its project's, which names an unregistered project
    let plans = crate::server::service()
        .plans(&name.project)
        .await
        .map_err(failed)?;
    plans
        .into_iter()
        .find(|info| info.name == name)
        .ok_or_else(|| ServerFnError::new(format!("`{name}` is not a plan of its project")))
}

/// A plan's payload for the viewer, its doc links absolute paths to the pages the service
/// renders
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[get("/app/payload?project&plan")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::payload", skip_all, err(Debug))
)]
pub async fn payload(project: String, plan: String) -> Result<Payload, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    crate::server::service()
        .payload(&name)
        .await
        .map_err(failed)
}

/// A project's queue: each entry with its rank, and whether next takes it, and if not why
///
/// # Arguments
///
/// * `project` - The project whose queue it is
#[get("/app/queue?project")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::queue", skip_all, err(Debug))
)]
pub async fn queue(project: String) -> Result<QueueListing, ServerFnError> {
    // refused for a project that isn't registered
    crate::server::service()
        .queue(&project)
        .await
        .map_err(failed)
}

/// Edits a project's queue: puts a node in it, moves one, or takes one off, writing its one row
///
/// A node that isn't queued, or isn't in its plan, is an answer, not a failure, and changes
/// nothing.
///
/// # Arguments
///
/// * `project` - The project whose queue it is
/// * `edit` - The edit
#[post("/app/queue")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::edit_queue", skip_all, err(Debug))
)]
pub async fn edit_queue(project: String, edit: QueueEdit) -> Result<Queued, ServerFnError> {
    // under the queue's lock, told to the change stream once it is made
    crate::server::service()
        .edit_queue(&project, edit, &caller().author())
        .await
        .map_err(failed)
}

/// A node's decisions: each plan its sessions asked approval for and each question they asked,
/// and what came of it, each plan rendered
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
#[get("/app/decisions?project&plan&id")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::node_decisions", skip_all, err(Debug))
)]
pub async fn node_decisions(
    project: String,
    plan: String,
    id: String,
) -> Result<Vec<Decision>, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    crate::server::service()
        .node_decisions(&node)
        .await
        .map_err(failed)
}

/// The most edits of a node the app reads at once, however far back it is asked to show
pub const HISTORY_SHOWN_MAX: usize = 50 * HISTORY_MAX;

/// A node's history: who made each edit of it, what it changed and when, newest first, as many
/// pages as hold a number of edits, with the cursor onto the older ones if any are kept
///
/// Each read starts from the newest, so a read again after an edit shows what was shown before
/// and the edit too, with nothing between them left out.
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `count` - How many edits to read, a page at the least and [`HISTORY_SHOWN_MAX`] at most
#[get("/app/history?project&plan&id&count")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::node_history", skip_all, err(Debug))
)]
pub async fn node_history(
    project: String,
    plan: String,
    id: String,
    count: usize,
) -> Result<HistoryPage, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    history_pages(&node, count).await.map_err(failed)
}

/// Reads a node's history from its newest edit, a page at a time, until it holds a number of
/// edits or none is left
///
/// # Arguments
///
/// * `node` - The node's full name
/// * `count` - How many edits to read, a page at the least and [`HISTORY_SHOWN_MAX`] at most
#[cfg(feature = "server")]
#[tracing::instrument(name = "calls::history_pages", skip_all, err(Debug))]
async fn history_pages(node: &reef_model::NodeName, count: usize) -> anyhow::Result<HistoryPage> {
    let service = crate::server::service();
    // a page at the least, and never more than the most the app reads
    let count = count.clamp(HISTORY_MAX, HISTORY_SHOWN_MAX);
    let mut page = service.node_history(node, &HistoryQuery::default()).await?;
    // then each older page in turn, until it holds as many as asked or none is left
    while page.edits.len() < count
        && let Some(older) = page.older.take()
    {
        let next = service
            .node_history(node, &HistoryQuery::older(older))
            .await?;
        page.edits.extend(next.edits);
        page.older = next.older;
    }
    Ok(page)
}

/// A node's timeline: its edits, starts, captures and comments, newest first, its edits read
/// as the History tab reads them, and whether older edits are kept
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `count` - How many of its edits to read, a page at the least and [`HISTORY_SHOWN_MAX`] at
///   most
#[get("/app/timeline?project&plan&id&count")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::node_timeline", skip_all, err(Debug))
)]
pub async fn node_timeline(
    project: String,
    plan: String,
    id: String,
    count: usize,
) -> Result<NodeTimeline, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    let service = crate::server::service();
    // its edits and starts, its revisions, its published threads, and its latest start
    let page = history_pages(&node, count).await.map_err(failed)?;
    let revisions = service.revisions(&node).await.map_err(failed)?;
    let threads = service
        .node_threads(&node, reef_service::Pending::Leave)
        .await
        .map_err(failed)?;
    let work = service.work_of(&node).await.map_err(failed)?;
    // merged, newest first
    Ok(crate::side::timeline::merge(page, work, revisions, threads))
}

/// A project's runners' sessions, and every loop feeding it, a terminal's or a runner's
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Runners {
    /// The sessions of runners taking nodes from it, each gone if its runner went silent
    pub sessions: Vec<Session>,
    /// The loops feeding it, by pane
    pub loops: Vec<Loop>,
}

/// A project's runners' sessions, and the loops feeding it
///
/// # Arguments
///
/// * `project` - The project
#[get("/app/sessions?project")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::runners", skip_all, err(Debug))
)]
pub async fn runners(project: String) -> Result<Runners, ServerFnError> {
    // refused for a project that isn't registered
    let service = crate::server::service();
    let sessions = service.project_sessions(&project).await.map_err(failed)?;
    let loops = service.project_loops(&project).await.map_err(failed)?;
    Ok(Runners { sessions, loops })
}

/// A runner's session's transcript after the entries read already
///
/// # Arguments
///
/// * `id` - The session's id
/// * `after` - How many of its entries were read already
#[get("/app/session?id&after")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::session_transcript", skip_all, err(Debug))
)]
pub async fn session_transcript(id: String, after: u64) -> Result<Transcript, ServerFnError> {
    // as far back as the service keeps
    crate::server::service()
        .session_transcript(&id, after)
        .await
        .map_err(failed)
}

/// Asks something of a runner's session: starts, pauses or stops its loop, answers one of its
/// prompts, or replies to Claude
///
/// A prompt that no longer waits, a Claude that isn't waiting, or a runner that is gone is an
/// answer, not a failure, and sends nothing.
///
/// # Arguments
///
/// * `id` - The session's id
/// * `act` - What is asked of it
#[post("/app/session")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::act_on_session", skip_all, err(Debug))
)]
pub async fn act_on_session(id: String, act: SessionAct) -> Result<SessionActed, ServerFnError> {
    // a loop changed as its pane's would be, or an order queued for the runner, in the name of
    // whoever the browser signs in
    crate::server::service()
        .act_on_session(&id, act, &caller().author())
        .await
        .map_err(failed)
}

/// A node's captured revisions, oldest first, each listing its files without their hunks
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
#[get("/app/revisions?project&plan&id")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::revisions", skip_all, err(Debug))
)]
pub async fn revisions(
    project: String,
    plan: String,
    id: String,
) -> Result<Vec<Revision>, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    crate::server::service()
        .revisions(&node)
        .await
        .map_err(failed)
}

/// One file of a node's revision, its hunks and all, fetched when it is opened, or `None` if the
/// revision doesn't list it
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `revision` - The revision's number
/// * `path` - The path the file is shown under
#[get("/app/revision-file?project&plan&id&revision&path")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::revision_file", skip_all, err(Debug))
)]
pub async fn revision_file(
    project: String,
    plan: String,
    id: String,
    revision: u32,
    path: String,
) -> Result<Option<FileDiff>, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    crate::server::service()
        .revision_file(&node, revision, &path)
        .await
        .map_err(failed)
}

/// A plan's comment threads, on every node of it, by number, each comment rendered, those
/// pending in a review included, as their reviewer sees them
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[get("/app/threads?project&plan")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::plan_threads", skip_all, err(Debug))
)]
pub async fn plan_threads(project: String, plan: String) -> Result<Vec<Thread>, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    crate::server::service()
        .threads(&name, reef_service::Pending::Include)
        .await
        .map_err(failed)
}

/// A plan's open reviews, by number
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[get("/app/reviews?project&plan")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::plan_reviews", skip_all, err(Debug))
)]
pub async fn plan_reviews(project: String, plan: String) -> Result<Vec<Review>, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    crate::server::service()
        .open_reviews(&name)
        .await
        .map_err(failed)
}

/// Submits a node's open review, publishing its comments at once, or discards it, over the
/// version it was read at
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `state` - Whether it is submitted or discarded
/// * `read` - The version of the review the mark was made against
#[post("/app/review")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::mark_review", skip_all, err(Debug))
)]
pub async fn mark_review(
    project: String,
    plan: String,
    id: String,
    state: ReviewState,
    read: u64,
) -> Result<Reviewed, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    crate::server::service()
        .mark_review(&node, MarkReview { state, read }, &caller().author())
        .await
        .map_err(failed)
}

/// Starts a thread on a node, or on a line of one of its revisions
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `target` - What the thread is on
/// * `body` - Its first comment's body, as markdown
/// * `review` - Whether it is written in the node's review, started for it if none is open
#[post("/app/thread")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::start_thread", skip_all, err(Debug))
)]
pub async fn start_thread(
    project: String,
    plan: String,
    id: String,
    target: Target,
    body: String,
    review: bool,
) -> Result<Commented, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let node = plan_name(project, plan)?
        .node(id)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    crate::server::service()
        .start_thread(
            &node,
            NewThread {
                target,
                body,
                by: Writer::User,
                review,
            },
            &caller().author(),
        )
        .await
        .map_err(failed)
}

/// Replies in a thread of a plan
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `thread` - The thread's number
/// * `body` - The reply's body, as markdown
/// * `review` - Whether it is written in the node's review, started for it if none is open
#[post("/app/reply")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::reply_to", skip_all, err(Debug))
)]
pub async fn reply_to(
    project: String,
    plan: String,
    thread: u32,
    body: String,
    review: bool,
) -> Result<Commented, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    crate::server::service()
        .reply(
            &name,
            thread,
            NewReply {
                body,
                by: Writer::User,
                review,
            },
            &caller().author(),
        )
        .await
        .map_err(failed)
}

/// Marks a thread open or resolved, over the version it was read at
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `thread` - The thread's number
/// * `state` - The state to mark it
/// * `read` - The version of the thread the mark was made against
#[post("/app/mark")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::mark_thread", skip_all, err(Debug))
)]
pub async fn mark_thread(
    project: String,
    plan: String,
    thread: u32,
    state: ThreadState,
    read: u64,
) -> Result<Marked, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    crate::server::service()
        .mark_thread(
            &name,
            thread,
            MarkThread { state, read },
            &caller().author(),
        )
        .await
        .map_err(failed)
}

/// A node as the editor shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeText {
    /// The node's id
    pub id: String,
    /// Its spec, as frontmatter
    pub yaml: String,
    /// The version it was read at, which a save names
    pub version: u64,
    /// Each problem the plan has at this node, as check says it
    pub problems: Vec<String>,
}

/// What came of saving a node's YAML
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Saved {
    /// Written, at this new version
    Applied {
        /// The node's version now
        version: u64,
    },
    /// The node changed after the editor read it, so nothing was written
    Stale {
        /// The node as it is now, or `None` if it was removed
        now: Option<NodeText>,
    },
    /// The plan would have problems it hadn't, so nothing was written
    Refused {
        /// Each problem it would add, as check says it
        problems: Vec<String>,
    },
    /// The YAML isn't this node's spec, so nothing was written
    Invalid {
        /// What is wrong with it
        message: String,
    },
}

/// A node's YAML, at the version it is at, with the problems the plan has at it
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
#[get("/app/node?project&plan&id")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::node_text", skip_all, err(Debug))
)]
pub async fn node_text(
    project: String,
    plan: String,
    id: String,
) -> Result<NodeText, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    text_of(&name, &id)
        .await?
        .ok_or_else(|| ServerFnError::new(format!("`{name}/{id}` is not in its plan")))
}

/// Saves a node's YAML over the version the editor read, unless the node moved on since or the
/// plan would have a problem it hadn't
///
/// The YAML goes in as `reef node put` puts it: the node keeps its place and body, and members
/// listed inline go in as new nodes with it, all checked as one change.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node being edited, which the YAML must keep
/// * `yaml` - The node's spec, as frontmatter
/// * `read` - The version the editor read
#[post("/app/save")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "calls::save_node", skip_all, err(Debug))
)]
pub async fn save_node(
    project: String,
    plan: String,
    id: String,
    yaml: String,
    read: u64,
) -> Result<Saved, ServerFnError> {
    // a name that breaks the naming rule is refused before the store is read
    let name = plan_name(project, plan)?;
    // the YAML must be this node's spec
    let spec = match NodeSpec::from_yaml(&yaml) {
        Ok(spec) => spec,
        Err(error) => {
            return Ok(Saved::Invalid {
                message: error.message,
            });
        }
    };
    if spec.id != id {
        return Ok(Saved::Invalid {
            message: format!(
                "the YAML is of `{}`, not `{id}`: a node's id can't change here",
                spec.id
            ),
        });
    }
    // the node at the version read, its members lifted out, placed as the CLI places them
    let service = crate::server::service();
    let listing = service.list(&name).await.map_err(failed)?;
    let puts = puts_for(&listing, lift(spec), Some(read));
    // checked against the whole plan as one change
    match service
        .batch(&name, Puts::new(puts), &caller().author())
        .await
        .map_err(failed)?
    {
        Checked::Applied { versions } => Ok(Saved::Applied {
            version: versions.first().copied().unwrap_or(read),
        }),
        Checked::Stale(_) => Ok(Saved::Stale {
            now: text_of(&name, &id).await?,
        }),
        Checked::Refused(problems) => Ok(Saved::Refused {
            problems: problems.iter().map(ToString::to_string).collect(),
        }),
    }
}

/// A node as the editor shows it, if its plan has it
///
/// # Arguments
///
/// * `name` - The plan
/// * `id` - The node's id
#[cfg(feature = "server")]
async fn text_of(name: &PlanName, id: &str) -> Result<Option<NodeText>, ServerFnError> {
    // the node as stored, if it is
    let service = crate::server::service();
    let Some(stored) = service.get(name, id).await.map_err(failed)? else {
        return Ok(None);
    };
    // the problems check finds at this node, which a stored node's are named for
    let at = format!("{name}/{id}");
    let problems = service
        .check(name)
        .await
        .map_err(failed)?
        .into_iter()
        .filter(|problem| problem.source == at)
        .map(|problem| problem.message)
        .collect();
    Ok(Some(NodeText {
        id: id.to_string(),
        yaml: stored.spec.to_yaml(),
        version: stored.version,
        problems,
    }))
}

/// A plan's full name from the route's two parts
///
/// # Arguments
///
/// * `project` - The project's name
/// * `plan` - The plan's name within it
#[cfg(feature = "server")]
fn plan_name(project: String, plan: String) -> Result<PlanName, ServerFnError> {
    PlanName::new(project, plan).map_err(|error| ServerFnError::new(error.to_string()))
}

/// A service's error as a server function's, with its causes
///
/// # Arguments
///
/// * `error` - What went wrong
#[cfg(feature = "server")]
fn failed(error: anyhow::Error) -> ServerFnError {
    ServerFnError::new(format!("{error:#}"))
}
