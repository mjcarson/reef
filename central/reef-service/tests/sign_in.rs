//! The sign-in suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own, asking every caller to sign in
//! unless the case is about sign-in off, and calls it as the CLI and a browser do.

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
);
