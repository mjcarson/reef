//! The JSON API's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own and talks to it with a client that
//! knows only `reef-api`'s types, as the CLI, the MCP server and the hooks will.

// an in-process pool's open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_service::api::Listeners;
use reef_service::{Conf, TlsSettings, api};

/// Expands each case into a test against the twin, and one against an in-process pool
macro_rules! suite {
    ($($case:ident),* $(,)?) => {
        /// Each case against the in-memory twin
        mod memory {
            $(
                #[tokio::test]
                async fn $case() -> anyhow::Result<()> {
                    // a fresh twin with the case's project registered and its plan listed,
                    // served on a port of its own
                    let name = format!("api-{}", stringify!($case));
                    let ax = crate::common::Fx::memory(&name).await?.api().await?;
                    crate::common::api_cases::$case(ax).await
                }
            )*
        }

        /// Each case against shoal, one pool at a time
        #[cfg(feature = "embedded")]
        mod shoal {
            $(
                #[tokio::test(flavor = "multi_thread")]
                async fn $case() -> anyhow::Result<()> {
                    // one pool at a time, over a fresh directory of the case's own
                    let _serial = crate::common::SERIAL.lock().await;
                    let name = format!("api-{}", stringify!($case));
                    let ax = crate::common::Fx::shoal(&name).await?.api().await?;
                    crate::common::api_cases::$case(ax).await
                }
            )*
        }
    };
}

suite!(
    client_puts_a_node_and_lists_plans,
    refused_and_stale_puts_answer_with_their_status,
    errors_say_what_is_missing,
    whole_plan_written_as_a_revision,
    change_stream_tells_of_each_write,
    anchored_doc_link_opens_its_section,
    next_says_what_is_ready_and_what_waits,
    queue_routes_answer_with_their_status,
    page_routes_answer_with_their_status,
    whole_plan_write_over_two_mebibytes_is_taken,
    work_routes_answer_with_their_status,
    comment_routes_answer_with_their_status,
    a_capture_announces_the_threads_it_followed,
    resolves_answer_with_their_status,
    reviews_answer_with_their_status,
    loop_routes_answer_with_their_status,
    a_pushed_doc_change_appears_after_a_fetch,
    remote_registrations_answer_with_their_status,
);

/// The API listens on a loopback address over plain HTTP, and refuses any other, which only TLS
/// serves
#[tokio::test]
async fn server_binds_to_localhost_only() -> anyhow::Result<()> {
    // every address but a loopback one is refused, naming the key that set it
    for address in [
        "0.0.0.0:0",
        "[::]:0",
        "192.168.1.2:0",
        "[::ffff:127.0.0.1]:0",
    ] {
        let mut conf = Conf::default();
        conf.api.address = address.parse()?;
        let refused = api::bind(&conf).await.expect_err(address).to_string();
        assert!(refused.contains("localhost only"), "{address}: {refused}");
        assert!(refused.contains("`api.address`"), "{address}: {refused}");
    }
    // and IPv4's and IPv6's loopback are listened on, each on a port the OS chose
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let mut conf = Conf::default();
        conf.api.address = address.parse()?;
        let listener = api::bind(&conf).await?;
        assert!(listener.local_addr()?.ip().is_loopback(), "{address}");
    }
    // the default is localhost too
    assert!(Conf::default().api.address.ip().is_loopback());
    Ok(())
}

/// TLS is refused with sign-in off, plain HTTP still binds loopback only beside it, and a
/// certificate or key that can't be read is named with its key
#[tokio::test]
async fn tls_needs_sign_in_and_a_certificate_it_can_read() -> anyhow::Result<()> {
    // a certificate, and another's key, in a directory of the case's own
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("api-tls");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let minted = common::tls::minted(&dir.join("ours"))?;
    let other = common::tls::minted(&dir.join("other"))?;
    // a config serving it on every interface, beside loopback, asking callers to sign in
    let conf = |cert: &std::path::Path, key: &std::path::Path, required: bool| {
        let mut conf = Conf::default();
        conf.api.address = "127.0.0.1:0".parse().unwrap();
        conf.auth.required = required;
        conf.api.tls = Some(TlsSettings {
            address: "0.0.0.0:0".parse().unwrap(),
            cert: cert.to_path_buf(),
            key: key.to_path_buf(),
        });
        conf
    };
    // with sign-in off, other hosts would reach a service that answers anyone
    let refused = Listeners::bind(&conf(&minted.cert, &minted.key, false))
        .await
        .err()
        .expect("TLS was served with sign-in off")
        .to_string();
    assert!(refused.contains("`auth.required`"), "{refused}");
    assert!(refused.contains("`api.tls`"), "{refused}");
    // a chain that isn't there, holds no certificate, or isn't its key's, names the files
    let missing = dir.join("missing.pem");
    let garbage = dir.join("garbage.pem");
    std::fs::write(&garbage, "not a certificate\n")?;
    for (cert, key, says) in [
        (&missing, &minted.key, "missing.pem"),
        (&garbage, &minted.key, "holds no PEM certificate"),
        (&minted.cert, &missing, "missing.pem"),
        (&minted.cert, &garbage, "garbage.pem"),
        (&minted.cert, &other.key, "serving the certificate"),
    ] {
        let refused = Listeners::bind(&conf(cert, key, true))
            .await
            .err()
            .expect(says)
            .to_string();
        assert!(refused.contains(says), "{says}: {refused}");
        assert!(refused.contains("`api.tls.cert`"), "{says}: {refused}");
    }
    // plain HTTP beside it still takes loopback alone
    let mut wide = conf(&minted.cert, &minted.key, true);
    wide.api.address = "0.0.0.0:0".parse()?;
    let refused = Listeners::bind(&wide)
        .await
        .err()
        .expect("plain HTTP bound wide");
    assert!(refused.to_string().contains("localhost only"), "{refused}");
    // and a certificate its key signs serves on every interface, beside loopback
    let listeners = Listeners::bind(&conf(&minted.cert, &minted.key, true)).await?;
    let tls = listeners.tls.as_ref().expect("TLS was named");
    assert!(
        axum::serve::Listener::local_addr(tls)?
            .ip()
            .is_unspecified()
    );
    let roots = listeners.roots()?;
    assert!(roots[0].starts_with("http://127.0.0.1:"), "{roots:?}");
    assert!(roots[1].starts_with("https://0.0.0.0:"), "{roots:?}");
    Ok(())
}
