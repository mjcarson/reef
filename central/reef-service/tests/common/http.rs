//! The JSON API suite's fixtures: a service served on a loopback port, and a client of it that
//! knows only `reef-api`'s types
//!
//! Each case's server runs on a port of its own, chosen by the OS, over the same store a service
//! case would get: the `reef` project registered for a fresh git repo, and `reef/central` listed.

use reef_api::ApiError;
use reef_model::PlanName;
use reef_service::{Backend, Conf, Service, api};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use super::Fx;
use super::faulty::Faulty;

/// How long a case waits for any one answer or event
pub const WAIT: Duration = Duration::from_secs(10);

/// A client of the JSON API, speaking only `reef-api`'s types
pub struct Client {
    /// The HTTP client, which keeps its connections open between requests
    http: reqwest::Client,
    /// The server's root, such as `http://127.0.0.1:41234`
    pub base: String,
}

impl Client {
    /// A client of the server at an address
    ///
    /// # Arguments
    ///
    /// * `base` - The server's root
    pub fn new(base: String) -> Client {
        Client {
            http: reqwest::Client::new(),
            base,
        }
    }

    /// Sends a request with an optional JSON body, and reads the answer's status and body
    ///
    /// # Arguments
    ///
    /// * `method` - The request's method
    /// * `path` - The route's path, from `reef_api::paths`
    /// * `body` - The body to send as JSON, if any
    pub async fn send<Q: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Q>,
    ) -> anyhow::Result<(StatusCode, T)> {
        // the request, with its body as JSON if it has one
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = tokio::time::timeout(WAIT, request.send()).await??;
        let status = response.status();
        // the body as the route's answer, saying what it was if it isn't
        let text = response.text().await?;
        let answer = serde_json::from_str(&text).map_err(|error| {
            anyhow::anyhow!("{status} from {path} is not the answer expected ({error}): {text}")
        })?;
        Ok((status, answer))
    }

    /// Gets a route's answer
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<(StatusCode, T)> {
        self.send::<(), T>(Method::GET, path, None).await
    }

    /// Posts a JSON body to a route, and reads its answer
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path
    /// * `body` - The body to send
    pub async fn post<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Q,
    ) -> anyhow::Result<(StatusCode, T)> {
        self.send(Method::POST, path, Some(body)).await
    }

    /// Puts a JSON body to a route, and reads its answer
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path
    /// * `body` - The body to send
    pub async fn put<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Q,
    ) -> anyhow::Result<(StatusCode, T)> {
        self.send(Method::PUT, path, Some(body)).await
    }

    /// Posts raw text as a JSON body, for a body the route can't read
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path
    /// * `text` - The body as sent
    pub async fn post_text(
        &self,
        path: &str,
        text: &str,
    ) -> anyhow::Result<(StatusCode, ApiError)> {
        let request = self
            .http
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .body(text.to_string());
        let response = tokio::time::timeout(WAIT, request.send()).await??;
        let status = response.status();
        Ok((status, response.json().await?))
    }

    /// Opens a route as a stream, returning once the answer's head is in
    ///
    /// # Arguments
    ///
    /// * `path` - The route's path
    pub async fn open(&self, path: &str) -> anyhow::Result<reqwest::Response> {
        let request = self.http.get(format!("{}{path}", self.base));
        Ok(tokio::time::timeout(WAIT, request.send()).await??)
    }
}

/// A served service, a client of it, and the project and plan a case works in
pub struct Ax<B: Backend + 'static> {
    /// The client, which knows only `reef-api`'s types
    pub client: Client,
    /// The service the server calls, for a case to end its change stream
    pub service: Arc<Service<Faulty<B>>>,
    /// The plan the case works in, listed before the case starts
    pub plan: PlanName,
    /// The repo of the plan's project, `reef`
    pub repo: PathBuf,
    /// A directory of the case's own, under cargo's target dir, which holds the repo
    pub dir: PathBuf,
    /// The server's task, stopped when the fixture is dropped
    server: JoinHandle<anyhow::Result<()>>,
}

impl<B: Backend + 'static> Drop for Ax<B> {
    /// Stops the server, letting go of its hold on the service
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl<B: Backend + 'static> Fx<B> {
    /// Serves this fixture's store on a loopback port of its own, and makes a client of it
    pub async fn api(self) -> anyhow::Result<Ax<B>> {
        // the service, shared by the server and the case
        let sx = self.serve();
        let service = Arc::new(sx.service);
        // any free port on localhost
        let mut conf = Conf::default();
        conf.api.address = "127.0.0.1:0".parse()?;
        let listener = api::bind(&conf).await?;
        let base = format!("http://{}", listener.local_addr()?);
        // served until the fixture is dropped
        let server = tokio::spawn(api::serve(
            listener,
            service.clone(),
            std::future::pending(),
        ));
        Ok(Ax {
            client: Client::new(base),
            service,
            plan: sx.plan,
            repo: sx.repo,
            dir: sx.dir,
            server,
        })
    }
}

/// One server-sent event: its name and its data
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEvent {
    /// The event's name
    pub event: String,
    /// Its data, the lines joined
    pub data: String,
}

/// Reads server-sent events off an open stream, skipping the comments that keep it alive
pub struct Events {
    /// The open answer
    response: reqwest::Response,
    /// What has come in and not yet been read as an event
    pending: String,
}

impl Events {
    /// Reads events off an answer whose head is in
    ///
    /// # Arguments
    ///
    /// * `response` - The open answer
    pub fn new(response: reqwest::Response) -> Events {
        Events {
            response,
            pending: String::new(),
        }
    }

    /// The next event, or `None` once the stream has ended
    pub async fn next(&mut self) -> anyhow::Result<Option<SentEvent>> {
        loop {
            // an event is complete at its blank line
            if let Some(end) = self.pending.find("\n\n") {
                let block: String = self.pending.drain(..end + 2).collect();
                if let Some(event) = parse(&block) {
                    return Ok(Some(event));
                }
                // a block of comments alone is a keep-alive
                continue;
            }
            // else read more, or find the stream ended
            match tokio::time::timeout(WAIT, self.response.chunk()).await?? {
                Some(chunk) => self.pending.push_str(std::str::from_utf8(&chunk)?),
                None => return Ok(None),
            }
        }
    }
}

/// Reads one event's block, or `None` for a block of comments alone
///
/// # Arguments
///
/// * `block` - The lines up to and including its blank line
fn parse(block: &str) -> Option<SentEvent> {
    let mut event = None;
    let mut data = Vec::new();
    for line in block.lines() {
        // `event:` names it, `data:` lines carry it, and `:` lines are comments
        if let Some(name) = line.strip_prefix("event:") {
            event = Some(name.trim_start().to_string());
        } else if let Some(line) = line.strip_prefix("data:") {
            data.push(line.trim_start().to_string());
        }
    }
    // an event with no name is a `message`, as a browser reads it
    match (event, data.is_empty()) {
        (None, true) => None,
        (event, _) => Some(SentEvent {
            event: event.unwrap_or_else(|| "message".to_string()),
            data: data.join("\n"),
        }),
    }
}
