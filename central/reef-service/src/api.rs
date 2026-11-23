//! reef's JSON API: a plain axum router over a service
//!
//! The CLI, the MCP server and the hooks call these routes, and reef-app mounts the same router
//! beside its server functions, so one port serves both and no client depends on the app's wire
//! format. Every route is a thin call to one [`Service`] operation, its body and its answer
//! `reef-api`'s types, and its path one of [`reef_api::paths`]:
//!
//! | Route | Service | Answer |
//! |---|---|---|
//! | `GET /api/v1/projects` | `projects` | every [`Project`] |
//! | `POST /api/v1/projects` | `add_project` | [`AddedProject`]: 201, or 409 if taken |
//! | `GET /api/v1/find-project?dir=` | `find_project` | the [`Project`], or 404 |
//! | `GET …/projects/{project}/plans` | `plans` | every [`PlanInfo`] |
//! | `POST …/projects/{project}/plans` | `add_plan` | [`AddedPlan`]: 201, or 200 if listed |
//! | `GET …/projects/{project}/queue` | `queue` | its [`QueueListing`] |
//! | `POST …/projects/{project}/queue` | `edit_queue` | [`Queued`]: 200, 409 if not queued, 422 if not in its plan |
//! | `GET …/projects/{project}/next` | `project_next` | its plans' ready work, the queue's first: [`Next`] |
//! | `GET …/projects/{project}/work` | `work` | every [`WorkRecord`] of its nodes, by node |
//! | `GET …/plans/{plan}` | `plans` | its [`PlanInfo`], or 404 |
//! | `GET …/plans/{plan}/nodes` | `list` | its [`Listing`] |
//! | `POST …/plans/{plan}/nodes` | `put` | [`Checked`]: 200, 409 if stale, 422 if refused |
//! | `GET …/plans/{plan}/nodes/{id}` | `get` | the [`StoredNode`], or 404 |
//! | `PUT …/plans/{plan}/revision` | `write_plan` | [`Rewrite`]: 200, or 422 if refused |
//! | `GET …/plans/{plan}/payload` | `payload` | the viewer's [`Payload`] |
//! | `GET …/plans/{plan}/check` | `check` | its problems |
//! | `GET …/plans/{plan}/next` | `next` | its ready work, its queue's first, and what the rest waits on: [`Next`] |
//! | `GET …/plans/{plan}/docs/{*path}` | `doc_page` | a doc rendered as its page, or 404 |
//! | `GET …/plans/{plan}/pages` | `pages` | its own pages: [`PageListing`] |
//! | `POST …/plans/{plan}/pages` | `edit_page` | [`PageEdited`]: 200, 409 if stale, 422 if refused |
//! | `GET …/plans/{plan}/pages/{*path}` | `page` | the [`StoredPage`], or 404 |
//! | `GET …/plans/{plan}/work/{id}` | `work_of` | the node's [`WorkRecord`], or 404 |
//! | `POST …/plans/{plan}/work/{id}` | `start_work` | [`Started`]: 200, 409 if stale, 422 if not in its plan |
//! | `POST …/plans/{plan}/work/{id}/captures` | `add_capture` | [`Captured`]: 200 if kept or unchanged, its trailers' threads resolved, 409 if started again since, 422 if not started |
//! | `GET …/plans/{plan}/work/{id}/revisions` | `revisions` | every [`Revision`] of the node, oldest first |
//! | `GET …/plans/{plan}/work/{id}/revisions/{revision}` | `node_revision` | the [`Revision`], or 404 |
//! | `GET …/plans/{plan}/work/{id}/revisions/{revision}/files/{*path}` | `revision_file` | the [`FileDiff`] the revision lists, or 404 |
//! | `GET …/plans/{plan}/comments?pending=true` | `threads` | every [`Thread`] of the plan, by number, those pending in a review only if asked for |
//! | `GET …/plans/{plan}/nodes/{id}/comments?pending=true` | `node_threads` | every [`Thread`] on the node, by number, those pending in a review only if asked for |
//! | `POST …/plans/{plan}/nodes/{id}/comments` | `start_thread` | [`Commented`]: 200, 422 if not in its plan or on no line of its revisions |
//! | `GET …/plans/{plan}/nodes/{id}/review` | `node_review` | the node's open [`Review`], or `null` |
//! | `PUT …/plans/{plan}/nodes/{id}/review` | `mark_review` | [`Reviewed`]: 200, 409 if stale, 422 if the node has no open review |
//! | `GET …/plans/{plan}/comments/{thread}?pending=true` | `thread` | the [`Thread`], or 404, as a pending one is unless asked for |
//! | `POST …/plans/{plan}/comments/{thread}/replies` | `reply` | [`Commented`]: 200, or 422 if no thread starts there |
//! | `PUT …/plans/{plan}/comments/{thread}/state` | `mark_thread` | [`Marked`]: 200, 409 if stale, 422 if no thread starts there or it is pending in a review |
//! | `POST …/plans/{plan}/comments/{thread}/resolution` | `resolve_thread` | [`Resolved`]: 200, 409 if stale, 422 if no thread starts there, it is pending in a review, or the revision isn't stored |
//! | `GET /api/v1/loops` | `loops` | every [`Loop`], by pane |
//! | `POST /api/v1/loops` | `edit_loop` | [`LoopEdited`]: 200 if written or joined, 409 if the pane's loop is busy or another loop or pane holds the node, 422 if the pane has no loop, the node isn't the loop's, its plan's or any loop's member, or the group is wider than the loop |
//! | `GET /api/v1/projects/{project}/loops` | `project_loops` | every [`Loop`] feeding the project, by pane |
//! | `GET /api/v1/changes?project=&plan=` | `changes` | server-sent [`Change`]s |
//!
//! - **A refusal is an answer.** A stale or refused write comes back as its outcome, with a
//!   status that says which, so a client reads one type whatever the status.
//! - **An error names its kind.** A project that isn't registered, a plan that isn't listed, a
//!   name that breaks the rule, a body that can't be read: each is an [`ApiError`] with a code
//!   and check's or the service's own message.
//! - **A doc is a page.** The payload's doc links lead under a plan's `docs/`, where each doc is
//!   served as the HTML page mdbook would write for it, so the viewer's *Read here* works as it
//!   does in a book. A page opened on its own runs no script a doc holds. A plan's own pages,
//!   read and edited as markdown under its `pages/`, are served there too, over its repo's.
//! - **A whole plan, a page or a capture is a big body.** Those three routes take up to
//!   [`WRITE_BODY_MAX`] bytes, since a plan's pages, or a node's diff, run past axum's default
//!   of 2 MiB.
//! - **The change stream is server-sent events,** one a change, its event named by its kind and
//!   its data the change as JSON. It is live only, and narrowed by `project` and `plan`.
//! - **Localhost only.** [`bind`] refuses any address but a loopback one until users sign in
//!   (M10).

use axum::extract::{
    DefaultBodyLimit, FromRequest, FromRequestParts, Path as Segment, Query, Request, State,
};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use reef_api::{
    AddCapture, AddedPlan, AddedProject, ApiError, Attach, Captured, Change, Checked, Commented,
    Decision, DecisionsRecorded, EditLoop, ErrorCode, FileDiff, Listing, Loop, LoopEdited,
    MarkReview, MarkThread, Marked, NewPlan, NewProject, NewReply, NewThread, Ordered, PageEdit,
    PageEdited, PageListing, PlanInfo, PlanWrite, Project, Puts, QueueEdit, QueueListing, Queued,
    RecordDecisions, Report, ResolveThread, Resolved, Review, Reviewed, Revision, Rewrite, Session,
    SessionAct, SessionActed, StartWork, Started, StoredNode, StoredPage, Thread, Transcript,
    WRITE_BODY_MAX, WorkRecord, paths,
};
use reef_model::{Diagnostic, NameError, Next, NodeName, Payload, PlanName};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::convert::Infallible;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::{Stream, StreamExt};
use tracing::instrument;

use crate::backend::Backend;
use crate::conf::Conf;
use crate::service::{NoSession, Service};
use crate::store::{NotListed, NotRegistered, Pending, Unkeepable};

/// A handle on a service that a router holds and clones into every request
///
/// The binary and the tests hold an `Arc<Service>`, and reef-app the `&'static Service` that
/// [`Service::shared`] opens once however often `dioxus::serve` reruns its setup.
pub trait Serves: Clone + Send + Sync + 'static {
    /// The backend under the service
    type Backend: Backend + 'static;

    /// The service this handle reaches
    fn service(&self) -> &Service<Self::Backend>;
}

impl<B: Backend + 'static> Serves for Arc<Service<B>> {
    /// The backend under the service
    type Backend = B;

    /// The service this handle shares
    fn service(&self) -> &Service<B> {
        self
    }
}

impl<B: Backend + 'static> Serves for &'static Service<B> {
    /// The backend under the service
    type Backend = B;

    /// The service this handle borrows for the life of the process
    fn service(&self) -> &Service<B> {
        self
    }
}

/// Builds the JSON API's router over a service
///
/// # Arguments
///
/// * `serves` - The service every route calls
///
/// # Examples
///
/// ```
/// use reef_service::{Service, api};
/// use std::sync::Arc;
///
/// let router = api::router(Arc::new(Service::memory()));
/// ```
pub fn router<S: Serves>(serves: S) -> Router {
    Router::new()
        .route(paths::PROJECTS, get(projects::<S>).post(add_project::<S>))
        .route(paths::FIND_PROJECT, get(find_project::<S>))
        .route(paths::PLANS, get(plans::<S>).post(add_plan::<S>))
        .route(paths::QUEUE, get(queue::<S>).post(edit_queue::<S>))
        .route(paths::PROJECT_NEXT, get(project_next::<S>))
        .route(paths::WORK, get(work::<S>))
        .route(paths::PLAN, get(plan::<S>))
        .route(paths::NODES, get(list::<S>).post(put::<S>))
        .route(paths::NODE, get(node::<S>))
        .route(
            paths::REVISION,
            axum::routing::put(write_plan::<S>).layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(paths::PAYLOAD, get(payload::<S>))
        .route(paths::CHECK, get(check::<S>))
        .route(paths::NEXT, get(next::<S>))
        .route(paths::DOC, get(doc::<S>))
        .route(
            paths::PAGES,
            get(pages::<S>)
                .post(edit_page::<S>)
                .layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(paths::PAGE, get(page::<S>))
        .route(paths::NODE_WORK, get(work_of::<S>).post(start_work::<S>))
        .route(
            paths::CAPTURES,
            axum::routing::post(add_capture::<S>).layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(paths::REVISIONS, get(revisions::<S>))
        .route(paths::NODE_REVISION, get(node_revision::<S>))
        .route(paths::REVISION_FILE, get(revision_file::<S>))
        .route(paths::PLAN_COMMENTS, get(threads::<S>))
        .route(
            paths::NODE_COMMENTS,
            get(node_threads::<S>).post(start_thread::<S>),
        )
        .route(
            paths::NODE_REVIEW,
            get(node_review::<S>).put(mark_review::<S>),
        )
        .route(
            paths::NODE_DECISIONS,
            get(node_decisions::<S>)
                .post(record_decisions::<S>)
                .layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(paths::THREAD, get(thread::<S>))
        .route(paths::REPLIES, axum::routing::post(reply::<S>))
        .route(paths::THREAD_STATE, axum::routing::put(mark_thread::<S>))
        .route(
            paths::THREAD_RESOLUTION,
            axum::routing::post(resolve_thread::<S>),
        )
        .route(paths::LOOPS, get(loops::<S>).post(edit_loop::<S>))
        .route(paths::PROJECT_LOOPS, get(project_loops::<S>))
        .route(
            paths::SESSIONS,
            axum::routing::post(attach_session::<S>).layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(
            paths::SESSION,
            get(session_transcript::<S>)
                .post(report_session::<S>)
                .delete(detach_session::<S>)
                .layer(DefaultBodyLimit::max(WRITE_BODY_MAX)),
        )
        .route(
            paths::SESSION_ORDERS,
            get(session_orders::<S>).post(act_on_session::<S>),
        )
        .route(paths::PROJECT_SESSIONS, get(project_sessions::<S>))
        .route(paths::CHANGES, get(changes::<S>))
        .with_state(serves)
}

/// Listens where reef's config says the API is served, refusing any but a loopback address
///
/// Nobody signs in until M10, so nothing but this host may reach the API.
///
/// # Arguments
///
/// * `conf` - reef's config, whose `api.address` names where to listen
#[instrument(name = "api::bind", skip_all, err(Debug))]
pub async fn bind(conf: &Conf) -> anyhow::Result<TcpListener> {
    let address = conf.api.address;
    // only this host, until users sign in
    anyhow::ensure!(
        address.ip().is_loopback(),
        "reef's JSON API serves localhost only until users sign in, and {address} is not a \
         loopback address: set {} to 127.0.0.1 or [::1]",
        conf.key("api.address")
    );
    // a port another program holds is named, with the key that chose it
    TcpListener::bind(address).await.map_err(|error| {
        anyhow::anyhow!(
            "listening on {address}, set by {}: {error}",
            conf.key("api.address")
        )
    })
}

/// Serves the API on a listener until told to stop, then waits for the requests still open
///
/// Stopping ends the service's change stream first, since a follower's stream never ends of
/// its own accord and a graceful stop waits for every open request.
///
/// # Arguments
///
/// * `listener` - Where to serve, from [`bind`]
/// * `serves` - The service every route calls
/// * `stop` - Resolves when the server should stop
#[instrument(name = "api::serve", skip_all, err(Debug))]
pub async fn serve<S: Serves>(
    listener: TcpListener,
    serves: S,
    stop: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let router = router(serves.clone());
    // once told to stop, end the change stream so its followers' requests finish
    let stopping = async move {
        stop.await;
        serves.service().end_changes();
    };
    axum::serve(listener, router)
        .with_graceful_shutdown(stopping)
        .await?;
    Ok(())
}

/// An error a route answers with: its code's status, and the error as the body
#[derive(Debug)]
pub struct Failure(pub ApiError);

impl Failure {
    /// A failure of a kind, saying what went wrong
    ///
    /// # Arguments
    ///
    /// * `code` - What kind of error it is
    /// * `message` - What went wrong
    fn new<T: Into<String>>(code: ErrorCode, message: T) -> Failure {
        Failure(ApiError::new(code, message))
    }
}

impl From<anyhow::Error> for Failure {
    /// Names the kind of a service's error by the error it carries
    ///
    /// # Arguments
    ///
    /// * `error` - The error the service returned
    fn from(error: anyhow::Error) -> Failure {
        // the errors a client caused, or asked about, by their types; anything else failed here
        let code = if error.downcast_ref::<NotRegistered>().is_some() {
            ErrorCode::NotRegistered
        } else if error.downcast_ref::<NotListed>().is_some() {
            ErrorCode::NotListed
        } else if error.downcast_ref::<NoSession>().is_some() {
            ErrorCode::NotFound
        } else if error.downcast_ref::<NameError>().is_some() {
            ErrorCode::BadName
        } else if error.downcast_ref::<Unkeepable>().is_some() {
            ErrorCode::BadRequest
        } else {
            ErrorCode::Internal
        };
        // the whole chain, so a failure says what it failed at
        Failure::new(code, format!("{error:#}"))
    }
}

impl From<NameError> for Failure {
    /// A name that breaks the rule names keep to
    ///
    /// # Arguments
    ///
    /// * `error` - What was refused
    fn from(error: NameError) -> Failure {
        Failure::new(ErrorCode::BadName, error.to_string())
    }
}

impl IntoResponse for Failure {
    /// Answers with the error's status and the error as JSON
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self.0)).into_response()
    }
}

/// A request's JSON body, refused as a `bad-request` error when it can't be read
struct Body<T>(T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for Body<T> {
    /// A body that isn't JSON, or isn't the JSON the route takes
    type Rejection = Failure;

    /// Reads the body as JSON
    ///
    /// # Arguments
    ///
    /// * `request` - The request
    /// * `state` - The router's state
    async fn from_request(request: Request, state: &S) -> Result<Body<T>, Failure> {
        // axum's own reading, with its rejection said as an API error
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| Failure::new(ErrorCode::BadRequest, rejection.body_text()))?;
        Ok(Body(value))
    }
}

/// A route's path segments, percent-decoded, refused as a `bad-request` error when they can't be
struct Segments<T>(T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for Segments<T> {
    /// Segments that can't be decoded
    type Rejection = Failure;

    /// Reads the path's segments into their fields
    ///
    /// # Arguments
    ///
    /// * `parts` - The request's head
    /// * `state` - The router's state
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Segments<T>, Failure> {
        // axum's own reading, with its rejection said as an API error
        let Segment(value) = Segment::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| Failure::new(ErrorCode::BadRequest, rejection.body_text()))?;
        Ok(Segments(value))
    }
}

/// A request's query, refused as a `bad-request` error when it can't be read
struct Params<T>(T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for Params<T> {
    /// A query that can't be read into the route's fields
    type Rejection = Failure;

    /// Reads the query into its fields
    ///
    /// # Arguments
    ///
    /// * `parts` - The request's head
    /// * `state` - The router's state
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Params<T>, Failure> {
        // axum's own reading, with its rejection said as an API error
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| Failure::new(ErrorCode::BadRequest, rejection.body_text()))?;
        Ok(Params(value))
    }
}

/// The segments of a route under one project
#[derive(Deserialize)]
struct ProjectSegments {
    /// The project's name
    project: String,
}

/// The segments of a route under one plan
#[derive(Deserialize)]
struct PlanSegments {
    /// The project the plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
}

impl PlanSegments {
    /// The plan these segments name, refusing a name that breaks the rule
    fn name(self) -> Result<PlanName, Failure> {
        Ok(PlanName::new(self.project, self.plan)?)
    }
}

/// The segments of a route to one node
#[derive(Deserialize)]
struct NodeSegments {
    /// The project the node's plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The node's id
    id: String,
}

impl NodeSegments {
    /// The node these segments name, refusing a name that breaks the rule
    fn name(self) -> Result<NodeName, Failure> {
        Ok(PlanName::new(self.project, self.plan)?.node(self.id)?)
    }
}

/// The segments of a route to one of a node's revisions
#[derive(Deserialize)]
struct RevisionSegments {
    /// The project the node's plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The node's id
    id: String,
    /// The revision's number
    revision: u32,
}

impl RevisionSegments {
    /// The node these segments name, refusing a name that breaks the rule
    fn name(&self) -> Result<NodeName, Failure> {
        let plan = PlanName::new(self.project.clone(), self.plan.clone())?;
        Ok(plan.node(self.id.clone())?)
    }
}

/// The segments of a route to one file of a node's revision
#[derive(Deserialize)]
struct RevisionFileSegments {
    /// The project the node's plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The node's id
    id: String,
    /// The revision's number
    revision: u32,
    /// The path the file is shown under
    path: String,
}

/// The segments of a route to one of a plan's comment threads
#[derive(Deserialize)]
struct ThreadSegments {
    /// The project the plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The thread's number
    thread: u32,
}

impl ThreadSegments {
    /// The plan these segments name, refusing a name that breaks the rule
    fn name(&self) -> Result<PlanName, Failure> {
        Ok(PlanName::new(self.project.clone(), self.plan.clone())?)
    }
}

/// The segments of a route to one of a plan's docs
#[derive(Deserialize)]
struct DocSegments {
    /// The project the plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The doc's page, from the root of the plan's docs
    path: String,
}

/// The segments of a route to one of a plan's own pages
#[derive(Deserialize)]
struct PageSegments {
    /// The project the plan belongs to
    project: String,
    /// The plan's name within its project
    plan: String,
    /// The page's path
    path: String,
}

/// The query of the route that finds a directory's project
#[derive(Deserialize)]
struct FindQuery {
    /// The directory, absolute, as the service's host sees it
    dir: PathBuf,
}

/// The segments of a route under one runner's session
#[derive(Deserialize)]
struct SessionSegments {
    /// The session's id
    session: String,
}

/// The query a session's transcript takes
#[derive(Deserialize)]
struct AfterQuery {
    /// How many of its entries were read already
    #[serde(default)]
    after: u64,
}

/// The query a runner's ask for its orders takes
#[derive(Deserialize)]
struct OrdersQuery {
    /// The number of the last order it read
    #[serde(default)]
    after: u64,
    /// How long to wait for an order if none is there, in milliseconds
    #[serde(default)]
    wait: u64,
}

/// The query a thread's routes take, to serve threads and comments pending in a review too
#[derive(Deserialize)]
struct PendingQuery {
    /// Whether to serve what is pending, which is left out unless asked for
    #[serde(default)]
    pending: bool,
}

impl PendingQuery {
    /// What the store is asked for
    fn pending(&self) -> Pending {
        match self.pending {
            true => Pending::Include,
            false => Pending::Leave,
        }
    }
}

/// The query that narrows the change stream
#[derive(Deserialize)]
struct ChangesQuery {
    /// Only this project's changes
    project: Option<String>,
    /// Only this plan's, within that project
    plan: Option<String>,
}

impl ChangesQuery {
    /// Whether a follower that asked for this should be sent a change
    ///
    /// # Arguments
    ///
    /// * `change` - The change
    fn follows(&self, change: &Change) -> bool {
        // a gap in the stream concerns every follower
        let Some(project) = change.project() else {
            return true;
        };
        // a change to another project is not followed
        if self
            .project
            .as_deref()
            .is_some_and(|wanted| wanted != project)
        {
            return false;
        }
        match (&self.plan, change.plan()) {
            // the whole project is followed
            (None, _) => true,
            // or one plan of it
            (Some(wanted), Some(plan)) => &plan.plan == wanted,
            // and a project's own change is no plan's
            (Some(_), None) => false,
        }
    }
}

/// Refuses a path a client sent that isn't an absolute directory on this host
///
/// The service reads repos as its host sees them, so a relative path would be read against
/// wherever the service was started.
///
/// # Arguments
///
/// * `path` - The path the client sent
/// * `field` - The field it was sent in, for the message
async fn directory(path: &Path, field: &str) -> Result<(), Failure> {
    // a relative path names nothing a client can mean
    if !path.is_absolute() {
        return Err(Failure::new(
            ErrorCode::BadRequest,
            format!("`{field}` must be absolute, and {} is not", path.display()),
        ));
    }
    // and it must be a directory here
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_dir() => Ok(()),
        _ => Err(Failure::new(
            ErrorCode::BadRequest,
            format!(
                "`{field}` {} is not a directory on the service's host",
                path.display()
            ),
        )),
    }
}

/// Lists every project, by name
///
/// # Arguments
///
/// * `serves` - The service
#[instrument(name = "api::projects", skip_all, err(Debug))]
async fn projects<S: Serves>(State(serves): State<S>) -> Result<Json<Vec<Project>>, Failure> {
    // every project, by name
    let projects = serves.service().projects().await?;
    Ok(Json(projects))
}

/// Registers a project for a repo: 201 if it was, 409 if its name or repo is taken
///
/// # Arguments
///
/// * `serves` - The service
/// * `new` - The project's name and a directory in its repo
#[instrument(name = "api::add_project", skip_all, err(Debug))]
async fn add_project<S: Serves>(
    State(serves): State<S>,
    Body(new): Body<NewProject>,
) -> Result<(StatusCode, Json<AddedProject>), Failure> {
    // the repo is a directory on this host
    directory(&new.repo, "repo").await?;
    // register it, unless its name or its repo is another project's
    let added = serves.service().add_project(&new.name, &new.repo).await?;
    let status = match added {
        AddedProject::Added(_) => StatusCode::CREATED,
        AddedProject::NameTaken | AddedProject::RepoTaken { .. } => StatusCode::CONFLICT,
    };
    Ok((status, Json(added)))
}

/// Finds the project a directory belongs to, a linked worktree's main repo's for one in it
///
/// # Arguments
///
/// * `serves` - The service
/// * `query` - The directory
#[instrument(name = "api::find_project", skip_all, err(Debug))]
async fn find_project<S: Serves>(
    State(serves): State<S>,
    Params(query): Params<FindQuery>,
) -> Result<Json<Project>, Failure> {
    // the directory is one on this host
    directory(&query.dir, "dir").await?;
    // the project registered for its repo, or for its nearest registered ancestor outside git
    match serves.service().find_project(&query.dir).await? {
        Some(project) => Ok(Json(project)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("no project is registered for {}", query.dir.display()),
        )),
    }
}

/// Lists a project's plans, by name
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::plans", skip_all, err(Debug))]
async fn plans<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<Vec<PlanInfo>>, Failure> {
    // every plan the project lists, refused if it isn't registered
    let plans = serves.service().plans(&segments.project).await?;
    Ok(Json(plans))
}

/// Lists a project's queue: each entry with its rank, and whether next takes it
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::queue", skip_all, err(Debug))]
async fn queue<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<QueueListing>, Failure> {
    // every entry in the queue's order, refused if the project isn't registered
    let listing = serves.service().queue(&segments.project).await?;
    Ok(Json(listing))
}

/// Edits a project's queue: 200 if made, 409 if a node named isn't queued, 422 if the node
/// added is not in its plan
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
/// * `edit` - The edit
#[instrument(name = "api::edit_queue", skip_all, err(Debug))]
async fn edit_queue<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
    Body(edit): Body<QueueEdit>,
) -> Result<(StatusCode, Json<Queued>), Failure> {
    // one row written, under the queue's lock
    let queued = serves.service().edit_queue(&segments.project, edit).await?;
    let status = match queued {
        Queued::Applied { .. } => StatusCode::OK,
        Queued::NotQueued { .. } => StatusCode::CONFLICT,
        Queued::NotInPlan { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(queued)))
}

/// Says what work across a project's plans is ready to start, the queue's first
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::project_next", skip_all, err(Debug))]
async fn project_next<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<Next>, Failure> {
    // the queue's ready entries, then each plan's ready work, then what waits
    let next = serves.service().project_next(&segments.project).await?;
    Ok(Json(next))
}

/// Lists a plan in a project: 201 if it was added, 200 if it was listed already
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
/// * `new` - The plan's name within it
#[instrument(name = "api::add_plan", skip_all, err(Debug))]
async fn add_plan<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
    Body(new): Body<NewPlan>,
) -> Result<(StatusCode, Json<AddedPlan>), Failure> {
    // a plan's full name keeps the rule names share
    let plan = PlanName::new(segments.project, new.name)?;
    // list it, leaving one already listed as it is
    let added = serves.service().add_plan(&plan).await?;
    let status = match added {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    };
    Ok((status, Json(AddedPlan { added })))
}

/// Gets one plan, and the revision readers load
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::plan", skip_all, err(Debug))]
async fn plan<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<PlanInfo>, Failure> {
    let plan = segments.name()?;
    // the plan among its project's, refused if the project isn't registered
    let plans = serves.service().plans(&plan.project).await?;
    match plans.into_iter().find(|info| info.name == plan) {
        Some(info) => Ok(Json(info)),
        None => Err(anyhow::Error::from(NotListed(plan)).into()),
    }
}

/// Lists a plan's nodes, in the order the plan was written in
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::list", skip_all, err(Debug))]
async fn list<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<Listing>, Failure> {
    let plan = segments.name()?;
    // the plan's current revision, each node at the version a put of it names
    let listing = serves.service().list(&plan).await?;
    Ok(Json(listing))
}

/// Puts and removes a batch of nodes in a plan: 200 if written, 409 if stale, 422 if refused
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
/// * `batch` - The nodes to write and to remove, each with the version its writer read
#[instrument(name = "api::put", skip_all, err(Debug))]
async fn put<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
    Body(batch): Body<Puts>,
) -> Result<(StatusCode, Json<Checked>), Failure> {
    let plan = segments.name()?;
    // checked against the whole plan, under its write lock
    let checked = serves.service().batch(&plan, batch).await?;
    let status = match checked {
        Checked::Applied { .. } => StatusCode::OK,
        Checked::Stale(_) => StatusCode::CONFLICT,
        Checked::Refused(_) => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(checked)))
}

/// Gets one node of a plan, at the version a put of it names
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the node's id
#[instrument(name = "api::node", skip_all, err(Debug))]
async fn node<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
) -> Result<Json<StoredNode>, Failure> {
    // the node's full name keeps the rule names share
    let plan = PlanName::new(segments.project, segments.plan)?;
    let name = plan.node(segments.id.as_str())?;
    // the node as it stands, or a 404 naming it
    match serves.service().get(&plan, &segments.id).await? {
        Some(node) => Ok(Json(node)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("node `{name}` is not in its plan"),
        )),
    }
}

/// Writes a whole plan as a new revision: 200 if it was made current, 422 if refused
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
/// * `write` - Every node of the plan, and its pages if they change
#[instrument(name = "api::write_plan", skip_all, err(Debug))]
async fn write_plan<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
    Body(write): Body<PlanWrite>,
) -> Result<(StatusCode, Json<Rewrite>), Failure> {
    let plan = segments.name()?;
    // checked against the plan it replaces, under its write lock
    let rewrite = serves.service().write_plan(&plan, write).await?;
    let status = match rewrite {
        Rewrite::Applied(_) => StatusCode::OK,
        Rewrite::Refused(_) => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(rewrite)))
}

/// Builds a plan into the viewer's payload
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::payload", skip_all, err(Debug))]
async fn payload<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<Payload>, Failure> {
    let plan = segments.name()?;
    // every node, its body rendered and its links resolved
    let payload = serves.service().payload(&plan).await?;
    Ok(Json(payload))
}

/// Finds a plan's problems, as `mdbook-reef check` finds a book's
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::check", skip_all, err(Debug))]
async fn check<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<Vec<Diagnostic>>, Failure> {
    let plan = segments.name()?;
    // check's messages, the graph's first
    let problems = serves.service().check(&plan).await?;
    Ok(Json(problems))
}

/// Says what work in a plan is ready to start, and what the rest waits on
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::next", skip_all, err(Debug))]
async fn next<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<Next>, Failure> {
    let plan = segments.name()?;
    // the plan's queued work first, then in plan order, then each piece that waits
    let next = serves.service().next(&plan).await?;
    Ok(Json(next))
}

/// Serves one of a plan's docs as the page its links lead to, or 404 if no doc is written to it
///
/// The page runs no script, so a doc's own can't act as this origin when the page is opened.
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the page's path
#[instrument(name = "api::doc", skip_all, err(Debug))]
async fn doc<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<DocSegments>,
) -> Result<Response, Failure> {
    let plan = PlanName::new(segments.project, segments.plan)?;
    // the doc rendered as its page, or a 404 naming the page
    match serves.service().doc_page(&plan, &segments.path).await? {
        Some(html) => {
            let policy = HeaderValue::from_static("script-src 'none'");
            Ok(([(header::CONTENT_SECURITY_POLICY, policy)], Html(html)).into_response())
        }
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("no doc of `{plan}` is written to `{}`", segments.path),
        )),
    }
}

/// Lists a plan's own pages, by path
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::pages", skip_all, err(Debug))]
async fn pages<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
) -> Result<Json<PageListing>, Failure> {
    let plan = segments.name()?;
    // every page as it stands, each at the version an edit of it names
    let listing = serves.service().pages(&plan).await?;
    Ok(Json(listing))
}

/// Writes or takes out one of a plan's own pages: 200 if written, 409 if stale, 422 if refused
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
/// * `edit` - The page to write or take out, with the version its writer read
#[instrument(name = "api::edit_page", skip_all, err(Debug))]
async fn edit_page<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
    Body(edit): Body<PageEdit>,
) -> Result<(StatusCode, Json<PageEdited>), Failure> {
    let plan = segments.name()?;
    // checked against the whole plan, under its write lock
    let edited = serves.service().edit_page(&plan, edit).await?;
    let status = match edited {
        PageEdited::Applied { .. } => StatusCode::OK,
        PageEdited::Stale { .. } => StatusCode::CONFLICT,
        PageEdited::Refused(_) => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(edited)))
}

/// Gets one of a plan's own pages, at the version an edit of it names
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the page's path
#[instrument(name = "api::page", skip_all, err(Debug))]
async fn page<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PageSegments>,
) -> Result<Json<StoredPage>, Failure> {
    let plan = PlanName::new(segments.project, segments.plan)?;
    // the page as it stands, or a 404 naming it
    match serves.service().page(&plan, &segments.path).await? {
        Some(page) => Ok(Json(page)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("no page `{}` in `{plan}`", segments.path),
        )),
    }
}

/// Lists the work records of a project's nodes, by node, across its plans
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::work", skip_all, err(Debug))]
async fn work<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<Vec<WorkRecord>>, Failure> {
    // every record, refused if the project isn't registered
    let records = serves.service().work(&segments.project).await?;
    Ok(Json(records))
}

/// Gets one node's work record, or a 404 if it wasn't started
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
#[instrument(name = "api::work_of", skip_all, err(Debug))]
async fn work_of<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
) -> Result<Json<WorkRecord>, Failure> {
    // the node's full name keeps the rule names share
    let node = segments.name()?;
    // its record as it stands, or a 404 naming it
    match serves.service().work_of(&node).await? {
        Some(record) => Ok(Json(record)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("node `{node}` has not been started"),
        )),
    }
}

/// Records a node's start: 200 if recorded, 409 if stale, 422 if the node is not in its plan
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
/// * `start` - What the start read, and the version of the record it replaces
#[instrument(name = "api::start_work", skip_all, err(Debug))]
async fn start_work<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Body(start): Body<StartWork>,
) -> Result<(StatusCode, Json<Started>), Failure> {
    let node = segments.name()?;
    // one row written, under the project's work lock
    let started = serves.service().start_work(&node, start).await?;
    let status = match started {
        Started::Recorded { .. } => StatusCode::OK,
        Started::Stale { .. } => StatusCode::CONFLICT,
        Started::NotInPlan { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(started)))
}

/// Adds a capture's SHAs to its node's record: 200 if added, 409 if the node was started again
/// since, 422 if it was never started
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
/// * `add` - The start the capture was taken from, and what it read
#[instrument(name = "api::add_capture", skip_all, err(Debug))]
async fn add_capture<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Body(add): Body<AddCapture>,
) -> Result<(StatusCode, Json<Captured>), Failure> {
    let node = segments.name()?;
    // one row written, under the project's work lock
    let captured = serves.service().add_capture(&node, add).await?;
    let status = match captured {
        Captured::Added { .. } | Captured::Replaced { .. } | Captured::Unchanged { .. } => {
            StatusCode::OK
        }
        Captured::Restarted { .. } => StatusCode::CONFLICT,
        Captured::NotStarted { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(captured)))
}

/// Lists a node's revisions, oldest first, each with its files but not their hunks
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
#[instrument(name = "api::revisions", skip_all, err(Debug))]
async fn revisions<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
) -> Result<Json<Vec<Revision>>, Failure> {
    let node = segments.name()?;
    // every revision, none for a node never captured
    let revisions = serves.service().revisions(&node).await?;
    Ok(Json(revisions))
}

/// Gets one of a node's revisions, or a 404 if it has none of that number
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id, and the revision's number
#[instrument(name = "api::node_revision", skip_all, err(Debug))]
async fn node_revision<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<RevisionSegments>,
) -> Result<Json<Revision>, Failure> {
    let node = segments.name()?;
    // the revision, or a 404 naming it
    match serves
        .service()
        .node_revision(&node, segments.revision)
        .await?
    {
        Some(revision) => Ok(Json(revision)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("node `{node}` has no revision {}", segments.revision),
        )),
    }
}

/// Gets one file of a node's revision, its hunks and all, or a 404 if the revision doesn't list
/// it
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id, the revision's number and the file's path
#[instrument(name = "api::revision_file", skip_all, err(Debug))]
async fn revision_file<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<RevisionFileSegments>,
) -> Result<Json<FileDiff>, Failure> {
    let plan = PlanName::new(segments.project, segments.plan)?;
    let node = plan.node(segments.id)?;
    let (revision, path) = (segments.revision, segments.path);
    // the file, or a 404 naming it
    match serves
        .service()
        .revision_file(&node, revision, &path)
        .await?
    {
        Some(file) => Ok(Json(file)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("revision {revision} of node `{node}` has no file `{path}`"),
        )),
    }
}

/// Lists a plan's comment threads, by number, each comment rendered
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name
#[instrument(name = "api::threads", skip_all, err(Debug))]
async fn threads<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
    Params(query): Params<PendingQuery>,
) -> Result<Json<Vec<Thread>>, Failure> {
    let plan = segments.name()?;
    // every thread on every node of it, none for a plan never commented on
    let threads = serves.service().threads(&plan, query.pending()).await?;
    Ok(Json(threads))
}

/// Lists one node's comment threads, by number, each comment rendered
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
#[instrument(name = "api::node_threads", skip_all, err(Debug))]
async fn node_threads<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Params(query): Params<PendingQuery>,
) -> Result<Json<Vec<Thread>>, Failure> {
    let node = segments.name()?;
    // every thread on it, none for a node never commented on
    let threads = serves
        .service()
        .node_threads(&node, query.pending())
        .await?;
    Ok(Json(threads))
}

/// Lists a node's decisions, by number, each plan rendered
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
#[instrument(name = "api::node_decisions", skip_all, err(Debug))]
async fn node_decisions<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
) -> Result<Json<Vec<Decision>>, Failure> {
    let node = segments.name()?;
    // none for a node no session decided on
    let decisions = serves.service().node_decisions(&node).await?;
    Ok(Json(decisions))
}

/// Keeps a node's decisions, each by the call that asked: 200 if kept, 422 if the node isn't
/// in its plan
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
/// * `record` - The decisions a hook read from a session's transcript
#[instrument(name = "api::record_decisions", skip_all, err(Debug))]
async fn record_decisions<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Body(record): Body<RecordDecisions>,
) -> Result<(StatusCode, Json<DecisionsRecorded>), Failure> {
    let node = segments.name()?;
    // under the project's decisions' lock, told to the change stream if anything was new
    let recorded = serves.service().record_decisions(&node, record).await?;
    let status = match recorded {
        DecisionsRecorded::Recorded { .. } => StatusCode::OK,
        DecisionsRecorded::NotInPlan { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(recorded)))
}

/// Starts a thread on a node or one of its lines: 200 if kept, 422 if the node isn't in its
/// plan or the line isn't in its revisions
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
/// * `new` - What the thread is on, and its first comment's body
#[instrument(name = "api::start_thread", skip_all, err(Debug))]
async fn start_thread<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Body(new): Body<NewThread>,
) -> Result<(StatusCode, Json<Commented>), Failure> {
    let node = segments.name()?;
    // one row written, under the project's comment lock
    let commented = serves.service().start_thread(&node, new).await?;
    Ok((commented_status(&commented), Json(commented)))
}

/// Gets one comment thread of a plan, or a 404 if no thread starts at that number
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the thread's number
#[instrument(name = "api::thread", skip_all, err(Debug))]
async fn thread<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ThreadSegments>,
    Params(query): Params<PendingQuery>,
) -> Result<Json<Thread>, Failure> {
    let plan = segments.name()?;
    // the thread, or a 404 naming it
    let read = serves
        .service()
        .thread(&plan, segments.thread, query.pending());
    match read.await? {
        Some(thread) => Ok(Json(thread)),
        None => Err(Failure::new(
            ErrorCode::NotFound,
            format!("`{plan}` has no comment thread {}", segments.thread),
        )),
    }
}

/// Replies in a thread: 200 if kept, 422 if no thread starts at that number
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the thread's number
/// * `reply` - The reply's body
#[instrument(name = "api::reply", skip_all, err(Debug))]
async fn reply<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ThreadSegments>,
    Body(reply): Body<NewReply>,
) -> Result<(StatusCode, Json<Commented>), Failure> {
    let plan = segments.name()?;
    // one row written, under the project's comment lock
    let commented = serves
        .service()
        .reply(&plan, segments.thread, reply)
        .await?;
    Ok((commented_status(&commented), Json(commented)))
}

/// Marks a thread open or resolved: 200 if marked, 409 if stale, 422 if no thread starts at
/// that number
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the thread's number
/// * `mark` - The state to mark it, and the version it was marked against
#[instrument(name = "api::mark_thread", skip_all, err(Debug))]
async fn mark_thread<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ThreadSegments>,
    Body(mark): Body<MarkThread>,
) -> Result<(StatusCode, Json<Marked>), Failure> {
    let plan = segments.name()?;
    // the thread's first row written again, under the project's comment lock
    let marked = serves
        .service()
        .mark_thread(&plan, segments.thread, mark)
        .await?;
    let status = match marked {
        Marked::Marked { .. } => StatusCode::OK,
        Marked::Stale { .. } => StatusCode::CONFLICT,
        Marked::NoThread { .. } | Marked::Pending { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(marked)))
}

/// Gets a node's open review, or `null` if it has none
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
#[instrument(name = "api::node_review", skip_all, err(Debug))]
async fn node_review<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
) -> Result<Json<Option<Review>>, Failure> {
    let node = segments.name()?;
    // the review still being written, if one is
    let review = serves.service().node_review(&node).await?;
    Ok(Json(review))
}

/// Submits a node's open review or discards it: 200 if marked, 409 if stale, 422 if the node
/// has no open review
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The node's project, plan and id
/// * `mark` - Whether it is submitted or discarded, and the version it was marked against
#[instrument(name = "api::mark_review", skip_all, err(Debug))]
async fn mark_review<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<NodeSegments>,
    Body(mark): Body<MarkReview>,
) -> Result<(StatusCode, Json<Reviewed>), Failure> {
    let node = segments.name()?;
    // the review's row written again, under the project's comment lock
    let reviewed = serves.service().mark_review(&node, mark).await?;
    let status = match reviewed {
        Reviewed::Marked { .. } => StatusCode::OK,
        Reviewed::Stale { .. } => StatusCode::CONFLICT,
        Reviewed::NoReview { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(reviewed)))
}

/// Resolves a thread by hand, a revision linked to it: 200 if resolved, 409 if stale, 422 if no
/// thread starts at that number or the revision isn't stored
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The plan's project and name, and the thread's number
/// * `resolve` - The revision that answered it, and the version it was resolved against
#[instrument(name = "api::resolve_thread", skip_all, err(Debug))]
async fn resolve_thread<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ThreadSegments>,
    Body(resolve): Body<ResolveThread>,
) -> Result<(StatusCode, Json<Resolved>), Failure> {
    let plan = segments.name()?;
    // the thread's first row written again, under the project's comment lock
    let resolved = serves
        .service()
        .resolve_thread(&plan, segments.thread, resolve)
        .await?;
    let status = match resolved {
        Resolved::Resolved { .. } => StatusCode::OK,
        Resolved::Stale { .. } => StatusCode::CONFLICT,
        Resolved::NoThread { .. } | Resolved::NoRevision { .. } | Resolved::Pending { .. } => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
    };
    Ok((status, Json(resolved)))
}

/// Lists every loop, by pane
///
/// # Arguments
///
/// * `serves` - The service
#[instrument(name = "api::loops", skip_all, err(Debug))]
async fn loops<S: Serves>(State(serves): State<S>) -> Result<Json<Vec<Loop>>, Failure> {
    // every loop of every project, none before any starts
    let loops = serves.service().loops().await?;
    Ok(Json(loops))
}

/// Lists the loops feeding one project, by pane
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::project_loops", skip_all, err(Debug))]
async fn project_loops<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<Vec<Loop>>, Failure> {
    // refused if the project isn't registered
    let loops = serves.service().project_loops(&segments.project).await?;
    Ok(Json(loops))
}

/// Changes a pane's loop: 200 if written or joined, 409 if the pane's loop is busy, another loop
/// or pane holds the node or the loop acted on the node's end already, 422 if the pane has no
/// loop, the node isn't the loop's, its plan's or any loop's member, or the group is wider than
/// the loop
///
/// # Arguments
///
/// * `serves` - The service
/// * `edit` - The pane, and the change to its loop
#[instrument(name = "api::edit_loop", skip_all, err(Debug))]
async fn edit_loop<S: Serves>(
    State(serves): State<S>,
    Body(edit): Body<EditLoop>,
) -> Result<(StatusCode, Json<LoopEdited>), Failure> {
    // one row written, under the loops' lock
    let edited = serves.service().edit_loop(edit).await?;
    let status = match edited {
        LoopEdited::Started { .. }
        | LoopEdited::Resumed { .. }
        | LoopEdited::Edited { .. }
        | LoopEdited::Stopped { .. }
        | LoopEdited::Joined { .. } => StatusCode::OK,
        LoopEdited::Busy { .. } | LoopEdited::Held { .. } | LoopEdited::ActedAlready { .. } => {
            StatusCode::CONFLICT
        }
        LoopEdited::NoLoop { .. }
        | LoopEdited::NotInLoop { .. }
        | LoopEdited::NotInPlan { .. }
        | LoopEdited::NotMember { .. }
        | LoopEdited::TooWide { .. } => StatusCode::UNPROCESSABLE_ENTITY,
    };
    Ok((status, Json(edited)))
}

/// The status a comment's outcome answers with: 200 if kept, else 422
///
/// # Arguments
///
/// * `commented` - What came of the comment
fn commented_status(commented: &Commented) -> StatusCode {
    match commented {
        Commented::Added { .. } => StatusCode::OK,
        Commented::NotInPlan { .. } | Commented::NoLine { .. } | Commented::NoThread { .. } => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
    }
}

/// Attaches a runner's session, or attaches it again with what it holds
///
/// # Arguments
///
/// * `serves` - The service
/// * `attach` - The runner's pane, project, host and checkout, and what it holds already
#[instrument(name = "api::attach_session", skip_all, err(Debug))]
async fn attach_session<S: Serves>(
    State(serves): State<S>,
    Body(attach): Body<Attach>,
) -> Result<Json<Session>, Failure> {
    // refused for a pane that isn't a runner's, or a project or plan that isn't there
    let session = serves.service().attach_session(attach).await?;
    Ok(Json(session))
}

/// A session's transcript after the entries read already, 404 if no runner attached it
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The session's id
/// * `query` - How many of its entries were read already
#[instrument(name = "api::session_transcript", skip_all, err(Debug))]
async fn session_transcript<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<SessionSegments>,
    Params(query): Params<AfterQuery>,
) -> Result<Json<Transcript>, Failure> {
    // as far back as it keeps
    let transcript = serves
        .service()
        .session_transcript(&segments.session, query.after)
        .await?;
    Ok(Json(transcript))
}

/// Takes a runner's report, 404 if this service doesn't know its session, so it attaches again
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The session's id
/// * `report` - Its entries since its last report, and where it stands now
#[instrument(name = "api::report_session", skip_all, err(Debug))]
async fn report_session<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<SessionSegments>,
    Body(report): Body<Report>,
) -> Result<Json<Session>, Failure> {
    // kept in memory, and told to the change stream
    let session = serves
        .service()
        .report_session(&segments.session, report)
        .await?;
    Ok(Json(session))
}

/// Lets go of a runner's session, as its runner does when it stops
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The session's id
#[instrument(name = "api::detach_session", skip_all, err(Debug))]
async fn detach_session<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<SessionSegments>,
) -> Result<Json<Session>, Failure> {
    // kept to read, given no more orders
    let session = serves.service().detach_session(&segments.session).await?;
    Ok(Json(session))
}

/// The orders for a runner after the last it read, waiting a while for one if none is there
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The session's id
/// * `query` - The last order read, and how long to wait
#[instrument(name = "api::session_orders", skip_all, err(Debug))]
async fn session_orders<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<SessionSegments>,
    Params(query): Params<OrdersQuery>,
) -> Result<Json<Vec<Ordered>>, Failure> {
    // held open until an order comes, or the wait ends with none
    let wait = std::time::Duration::from_millis(query.wait);
    let orders = serves
        .service()
        .session_orders(&segments.session, query.after, wait)
        .await?;
    Ok(Json(orders))
}

/// Does what the app asks of a runner's session: 200 if the runner was told or its loop
/// changed, 409 if no such prompt waits, Claude isn't waiting for a reply, or the runner is gone
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The session's id
/// * `act` - What is asked of it
#[instrument(name = "api::act_on_session", skip_all, err(Debug))]
async fn act_on_session<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<SessionSegments>,
    Body(act): Body<SessionAct>,
) -> Result<(StatusCode, Json<SessionActed>), Failure> {
    // a loop changed as its pane's would be, or an order queued for the runner
    let acted = serves
        .service()
        .act_on_session(&segments.session, act)
        .await?;
    let status = match acted {
        SessionActed::Ordered { .. } | SessionActed::Looped { .. } => StatusCode::OK,
        SessionActed::NoPrompt { .. }
        | SessionActed::NotWaiting { .. }
        | SessionActed::Gone { .. } => StatusCode::CONFLICT,
    };
    Ok((status, Json(acted)))
}

/// Lists the sessions of runners taking nodes from a project
///
/// # Arguments
///
/// * `serves` - The service
/// * `segments` - The project's name
#[instrument(name = "api::project_sessions", skip_all, err(Debug))]
async fn project_sessions<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<ProjectSegments>,
) -> Result<Json<Vec<Session>>, Failure> {
    // refused if the project isn't registered
    let sessions = serves.service().project_sessions(&segments.project).await?;
    Ok(Json(sessions))
}

/// Follows the change stream: one server-sent event a change, from now on
///
/// The follower subscribes before the answer starts, so no change made after the request is
/// missed. One that falls behind is sent a `missed` change saying how many it lost.
///
/// # Arguments
///
/// * `serves` - The service
/// * `query` - The project, and the plan within it, to narrow the stream to
#[instrument(name = "api::changes", skip_all, err(Debug))]
async fn changes<S: Serves>(
    State(serves): State<S>,
    Params(query): Params<ChangesQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Failure> {
    // a plan is named within its project
    if query.plan.is_some() && query.project.is_none() {
        return Err(Failure::new(
            ErrorCode::BadRequest,
            "`plan` narrows the change stream within a `project`: name the project too",
        ));
    }
    // subscribe now, so every change after this request is sent
    let receiver = serves.service().changes();
    let stream = BroadcastStream::new(receiver).filter_map(move |received| {
        // a follower that fell behind is told how many changes it missed
        let change = match received {
            Ok(change) => change,
            Err(BroadcastStreamRecvError::Lagged(count)) => Change::Missed { count },
        };
        // only the changes it asked for, each an event named by its kind
        query.follows(&change).then(|| Ok(event(&change)))
    });
    // a comment now and then keeps a quiet stream's connection open
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// A change as a server-sent event: named by its kind, its data the change as JSON
///
/// # Arguments
///
/// * `change` - The change
fn event(change: &Change) -> Event {
    // a change is plain data, so this can't fail; if it did, the stream says so in a comment
    match Event::default().event(change.kind()).json_data(change) {
        Ok(event) => event,
        Err(error) => Event::default().comment(format!("a change could not be sent: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{ChangesQuery, Failure};
    use crate::store::{NotListed, NotRegistered, Unkeepable};
    use reef_api::{Change, ErrorCode, Project};
    use reef_model::PlanName;

    /// The change stream narrowed to a project, or to one of its plans
    ///
    /// # Arguments
    ///
    /// * `project` - The project followed, if one is
    /// * `plan` - The plan followed within it, if one is
    fn query(project: Option<&str>, plan: Option<&str>) -> ChangesQuery {
        ChangesQuery {
            project: project.map(str::to_string),
            plan: plan.map(str::to_string),
        }
    }

    /// A follower is sent only the changes it asked for, and every gap
    #[test]
    fn follows_only_what_was_asked_for() {
        // a change to each of two plans, a project's own change, and a gap
        let central = Change::PlanAdded {
            plan: PlanName::new("reef", "central").unwrap(),
        };
        let other = Change::PlanAdded {
            plan: PlanName::new("reef", "other").unwrap(),
        };
        let project = Change::ProjectAdded {
            project: Project {
                name: "reef".to_string(),
                repo: "/src/reef".into(),
            },
        };
        let gap = Change::Missed { count: 2 };
        let queue = Change::QueueChanged {
            project: "reef".to_string(),
        };
        // every change, unnarrowed
        let all = query(None, None);
        assert!(
            [&central, &other, &project, &queue, &gap]
                .iter()
                .all(|c| all.follows(c))
        );
        // a project's changes, its queue's among them, and none of another's
        assert!(query(Some("reef"), None).follows(&project));
        assert!(query(Some("reef"), None).follows(&queue));
        assert!(!query(Some("shoal"), None).follows(&central));
        assert!(!query(Some("shoal"), None).follows(&queue));
        // one plan's, and not its sibling's or its project's own, its queue being the project's
        let one = query(Some("reef"), Some("central"));
        assert!(one.follows(&central));
        assert!(!one.follows(&other));
        assert!(!one.follows(&project));
        assert!(!one.follows(&queue));
        // and a gap reaches everyone
        assert!(one.follows(&gap));
    }

    /// Each of the service's errors a client caused is named by its kind, and the rest are
    /// internal
    #[test]
    fn errors_are_named_by_their_type() {
        // the errors the service raises by type
        let plan = PlanName::new("reef", "central").unwrap();
        let cases = [
            (
                anyhow::Error::from(NotRegistered("x".to_string())),
                ErrorCode::NotRegistered,
            ),
            (anyhow::Error::from(NotListed(plan)), ErrorCode::NotListed),
            (
                anyhow::Error::from(PlanName::new("", "a").unwrap_err()),
                ErrorCode::BadName,
            ),
            (
                anyhow::Error::from(Unkeepable("no".to_string())),
                ErrorCode::BadRequest,
            ),
            (anyhow::anyhow!("shoal fell over"), ErrorCode::Internal),
        ];
        for (error, code) in cases {
            assert_eq!(Failure::from(error).0.code, code);
        }
        // and a context on top still finds the error under it
        let wrapped = anyhow::Error::from(NotRegistered("x".to_string())).context("listing");
        assert_eq!(Failure::from(wrapped).0.code, ErrorCode::NotRegistered);
    }
}
