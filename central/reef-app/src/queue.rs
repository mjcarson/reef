//! The queue panel: the plan's order of work, beside it, reordered by dragging
//!
//! - **One queue a plan, outside the canvas's key.** Each plan has a queue of its own, which the
//!   panel shows beside the canvas in the plan's page, read again when the route names another
//!   plan, and the sidebar's buttons reach it through context.
//! - **Followed on the plan's stream.** An entry is ready or waits on its own plan alone, so the
//!   queue is read again after any change to the plan or its queue, not just its own edits.
//! - **A drag is one move.** It writes the one row of the entry moved, placed after the entry
//!   it was dropped below, or at the top. The place is worked out by name from the queue as the
//!   drag began, so a change read mid-drag can't put it elsewhere.
//! - **Dragged with the pointer, as the canvas is**: pointer capture and a movement threshold,
//!   so touch works too, and the arrow keys, Home, End and Delete do the same from the keyboard.

use crate::calls;
use crate::canvas::dom::{self, El};
use crate::live::use_queue_changes;
use dioxus::prelude::*;
use reef_api::{Listed, Place, Placed, QueueEdit, QueueListing, Queued};
use reef_model::next::needs_lines;
use reef_model::{EntryState, NodeName};

/// How far a press must move before it is a drag rather than a click, in CSS pixels
const MIN_DRAG: f64 = 4.0;

/// A plan's queue as the page reads it, shared through context with whatever edits it
#[derive(Clone, Copy)]
pub struct Queue {
    /// The project whose queue it is
    project: ReadSignal<String>,
    /// The queue as last read
    pub listing: Resource<Result<QueueListing, ServerFnError>>,
    /// Why the last edit changed nothing, or failed, until the next
    pub problem: Signal<Option<String>>,
}

impl Queue {
    /// Whether a node is queued, as the queue was last read
    ///
    /// # Arguments
    ///
    /// * `node` - The node
    pub fn holds(&self, node: &NodeName) -> bool {
        match &*self.listing.read() {
            Some(Ok(listing)) => listing.entries.iter().any(|l| l.entry.node == *node),
            _ => false,
        }
    }

    /// Sends an edit, keeps why if it changed nothing, and reads the queue again
    ///
    /// # Arguments
    ///
    /// * `edit` - The edit
    pub fn edit(self, edit: QueueEdit) {
        let (mut problem, mut listing) = (self.problem, self.listing);
        let project = self.project.peek().clone();
        spawn(async move {
            // what came of it, in words for one that changed nothing
            let said = match calls::edit_queue(project, edit).await {
                Ok(Queued::Applied { .. }) => None,
                Ok(Queued::NotQueued { node }) => Some(format!("`{node}` isn't queued")),
                Ok(Queued::NotInPlan { node }) => Some(format!("`{node}` is not in its plan")),
                Err(error) => Some(format!("Editing the queue failed: {error}")),
            };
            problem.set(said);
            // the change stream says so too, but this page needn't wait for it
            listing.restart();
        });
    }
}

/// Reads a plan's queue, again after each change to the plan or its queue, and shares it with
/// the page below
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it, which the route can change under the page
pub fn use_queue(project: ReadSignal<String>, plan: ReadSignal<String>) -> Queue {
    // the plan's changes, any of which can make an entry ready or waiting
    let changes = use_queue_changes(project, plan);
    // read in the client alone, as the payload is, after each change and each plan routed to
    let listing = use_resource(move || async move {
        let _ = changes();
        calls::queue(project(), plan()).await
    });
    let problem = use_signal(|| None::<String>);
    use_context_provider(|| Queue {
        project,
        listing,
        problem,
    })
}

/// A drag of one entry, from where it was pressed
#[derive(Debug, Clone, PartialEq)]
struct Drag {
    /// The pointer dragging it
    pointer: i32,
    /// The entry's place when the drag began
    from: usize,
    /// Where the press began, from the viewport's top
    y0: f64,
    /// Whether it has moved far enough to be a drag
    moved: bool,
    /// The place among the other entries it would go to if let go now
    to: usize,
    /// The queue's nodes as the drag began, which the move is worked out from
    order: Vec<NodeName>,
}

impl Drag {
    /// The edit letting go makes, or `None` for one that leaves the entry where it was
    fn edit(&self) -> Option<QueueEdit> {
        // a press that never moved, or a drop back where it was, changes nothing
        if !self.moved || self.to == self.from {
            return None;
        }
        let node = self.order.get(self.from)?.clone();
        let others: Vec<&NodeName> = self.order.iter().filter(|n| **n != node).collect();
        // after the entry it was dropped below, or at the top
        let at = match self.to.checked_sub(1) {
            Some(above) => Place::After((*others.get(above)?).clone()),
            None => Place::Top,
        };
        Some(QueueEdit::Move { node, at })
    }
}

/// The queue panel: each entry of the plan's queue, its place, its state and why it waits, to
/// drag into order
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan whose queue it is
#[component]
pub fn QueuePanel(project: ReadSignal<String>, plan: ReadSignal<String>) -> Element {
    let queue = use_context::<Queue>();
    let drag = use_signal(|| None::<Drag>);
    let problem = queue.problem.read().clone();
    let dragging = drag.read().as_ref().is_some_and(|d| d.moved);
    let body = match &*queue.listing.read() {
        None => rsx! {
            p { class: "reef-app-muted", "Loading…" }
        },
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error", "Reading the queue failed: {error}" }
        },
        Some(Ok(listing)) if listing.entries.is_empty() => rsx! {
            p { class: "reef-app-muted reef-app-queue-empty",
                "Nothing queued. Queue a node from its sidebar, or with "
                code { "reef queue add <ID>" }
                "."
            }
        },
        Some(Ok(listing)) => {
            // names in full across several plans, and the order a drag is worked out from
            let qualified = listing.plans.len() > 1;
            let order: Vec<NodeName> = listing
                .entries
                .iter()
                .map(|l| l.entry.node.clone())
                .collect();
            rsx! {
                ol { class: if dragging { "reef-app-queue-list dragging" } else { "reef-app-queue-list" },
                    for (k , listed) in listing.entries.iter().cloned().enumerate() {
                        QueueItem {
                            key: "{listed.entry.node}",
                            k,
                            listed,
                            order: order.clone(),
                            qualified,
                            drag,
                        }
                    }
                }
            }
        }
    };
    rsx! {
        aside { class: "reef-app-queue", "aria-label": "The queue of {project}/{plan}",
            h2 { class: "reef-app-queue-head", "Queue" }
            if let Some(problem) = problem {
                p { class: "reef-app-error reef-app-queue-problem", "{problem}" }
            }
            {body}
        }
    }
}

/// One entry of the queue: a grip to drag it by, its place, name, state and title, why it
/// waits, and a button that takes it off
///
/// # Arguments
///
/// * `k` - Its place, from 0
/// * `listed` - The entry
/// * `order` - The queue's nodes, in order
/// * `qualified` - Whether to name it in full, as across several plans
/// * `drag` - The drag under way, if any
#[component]
fn QueueItem(
    k: usize,
    listed: Listed,
    order: Vec<NodeName>,
    qualified: bool,
    drag: Signal<Option<Drag>>,
) -> Element {
    let queue = use_context::<Queue>();
    let mut grip = use_signal(|| None::<El>);
    let entry = listed.entry;
    let node = entry.node.clone();
    let name = match qualified {
        true => node.to_string(),
        false => node.id.clone(),
    };
    let state = entry.state.as_str();
    let lines = details(&node, &entry.state);
    // who put it where it is, if that is known
    let placed = listed.placed.as_ref().and_then(Placed::describe);
    // how the drag under way marks this entry: dragged, or where the dragged one would land
    let marks = drag.read().as_ref().filter(|d| d.moved).map(|d| {
        let others = d.order.len().saturating_sub(1);
        // this entry's place among the others, if it isn't the one dragged
        let among = match k.cmp(&d.from) {
            std::cmp::Ordering::Less => Some(k),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(k - 1),
        };
        match among {
            None => " dragged",
            Some(_) if d.to == d.from => "",
            Some(i) if i == d.to => " drop-before",
            Some(i) if d.to == others && i + 1 == others => " drop-after",
            Some(_) => "",
        }
    });
    let class = format!("reef-app-queue-entry{}", marks.unwrap_or(""));
    // the edits a key makes: a place up or down, to either end, or off
    let keyed = order.clone();
    let off = node.clone();
    let on_key = move |event: KeyboardEvent| {
        let node = keyed[k].clone();
        let at = match event.key() {
            Key::ArrowUp if k == 1 => Place::Top,
            Key::ArrowUp if k > 1 => Place::Before(keyed[k - 1].clone()),
            Key::ArrowDown if k + 1 < keyed.len() => Place::After(keyed[k + 1].clone()),
            Key::Home if k > 0 => Place::Top,
            Key::End if k + 1 < keyed.len() => Place::Bottom,
            Key::Delete => {
                event.prevent_default();
                queue.edit(QueueEdit::Remove { node });
                return;
            }
            _ => return,
        };
        event.prevent_default();
        queue.edit(QueueEdit::Move { node, at });
    };
    rsx! {
        li {
            class: "{class}",
            tabindex: "0",
            "data-node": "{node}",
            "data-state": "{state}",
            onkeydown: on_key,
            span {
                class: "reef-app-queue-grip",
                title: "Drag to reorder",
                "aria-hidden": "true",
                onmounted: move |event| grip.set(dom::mounted(&event.data())),
                onpointerdown: move |event| {
                    // the primary button, or a touch, starts a press on this entry
                    if event.trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                        return;
                    }
                    event.prevent_default();
                    if let Some(el) = grip.peek().as_ref() {
                        dom::capture(el, event.pointer_id());
                    }
                    let y0 = event.client_coordinates().y;
                    drag.set(Some(Drag { pointer: event.pointer_id(), from: k, y0, moved: false, to: k, order: order.clone() }));
                },
                onpointermove: move |event| {
                    let Some(mut held) = drag.peek().clone() else { return };
                    if held.pointer != event.pointer_id() {
                        return;
                    }
                    // a press becomes a drag once it has moved far enough
                    let y = event.client_coordinates().y;
                    held.moved |= (y - held.y0).abs() > MIN_DRAG;
                    // it goes after every other entry whose middle is above the pointer
                    let rows = grip.peek().as_ref().and_then(|el| dom::closest(el, "ol")).map(|list| dom::query_all(&list, "li.reef-app-queue-entry")).unwrap_or_default();
                    held.to = rows.iter().enumerate().filter(|(i, _)| *i != held.from).filter(|(_, row)| {
                        let b = dom::bounds(row);
                        b.top + b.height / 2.0 < y
                    }).count();
                    drag.set(Some(held));
                },
                onpointerup: move |event| released(queue, grip, drag, event.pointer_id()),
                onpointercancel: move |event| released(queue, grip, drag, event.pointer_id()),
                "⠿"
            }
            span { class: "reef-app-queue-place", "{k + 1}." }
            div { class: "reef-app-queue-body",
                div { class: "reef-app-queue-top",
                    code { class: "reef-app-queue-id", "{name}" }
                    span { class: "reef-app-queue-state s-{state}", "{state}" }
                }
                if !entry.title.is_empty() {
                    div { class: "reef-app-queue-title", "{entry.title}" }
                }
                for line in lines {
                    div { class: "reef-app-queue-needs", "{line}" }
                }
                if let Some(placed) = placed {
                    div { class: "reef-app-queue-placed", "{placed}" }
                }
            }
            button {
                class: "reef-app-queue-off",
                "data-act": "unqueue",
                title: "Take off the queue",
                onclick: move |_| queue.edit(QueueEdit::Remove { node: off.clone() }),
                "×"
            }
        }
    }
}

/// An entry let go: the pointer released, and the move made if it was dragged somewhere new
///
/// # Arguments
///
/// * `queue` - The queue
/// * `grip` - The entry's grip, holding the pointer
/// * `drag` - The drag
/// * `pointer` - The pointer let go
fn released(queue: Queue, grip: Signal<Option<El>>, mut drag: Signal<Option<Drag>>, pointer: i32) {
    // only the pointer that pressed ends the drag
    let Some(held) = drag.peek().clone() else {
        return;
    };
    if held.pointer != pointer {
        return;
    }
    drag.set(None);
    if let Some(el) = grip.peek().as_ref() {
        dom::release(el, pointer);
    }
    // one move, which writes the one row of the entry moved
    if let Some(edit) = held.edit() {
        queue.edit(edit);
    }
}

/// The lines under an entry: the needs it waits on, or the members next takes for a group
///
/// # Arguments
///
/// * `node` - The entry's node
/// * `state` - The entry's state
fn details(node: &NodeName, state: &EntryState) -> Vec<String> {
    match state {
        EntryState::Waiting(needs) => needs_lines(needs),
        // a group stands for its ready members, which are named
        EntryState::Ready(ids) if !(ids.len() == 1 && ids[0] == node.id) => {
            vec![format!("members ready: {}", ids.join(", "))]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::Drag;
    use reef_api::{Place, QueueEdit};
    use reef_model::NodeName;

    /// A drag of an entry from one place to another
    ///
    /// # Arguments
    ///
    /// * `from` - Where it was pressed
    /// * `to` - Where among the others it would land
    fn drag(from: usize, to: usize) -> Drag {
        let order: Vec<NodeName> = ["p/a/A", "p/a/B", "p/a/C"]
            .iter()
            .map(|n| n.parse().unwrap())
            .collect();
        Drag {
            pointer: 1,
            from,
            y0: 0.0,
            moved: true,
            to,
            order,
        }
    }

    /// A drop is one move after the entry above where it lands, or to the top, and a drop where
    /// it was, or a press that never moved, is none
    #[test]
    fn a_drop_is_one_move_by_name() {
        let named = |n: &str| -> NodeName { n.parse().unwrap() };
        // C to the top, and A below B
        assert_eq!(
            drag(2, 0).edit(),
            Some(QueueEdit::Move {
                node: named("p/a/C"),
                at: Place::Top
            })
        );
        assert_eq!(
            drag(0, 1).edit(),
            Some(QueueEdit::Move {
                node: named("p/a/A"),
                at: Place::After(named("p/a/B"))
            })
        );
        // A to the bottom, below C
        assert_eq!(
            drag(0, 2).edit(),
            Some(QueueEdit::Move {
                node: named("p/a/A"),
                at: Place::After(named("p/a/C"))
            })
        );
        // back where it was, or never moved
        assert_eq!(drag(1, 1).edit(), None);
        let still = Drag {
            moved: false,
            ..drag(2, 0)
        };
        assert_eq!(still.edit(), None);
    }
}
