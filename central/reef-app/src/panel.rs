//! The node panel: the node the viewer selected, beside the viewer's own sidebar
//!
//! The viewer's sidebar keeps the why, the what, acceptance and performance. This panel holds
//! what the app adds to a node.

use dioxus::prelude::*;
use reef_model::Payload;

/// The selected node, as the plan's payload has it
///
/// # Arguments
///
/// * `payload` - The plan's nodes
/// * `id` - The selected node's id
#[component]
pub fn NodePanel(payload: ReadSignal<Payload>, id: ReadSignal<String>) -> Element {
    // the node as the payload has it, which a newer revision may have dropped
    let node = payload
        .read()
        .nodes
        .iter()
        .find(|node| node.id == id())
        .cloned();
    let Some(node) = node else {
        return rsx! {};
    };
    rsx! {
        aside { class: "reef-app-panel",
            p { class: "reef-app-panel-id", code { "{node.id}" } }
            h2 { class: "reef-app-panel-title", "{node.title}" }
        }
    }
}
