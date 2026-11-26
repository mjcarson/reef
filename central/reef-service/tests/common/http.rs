//! The JSON API suite's fixtures: a service served on a loopback port, and a client of it that
//! knows only `reef-api`'s types
//!
//! Each case's server runs on a port of its own, chosen by the OS, over the same store a service
//! case would get: the `reef` project registered for a fresh git repo, and `reef/central` listed.
//! It speaks HTTPS, serving reef's own certificate, made in the case's own directory so no case
//! touches the user's, and its client trusts reef's own CA alone. A case that serves other hosts
//! too gets a second port, on every interface, with a certificate minted for it, and a client
//! that trusts that.

use reef_api::ApiError;
use reef_model::PlanName;
use reef_service::api::{Listeners, Local};
use reef_service::{Backend, Conf, Service, TlsSettings, api};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use super::Fx;
use super::faulty::Faulty;
use super::tls::{Minted, minted};

/// How long a case waits for any one answer or event
pub const WAIT: Duration = Duration::from_secs(10);

/// A client of the JSON API, speaking only `reef-api`'s types
#[derive(Clone)]
pub struct Client {
    /// The HTTP client, which keeps its connections open between requests and follows no
    /// redirect, so a case sees where the server sends a browser
    http: reqwest::Client,
    /// The server's root, such as `http://127.0.0.1:41234`
    pub base: String,
    /// The token every request sends as `Authorization: Bearer`, if it sends one
    token: Option<String>,
}

impl Client {
    /// A client of a server at a root, trusting the system's roots alone, as for a plain-HTTP
    /// request that is only redirected
    ///
    /// # Arguments
    ///
    /// * `base` - The server's root
    pub fn new(base: String) -> Client {
        Client {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("a plain HTTP client builds"),
            base,
            token: None,
        }
    }

    /// A client of a server over TLS, trusting the roots a PEM file holds
    ///
    /// # Arguments
    ///
    /// * `base` - The server's root, an `https://` one
    /// * `ca` - The PEM file of the roots its certificate is trusted by
    pub fn trusting(base: String, ca: &std::path::Path) -> anyhow::Result<Client> {
        // the case's own CA alone, past any proxy, following no redirect
        let pem = std::fs::read(ca)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(&pem)?)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Client {
            http,
            base,
            token: None,
        })
    }

    /// The same client, every request sending a token
    ///
    /// # Arguments
    ///
    /// * `token` - The token to send as `Authorization: Bearer`
    pub fn signed(&self, token: &str) -> Client {
        Client {
            token: Some(token.to_string()),
            ..self.clone()
        }
    }

    /// A request to a route, sending this client's token if it has one, for a case to add
    /// headers or a body of its own
    ///
    /// # Arguments
    ///
    /// * `method` - The request's method
    /// * `path` - The route's path, from `reef_api::paths`
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let request = self.http.request(method, format!("{}{path}", self.base));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    /// Sends a request a case built, waiting no longer than any answer is waited for
    ///
    /// # Arguments
    ///
    /// * `request` - The request
    pub async fn raw(&self, request: reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
        Ok(tokio::time::timeout(WAIT, request.send()).await??)
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
        let mut request = self.request(method, path);
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
        let request = self.request(Method::GET, path);
        Ok(tokio::time::timeout(WAIT, request.send()).await??)
    }
}

/// The listener a served service has over TLS, beside its loopback one
pub struct OverTls {
    /// A client of it, trusting the case's CA, signed in as nobody
    pub client: Client,
    /// The address it listens on, every interface's, at a port of its own
    pub address: SocketAddr,
    /// The certificate it serves, its key, and the CA it is trusted by
    pub minted: Minted,
}

/// A served service, a client of it, and the project and plan a case works in
pub struct Ax<B: Backend + 'static> {
    /// The client, over HTTPS to this host's port, which knows only `reef-api`'s types
    pub client: Client,
    /// The address the server listens on, for a config to name
    pub address: SocketAddr,
    /// The directory reef's own CA and certificate are kept in, for a config's `[api] certs`
    pub certs: PathBuf,
    /// reef's own CA, which the client trusts
    pub ca: PathBuf,
    /// The listener over TLS, for a case that serves one
    pub tls: Option<OverTls>,
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

impl<B: Backend + 'static> Ax<B> {
    /// The config the CLI calls this server by: its loopback address, over HTTPS, trusting reef's
    /// own CA from the case's directory, as the CLI on a server's host does
    pub fn cli_config(&self) -> String {
        format!(
            "[api]\naddress = \"{}\"\ncerts = \"{}\"\n",
            self.address,
            self.certs.display()
        )
    }
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
        self.api_with(false).await
    }

    /// Serves this fixture's store asking every caller to sign in, and makes a client of it,
    /// which has signed nobody in
    pub async fn api_signing_in(self) -> anyhow::Result<Ax<B>> {
        self.api_with(true).await
    }

    /// Serves this fixture's store asking every caller to sign in, on a loopback port and for
    /// other hosts on every interface, with a certificate minted for this host there
    pub async fn api_over_tls(self) -> anyhow::Result<Ax<B>> {
        // a certificate minted for this host, which the other hosts' listener names
        let minted = minted(&self.dir.join("tls"))?;
        let tls = TlsSettings {
            address: "0.0.0.0:0".parse()?,
            cert: Some(minted.cert.clone()),
            key: Some(minted.key.clone()),
        };
        self.api_on(true, Some((tls, minted))).await
    }

    /// Serves this fixture's store on a loopback port of its own, asking callers to sign in or
    /// not, and makes a client of it
    ///
    /// # Arguments
    ///
    /// * `required` - Whether every caller must sign in
    async fn api_with(self, required: bool) -> anyhow::Result<Ax<B>> {
        self.api_on(required, None).await
    }

    /// Serves this fixture's store over HTTPS on a loopback port of its own, with reef's own
    /// certificate made in the case's directory, and for other hosts too if a listener is given
    ///
    /// # Arguments
    ///
    /// * `required` - Whether every caller must sign in
    /// * `wide` - Other hosts' listener, and the certificate minted for it, if the case has one
    async fn api_on(
        self,
        required: bool,
        wide: Option<(TlsSettings, Minted)>,
    ) -> anyhow::Result<Ax<B>> {
        // the service, shared by the server and the case
        let sx = self.serve();
        let service = Arc::new(sx.service.requiring_sign_in(required));
        // any free port on localhost, reef's own certificate in the case's own directory
        let certs = sx.dir.join("certs");
        let mut conf = Conf::default();
        conf.api.address = "127.0.0.1:0".parse()?;
        conf.api.certs = Some(certs.clone());
        conf.auth.required = required;
        let minted = wide.as_ref().map(|(_, minted)| minted.clone());
        conf.api.tls = wide.map(|(tls, _)| tls);
        let listeners = Listeners::bind(&conf).await?;
        let address = match &listeners.local {
            Local::Tls(local) => axum::serve::Listener::local_addr(local)?,
            Local::Plain(plain) => plain.local_addr()?,
        };
        let ca = listeners
            .own
            .as_ref()
            .map(|own| own.ca.clone())
            .ok_or_else(|| anyhow::anyhow!("no certificate of reef's own was made"))?;
        // other hosts' listener, called by its name, which the minted certificate names
        let over = match (&listeners.tls, minted) {
            (Some(tls), Some(minted)) => {
                let tls = axum::serve::Listener::local_addr(tls)?;
                Some(OverTls {
                    client: Client::trusting(
                        format!("https://localhost:{}", tls.port()),
                        &minted.ca,
                    )?,
                    address: tls,
                    minted,
                })
            }
            _ => None,
        };
        // served until the fixture is dropped
        let server = tokio::spawn(api::serve(
            listeners,
            service.clone(),
            std::future::pending(),
        ));
        Ok(Ax {
            client: Client::trusting(format!("https://{address}"), &ca)?,
            address,
            certs,
            ca,
            tls: over,
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
