//! The store's suite, every case run against the in-memory twin and, with `embedded`, shoal
//!
//! Shoal is the twin's oracle: a case that passes on one and fails on the other is a twin that
//! doesn't answer as shoal does.

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
                    let fx = crate::common::Fx::memory(stringify!($case)).await?;
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
    linked_worktree_finds_its_project,
    subdirectory_finds_its_project,
    unregistered_directory_finds_none,
    projects_list_by_name,
    plans_list_by_name,
    plan_of_unknown_project_is_refused,
    queue_of_new_project_is_empty,
    queue_reads_in_the_order_entries_are_placed,
    a_move_writes_one_row,
    interrupted_move_leaves_a_duplicate_dropped_on_read,
    long_ranks_are_rebalanced_in_order,
    killed_rebalance_leaves_the_queue_in_order,
    edits_of_entries_not_queued_are_refused,
    concurrent_edits_both_land,
    each_project_has_its_own_queue,
    queue_deletes_only_the_row_read,
    bulk_write_with_pages_replaces_them,
    bulk_write_without_pages_keeps_them,
    node_batch_leaves_pages_in_place,
    killed_page_write_leaves_old_pages_whole,
    next_write_prunes_killed_pages,
    killed_page_prune_is_finished_by_the_next_put,
    page_edit_lands_in_place,
    stale_page_edit_is_refused,
    page_edit_during_import_waits_and_is_refused,
    pages_it_cant_keep_are_refused,
    page_write_refused_in_place_is_an_error,
    work_of_unstarted_node_is_none,
    start_reads_back_after_a_restart,
    second_start_is_stale_unless_it_names_the_record,
    a_worktree_is_on_its_latest_start,
    captures_add_to_their_own_start,
    refused_and_unkeepable_starts_write_nothing,
    each_project_has_its_own_work,
    captures_are_kept_as_numbered_revisions,
    drafts_are_rewritten_until_sealed,
    killed_capture_serves_none_of_its_files,
    captures_keep_what_fits_and_refuse_what_cant_be_read,
    a_comment_and_its_reply_survive_a_restart,
    a_line_comment_off_its_revision_is_refused,
    marking_over_an_older_version_is_stale,
    deleting_a_commented_line_outdates_its_thread,
    captures_follow_their_nodes_threads,
    drafts_hold_no_thread_until_sealed,
    trailers_resolve_each_thread_once_a_change,
    resolving_by_hand_is_over_the_version_read,
    a_pane_keeps_one_loop,
    two_loops_never_hold_one_node,
    a_clears_session_takes_its_panes_loop,
    a_beaten_loop_write_is_an_error,
    a_loop_acts_on_a_nodes_end_once,
    holding_the_acted_node_again_clears_its_end,
    a_loop_holds_its_group_as_one,
    a_pane_joins_a_member,
);
