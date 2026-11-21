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
//! | `GET …/plans/{plan}` | `plans` | its [`PlanInfo`], or 404 |
//! | `GET …/plans/{plan}/nodes` | `list` | its [`Listing`] |
//! | `POST …/plans/{plan}/nodes` | `put` | [`Checked`]: 200, 409 if stale, 422 if refused |
//! | `GET …/plans/{plan}/nodes/{id}` | `get` | the [`StoredNode`], or 404 |
//! | `PUT …/plans/{plan}/revision` | `write_plan` | [`Rewrite`]: 200, or 422 if refused |
//! | `GET …/plans/{plan}/payload` | `payload` | the viewer's [`Payload`] |
//! | `GET …/plans/{plan}/check` | `check` | its problems |
//! | `GET …/plans/{plan}/next` | `next` | its ready work, and what the rest waits on: [`Next`] |
//! | `GET …/plans/{plan}/docs/{*path}` | `doc_page` | a doc rendered as its page, or 404 |
//! | `GET /api/v1/changes?project=&plan=` | `changes` | server-sent [`Change`]s |
//!
//! - **A refusal is an answer.** A stale or refused write comes back as its outcome, with a
//!   status that says which, so a client reads one type whatever the status.
//! - **An error names its kind.** A project that isn't registered, a plan that isn't listed, a
//!   name that breaks the rule, a body that can't be read: each is an [`ApiError`] with a code
//!   and check's or the service's own message.
//! - **A doc is a page.** The payload's doc links lead under a plan's `docs/`, where each doc is
//!   served as the HTML page mdbook would write for it, so the viewer's *Read here* works as it
//!   does in a book. A page opened on its own runs no script a doc holds.
//! - **The change stream is server-sent events,** one a change, its event named by its kind and
//!   its data the change as JSON. It is live only, and narrowed by `project` and `plan`.
//! - **Localhost only.** [`bind`] refuses any address but a loopback one until users sign in
//!   (M10).

use axum::extract::{FromRequest, FromRequestParts, Path as Segment, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use reef_api::{
    AddedPlan, AddedProject, ApiError, Change, Checked, ErrorCode, Listing, NewPlan, NewProject,
    PlanInfo, PlanWrite, Project, Puts, Rewrite, StoredNode, paths,
};
use reef_model::{Diagnostic, NameError, Next, Payload, PlanName};
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
use crate::service::Service;
use crate::store::{NotListed, NotRegistered, Unkeepable};

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
        .route(paths::PLAN, get(plan::<S>))
        .route(paths::NODES, get(list::<S>).post(put::<S>))
        .route(paths::NODE, get(node::<S>))
        .route(paths::REVISION, axum::routing::put(write_plan::<S>))
        .route(paths::PAYLOAD, get(payload::<S>))
        .route(paths::CHECK, get(check::<S>))
        .route(paths::NEXT, get(next::<S>))
        .route(paths::DOC, get(doc::<S>))
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

/// The query of the route that finds a directory's project
#[derive(Deserialize)]
struct FindQuery {
    /// The directory, absolute, as the service's host sees it
    dir: PathBuf,
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
/// * `write` - Every node of the plan
#[instrument(name = "api::write_plan", skip_all, err(Debug))]
async fn write_plan<S: Serves>(
    State(serves): State<S>,
    Segments(segments): Segments<PlanSegments>,
    Body(write): Body<PlanWrite>,
) -> Result<(StatusCode, Json<Rewrite>), Failure> {
    let plan = segments.name()?;
    // checked against the plan it replaces, under its write lock
    let rewrite = serves.service().write_plan(&plan, write.nodes).await?;
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
    // the ready work in plan order, then each piece that waits with its unmet needs
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
        // every change, unnarrowed
        let all = query(None, None);
        assert!(
            [&central, &other, &project, &gap]
                .iter()
                .all(|c| all.follows(c))
        );
        // a project's changes, and none of another's
        assert!(query(Some("reef"), None).follows(&project));
        assert!(!query(Some("shoal"), None).follows(&central));
        // one plan's, and not its sibling's or its project's own
        let one = query(Some("reef"), Some("central"));
        assert!(one.follows(&central));
        assert!(!one.follows(&other));
        assert!(!one.follows(&project));
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
