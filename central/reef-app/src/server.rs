//! The app's server: its pages, its server functions and reef's JSON API on one port
//!
//! It listens on reef's `api.address`, where the CLI, the MCP server and the hooks look for the
//! JSON API, so the app takes reef-serve's place. Under `dx serve`, dx names the address instead
//! and proxies to it. Either way only a loopback address is taken, until users sign in.

use axum::Router;
use axum::http::header;
use axum::routing::get;
use reef_service::{Conf, Service, api};
use std::net::SocketAddr;
use std::sync::OnceLock;
use tokio::net::TcpListener;
use tracing::instrument;

/// The viewer's stylesheet, as mdbook-reef installs it beside a book
const REEF_CSS: &str = include_str!("../../../crates/mdbook-reef/assets/reef.css");

/// The service every server function calls, set once the router is built
static SERVICE: OnceLock<&'static Service> = OnceLock::new();

/// The service the server functions call
///
/// # Panics
///
/// If no router was built yet, since only a router's requests run server functions.
pub fn service() -> &'static Service {
    SERVICE
        .get()
        .copied()
        .expect("the app's router is built before any server function runs")
}

/// Builds the app's router over a service: its pages and server functions, reef.css, and the
/// JSON API
///
/// The server functions call the service given to the first router built in this process.
///
/// # Arguments
///
/// * `service` - The service the JSON API and the server functions call
pub fn router(service: &'static Service) -> Router {
    // the server functions reach the service through here, and a second router changes nothing
    let _ = SERVICE.set(service);
    // the app's pages and server functions, the stylesheet they link, and the JSON API's routes
    dioxus::server::router(crate::App)
        .route(
            "/reef.css",
            get(|| async { ([(header::CONTENT_TYPE, "text/css")], REEF_CSS) }),
        )
        .merge(api::router(service))
}

/// Reads reef's config, listens, opens the service and serves the app until a signal
///
/// The address is taken before shoal starts, so a port that is taken or an address that isn't
/// local stops it before a pool locks the data directory.
#[instrument(name = "server::serve", skip_all, err(Debug))]
pub async fn serve() -> anyhow::Result<()> {
    // Dioxus's logger, which an in-process pool's spans go to as well
    dioxus::logger::initialize_default();
    // reef's config, from `$REEF_CONFIG` or the default path
    let conf = Conf::load(None)?;
    let listener = listen(&conf).await?;
    // shoal in this process, or the external node the config names: opened once per process
    let service = Service::shared(&conf).await?;
    eprintln!(
        "reef-app: serving the app and reef's JSON API on http://{}, shoal {}",
        listener.local_addr()?,
        service.mode().name()
    );
    // once told to stop, end the change stream, so its followers' requests finish
    let stopping = async move {
        stopped().await;
        service.end_changes();
    };
    axum::serve(listener, router(service))
        .with_graceful_shutdown(stopping)
        .await?;
    Ok(())
}

/// Listens where dx says, under `dx serve`, or else where reef's config puts the JSON API
///
/// # Arguments
///
/// * `conf` - reef's config, whose `api.address` names where the JSON API is served
async fn listen(conf: &Conf) -> anyhow::Result<TcpListener> {
    // dx sets `PORT` (and `IP`) for the server it runs, and proxies its own port to it
    let Some(_) = dioxus::cli_config::server_port() else {
        return api::bind(conf).await;
    };
    let address: SocketAddr = dioxus::cli_config::fullstack_address_or_localhost();
    // only this host, until users sign in, whoever chose the address
    anyhow::ensure!(
        address.ip().is_loopback(),
        "reef-app serves localhost only until users sign in, and dx gave it {address}"
    );
    TcpListener::bind(address)
        .await
        .map_err(|error| anyhow::anyhow!("listening on {address}, which dx gave: {error}"))
}

/// Resolves on the first SIGINT or SIGTERM
async fn stopped() {
    // a terminal's ctrl-c, or a supervisor's or dx's stop
    let interrupt = tokio::signal::ctrl_c();
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            // with no handler for SIGTERM, only ctrl-c stops it gracefully
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
}
