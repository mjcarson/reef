//! The sidebar's Edit tab: the selected node's YAML to edit, and the problems the plan has at it
//!
//! The Details tab keeps the why, the what, acceptance and performance. This tab holds what the
//! app adds to a node: its spec as YAML, saved as `reef node put` saves it.
//!
//! A save names the version the editor read, so a node that changed since is never written over.
//! While the draft is untouched, each change to the plan reads the node again; once it is edited,
//! the draft stays, and a save after another writer's tells the user and shows the newer version.

use crate::calls::{self, NodeText, Saved};
use dioxus::prelude::*;

/// The selected node's YAML to edit, and its problems
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The selected node's id
/// * `changes` - How many changes to the plan have come, which read the node again
#[component]
pub fn NodeEditor(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    changes: ReadSignal<u64>,
) -> Element {
    // the node as stored, read again when another node is selected or the plan changes
    let text = use_resource(move || async move {
        let _ = changes();
        calls::node_text(project(), plan(), id()).await
    });
    // the text being edited, what it started from, and what came of the last save
    let mut draft = use_signal(String::new);
    let mut base = use_signal(|| None::<NodeText>);
    let mut outcome = use_signal(|| None::<Saved>);
    // take each reading into the draft, unless the draft holds edits to the same node
    use_effect(move || {
        let Some(Ok(read)) = &*text.read() else {
            return;
        };
        let untouched = match &*base.peek() {
            Some(started) => started.id != read.id || *draft.peek() == started.yaml,
            None => true,
        };
        if untouched {
            // another node, or none edited yet: start from what is stored, with no outcome
            if base
                .peek()
                .as_ref()
                .is_some_and(|started| started.id != read.id)
            {
                outcome.set(None);
            }
            draft.set(read.yaml.clone());
            base.set(Some(read.clone()));
        }
    });
    // save the draft over the version it started from
    let save = move |_| async move {
        let Some(started) = base() else {
            return;
        };
        let yaml = draft();
        let saved = calls::save_node(
            project(),
            plan(),
            started.id.clone(),
            yaml.clone(),
            started.version,
        )
        .await;
        match saved {
            Ok(Saved::Applied { version }) => {
                // the draft is what is stored now, so the next reading replaces it
                base.set(Some(NodeText {
                    yaml,
                    version,
                    ..started
                }));
                outcome.set(Some(Saved::Applied { version }));
            }
            Ok(saved) => outcome.set(Some(saved)),
            Err(error) => outcome.set(Some(Saved::Invalid {
                message: error.to_string(),
            })),
        }
    };
    // what the editor shows under the YAML, and whether there is anything to save
    let problems = match &*text.read() {
        Some(Ok(read)) => read.problems.clone(),
        _ => Vec::new(),
    };
    let edited = base().is_some_and(|started| started.yaml != draft());
    rsx! {
        div { class: "reef-app-editor",
            if let Some(Err(error)) = &*text.read() {
                p { class: "reef-app-error", "Reading the node failed: {error}" }
            }
            if !problems.is_empty() {
                ul { class: "reef-app-problems",
                    for problem in problems {
                        li { "{problem}" }
                    }
                }
            }
            textarea {
                class: "reef-app-yaml",
                spellcheck: false,
                aria_label: "The node's YAML",
                value: "{draft}",
                oninput: move |event| draft.set(event.value()),
            }
            div { class: "reef-app-actions",
                button { class: "reef-app-save", disabled: !edited, onclick: save, "Save" }
                button {
                    class: "reef-app-revert",
                    disabled: !edited,
                    onclick: move |_| {
                        if let Some(started) = base() {
                            draft.set(started.yaml);
                        }
                        outcome.set(None);
                    },
                    "Revert"
                }
            }
            Outcome { outcome, draft, base }
        }
    }
}

/// What came of the last save: written, refused, invalid, or stale with the newer version
///
/// # Arguments
///
/// * `outcome` - What came of it, if anything has been saved
/// * `draft` - The text being edited, which a stale save can replace with the newer version's
/// * `base` - What the draft started from, which the newer version replaces
#[component]
fn Outcome(
    outcome: Signal<Option<Saved>>,
    draft: Signal<String>,
    base: Signal<Option<NodeText>>,
) -> Element {
    match outcome() {
        None => rsx! {},
        Some(Saved::Applied { version }) => rsx! {
            p { class: "reef-app-saved reef-app-muted", "Saved, at version {version}." }
        },
        Some(Saved::Refused { problems }) => rsx! {
            div { class: "reef-app-refused",
                p { class: "reef-app-error", "Not saved: the plan would have these problems." }
                ul { class: "reef-app-problems",
                    for problem in problems {
                        li { "{problem}" }
                    }
                }
            }
        },
        Some(Saved::Invalid { message }) => rsx! {
            p { class: "reef-app-error reef-app-invalid", "Not saved: {message}" }
        },
        Some(Saved::Stale { now: None }) => rsx! {
            p { class: "reef-app-error reef-app-stale", "Not saved: the node was removed since you opened it." }
        },
        Some(Saved::Stale { now: Some(now) }) => {
            // the newer version, to read beside the draft or to start again from
            let newer = now.clone();
            rsx! {
                div { class: "reef-app-stale",
                    p { class: "reef-app-error",
                        "Not saved: the node changed since you opened it. It is at version {now.version} now:"
                    }
                    pre { class: "reef-app-newer", "{now.yaml}" }
                    button {
                        class: "reef-app-take",
                        onclick: move |_| {
                            draft.set(newer.yaml.clone());
                            base.set(Some(newer.clone()));
                            outcome.set(None);
                        },
                        "Edit the newer version"
                    }
                }
            }
        }
    }
}
