//! The Comments tab: the selected node's threads, general ones first with a box to start one,
//! then those on its lines, each with a way to its line in the diff
//!
//! Open threads come before resolved ones. A thread on a line opens the Changes tab at that
//! line's revision, its file open and the line brought into view.

use super::threads::{Compose, Composer, ThreadView, Threads, target_label};
use super::{Review, Tab};
use crate::canvas::View;
use dioxus::prelude::*;
use reef_api::{Target, Thread};

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
    let open = on_node.iter().filter(|thread| thread.is_open()).count();
    let (general, lines): (Vec<Thread>, Vec<Thread>) = on_node
        .into_iter()
        .partition(|thread| thread.target == Target::Node);
    let total = general.len() + lines.len();
    rsx! {
        div { class: "reef-app-comments",
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

/// A thread on a line: where it is, a way to that line in the diff, and the thread
///
/// # Arguments
///
/// * `thread` - The thread
#[component]
fn LineThread(thread: Thread) -> Element {
    let mut view = use_context::<View>();
    let review = use_context::<Review>();
    let place = target_label(&thread.target);
    let anchor = thread.target.anchor().cloned();
    rsx! {
        div { class: "reef-app-line-thread",
            div { class: "reef-app-line-thread-head",
                code { "{place}" }
                if let Some(anchor) = anchor {
                    button {
                        class: "reef-button reef-app-show-line",
                        onclick: move |_| {
                            // the line's revision, its file open, and the line to bring into view
                            let Review { mut revision, mut open, mut focus } = review;
                            revision.set(Some(anchor.revision));
                            open.write().insert(anchor.path.clone());
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
