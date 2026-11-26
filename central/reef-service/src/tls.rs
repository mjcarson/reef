//! Serving over TLS: every listener reef has, the loopback one and the one other hosts reach
//!
//! `[api] address` is loopback only, for the service's own host: its CLI, its hooks and the
//! first admin, and a browser through a tunnel to it. It serves reef's own certificate
//! ([`certs`](crate::certs)), made for this host's names, so the CLI here trusts reef's CA and
//! calls it by name. `[api.tls]` adds a second listener, on any address, serving the
//! certificate it names, else reef's own, so the JSON API and the app reach other hosts.
//!
//! - **HTTPS only.** A connection that doesn't open with a TLS handshake is answered as plain
//!   HTTP once, with a redirect to https at the host, port and path it asked for
//!   ([`redirect_answer`]), so `http://` typed in a browser, or through a tunnel, lands on the
//!   same port over TLS.
//! - **Sign-in off loopback.** Other hosts reaching a service that answers anyone would let
//!   anyone on the network write, so [`bind`] refuses `[api.tls]` on an address that isn't a
//!   loopback one unless `[auth] required` is set.
//! - **Read once, as the server starts.** The chain and its key are PEM files read by [`bind`],
//!   so a renewed certificate is served from the next restart.
//! - **Handshakes run beside each other.** [`TlsListener`] hands axum a connection once its
//!   handshake is done, and runs each handshake as a task of its own, under [`HANDSHAKE_WAIT`],
//!   so a client that opens a connection and says nothing never holds up the next.
//! - **HTTP/2 first, then HTTP/1.1,** offered by ALPN. Over HTTP/2 a browser carries every
//!   tab's change streams and calls on one connection, where HTTP/1.1 gives it six to a host
//!   across all its tabs, which a few plan pages' streams take up. Browsers speak HTTP/2 only
//!   over TLS, which is why every listener speaks it.

use axum::extract::connect_info::Connected;
use axum::http::uri::{Authority, PathAndQuery};
use axum::serve::{IncomingStream, Listener};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_rustls::server::TlsStream;
use tracing::instrument;

use crate::auth::Peer;
use crate::certs::{self, Own};
use crate::conf::Conf;

/// How long a connection has to finish its TLS handshake before it is dropped
pub const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);

/// How many handshakes run at once before new connections wait in the kernel's backlog
pub const HANDSHAKES_MAX: usize = 128;

/// How long to wait before accepting again after an error a retry can't fix at once
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// The first byte of a TLS record that opens a handshake, which every TLS client sends first
const HANDSHAKE_RECORD: u8 = 0x16;

/// The most of a plain-HTTP request's head read to answer it with a redirect
pub const PLAIN_HEAD_MAX: usize = 8 * 1024;

/// A listener that hands axum each connection once its TLS handshake is done
pub struct TlsListener {
    /// The TCP listener connections arrive on
    tcp: TcpListener,
    /// The TLS config every handshake is made with
    acceptor: TlsAcceptor,
    /// The handshakes still running, each a connection and the peer it came from once done
    handshakes: JoinSet<Option<(TlsStream<TcpStream>, SocketAddr)>>,
}

impl TlsListener {
    /// A listener that makes every handshake with one TLS config
    ///
    /// # Arguments
    ///
    /// * `tcp` - The TCP listener connections arrive on
    /// * `config` - The TLS config, with the certificate it serves
    pub fn new(tcp: TcpListener, config: ServerConfig) -> TlsListener {
        TlsListener {
            tcp,
            acceptor: TlsAcceptor::from(Arc::new(config)),
            handshakes: JoinSet::new(),
        }
    }

    /// Starts a connection's handshake as a task of its own
    ///
    /// # Arguments
    ///
    /// * `stream` - The connection, as TCP accepted it
    /// * `peer` - The address it came from
    fn start(&mut self, stream: TcpStream, peer: SocketAddr) {
        let acceptor = self.acceptor.clone();
        self.handshakes.spawn(async move {
            // a handshake, or a plain request answered, that fails or takes too long is
            // dropped, saying so to the logs alone
            match tokio::time::timeout(HANDSHAKE_WAIT, open(acceptor, stream)).await {
                Ok(Ok(Some(stream))) => Some((stream, peer)),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    tracing::debug!(%peer, %error, "a connection's TLS handshake failed");
                    None
                }
                Err(_) => {
                    tracing::debug!(%peer, "a connection's TLS handshake took too long");
                    None
                }
            }
        });
    }
}

/// Opens a connection: its TLS handshake made if it opens with one, else its plain-HTTP request
/// answered with a redirect to https, and `None` returned
///
/// # Arguments
///
/// * `acceptor` - The TLS config every handshake is made with
/// * `stream` - The connection, as TCP accepted it
async fn open(
    acceptor: TlsAcceptor,
    mut stream: TcpStream,
) -> std::io::Result<Option<TlsStream<TcpStream>>> {
    // the first byte, left for the handshake to read; a connection that closes says nothing
    let mut first = [0u8; 1];
    if stream.peek(&mut first).await? == 0 {
        return Ok(None);
    }
    // a TLS client's first record opens its handshake
    if first[0] == HANDSHAKE_RECORD {
        return acceptor.accept(stream).await.map(Some);
    }
    // anything else is read as plain HTTP's head, up to its blank line, and redirected
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while head.len() < PLAIN_HEAD_MAX && !head.windows(4).any(|end| end == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    stream.write_all(redirect_answer(&head).as_bytes()).await?;
    stream.shutdown().await?;
    Ok(None)
}

/// What a plain-HTTP request on a TLS port is answered with: a 307 to https at the host, port and
/// path it asked for, else a 400 saying reef answers over HTTPS
///
/// A 307 keeps the request's method and body, and isn't kept by a browser as a permanent one.
///
/// # Arguments
///
/// * `head` - The request's head as it came, up to its blank line
///
/// # Examples
///
/// ```
/// use reef_service::tls::redirect_answer;
///
/// let answer = redirect_answer(b"GET /plans/reef/central?x=1 HTTP/1.1\r\nHost: localhost:11180\r\n\r\n");
/// assert!(answer.starts_with("HTTP/1.1 307 Temporary Redirect\r\n"));
/// assert!(answer.contains("\r\nLocation: https://localhost:11180/plans/reef/central?x=1\r\n"));
/// assert!(redirect_answer(b"GET / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400 Bad Request\r\n"));
/// ```
pub fn redirect_answer(head: &[u8]) -> String {
    // the request's target and the host it was sent to, each one a URL can hold
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut headers);
    let parsed = matches!(request.parse(head), Ok(httparse::Status::Complete(_)));
    let host = request
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("host"))
        .and_then(|header| std::str::from_utf8(header.value).ok())
        .and_then(|host| host.trim().parse::<Authority>().ok());
    let path = request
        .path
        .filter(|path| path.starts_with('/'))
        .and_then(|path| path.parse::<PathAndQuery>().ok());
    // the same place over https, else what to do instead
    let (status, location, body) = match (parsed, host, path) {
        (true, Some(host), Some(path)) => {
            let location = format!("https://{host}{path}");
            let body = format!("reef answers over HTTPS only: {location}\n");
            ("307 Temporary Redirect", Some(location), body)
        }
        _ => (
            "400 Bad Request",
            None,
            "reef answers over HTTPS only: call this host and port with https://\n".to_string(),
        ),
    };
    let location = location
        .map(|location| format!("Location: {location}\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 {status}\r\n{location}Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

impl Listener for TlsListener {
    /// A connection over TLS
    type Io = TlsStream<TcpStream>;
    /// The address a connection came from
    type Addr = SocketAddr;

    /// Takes the next connection whose handshake is done, accepting new ones meanwhile
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            // new connections while there is room for their handshakes, else only finished ones
            let room = self.handshakes.len() < HANDSHAKES_MAX;
            tokio::select! {
                accepted = self.tcp.accept(), if room => match accepted {
                    Ok((stream, peer)) => self.start(stream, peer),
                    Err(error) => backoff(error).await,
                },
                Some(done) = self.handshakes.join_next() => {
                    // a connection whose handshake is done is served; a failed one is gone
                    if let Ok(Some(ready)) = done {
                        return ready;
                    }
                }
            }
        }
    }

    /// The address the listener is bound to
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

impl Connected<IncomingStream<'_, TlsListener>> for Peer {
    /// A connection over TLS, and the address it came from
    ///
    /// # Arguments
    ///
    /// * `stream` - The connection axum accepted
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Peer {
        Peer {
            address: *stream.remote_addr(),
            tls: true,
        }
    }
}

/// Waits out an error accepting a connection, as axum's own listener does
///
/// An error of one connection, such as its peer resetting it, is gone at once; any other, such
/// as running out of file descriptors, is waited on, so the loop doesn't spin.
///
/// # Arguments
///
/// * `error` - What accepting failed with
async fn backoff(error: std::io::Error) {
    // one connection's own trouble, which the next connection doesn't share
    let theirs = matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    );
    if theirs {
        return;
    }
    // anything else waits a moment before trying again
    tracing::error!(%error, "accepting a TLS connection failed");
    tokio::time::sleep(ACCEPT_BACKOFF).await;
}

/// reef's own certificate, made or read back for every name this config's listeners are
/// reached by: this host's, `[api] names`, and `[api.tls]`'s address if it names one host
///
/// # Arguments
///
/// * `conf` - reef's config
#[instrument(name = "tls::own", skip_all, err(Debug))]
pub fn own(conf: &Conf) -> anyhow::Result<Own> {
    // the names configured, and the wide listener's own address
    let mut extra = conf.api.names.clone();
    if let Some(tls) = &conf.api.tls {
        let ip = tls.address.ip();
        if !ip.is_unspecified() && !ip.is_loopback() {
            extra.push(ip.to_string());
        }
    }
    // kept where the config says, else in reef's data home, as the CLI here looks for it
    let dir = conf.certs_dir()?;
    certs::ensure(&dir, &certs::names(&extra), std::time::SystemTime::now()).map_err(|error| {
        anyhow::anyhow!(
            "{error:#}: reef's own certificate is kept in {}, set by {}",
            dir.display(),
            conf.key("api.certs")
        )
    })
}

/// Listens over HTTPS on this host's loopback address, serving reef's own certificate
///
/// # Arguments
///
/// * `conf` - reef's config, whose `api.address` names where to listen
/// * `own` - reef's own certificate, from [`own`]
#[instrument(name = "tls::bind_local", skip_all, err(Debug))]
pub async fn bind_local(conf: &Conf, own: &Own) -> anyhow::Result<TlsListener> {
    // only this host, whom reef's own certificate names
    let address = conf.api.address;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "{address} is not a loopback address: set {} to 127.0.0.1 or [::1], and serve other \
         hosts with `[api.tls]`",
        conf.key("api.address")
    );
    let config = server_config(&own.cert, &own.key)?;
    // a port another program holds is named, with the key that chose it
    let tcp = TcpListener::bind(address).await.map_err(|error| {
        anyhow::anyhow!(
            "listening on {address}, set by {}: {error}",
            conf.key("api.address")
        )
    })?;
    Ok(TlsListener::new(tcp, config))
}

/// Listens for other hosts where `[api.tls]` says, if it says anywhere, serving the certificate
/// it names, else reef's own
///
/// Refuses an address that isn't a loopback one with sign-in off, since other hosts would reach
/// a service that answers anyone, and names the key behind a certificate or key that can't be
/// read.
///
/// # Arguments
///
/// * `conf` - reef's config, whose `api.tls` names where to listen and the certificate to serve
/// * `own` - reef's own certificate, from [`own`], served if `api.tls` names none
#[instrument(name = "tls::bind", skip_all, err(Debug))]
pub async fn bind(conf: &Conf, own: &Own) -> anyhow::Result<Option<TlsListener>> {
    // no `[api.tls]` is this host's loopback port alone
    let Some(tls) = &conf.api.tls else {
        return Ok(None);
    };
    // other hosts reach it only if every caller signs in
    anyhow::ensure!(
        tls.address.ip().is_loopback() || conf.auth.required,
        "{} serves reef to other hosts, so every caller must sign in: set {} to true",
        conf.key("api.tls"),
        conf.key("auth.required")
    );
    // the certificate and its key named, both or neither, read before anything listens
    let config = match (&tls.cert, &tls.key) {
        (Some(cert), Some(key)) => server_config(cert, key).map_err(|error| {
            anyhow::anyhow!(
                "{error}, as {} and {} name them",
                conf.key("api.tls.cert"),
                conf.key("api.tls.key")
            )
        })?,
        (None, None) => server_config(&own.cert, &own.key)?,
        _ => anyhow::bail!(
            "{} and {} name a certificate and its key together: set both, or neither for reef's \
             own",
            conf.key("api.tls.cert"),
            conf.key("api.tls.key")
        ),
    };
    // any address, named with its key if it is taken
    let address = tls.address;
    let tcp = TcpListener::bind(address).await.map_err(|error| {
        anyhow::anyhow!(
            "listening on {address}, set by {}: {error}",
            conf.key("api.tls.address")
        )
    })?;
    Ok(Some(TlsListener::new(tcp, config)))
}

/// The TLS config that serves a certificate chain with its key, over HTTP/2 or HTTP/1.1
///
/// # Arguments
///
/// * `cert` - The PEM file holding the chain, the server's own certificate first
/// * `key` - The PEM file holding its private key
pub fn server_config(cert: &Path, key: &Path) -> anyhow::Result<ServerConfig> {
    // every certificate in the chain's file, and at least one
    let chain = CertificateDer::pem_file_iter(cert)
        .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
        .map_err(|error| {
            anyhow::anyhow!(
                "reading the certificate chain at {}: {error}",
                cert.display()
            )
        })?;
    anyhow::ensure!(
        !chain.is_empty(),
        "the certificate chain at {} holds no PEM certificate",
        cert.display()
    );
    // the key, in whichever PEM form it was written
    let private = PrivateKeyDer::from_pem_file(key).map_err(|error| {
        anyhow::anyhow!("reading the private key at {}: {error}", key.display())
    })?;
    // ring's provider, named, since the build holds aws-lc-rs's too; a key that isn't the
    // certificate's is refused here
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, private)
        .map_err(|error| {
            anyhow::anyhow!(
                "serving the certificate at {} with the key at {}: {error}",
                cert.display(),
                key.display()
            )
        })?;
    // HTTP/2 for a client that speaks it, so a browser's streams share one connection, else
    // HTTP/1.1, as the plain listener serves
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}
