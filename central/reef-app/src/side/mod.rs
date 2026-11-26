//! The sidebar: the selected node's header, then its tabs
//!
//! **Details** is what reef-view's sidebar shows: why and what, acceptance, performance, docs,
//! design choices and the nodes around it. **Changes** is the node's captured revisions, each
//! file's diff drawn once it is opened, with the threads on its lines. **Comments** is the
//! node's threads, general and on its lines, with how many are open. **Decisions** is each plan
//! its sessions asked approval for and each question they asked, and what came of it. **History** is
//! who made each edit of the node, what it changed and when. **Edit** is the node's
//! YAML, saved as `reef node put` saves it. Inside a plan's page, its actions queue the node first or last in its project's
//! queue, or take it off.

mod changes;
mod comments;
mod decisions;
mod details;
pub mod diff;
mod docs;
pub mod history;
mod perf;
pub mod threads;

pub use changes::when;
pub use decisions::picked;

use crate::canvas::{View, dom};
use crate::panel::NodeEditor;
use crate::queue::Queue;
use changes::Changes;
use comments::Comments;
use decisions::Decisions;
use details::Details;
use dioxus::prelude::*;
use history::History;
use reef_api::{Anchor, Place, QueueEdit};
use reef_draw::markup::{status_class, status_label};
use reef_model::PlanName;
use std::collections::HashSet;

/// The sidebar's tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// What the plan says about the node
    Details,
    /// The node's captured revisions, and their files' diffs
    Changes,
    /// The node's comment threads
    Comments,
    /// The plans and questions its sessions asked, and what came of each
    Decisions,
    /// Who made each edit of the node, what it changed and when
    History,
    /// The node's YAML, to edit and save
    Edit,
}

/// What the Changes tab shows of the selected node, kept while another tab is open and let go
/// when another node is selected
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Review {
    /// The revision picked, or `None` for the latest
    pub revision: Signal<Option<u32>>,
    /// The files open, by the path each is shown under
    pub open: Signal<HashSet<String>>,
    /// A line to bring into view once its file is drawn
    pub focus: Signal<Option<Anchor>>,
}

/// The sidebar of the selected node
///
/// # Arguments
///
/// * `id` - The selected node's id
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `changes` - How many changes to the plan have come, which the editor reads the node again
///   on, and each doc's section too
#[component]
pub fn Sidebar(id: String, project: String, plan: String, changes: ReadSignal<u64>) -> Element {
    let mut view = use_context::<View>();
    // the plan's changes, which each doc it shows is read again on
    use_context_provider(|| docs::DocChanges(changes));
    // the project's queue, which a plan's page shares and a sidebar drawn alone lacks
    let queue = try_use_context::<Queue>();
    // what the Changes tab shows, which the tabs share
    let mut review = use_context_provider(|| Review {
        revision: Signal::new(None),
        open: Signal::new(HashSet::new()),
        focus: Signal::new(None),
    });
    // how many of the node's threads are open, which its tab says
    let open_threads = try_use_context::<threads::Threads>().map(|threads| {
        threads
            .of(&id)
            .iter()
            .filter(|thread| thread.is_open_published())
            .count()
    });
    // a newly selected node is read from its top, with none of its files open, while a newer
    // revision keeps the scroll
    let mut aside = use_signal(|| None::<dom::El>);
    let mut seen = use_signal(|| id.clone());
    use_effect(use_reactive!(|id| {
        if let Some(el) = aside.peek().as_ref() {
            dom::scroll_to_top(el);
        }
        if *seen.peek() != id {
            seen.set(id);
            review.revision.set(None);
            review.open.write().clear();
            review.focus.set(None);
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
    let (center_id, edit_id, changes_id, comments_id) =
        (id.clone(), id.clone(), id.clone(), id.clone());
    let (changes_project, changes_plan) = (project.clone(), plan.clone());
    let (decided_project, decided_plan, decided_id) = (project.clone(), plan.clone(), id.clone());
    let (history_project, history_plan, history_id) = (project.clone(), plan.clone(), id.clone());
    // the node in full, as the queue names it, and whether it is queued
    let queued = PlanName::new(project.clone(), plan.clone())
        .ok()
        .and_then(|name| name.node(id.clone()).ok())
        .zip(queue)
        .map(|(node, queue)| (queue.holds(&node), node, queue));
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
            if let Some((held, node, queue)) = queued {
                div { class: "reef-actions reef-app-queue-actions",
                    {
                        let top = node.clone();
                        rsx! {
                            button {
                                class: "reef-button",
                                "data-act": "queue-top",
                                title: "Put it first in the queue, or move it there",
                                onclick: move |_| queue.edit(QueueEdit::Add { node: top.clone(), at: Place::Top }),
                                "Queue first"
                            }
                        }
                    }
                    {
                        let bottom = node.clone();
                        rsx! {
                            button {
                                class: "reef-button",
                                "data-act": "queue-bottom",
                                title: "Put it last in the queue, or move it there",
                                onclick: move |_| queue.edit(QueueEdit::Add { node: bottom.clone(), at: Place::Bottom }),
                                "Queue last"
                            }
                        }
                    }
                    if held {
                        button {
                            class: "reef-button",
                            "data-act": "unqueue",
                            onclick: move |_| queue.edit(QueueEdit::Remove { node: node.clone() }),
                            "Off the queue"
                        }
                    }
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
                    class: tab_class(Tab::Changes),
                    role: "tab",
                    "aria-selected": "{tab == Tab::Changes}",
                    onclick: move |_| view.tab.set(Tab::Changes),
                    "Changes"
                }
                button {
                    class: tab_class(Tab::Comments),
                    role: "tab",
                    "aria-selected": "{tab == Tab::Comments}",
                    onclick: move |_| view.tab.set(Tab::Comments),
                    "Comments"
                    if let Some(open) = open_threads {
                        span { class: if open > 0 { "reef-app-count some" } else { "reef-app-count" }, "{open}" }
                    }
                }
                button {
                    class: tab_class(Tab::Decisions),
                    role: "tab",
                    "aria-selected": "{tab == Tab::Decisions}",
                    "data-tab": "decisions",
                    onclick: move |_| view.tab.set(Tab::Decisions),
                    "Decisions"
                }
                button {
                    class: tab_class(Tab::History),
                    role: "tab",
                    "aria-selected": "{tab == Tab::History}",
                    "data-tab": "history",
                    onclick: move |_| view.tab.set(Tab::History),
                    "History"
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
                Tab::Changes => rsx! {
                    Changes { project: changes_project, plan: changes_plan, id: changes_id }
                },
                Tab::Comments => rsx! {
                    Comments { id: comments_id }
                },
                Tab::Decisions => rsx! {
                    Decisions { project: decided_project, plan: decided_plan, id: decided_id }
                },
                Tab::History => rsx! {
                    History { project: history_project, plan: history_plan, id: history_id, changes }
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
