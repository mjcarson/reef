//! The node menu: what a right-click on a box or a group offers to do to its node
//!
//! It queues the node first or last in its plan's queue, or takes it off, sets its status, opens
//! its sidebar at a tab, and copies its id. It is HTML inside the view, beside the diagram rather
//! than in it, so the boxes' markup stays what reef-view draws. A backdrop over the window under
//! it takes the next press, wheel or right-click and closes it, so a press outside it never
//! selects or pans.

use super::{View, dom};
use crate::calls::{self, StatusSet};
use crate::queue::Queue;
use crate::side::Tab;
use dioxus::prelude::*;
use reef_api::{Place, QueueEdit};
use reef_model::{PlanName, Status};

/// A node's menu to show: the node, and the pointer it opened at, in client pixels
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// The node's id
    pub id: String,
    /// The pointer's x
    pub x: f64,
    /// The pointer's y
    pub y: f64,
}

/// The statuses the menu sets, each with its label
const STATUSES: [(Status, &str); 4] = [
    (Status::Todo, "To do"),
    (Status::InProgress, "In progress"),
    (Status::Blocked, "Blocked"),
    (Status::Done, "Done"),
];

/// The node menu, beside the pointer it opened at and kept inside the view
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
pub fn NodeMenu(project: String, plan: String) -> Element {
    let mut view = use_context::<View>();
    // the plan's queue, which a plan's page shares and a canvas drawn alone lacks
    let queue = try_use_context::<Queue>();
    // the menu's own size, to keep it inside the view
    let mut size = use_signal(|| (0.0, 0.0));
    // a status write under way, and why the last pick did nothing
    let mut pending = use_signal(|| false);
    let mut problem = use_signal(|| None::<String>);
    // a menu opened afresh, or closed, starts with nothing under way and nothing to say
    use_effect(move || {
        let _ = view.menu.read();
        pending.set(false);
        problem.set(None);
    });
    let Some(menu) = view.menu.read().clone() else {
        return rsx! {};
    };
    // a node the plan no longer has shows no menu
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    let Some(i) = g.index_of(&menu.id) else {
        return rsx! {};
    };
    let n = g.node(i);
    // a group's status is its members', so it has none to set
    let statuses = (!g.is_group(i)).then(|| n.status.unwrap_or_default());
    // the node in full, as the queue names it, and whether it is queued
    let queued = PlanName::new(project.clone(), plan.clone())
        .ok()
        .and_then(|name| name.node(menu.id.clone()).ok())
        .zip(queue)
        .map(|(node, queue)| (queue.holds(&node), node, queue));
    // beside the pointer, or before it where the view ends
    let c = view
        .root
        .peek()
        .as_ref()
        .map(dom::bounds)
        .unwrap_or_default();
    let (w, h) = size();
    let mut x = menu.x - c.left + 2.0;
    let mut y = menu.y - c.top + 2.0;
    if x + w > c.width - 4.0 {
        x = (menu.x - c.left - w - 2.0).max(4.0);
    }
    if y + h > c.height - 4.0 {
        y = (menu.y - c.top - h - 2.0).max(4.0);
    }
    let busy = pending();
    let copy_id = menu.id.clone();
    rsx! {
        div {
            class: "reef-app-menu-backdrop",
            onpointerdown: move |event| {
                // a press outside the menu only closes it
                event.prevent_default();
                event.stop_propagation();
                view.close_menu();
            },
            onwheel: move |_| view.close_menu(),
            oncontextmenu: move |event| {
                // and so does a right-click, which opens no other menu
                event.prevent_default();
                view.close_menu();
            },
        }
        div {
            class: "reef-app-menu",
            role: "dialog",
            "aria-label": "Node menu",
            "data-node": "{menu.id}",
            style: "left: {x:.0}px; top: {y:.0}px",
            onmounted: move |event| {
                // the first of its items takes the keyboard's focus
                if let Some(first) = dom::mounted(&event.data()).and_then(|el| dom::query(&el, "button")) {
                    dom::focus(&first);
                }
            },
            onresize: move |event| {
                if let Ok(s) = event.data().get_border_box_size() {
                    size.set((s.width, s.height));
                }
            },
            onkeydown: move |event| {
                // the canvas's own keys stay out of the menu, and Escape closes it
                event.stop_propagation();
                if event.key() == Key::Escape {
                    event.prevent_default();
                    view.close_menu();
                }
            },
            div { class: "reef-app-menu-head", "{n.title}" }
            if let Some((held, node, queue)) = queued {
                div { class: "reef-app-menu-items",
                    {
                        let top = node.clone();
                        rsx! {
                            button {
                                class: "reef-app-menu-item",
                                "data-act": "menu-queue-top",
                                title: "Put it first in the queue, or move it there",
                                onclick: move |_| {
                                    queue.edit(QueueEdit::Add { node: top.clone(), at: Place::Top });
                                    view.close_menu();
                                },
                                "Queue first"
                            }
                        }
                    }
                    {
                        let bottom = node.clone();
                        rsx! {
                            button {
                                class: "reef-app-menu-item",
                                "data-act": "menu-queue-bottom",
                                title: "Put it last in the queue, or move it there",
                                onclick: move |_| {
                                    queue.edit(QueueEdit::Add { node: bottom.clone(), at: Place::Bottom });
                                    view.close_menu();
                                },
                                "Queue last"
                            }
                        }
                    }
                    if held {
                        button {
                            class: "reef-app-menu-item",
                            "data-act": "menu-unqueue",
                            onclick: move |_| {
                                queue.edit(QueueEdit::Remove { node: node.clone() });
                                view.close_menu();
                            },
                            "Off the queue"
                        }
                    }
                }
            }
            if let Some(current) = statuses {
                div { class: "reef-app-menu-label", "Status" }
                div { class: "reef-app-menu-chips",
                    for (status, label) in STATUSES {
                        {
                            let (project, plan, id) = (project.clone(), plan.clone(), menu.id.clone());
                            rsx! {
                                button {
                                    class: if status == current { "reef-button on" } else { "reef-button" },
                                    "data-act": "menu-status",
                                    "data-status": status.as_str(),
                                    disabled: busy || status == current,
                                    onclick: move |_| set_status(view, (project.clone(), plan.clone(), id.clone()), status, pending, problem),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            div { class: "reef-app-menu-label", "Open" }
            div { class: "reef-app-menu-chips",
                for tab in Tab::ALL {
                    {
                        let id = menu.id.clone();
                        rsx! {
                            button {
                                class: "reef-button",
                                "data-act": "menu-tab",
                                "data-tab": tab.key(),
                                onclick: move |_| {
                                    // the node's sidebar, at the tab picked
                                    view.select(Some(id.clone()));
                                    view.tab.set(tab);
                                    view.close_menu();
                                },
                                "{tab.label()}"
                            }
                        }
                    }
                }
            }
            div { class: "reef-app-menu-items",
                button {
                    class: "reef-app-menu-item",
                    "data-act": "menu-copy",
                    onclick: move |_| {
                        // a page with no clipboard says so, and stays open
                        if dom::copy(&copy_id) {
                            view.close_menu();
                        } else {
                            problem.set(Some("This page can't reach the clipboard".to_string()));
                        }
                    },
                    "Copy id"
                }
            }
            if let Some(said) = problem() {
                p { class: "reef-app-error reef-app-menu-problem", "{said}" }
            }
        }
    }
}

/// Sets a node's status from its menu, which closes once it is written and otherwise says why
/// not
///
/// # Arguments
///
/// * `view` - The view whose menu it is
/// * `node` - The node's project, plan and id
/// * `status` - Where its work now stands
/// * `pending` - Whether a status write is under way
/// * `problem` - Why the write did nothing, once it is answered
fn set_status(
    view: View,
    node: (String, String, String),
    status: Status,
    mut pending: Signal<bool>,
    mut problem: Signal<Option<String>>,
) {
    let (project, plan, id) = node;
    pending.set(true);
    problem.set(None);
    spawn(async move {
        // what came of it, in words for one that wrote nothing
        let said = match calls::set_status(project, plan, id.clone(), status).await {
            Ok(StatusSet::Applied { .. } | StatusSet::Unchanged) => None,
            Ok(StatusSet::Group) => Some(format!(
                "`{id}` is a group, which is done when its members are"
            )),
            Ok(StatusSet::NotInPlan) => Some(format!("`{id}` is not in its plan")),
            Ok(StatusSet::Stale) => Some(format!("`{id}` changed meanwhile: try again")),
            Ok(StatusSet::Refused { problems }) => Some(format!(
                "The plan would have problems: {}",
                problems.join("; ")
            )),
            Err(error) => Some(format!("Setting its status failed: {error}")),
        };
        pending.set(false);
        // written, the plan's stream redraws the box, and the menu has done its work
        match said {
            None => view.close_menu(),
            Some(said) => problem.set(Some(said)),
        }
    });
}
