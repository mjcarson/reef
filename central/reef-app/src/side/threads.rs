//! Comment threads: the plan's, read once for the canvas, and each drawn with its replies, a box
//! to reply in and a button to resolve or reopen it
//!
//! The plan's threads are read once, by the canvas, and read again after each comment or mark
//! the plan's stream announces, so the toolbar's list of open threads, the Comments tab and the
//! threads under the diff's lines all show the same. A write reads them again at once too, so
//! its writer sees it without waiting for the stream.

use super::when;
use crate::calls;
use crate::live::use_work_changes;
use dioxus::prelude::*;
use reef_api::{Commented, Marked, Side, Target, Thread, ThreadState};

/// The plan's comment threads, as the canvas read them last, and the plan they are of
#[derive(Clone, Copy, PartialEq)]
pub struct Threads {
    /// The project the plan belongs to
    pub project: ReadSignal<String>,
    /// The plan's name within it
    pub plan: ReadSignal<String>,
    /// Every thread of the plan, by number, read again after each change to them
    pub list: Resource<Result<Vec<Thread>, ServerFnError>>,
    /// A thread to bring into view, once it is drawn
    pub focus: Signal<Option<u32>>,
}

impl Threads {
    /// The threads read last, or none while they are first read or if reading them failed
    pub fn now(&self) -> Vec<Thread> {
        match &*self.list.read() {
            Some(Ok(threads)) => threads.clone(),
            _ => Vec::new(),
        }
    }

    /// The threads on one node, read last
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn of(&self, id: &str) -> Vec<Thread> {
        let mut threads = self.now();
        threads.retain(|thread| thread.node.id == id);
        threads
    }

    /// Reads the threads again, after a write this page made
    pub fn reread(&self) {
        let mut list = self.list;
        list.restart();
    }
}

/// Reads a plan's comment threads, again after each change to them, and provides them to
/// everything inside the caller
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
pub fn use_threads(project: ReadSignal<String>, plan: ReadSignal<String>) -> Threads {
    // read in the client alone, again after each comment or mark announced
    let work = use_work_changes();
    let list = use_resource(move || async move {
        let _ = work();
        calls::plan_threads(project(), plan()).await
    });
    let focus = use_signal(|| None::<u32>);
    use_context_provider(|| Threads {
        project,
        plan,
        list,
        focus,
    })
}

/// Where a thread is, as a list names it: on the node as a whole, or at a line of a revision
///
/// # Arguments
///
/// * `target` - What the thread is on
pub fn target_label(target: &Target) -> String {
    match target {
        Target::Node => "general".to_string(),
        Target::Line(anchor) => {
            // a removed line is numbered in the base
            let side = match anchor.side {
                Side::Old => " (removed)",
                Side::New => "",
            };
            format!(
                "r{} · {}:{}{side}",
                anchor.revision, anchor.path, anchor.line
            )
        }
    }
}

/// The first line of a thread's first comment, cut short to fit a list
///
/// # Arguments
///
/// * `thread` - The thread
pub fn excerpt(thread: &Thread) -> String {
    // the first line with anything on it
    let first = thread
        .comments
        .first()
        .and_then(|comment| comment.body.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or_default()
        .trim();
    match first.char_indices().nth(80) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first.to_string(),
    }
}

/// One thread: its comments, rendered, then its state, a reply box and a resolve or reopen
/// button
///
/// # Arguments
///
/// * `thread` - The thread, as read last
#[component]
pub fn ThreadView(thread: Thread) -> Element {
    let threads = try_use_context::<Threads>();
    // whether its reply box is open, what the last mark answered, and whether this thread is
    // the one to bring into view
    let mut replying = use_signal(|| false);
    let mut problem = use_signal(|| None::<String>);
    let focused = threads.is_some_and(|threads| *threads.focus.read() == Some(thread.number));
    let open = thread.is_open();
    let (number, read) = (thread.number, thread.version);
    let class = match (open, focused) {
        (true, false) => "reef-app-thread open",
        (true, true) => "reef-app-thread open focus",
        (false, false) => "reef-app-thread resolved",
        (false, true) => "reef-app-thread resolved focus",
    };
    rsx! {
        div {
            class,
            "data-thread": "{number}",
            onmounted: move |event| {
                // the thread asked for is brought into view, then let go
                if focused && let Some(el) = crate::canvas::dom::mounted(&event.data()) {
                    crate::canvas::dom::scroll_into_view(&el);
                    if let Some(mut threads) = threads {
                        threads.focus.set(None);
                    }
                }
            },
            for comment in thread.comments.iter() {
                div { class: "reef-app-comment", "data-comment": "{comment.number}",
                    div { class: "reef-app-comment-head",
                        span { class: "reef-app-comment-number", "#{comment.number}" }
                        " · {when(comment.at)}"
                    }
                    div {
                        class: "reef-app-comment-body",
                        dangerous_inner_html: "{comment.html}",
                    }
                }
            }
            div { class: "reef-app-thread-foot",
                span { class: if open { "reef-app-thread-state open" } else { "reef-app-thread-state resolved" },
                    if open {
                        "open"
                    } else {
                        "resolved"
                    }
                }
                if !*replying.read() {
                    button {
                        class: "reef-button reef-app-reply-open",
                        onclick: move |_| replying.set(true),
                        "Reply"
                    }
                }
                button {
                    class: "reef-button reef-app-mark",
                    "data-act": if open { "resolve" } else { "reopen" },
                    onclick: move |_| async move {
                        let Some(threads) = threads else {
                            return;
                        };
                        // marked over the version read, so a change since comes back stale
                        let state = if open { ThreadState::Resolved } else { ThreadState::Open };
                        let marked = calls::mark_thread(
                                threads.project.peek().clone(),
                                threads.plan.peek().clone(),
                                number,
                                state,
                                read,
                            )
                            .await;
                        problem.set(match marked {
                            Ok(Marked::Marked { .. }) => None,
                            Ok(Marked::Stale { .. }) => {
                                Some("It changed since it was read, so here it is as it is now.".to_string())
                            }
                            Ok(Marked::NoThread { .. }) => Some("The thread is no longer there.".to_string()),
                            Err(error) => Some(format!("Marking the thread failed: {error}")),
                        });
                        threads.reread();
                    },
                    if open {
                        "Resolve"
                    } else {
                        "Reopen"
                    }
                }
            }
            if let Some(problem) = problem.read().clone() {
                p { class: "reef-app-error", "{problem}" }
            }
            if *replying.read() {
                Composer {
                    compose: Compose::Reply { thread: number },
                    label: "Reply",
                    on_close: move |_| replying.set(false),
                }
            }
        }
    }
}

/// What a composer writes: a new thread on a node or a line of it, or a reply in a thread
#[derive(Debug, Clone, PartialEq)]
pub enum Compose {
    /// A new thread
    Thread {
        /// The node it is on
        id: String,
        /// What it is on
        target: Target,
    },
    /// A reply in a thread
    Reply {
        /// The thread's number
        thread: u32,
    },
}

/// A box to write a comment in, as markdown, and a button to keep it
///
/// Once kept, the box empties and the plan's threads are read again.
///
/// # Arguments
///
/// * `compose` - What it writes
/// * `label` - What its button says
/// * `on_close` - Called once the comment is kept, or the box cancelled; a box with none stays
///   open and has no Cancel button
#[component]
pub fn Composer(
    compose: Compose,
    label: &'static str,
    on_close: Option<EventHandler<()>>,
) -> Element {
    let threads = try_use_context::<Threads>();
    // what is written, whether it is being sent, and what went wrong if it wasn't kept
    let mut text = use_signal(String::new);
    let mut sending = use_signal(|| false);
    let mut problem = use_signal(|| None::<String>);
    let empty = text.read().trim().is_empty();
    rsx! {
        div { class: "reef-app-composer",
            textarea {
                class: "reef-app-compose",
                placeholder: "Write a comment, in markdown",
                rows: "3",
                value: "{text}",
                oninput: move |event| text.set(event.value()),
            }
            div { class: "reef-app-actions",
                button {
                    class: "reef-button reef-app-send",
                    disabled: empty || *sending.read(),
                    onclick: {
                        let compose = compose.clone();
                        move |_| {
                            let compose = compose.clone();
                            async move {
                                let Some(threads) = threads else {
                                    return;
                                };
                                // sent once, as it stands
                                sending.set(true);
                                let body = text.peek().clone();
                                let (project, plan) = (threads.project.peek().clone(), threads.plan.peek().clone());
                                let sent = match compose {
                                    Compose::Thread { id, target } => {
                                        calls::start_thread(project, plan, id, target, body).await
                                    }
                                    Compose::Reply { thread } => calls::reply_to(project, plan, thread, body).await,
                                };
                                sending.set(false);
                                // kept, it empties and the box closes; otherwise it says why
                                match sent {
                                    Ok(Commented::Added { .. }) => {
                                        text.set(String::new());
                                        problem.set(None);
                                        threads.reread();
                                        if let Some(on_close) = on_close {
                                            on_close.call(());
                                        }
                                    }
                                    Ok(Commented::NoLine { .. }) => {
                                        problem.set(Some("That line isn't in its revision any more.".to_string()))
                                    }
                                    Ok(Commented::NotInPlan { .. }) => {
                                        problem.set(Some("The node isn't in its plan any more.".to_string()))
                                    }
                                    Ok(Commented::NoThread { .. }) => {
                                        problem.set(Some("The thread is no longer there.".to_string()))
                                    }
                                    Err(error) => problem.set(Some(format!("Keeping the comment failed: {error}"))),
                                }
                            }
                        }
                    },
                    "{label}"
                }
                if let Some(on_close) = on_close {
                    button {
                        class: "reef-button reef-app-cancel",
                        onclick: move |_| on_close.call(()),
                        "Cancel"
                    }
                }
            }
            if let Some(problem) = problem.read().clone() {
                p { class: "reef-app-error", "{problem}" }
            }
        }
    }
}
