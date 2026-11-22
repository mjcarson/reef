//! A client of reef's JSON API, speaking `reef-api`'s types
//!
//! Each method is one route, its path built by `reef_api::paths` and its answer one of
//! `reef-api`'s types. A route that can't do what was asked answers with an [`ApiError`], which
//! comes back as the error, so a caller can match on its code. A refusal, such as a project name
//! that is taken, is an answer, not an error. When nothing answers at the service's address, the
//! error says where the CLI looked, which key set it, and how to start the service.

use reef_api::{
    AddedPlan, AddedProject, ApiError, Checked, ErrorCode, Listing, NewPlan, NewProject, PlanInfo,
    Project, Puts, QueueEdit, QueueListing, Queued, StoredNode, paths,
};
use reef_model::{Diagnostic, Next, PlanName};
use serde::de::DeserializeOwned;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use crate::conf::Conf;

/// How long to wait for the service to take a connection
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// A client of reef's JSON API
pub struct Client {
    /// The HTTP client, which keeps its connection open between requests
    http: reqwest::Client,
    /// The service's root, such as `http://127.0.0.1:12180`
    base: String,
    /// The address the service is called at
    address: SocketAddr,
    /// Where that address came from, for a message that says what to change
    origin: String,
}

impl Client {
    /// A client of the service at the address reef's config names
    ///
    /// # Arguments
    ///
    /// * `conf` - The CLI's part of reef's config
    pub fn new(conf: &Conf) -> anyhow::Result<Client> {
        // the service is on this host, so no proxy a shell has set is asked to reach it
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(CONNECT_WAIT)
            .build()?;
        let address = conf.api.address;
        // a config that set the address is named, else the key that would change the default
        let origin = match conf.source {
            Some(_) => format!("set by {}", conf.key("api.address")),
            None => format!(
                "reef's default; set {} to change it",
                conf.key("api.address")
            ),
        };
        Ok(Client {
            http,
            base: format!("http://{address}"),
            address,
            origin,
        })
    }

    /// Lists every project, by name
    pub async fn projects(&self) -> anyhow::Result<Vec<Project>> {
        self.send(self.http.get(self.url(paths::PROJECTS)), paths::PROJECTS)
            .await
    }

    /// Registers a project for the repo that holds a directory
    ///
    /// # Arguments
    ///
    /// * `name` - The project's name
    /// * `repo` - A directory in its repo, absolute
    pub async fn add_project(&self, name: &str, repo: &Path) -> anyhow::Result<AddedProject> {
        // the service reads the repo as this host sees it
        let new = NewProject {
            name: name.to_string(),
            repo: repo.to_path_buf(),
        };
        let request = self.http.post(self.url(paths::PROJECTS)).json(&new);
        self.send(request, paths::PROJECTS).await
    }

    /// Finds the project a directory belongs to, or `None` if no project is registered for it
    ///
    /// A directory in a linked worktree belongs to its main repo's project.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory, absolute
    pub async fn find_project(&self, dir: &Path) -> anyhow::Result<Option<Project>> {
        // a query carries text, so a path that isn't utf-8 can't be sent
        let text = dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", dir.display()))?;
        let path = paths::find_project(text);
        // a directory no project holds is an answer here, not an error
        match self.send(self.http.get(self.url(&path)), &path).await {
            Ok(project) => Ok(Some(project)),
            Err(error) if is_code(&error, ErrorCode::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Lists a project's plans, by name
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn plans(&self, project: &str) -> anyhow::Result<Vec<PlanInfo>> {
        let path = paths::plans(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a plan in its project, leaving one already listed as it is
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn add_plan(&self, plan: &PlanName) -> anyhow::Result<AddedPlan> {
        // the plan's name within its project, posted to the project's plans
        let path = paths::plans(&plan.project);
        let new = NewPlan {
            name: plan.plan.clone(),
        };
        let request = self.http.post(self.url(&path)).json(&new);
        self.send(request, &path).await
    }

    /// Lists a plan's current revision, each node at the version a put of it names
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn list(&self, plan: &PlanName) -> anyhow::Result<Listing> {
        let path = paths::nodes(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Gets one node of a plan, at the version a put of it names
    ///
    /// A node that isn't in its plan comes back as a `not-found` error naming it.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `id` - The node's id
    pub async fn node(&self, plan: &PlanName, id: &str) -> anyhow::Result<StoredNode> {
        let path = paths::node(plan, id);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Puts and removes a batch of nodes in a plan, checked against the whole plan as one change
    ///
    /// A stale or refused batch is an answer, not an error, and writes nothing.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    /// * `batch` - The nodes to write and to remove, each with the version its writer read
    pub async fn batch(&self, plan: &PlanName, batch: &Puts) -> anyhow::Result<Checked> {
        let path = paths::nodes(plan);
        let request = self.http.post(self.url(&path)).json(batch);
        self.send(request, &path).await
    }

    /// Finds a plan's problems, as `mdbook-reef check` finds a book's
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn check(&self, plan: &PlanName) -> anyhow::Result<Vec<Diagnostic>> {
        let path = paths::check(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Says what work in a plan is ready to start, its entries of the queue first, then in plan
    /// order, and what the rest waits on
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan's full name
    pub async fn next(&self, plan: &PlanName) -> anyhow::Result<Next> {
        let path = paths::next(plan);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Says what work across a project's plans is ready to start, the queue's first, then each
    /// plan's in plan order, and what the rest waits on
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn project_next(&self, project: &str) -> anyhow::Result<Next> {
        let path = paths::project_next(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Lists a project's queue: each entry with its rank, and whether next takes it
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub async fn queue(&self, project: &str) -> anyhow::Result<QueueListing> {
        let path = paths::queue(project);
        self.send(self.http.get(self.url(&path)), &path).await
    }

    /// Edits a project's queue: puts a node in it, moves one, or takes one off
    ///
    /// A node that isn't queued, or isn't in its plan, is an answer, not an error, and changes
    /// nothing.
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    /// * `edit` - The edit
    pub async fn edit_queue(&self, project: &str, edit: &QueueEdit) -> anyhow::Result<Queued> {
        let path = paths::queue(project);
        let request = self.http.post(self.url(&path)).json(edit);
        self.send(request, &path).await
    }

    /// The full URL of a route's path
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path, from `reef_api::paths`
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Sends a request and reads its answer as a route's type, or as the error it answered with
    ///
    /// # Arguments
    ///
    /// * `request` - The request, ready to send
    /// * `path` - The route's path, for messages
    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        path: &str,
    ) -> anyhow::Result<T> {
        // a service that isn't there is said plainly, with how to start it
        let response = request
            .send()
            .await
            .map_err(|error| self.unreachable(error))?;
        let status = response.status().as_u16();
        // the whole body, then read as whichever answer its status calls for
        let text = response
            .text()
            .await
            .map_err(|error| anyhow::anyhow!("reading {path}'s answer: {error}"))?;
        decode(status, path, &text)
    }

    /// Says why a request got no answer at all
    ///
    /// # Arguments
    ///
    /// * `error` - What the HTTP client failed with
    fn unreachable(&self, error: reqwest::Error) -> anyhow::Error {
        // nothing listening is the service not running, which only starting it fixes
        if error.is_connect() {
            return anyhow::anyhow!(
                "reef's service isn't running at {} ({}): start it with `reef-serve`",
                self.address,
                self.origin
            );
        }
        // anything else, such as a connection dropped partway, as it came
        anyhow::anyhow!("calling reef's service at {}: {error}", self.address)
    }
}

/// Reads an answer's body by its status: a success as the route's type, a failure as an
/// [`ApiError`], else as the route's own refusal, such as a taken name
///
/// # Arguments
///
/// * `status` - The answer's HTTP status
/// * `path` - The route's path, for messages
/// * `text` - The answer's body
pub fn decode<T: DeserializeOwned>(status: u16, path: &str, text: &str) -> anyhow::Result<T> {
    // a success is the route's answer
    if (200..300).contains(&status) {
        return serde_json::from_str(text).map_err(|error| {
            anyhow::anyhow!("{status} from {path} is not the answer expected ({error}): {text}")
        });
    }
    // an error names its kind, and its message is the service's
    if let Ok(error) = serde_json::from_str::<ApiError>(text) {
        return Err(error.into());
    }
    // else a refusal the route answers with its outcome, or something no route says
    serde_json::from_str(text).map_err(|_| anyhow::anyhow!("{status} from {path}: {text}"))
}

/// Whether an error is the API's error of one kind
///
/// # Arguments
///
/// * `error` - The error a call returned
/// * `code` - The kind to look for
pub fn is_code(error: &anyhow::Error, code: ErrorCode) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|error| error.code == code)
}

#[cfg(test)]
mod tests {
    use super::{Client, decode, is_code};
    use crate::conf::Conf;
    use reef_api::{AddedProject, ErrorCode, Project};
    use std::path::PathBuf;

    /// A success reads as the route's answer
    #[test]
    fn success_reads_as_the_answer() {
        // a listing of one project
        let text = r#"[{"name": "reef", "repo": "/src/reef"}]"#;
        let projects: Vec<Project> = decode(200, "/p", text).expect("a listing reads");
        assert_eq!(projects[0].name, "reef");
        assert_eq!(projects[0].repo, PathBuf::from("/src/reef"));
        // and one that isn't the answer expected says what it was
        let wrong = decode::<Vec<Project>>(200, "/p", "{}").expect_err("an object read as a list");
        assert!(wrong.to_string().contains("200 from /p"), "{wrong}");
    }

    /// An error answers as the API's error, with its code, and a refusal as its outcome
    #[test]
    fn errors_and_refusals_read_as_themselves() {
        // a 404 naming its kind comes back as the error, which a caller can match on
        let text = r#"{"code": "not-found", "message": "no project is registered for /x"}"#;
        let error = decode::<Project>(404, "/f", text).expect_err("an error read as a project");
        assert!(is_code(&error, ErrorCode::NotFound));
        assert!(!is_code(&error, ErrorCode::Internal));
        assert_eq!(error.to_string(), "no project is registered for /x");
        // a taken name is the route's answer, not an error
        let taken: AddedProject = decode(409, "/p", r#""name-taken""#).expect("a refusal reads");
        assert_eq!(taken, AddedProject::NameTaken);
        // and a body no route sends names its status
        let odd = decode::<AddedProject>(502, "/p", "bad gateway").expect_err("text read");
        assert_eq!(odd.to_string(), "502 from /p: bad gateway");
    }

    /// With nothing listening, the error names the address, where it came from, and the fix
    #[tokio::test]
    async fn unreachable_service_says_how_to_start_it() {
        // a loopback port that was free a moment ago, and is closed now
        let address = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let mut conf = Conf::default();
        conf.api.address = address;
        let client = Client::new(&conf).unwrap();
        // any call says the service isn't running, where it looked, and how to start it
        let error = client.projects().await.expect_err("a closed port answered");
        let message = error.to_string();
        assert!(message.contains("isn't running"), "{message}");
        assert!(message.contains(&address.to_string()), "{message}");
        assert!(message.contains("`api.address`"), "{message}");
        assert!(message.contains("`reef-serve`"), "{message}");
    }
}
