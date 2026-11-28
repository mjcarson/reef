//! The JSON API's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own and talks to it with a client that
//! knows only `reef-api`'s types, as the CLI, the MCP server and the hooks will.

// an in-process pool's open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_service::api::Listeners;
use reef_service::{Conf, TlsSettings};

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
    a_bad_cursor_is_refused,
    two_plans_follow_two_branches_of_one_remote,
    a_local_plan_reads_its_branch_where_it_is_checked_out_or_at_its_tip,
    a_branch_change_that_breaks_a_link_is_refused,
    each_plans_queue_keeps_its_own_history,
    a_projects_branches_are_listed,
    a_moved_node_takes_everything_with_it,
    a_subtree_moves_whole_and_groups_with_their_members,
    test_run_routes_answer_with_their_status,
    test_runs_are_told_on_the_change_stream,
);

/// A config listening on this host's loopback port alone, over HTTPS, with reef's own
/// certificate kept in a directory of the case's own
///
/// # Arguments
///
/// * `dir` - The case's directory
/// * `address` - The loopback port's address
fn loopback(dir: &std::path::Path, address: &str) -> anyhow::Result<Conf> {
    let mut conf = Conf::default();
    conf.api.address = address.parse()?;
    conf.api.certs = Some(dir.join("certs"));
    Ok(conf)
}

/// A fresh directory of a case's own, under cargo's target dir
///
/// # Arguments
///
/// * `name` - The case's name
fn fresh_dir(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(dir)
}

/// The API listens over HTTPS on a loopback address, and refuses any other, which only
/// `[api.tls]` serves
#[tokio::test]
async fn server_binds_to_localhost_only() -> anyhow::Result<()> {
    let dir = fresh_dir("api-loopback")?;
    // every address but a loopback one is refused, naming the key that set it
    for address in [
        "0.0.0.0:0",
        "[::]:0",
        "192.168.1.2:0",
        "[::ffff:127.0.0.1]:0",
    ] {
        let refused = Listeners::bind(&loopback(&dir, address)?)
            .await
            .err()
            .expect(address)
            .to_string();
        assert!(
            refused.contains("not a loopback address"),
            "{address}: {refused}"
        );
        assert!(refused.contains("`api.address`"), "{address}: {refused}");
    }
    // and IPv4's and IPv6's loopback are listened on over HTTPS, each on a port the OS chose,
    // serving reef's own certificate, made in the directory named
    for address in ["127.0.0.1:0", "[::1]:0"] {
        let listeners = Listeners::bind(&loopback(&dir, address)?).await?;
        let root = &listeners.roots()?[0];
        assert!(root.starts_with("https://"), "{address}: {root}");
        let own = listeners.own.as_ref().expect("reef's own certificate");
        assert_eq!(own.ca, dir.join("certs/ca.pem"));
    }
    // the default is localhost too
    assert!(Conf::default().api.address.ip().is_loopback());
    Ok(())
}

/// Other hosts' listener serves the certificate it names or reef's own, on loopback too, the
/// loopback port still takes loopback alone beside it, and a certificate or key that can't be
/// read is named with its key
#[tokio::test]
async fn tls_serves_the_certificate_it_names_or_its_own() -> anyhow::Result<()> {
    // a certificate, and another's key, in a directory of the case's own
    let dir = fresh_dir("api-tls")?;
    let minted = common::tls::minted(&dir.join("ours"))?;
    let other = common::tls::minted(&dir.join("other"))?;
    // a config serving other hosts at an address, beside loopback
    let conf = |address: &str, cert: Option<&std::path::Path>, key: Option<&std::path::Path>| {
        let mut conf = loopback(&dir, "127.0.0.1:0").unwrap();
        conf.api.tls = Some(TlsSettings {
            address: address.parse().unwrap(),
            cert: cert.map(std::path::Path::to_path_buf),
            key: key.map(std::path::Path::to_path_buf),
        });
        conf
    };
    let ours = || conf("0.0.0.0:0", Some(&minted.cert), Some(&minted.key));
    // a second port on loopback, serving reef's own
    let local = conf("127.0.0.1:0", None, None);
    assert!(Listeners::bind(&local).await?.tls.is_some());
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
        let refused = Listeners::bind(&conf("0.0.0.0:0", Some(cert), Some(key)))
            .await
            .err()
            .expect(says)
            .to_string();
        assert!(refused.contains(says), "{says}: {refused}");
        assert!(refused.contains("`api.tls.cert`"), "{says}: {refused}");
    }
    // a certificate named without its key is refused, naming both keys
    let refused = Listeners::bind(&conf("0.0.0.0:0", Some(&minted.cert), None))
        .await
        .err()
        .expect("a certificate with no key")
        .to_string();
    assert!(refused.contains("together"), "{refused}");
    assert!(refused.contains("`api.tls.key`"), "{refused}");
    // the loopback port beside it still takes loopback alone
    let mut wide = ours();
    wide.api.address = "0.0.0.0:0".parse()?;
    let refused = Listeners::bind(&wide)
        .await
        .err()
        .expect("the loopback port bound wide");
    assert!(
        refused.to_string().contains("not a loopback address"),
        "{refused}"
    );
    // a certificate its key signs serves on every interface, beside loopback, both over HTTPS
    let listeners = Listeners::bind(&ours()).await?;
    let tls = listeners.tls.as_ref().expect("TLS was named");
    assert!(
        axum::serve::Listener::local_addr(tls)?
            .ip()
            .is_unspecified()
    );
    let roots = listeners.roots()?;
    assert!(roots[0].starts_with("https://127.0.0.1:"), "{roots:?}");
    assert!(roots[1].starts_with("https://0.0.0.0:"), "{roots:?}");
    // and with none named, other hosts are served reef's own
    assert!(
        Listeners::bind(&conf("0.0.0.0:0", None, None))
            .await?
            .tls
            .is_some()
    );
    Ok(())
}

/// A config that turns sign-in off is refused as reef-serve starts, naming the key and its file,
/// before anything listens or opens shoal, while one that says it is required reads as before
#[test]
fn a_config_turning_sign_in_off_is_refused_at_start() -> anyhow::Result<()> {
    // a config of the case's own, saying sign-in is off
    let dir = fresh_dir("api-sign-in-off")?;
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("reef.toml");
    let text = format!(
        "[api]\naddress = \"127.0.0.1:0\"\ncerts = \"{}\"\n\n[auth]\nrequired = false\n",
        dir.join("certs").display()
    );
    std::fs::write(&file, text)?;
    // reef-serve refuses to start on it, saying which key in which file to take out
    let started = std::process::Command::new(env!("CARGO_BIN_EXE_reef-serve"))
        .arg("--conf")
        .arg(&file)
        .env_remove(reef_service::conf::CONFIG_VAR)
        .output()?;
    let said = String::from_utf8_lossy(&started.stderr);
    assert!(!started.status.success(), "it started: {said}");
    assert!(
        said.contains(&format!("`auth.required` in {}", file.display())),
        "{said}"
    );
    assert!(said.contains("reef user add"), "{said}");
    // nothing was listened on or opened: no certificate was made
    assert!(!dir.join("certs").exists(), "it made a certificate");
    // and one saying sign-in is required is read, changing nothing
    std::fs::write(&file, "[auth]\nrequired = true\n")?;
    assert_eq!(Conf::load(Some(&file))?.auth.required, Some(true));
    Ok(())
}
