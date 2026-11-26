//! The History tab: who made each edit of the node, what it changed and when, newest first
//!
//! - **Read again on each change to the plan,** as the editor reads the node, since an edit of
//!   the node is a change to its plan.
//! - **One line an edit:** when, who if anyone known, the version it left the node at, and what
//!   it changed, as `reef node log` says it.
//! - **A whole plan's write is marked,** as an import's is, so it reads apart from an edit of the
//!   node alone.

use crate::calls;
use crate::side::when;
use dioxus::prelude::*;
use reef_api::Logged;

/// The node's history, read again after each change to its plan
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `changes` - How many changes to the plan have come, which it is read again on
#[component]
pub fn History(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    changes: ReadSignal<u64>,
) -> Element {
    // read in the client alone, after each change to the plan
    let history = use_resource(move || async move {
        let _ = changes();
        calls::node_history(project(), plan(), id()).await
    });
    let read = history.read();
    match &*read {
        None => rsx! {
            p { class: "reef-app-muted", "Loading…" }
        },
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error", "Reading the history failed: {error}" }
        },
        Some(Ok(entries)) => rsx! {
            HistoryList { entries: entries.clone() }
        },
    }
}

/// A history as a list, newest first, a line an edit, or a word that none is kept
///
/// # Arguments
///
/// * `entries` - The edits, newest first
#[component]
pub fn HistoryList(entries: Vec<Logged>) -> Element {
    // nothing kept is said plainly
    if entries.is_empty() {
        return rsx! {
            p { class: "reef-app-muted reef-app-history-none",
                "No edit of this node is kept yet. Each one from now on is, with who made it."
            }
        };
    }
    rsx! {
        ol { class: "reef-app-history",
            for logged in entries.iter() {
                {entry(logged)}
            }
        }
    }
}

/// One edit as a line: when, who, the version it left, and what it changed
///
/// # Arguments
///
/// * `logged` - The edit
fn entry(logged: &Logged) -> Element {
    // who made it, if anyone known did
    let who = logged.author.describe();
    let version = logged.version.map(|version| format!("v{version}"));
    let what = logged.describe();
    rsx! {
        li { class: "reef-app-history-entry", "data-at": "{logged.at}",
            div { class: "reef-app-history-head",
                span { class: "reef-app-muted reef-app-history-when", "{when(logged.at)}" }
                if let Some(who) = who {
                    " · "
                    span { class: "reef-app-author", "{who}" }
                }
                if let Some(version) = version {
                    " · "
                    span { class: "reef-app-history-version", "{version}" }
                }
                if logged.whole_plan {
                    " "
                    span {
                        class: "reef-app-tag reef-app-whole-plan",
                        title: "Written with the whole plan at once, as an import is",
                        "whole plan"
                    }
                }
            }
            div { class: "reef-app-history-what", "{what}" }
        }
    }
}
