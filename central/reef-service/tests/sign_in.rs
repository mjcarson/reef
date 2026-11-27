//! The sign-in suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own, asking every caller to sign in
//! unless the case is about sign-in off, over TLS beside it if the case is about TLS, and calls
//! it as the CLI and a browser do.

// an in-process pool's open nests futures past the compiler's default depth
#![recursion_limit = "256"]

mod common;

/// Expands each case into a test against the twin, and one against an in-process pool
macro_rules! suite {
    ($($case:ident),* $(,)?) => {
        /// Each case against the in-memory twin
        mod memory {
            $(
                #[tokio::test]
                async fn $case() -> anyhow::Result<()> {
                    // a fresh twin with the case's project registered and its plan listed
                    let name = format!("sign-in-{}", stringify!($case));
                    let fx = crate::common::Fx::memory(&name).await?;
                    crate::common::sign_in_cases::$case(fx).await
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
                    let name = format!("sign-in-{}", stringify!($case));
                    let fx = crate::common::Fx::shoal(&name).await?;
                    crate::common::sign_in_cases::$case(fx).await
                }
            )*
        }
    };
}

suite!(
    unauthenticated_writes_are_refused,
    unauthenticated_reads_are_refused_and_pages_ask_for_sign_in,
    the_first_user_is_an_admin_and_admins_add_the_rest,
    a_token_signs_requests_in_until_revoked,
    the_browser_signs_in_with_a_cookie,
    sign_in_off_changes_nothing,
    cross_origin_writes_are_refused,
    cross_origin_writes_are_refused_with_sign_in_off,
    signing_in_over_tls_sets_a_secure_cookie,
    tls_serves_http2_to_a_client_that_offers_it,
    plain_http_is_redirected_to_https,
    this_hosts_port_serves_the_cli_over_https_with_sign_in_off,
    only_an_admin_registers_what_is_on_the_host,
    a_revoked_token_signs_nothing_in,
    repeated_wrong_passwords_are_slowed,
    passwords_are_changed_by_their_user_or_set_by_an_admin,
    an_admin_removes_a_user_and_their_tokens,
    tokens_are_listed_and_revoked_by_their_user,
    a_change_stream_ends_when_its_token_is_revoked,
    the_host_recovers_a_lost_admin,
    the_served_client_is_its_build_its_bytes_and_a_header,
    the_client_routes_answer_without_a_token,
    with_no_client_the_route_says_not_found,
);
