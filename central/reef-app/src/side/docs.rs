//! A doc read in the sidebar: the section its link points at, read from the page the service
//! renders it as when the sidebar shows it

use crate::canvas::dom::{self, Part};
use dioxus::prelude::*;

/// How many changes to the plan have come, which a doc's section is read again on, since a
/// change can change the page it is cut from
#[derive(Debug, Clone, Copy)]
pub struct DocChanges(pub ReadSignal<u64>);

/// What the sidebar shows of a doc it can't read
const UNREADABLE: &str =
    r#"<p class="reef-note">This doc can't be read here. Use <em>Open page ↗</em>.</p>"#;

/// A doc's section, open, read as it is first shown and again after each change to the plan
///
/// # Arguments
///
/// * `href` - The doc's link
/// * `class` - The `details`' classes
/// * `head` - Its `summary`, which folds it
/// * `part` - What part of the section to show
#[component]
pub fn DocText(href: String, class: &'static str, head: Element, part: Part) -> Element {
    // read as soon as it shows, since it starts open; a browser may fire no `toggle` for that
    let link = href.clone();
    // and again after each change to the plan, which forgets what was read before it
    let seen = try_use_context::<DocChanges>().map_or(0, |changes| (changes.0)());
    let text = use_resource(use_reactive!(|(link, part, seen)| async move {
        let _ = seen;
        match dom::read_doc(link, part).await {
            Ok(html) => html,
            Err(error) => {
                dom::warn(&format!("reef: reading a doc: {error}"));
                UNREADABLE.to_string()
            }
        }
    }));
    let html = text
        .read()
        .clone()
        .unwrap_or_else(|| r#"<p class="reef-note">Loading…</p>"#.to_string());
    rsx! {
        details { class: "{class}", "data-href": "{href}", open: true,
            {head}
            div { class: "reef-doc-text", dangerous_inner_html: "{html}" }
        }
    }
}
