//! The store's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Shoal is the twin's oracle: a case that passes on one and fails on the other is a twin that
//! doesn't answer as shoal does.

mod common;

/// Expands each case into a test against the twin, and one against an in-process pool
macro_rules! suite {
    ($($case:ident),* $(,)?) => {
        /// Each case against the in-memory twin
        mod memory {
            $(
                #[tokio::test]
                async fn $case() -> anyhow::Result<()> {
                    // a fresh twin with the case's plan listed
                    let fx = crate::common::Fx::memory().await?;
                    crate::common::cases::$case(fx).await
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
                    let fx = crate::common::Fx::shoal(stringify!($case)).await?;
                    crate::common::cases::$case(fx).await
                }
            )*
        }
    };
}

suite!(
    put_then_get,
    get_of_missing_node_is_none,
    list_of_new_plan_is_empty,
    list_is_in_written_order,
    stale_put_is_refused,
    new_node_over_existing_is_refused,
    put_naming_missing_node_is_refused,
    concurrent_puts_from_one_version_one_wins,
    put_to_unlisted_plan_is_an_error,
    nested_members_are_refused,
    duplicate_plan_is_refused,
    bulk_write_replaces_the_revision,
    bulk_write_moves_versions_on,
    bulk_write_refuses_what_it_cant_keep,
    killed_write_leaves_old_revision_whole,
    next_write_prunes_what_a_killed_write_left,
    killed_prune_is_finished_by_the_next_put,
    put_during_bulk_write_waits_and_is_refused,
    deleting_a_missing_row_deletes_nothing,
);
