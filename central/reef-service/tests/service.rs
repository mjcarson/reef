//! The service's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Every write through the service is checked against the whole plan, its docs read from the
//! project's working tree, so each case's project is registered for a git repo of its own.

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
                    // a fresh twin with the case's project registered and its plan listed, under
                    // directories apart from the store suite's
                    let name = format!("service-{}", stringify!($case));
                    let fx = crate::common::Fx::memory(&name).await?;
                    crate::common::service_cases::$case(fx.serve()).await
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
                    let name = format!("service-{}", stringify!($case));
                    let fx = crate::common::Fx::shoal(&name).await?;
                    crate::common::service_cases::$case(fx.serve()).await
                }
            )*
        }
    };
}

suite!(
    refused_batch_writes_none_of_it,
    fix_gets_through_on_a_broken_plan,
    half_brief_fix_gets_through,
    batch_adds_a_group_with_its_members,
    missing_doc_is_refused_and_present_doc_accepted,
    link_leaving_the_repo_is_refused,
    stale_put_in_a_batch_writes_nothing,
    batch_it_cant_keep_is_an_error,
    concurrent_puts_that_together_cycle_one_is_refused,
    put_during_checked_import_is_judged_against_it,
    checked_import_refuses_a_broken_plan,
    payload_equals_graph_build_of_stored_nodes,
    put_to_unlisted_plan_is_an_error,
    node_write_refused_in_place_is_an_error,
    killed_batch_leaves_the_plan_as_it_was,
    untouched_nodes_keep_their_versions_through_a_batch,
    removes_are_checked_with_the_batch,
    next_follows_the_queue_across_plans,
    queue_lists_each_entry_with_its_state,
    queue_edit_is_a_change_and_a_refusal_none,
);
