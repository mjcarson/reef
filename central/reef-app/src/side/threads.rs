//! Comment threads: the plan's, read once for the canvas, and each drawn with its replies, a box
//! to reply in and a button to resolve or reopen it
//!
//! The plan's threads are read once, by the canvas, and read again after each comment or mark
//! the plan's stream announces, so the toolbar's list of open threads, the Comments tab and the
//! threads under the diff's lines all show the same. A write reads them again at once too, so
//! its writer sees it without waiting for the stream.
//!
//! A thread on a line is named where its line is in the latest revision it was followed to, and
//! one whose line a later revision lost says it is outdated.
//!
//! A comment may be written in its node's review instead, as a merge request's review is: it is
//! pending, tagged so and shown only here, never resolved, until the review is submitted from the
//! Comments tab, which publishes all of it at once.

use super::when;
use crate::calls;
use crate::live::use_work_changes;
use dioxus::prelude::*;
use reef_api::{Anchor, Commented, Marked, Side, Target, Thread, ThreadState};

/// A node's review, as the plan's open reviews were read last
pub type NodeReview = reef_api::Review;

/// The plan's comment threads, as the canvas read them last, and the plan they are of
#[derive(Clone, Copy, PartialEq)]
pub struct Threads {
    /// The project the plan belongs to
    pub project: ReadSignal<String>,
    /// The plan's name within it
    pub plan: ReadSignal<String>,
    /// Every thread of the plan, by number, those pending in a review included, read again
    /// after each change to them
    pub list: Resource<Result<Vec<Thread>, ServerFnError>>,
    /// The plan's open reviews, read with its threads
    pub reviews: Resource<Result<Vec<NodeReview>, ServerFnError>>,
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

    /// The open review of one node, read last, if it has one
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn review_of(&self, id: &str) -> Option<NodeReview> {
        match &*self.reviews.read() {
            Some(Ok(reviews)) => reviews.iter().find(|review| review.node.id == id).cloned(),
            _ => None,
        }
    }

    /// Reads the threads and reviews again, after a write this page made
    pub fn reread(&self) {
        let mut list = self.list;
        list.restart();
        let mut reviews = self.reviews;
        reviews.restart();
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
    let reviews = use_resource(move || async move {
        let _ = work();
        calls::plan_reviews(project(), plan()).await
    });
    let focus = use_signal(|| None::<u32>);
    use_context_provider(|| Threads {
        project,
        plan,
        list,
        reviews,
        focus,
    })
}

/// Where a thread is, as a list names it: on the node as a whole, or at its line in the latest
/// revision it was followed to, and the revision that lost the line if one did
///
/// # Arguments
///
/// * `thread` - The thread
pub fn place_label(thread: &Thread) -> String {
    // a line's thread is named where its line is now, or was last
    let place = match thread.latest_anchor() {
        Some(anchor) => anchor_label(anchor),
        None => "general".to_string(),
    };
    match thread.outdated {
        Some(revision) => format!("{place} · outdated in r{revision}"),
        None => place,
    }
}

/// The change linked to a thread last, as its foot names it: answering it if it is resolved, or
/// what it was reopened after, or `None` if no change was linked
///
/// A revision of the thread's own node is named by its number alone, and one of another node by
/// that node's id too. A commit whose trailer named the thread follows, by its short id and its
/// summary.
///
/// # Arguments
///
/// * `thread` - The thread
pub fn resolution_label(thread: &Thread) -> Option<String> {
    // the change linked last
    let linked = thread.resolved_by()?;
    let revision = match linked.node == thread.node {
        true => format!("r{}", linked.revision),
        false => format!("{} r{}", linked.node.id, linked.revision),
    };
    let change = match &linked.commit {
        Some(commit) => {
            let short = &commit.commit[..commit.commit.len().min(8)];
            format!("{revision} · {short} {}", commit.summary)
        }
        None => revision,
    };
    // a thread reopened since says what it was reopened after
    Some(match thread.is_open() {
        true => format!("reopened after {change}"),
        false => format!("answered by {change}"),
    })
}

/// Where a line is, as a list names it: its revision, its file and its number
///
/// # Arguments
///
/// * `anchor` - The line
fn anchor_label(anchor: &Anchor) -> String {
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

/// One thread: its comments, rendered, then whether it is outdated, its state, a reply box, a
/// resolve or reopen button, and the change that answered it
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
    let pending = thread.pending;
    let id = thread.node.id.clone();
    let (number, read, outdated) = (thread.number, thread.version, thread.outdated);
    // the change that answered it, its full commit id on hover
    let resolution = resolution_label(&thread);
    let commit = thread
        .resolved_by()
        .and_then(|linked| linked.commit.as_ref())
        .map(|commit| commit.commit.clone())
        .unwrap_or_default();
    let class = format!(
        "reef-app-thread {}{}{}{}",
        if open { "open" } else { "resolved" },
        if focused { " focus" } else { "" },
        if outdated.is_some() { " outdated" } else { "" },
        if pending { " pending" } else { "" },
    );
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
                        if comment.by == reef_api::Writer::Claude {
                            " · Claude"
                        }
                        // a reply pending in a review, in a thread already published
                        if comment.review.is_some() && !pending {
                            " "
                            span {
                                class: "reef-app-tag reef-app-pending",
                                title: "Written in a review not yet submitted",
                                "pending"
                            }
                        }
                    }
                    div {
                        class: "reef-app-comment-body",
                        dangerous_inner_html: "{comment.html}",
                    }
                }
            }
            div { class: "reef-app-thread-foot",
                // a line a later revision lost is said before the state
                if let Some(revision) = outdated {
                    span {
                        class: "reef-app-tag reef-app-outdated",
                        title: "Its line isn't in r{revision}",
                        "outdated"
                    }
                }
                // a thread pending in a review is said so, and can't be marked until it is
                // submitted
                if pending {
                    span {
                        class: "reef-app-tag reef-app-pending",
                        title: "Written in a review not yet submitted",
                        "pending"
                    }
                }
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
                // a thread pending in a review is marked once the review is submitted
                if !pending {
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
                                Ok(Marked::Pending { .. }) => {
                                    Some("It is pending in a review, so it is marked once that is submitted.".to_string())
                                }
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
            }
            if let Some(resolution) = resolution {
                p { class: "reef-app-thread-resolution", title: "{commit}", "{resolution}" }
            }
            if let Some(problem) = problem.read().clone() {
                p { class: "reef-app-error", "{problem}" }
            }
            if *replying.read() {
                Composer {
                    compose: Compose::Reply {
                        thread: number,
                        id: id.clone(),
                        pending,
                    },
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
        /// The id of the node it is on
        id: String,
        /// Whether the thread is pending in a review, so its replies are too
        pending: bool,
    },
}

impl Compose {
    /// The id of the node the comment is on
    pub fn node(&self) -> &str {
        match self {
            Compose::Thread { id, .. } | Compose::Reply { id, .. } => id,
        }
    }
}

/// A box to write a comment in, as markdown, a button to keep it at once, and one to keep it
/// pending in the node's review, starting one if none is open
///
/// A reply in a pending thread is kept only in its review. Once kept, the box empties and the
/// plan's threads are read again.
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
    // whether the node has a review open, and whether this replies in a pending thread, which
    // takes replies only in its review
    let in_review = threads.is_some_and(|threads| threads.review_of(compose.node()).is_some());
    let pending = matches!(compose, Compose::Reply { pending: true, .. });
    // sends the box as it stands, at once or in the node's review
    let send = {
        let compose = compose.clone();
        move |review: bool| {
            let compose = compose.clone();
            spawn(async move {
                let Some(threads) = threads else {
                    return;
                };
                // sent once, as it stands
                sending.set(true);
                let body = text.peek().clone();
                let (project, plan) = (threads.project.peek().clone(), threads.plan.peek().clone());
                let sent = match compose {
                    Compose::Thread { id, target } => {
                        calls::start_thread(project, plan, id, target, body, review).await
                    }
                    Compose::Reply { thread, .. } => {
                        calls::reply_to(project, plan, thread, body, review).await
                    }
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
                    Ok(Commented::NoLine { .. }) => problem.set(Some(
                        "That line isn't in its revision any more.".to_string(),
                    )),
                    Ok(Commented::NotInPlan { .. }) => {
                        problem.set(Some("The node isn't in its plan any more.".to_string()))
                    }
                    Ok(Commented::NoThread { .. }) => {
                        problem.set(Some("The thread is no longer there.".to_string()))
                    }
                    Err(error) => problem.set(Some(format!("Keeping the comment failed: {error}"))),
                }
            });
        }
    };
    let now = send.clone();
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
                // published at once, unless it replies in a pending thread
                if !pending {
                    button {
                        class: "reef-button reef-app-send",
                        disabled: empty || *sending.read(),
                        onclick: move |_| now(false),
                        if in_review {
                            "{label} now"
                        } else {
                            "{label}"
                        }
                    }
                }
                // or kept pending in the node's review, started for it if none is open
                button {
                    class: "reef-button reef-app-review-send",
                    "data-act": "review",
                    disabled: empty || *sending.read(),
                    title: "Pending, shown only here, until the review is submitted",
                    onclick: move |_| send(true),
                    if in_review || pending {
                        "Add to review"
                    } else {
                        "Start a review"
                    }
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
