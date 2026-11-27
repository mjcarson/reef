//! The Comments tab: the selected node's threads, general ones first with a box to start one,
//! then those on its lines, each with a way to its line in the diff
//!
//! A review open on the node heads the tab, saying how many comments are pending in it, with
//! buttons to submit it, publishing them at once, or discard it. Open threads come before
//! resolved ones. A thread on a line opens the Changes tab at the
//! latest revision its line was followed to, its file open and the line brought into view; an
//! outdated thread's is the last revision that held its line.

use super::threads::{Compose, Composer, NodeReview, ThreadView, Threads, place_label};
use super::{Review, Tab};
use crate::calls;
use crate::canvas::View;
use dioxus::prelude::*;
use reef_api::{ReviewState, Reviewed, Target, Thread};

/// The selected node's comment threads, and a box to start a general one
///
/// # Arguments
///
/// * `id` - The node's id
#[component]
pub fn Comments(id: String) -> Element {
    // the plan's threads, as the canvas read them, which a sidebar drawn alone has none of
    let Some(threads) = try_use_context::<Threads>() else {
        return rsx! {
            p { class: "reef-app-muted", "Comments show inside a plan's page." }
        };
    };
    if let Some(Err(error)) = &*threads.list.read() {
        return rsx! {
            p { class: "reef-app-error", "Reading the comments failed: {error}" }
        };
    }
    // the node's threads, open first, the general ones apart from those on its lines
    let mut on_node = threads.of(&id);
    on_node.sort_by_key(|thread| (!thread.is_open(), thread.number));
    let open = on_node
        .iter()
        .filter(|thread| thread.is_open_published())
        .count();
    let review = threads.review_of(&id);
    let (general, lines): (Vec<Thread>, Vec<Thread>) = on_node
        .into_iter()
        .partition(|thread| thread.target == Target::Node);
    let total = general.len() + lines.len();
    rsx! {
        div { class: "reef-app-comments",
            if let Some(review) = review {
                ReviewBar { key: "{review.number}", id: id.clone(), review }
            }
            p { class: "reef-app-muted reef-app-comments-sum",
                "{open} open of {total} "
                if total == 1 {
                    "thread"
                } else {
                    "threads"
                }
            }
            h3 { class: "reef-app-comments-head", "On the node" }
            for thread in general {
                ThreadView { key: "{thread.number}", thread }
            }
            // keyed by the node, so another node's box starts empty; a key holds only on a
            // block's first node, so the box is one of its own
            {
                rsx! {
                    Composer {
                        key: "{id}",
                        compose: Compose::Thread {
                            id: id.clone(),
                            target: Target::Node,
                        },
                        label: "Comment",
                    }
                }
            }
            if !lines.is_empty() {
                h3 { class: "reef-app-comments-head", "On its lines" }
            }
            for thread in lines {
                LineThread { key: "{thread.number}", thread }
            }
        }
    }
}

/// A thread on a line: where its line is now, a way to that line in the diff, and the thread
///
/// # Arguments
///
/// * `thread` - The thread
#[component]
fn LineThread(thread: Thread) -> Element {
    let mut view = use_context::<View>();
    let review = use_context::<Review>();
    let place = place_label(&thread);
    let anchor = thread.latest_anchor().cloned();
    rsx! {
        div { class: "reef-app-line-thread",
            div { class: "reef-app-line-thread-head",
                code { "{place}" }
                if let Some(anchor) = anchor {
                    button {
                        class: "reef-button reef-app-show-line",
                        onclick: move |_| {
                            // the line's revision, its file open, and the line to bring into view,
                            // its group opened again in the semantic view if it was closed since
                            let Review { mut revision, mut open, mut focus, mut applied, .. } = review;
                            revision.set(Some(anchor.revision));
                            open.write().insert(anchor.path.clone());
                            applied.set(None);
                            focus.set(Some(anchor.clone()));
                            view.tab.set(Tab::Changes);
                        },
                        "Show in diff"
                    }
                }
            }
            ThreadView { thread }
        }
    }
}

/// The node's open review: how many comments are pending in it, and buttons to submit it,
/// publishing them at once, or discard it, over the version read
///
/// # Arguments
///
/// * `id` - The node's id
/// * `review` - Its open review, as read last
#[component]
fn ReviewBar(id: String, review: NodeReview) -> Element {
    let threads = try_use_context::<Threads>();
    // what the last mark answered, if it wrote nothing
    let mut problem = use_signal(|| None::<String>);
    let read = review.version;
    let count = review.comments;
    // marks the review as the button says, then reads everything again
    let mark = move |state: ReviewState| {
        let id = id.clone();
        spawn(async move {
            let Some(threads) = threads else {
                return;
            };
            let (project, plan) = (threads.project.peek().clone(), threads.plan.peek().clone());
            let marked = calls::mark_review(project, plan, id, state, read).await;
            problem.set(match marked {
                Ok(Reviewed::Marked { .. }) => None,
                Ok(Reviewed::Stale { .. }) => Some(
                    "The review changed since it was read, so here it is as it is now.".to_string(),
                ),
                Ok(Reviewed::NoReview { .. }) => Some("The review is no longer open.".to_string()),
                Err(error) => Some(format!("Marking the review failed: {error}")),
            });
            threads.reread();
        });
    };
    let discard = mark.clone();
    rsx! {
        div { class: "reef-app-review-bar", "data-review": "{review.number}",
            span { class: "reef-app-review-sum",
                "Review · {count} pending"
            }
            button {
                class: "reef-button reef-app-review-submit",
                "data-act": "submit",
                onclick: move |_| mark(ReviewState::Submitted),
                "Submit review"
            }
            button {
                class: "reef-button reef-app-review-discard",
                "data-act": "discard",
                onclick: move |_| discard(ReviewState::Discarded),
                "Discard"
            }
            if let Some(problem) = problem.read().clone() {
                p { class: "reef-app-error", "{problem}" }
            }
        }
    }
}
