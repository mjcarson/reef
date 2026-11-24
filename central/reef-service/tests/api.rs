//! The JSON API's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own and talks to it with a client that
//! knows only `reef-api`'s types, as the CLI, the MCP server and the hooks will.

// an in-process pool's open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

use reef_service::{Conf, api};

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

/// The API listens on a loopback address, and refuses any other until users sign in
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
