//! The sidebar: the selected node's header, then its tabs
//!
//! **Details** is what reef-view's sidebar shows: why and what, acceptance, performance, docs,
//! design choices and the nodes around it. **Edit** is the node's YAML, saved as `reef node put`
//! saves it. M7's Changes and Comments tabs go beside them.

mod details;
mod docs;
mod perf;

use crate::canvas::{View, dom};
use crate::panel::NodeEditor;
use details::Details;
use dioxus::prelude::*;
use reef_draw::markup::{status_class, status_label};

/// The sidebar's tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// What the plan says about the node
    Details,
    /// The node's YAML, to edit and save
    Edit,
}

/// The sidebar of the selected node
///
/// # Arguments
///
/// * `id` - The selected node's id
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `changes` - How many changes to the plan have come, which the editor reads the node again on
#[component]
pub fn Sidebar(id: String, project: String, plan: String, changes: ReadSignal<u64>) -> Element {
    let mut view = use_context::<View>();
    // a newly selected node is read from its top, while a newer revision keeps the scroll
    let mut aside = use_signal(|| None::<dom::El>);
    use_effect(use_reactive!(|id| {
        let _ = id;
        if let Some(el) = aside.peek().as_ref() {
            dom::scroll_to_top(el);
        }
    }));
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    // a node the plan no longer has shows nothing until the selection lets it go
    let Some(i) = g.index_of(&id) else {
        return rsx! {};
    };
    let n = g.node(i);
    // its groups, outermost first, and whether it is a group drawn open
    let crumbs: Vec<(String, String)> = g
        .ancestors(i)
        .iter()
        .rev()
        .map(|&a| (g.node(a).id.clone(), g.node(a).title.clone()))
        .collect();
    let open = g.is_group(i).then(|| view.expanded.read().contains(&n.id));
    let (center_id, edit_id) = (id.clone(), id.clone());
    let (status, status_text) = (status_class(&g, i), status_label(&g, i));
    let tab = *view.tab.read();
    let tab_class = |t: Tab| {
        if t == tab {
            "reef-app-tab on"
        } else {
            "reef-app-tab"
        }
    };
    rsx! {
        aside {
            class: "reef-side",
            onmounted: move |event| aside.set(dom::mounted(&event.data())),
            onclick: move |event| {
                // a link in markup the sidebar was given, such as a trend's member, goes to its node
                if let Some(id) = dom::target(&*event.data()).and_then(|t| dom::closest(&t, "[data-goto]")).and_then(|a| dom::attr(&a, "data-goto")) {
                    event.prevent_default();
                    view.goto(&id);
                }
            },
            button {
                class: "reef-close",
                "data-act": "close",
                title: "Close (Esc)",
                onclick: move |_| view.select(None),
                "×"
            }
            div { class: "reef-badges",
                span { class: "reef-badge {status}", "{status_text}" }
                if n.optional {
                    span { class: "reef-badge optional", "optional" }
                }
                code { class: "reef-id", "{n.id}" }
            }
            h2 { "{n.title}" }
            if !crumbs.is_empty() {
                div { class: "reef-crumbs",
                    for (k, (cid, ctitle)) in crumbs.into_iter().enumerate() {
                        if k > 0 {
                            " › "
                        }
                        a {
                            href: "javascript:void 0",
                            "data-goto": "{cid}",
                            onclick: move |event| {
                                event.prevent_default();
                                event.stop_propagation();
                                view.goto(&cid);
                            },
                            "{ctitle}"
                        }
                    }
                }
            }
            div { class: "reef-actions",
                if let Some(page) = n.page.clone() {
                    a { class: "reef-button", href: "{page}", "Open page" }
                }
                if let Some(open) = open {
                    button {
                        class: "reef-button",
                        "data-act": "toggle-sel",
                        onclick: move |_| view.toggle_selected(),
                        if open {
                            "Collapse"
                        } else {
                            "Expand"
                        }
                    }
                }
                button {
                    class: "reef-button",
                    "data-act": "center",
                    onclick: move |_| view.goto(&center_id),
                    "Show in diagram"
                }
            }
            nav { class: "reef-app-tabs", role: "tablist",
                button {
                    class: tab_class(Tab::Details),
                    role: "tab",
                    "aria-selected": "{tab == Tab::Details}",
                    onclick: move |_| view.tab.set(Tab::Details),
                    "Details"
                }
                button {
                    class: tab_class(Tab::Edit),
                    role: "tab",
                    "aria-selected": "{tab == Tab::Edit}",
                    onclick: move |_| view.tab.set(Tab::Edit),
                    "Edit"
                }
            }
            match tab {
                Tab::Details => rsx! {
                    Details { i }
                },
                Tab::Edit => rsx! {
                    NodeEditor { project, plan, id: edit_id, changes }
                },
            }
        }
    }
}

/// A sidebar section, open, whose header folds it
///
/// # Arguments
///
/// * `head` - The header's markup
/// * `class` - A class of its own, if any
/// * `children` - Its body
#[component]
pub fn Sec(head: String, class: Option<&'static str>, children: Element) -> Element {
    let class = match class {
        Some(c) => format!("reef-sec {c}"),
        None => "reef-sec".to_string(),
    };
    rsx! {
        details { class: "{class}", open: true,
            summary { class: "reef-sec-head",
                h3 { dangerous_inner_html: "{head}" }
            }
            div { class: "reef-sec-body", {children} }
        }
    }
}
