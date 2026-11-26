//! Serving over TLS: the listener beside the loopback one, which other hosts reach
//!
//! `[api] address` is plain HTTP and loopback only, for the service's own host: its CLI, its
//! hooks and the first admin. `[api.tls]` adds a second listener, on any address, that speaks TLS
//! with the certificate and key it names, so the JSON API and the app reach other hosts.
//!
//! - **Only with sign-in required.** Other hosts reaching a service that answers anyone would let
//!   anyone on the network write, so [`bind`] refuses TLS unless `[auth] required` is set.
//! - **Read once, as the server starts.** The chain and its key are PEM files read by [`bind`],
//!   so a renewed certificate is served from the next restart.
//! - **Handshakes run beside each other.** [`TlsListener`] hands axum a connection once its
//!   handshake is done, and runs each handshake as a task of its own, under [`HANDSHAKE_WAIT`],
//!   so a client that opens a connection and says nothing never holds up the next.
//! - **HTTP/2 first, then HTTP/1.1,** offered by ALPN. Over HTTP/2 a browser carries every
//!   tab's change streams and calls on one connection, where HTTP/1.1 gives it six to a host
//!   across all its tabs, which a few plan pages' streams take up. The plain listener speaks
//!   HTTP/1.1, since browsers speak HTTP/2 only over TLS.

use axum::extract::connect_info::Connected;
use axum::serve::{IncomingStream, Listener};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_rustls::server::TlsStream;
use tracing::instrument;

use crate::auth::Peer;
use crate::conf::Conf;

/// How long a connection has to finish its TLS handshake before it is dropped
pub const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);

/// How many handshakes run at once before new connections wait in the kernel's backlog
pub const HANDSHAKES_MAX: usize = 128;

/// How long to wait before accepting again after an error a retry can't fix at once
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

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
            // a handshake that fails or takes too long is dropped, saying so to the logs alone
            match tokio::time::timeout(HANDSHAKE_WAIT, acceptor.accept(stream)).await {
                Ok(Ok(stream)) => Some((stream, peer)),
                Ok(Err(error)) => {
                    tracing::debug!(%peer, %error, "a TLS handshake failed");
                    None
                }
                Err(_) => {
                    tracing::debug!(%peer, "a TLS handshake took too long");
                    None
                }
            }
        });
    }
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

/// Listens over TLS where reef's config says, if it says anywhere
///
/// Refuses TLS with sign-in off, since other hosts would reach a service that answers anyone,
/// and names the key behind a certificate or key that can't be read.
///
/// # Arguments
///
/// * `conf` - reef's config, whose `api.tls` names where to listen and the certificate to serve
#[instrument(name = "tls::bind", skip_all, err(Debug))]
pub async fn bind(conf: &Conf) -> anyhow::Result<Option<TlsListener>> {
    // no `[api.tls]` is plain HTTP on loopback alone
    let Some(tls) = &conf.api.tls else {
        return Ok(None);
    };
    // other hosts reach it only if every caller signs in
    anyhow::ensure!(
        conf.auth.required,
        "{} serves reef to other hosts, so every caller must sign in: set {} to true",
        conf.key("api.tls"),
        conf.key("auth.required")
    );
    // the certificate and its key, read before anything listens
    let config = server_config(&tls.cert, &tls.key).map_err(|error| {
        anyhow::anyhow!(
            "{error}, as {} and {} name them",
            conf.key("api.tls.cert"),
            conf.key("api.tls.key")
        )
    })?;
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
