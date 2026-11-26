//! The authorship suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Each case serves a service on a loopback port of its own, writes as signed-in users, as Claude
//! for them and with no token, and reads back who each write names.

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
                    let name = format!("authors-{}", stringify!($case));
                    let fx = crate::common::Fx::memory(&name).await?;
                    crate::common::author_cases::$case(fx).await
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
                    let name = format!("authors-{}", stringify!($case));
                    let fx = crate::common::Fx::shoal(&name).await?;
                    crate::common::author_cases::$case(fx).await
                }
            )*
        }
    };
}

suite!(
    a_comment_shows_its_author,
    every_write_names_the_tokens_user,
    writes_with_sign_in_off_name_a_token_or_nobody,
    marks_starts_and_captures_name_their_authors,
    registrations_name_their_authors,
    loop_acts_name_their_authors,
    sessions_name_who_attached_them,
    admin_acts_are_logged_for_admins,
);
