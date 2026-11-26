//! The Timeline tab: everything that happened to the node, newest first, in one list
//!
//! - **One list from four sources.** Its edits and starts come from its history, its captures
//!   from its revisions, and its comments, thread marks and resolutions from its threads, merged
//!   by when each happened. A start a reef before C90 recorded is in no history, so the node's
//!   work record adds its latest start where no `started` entry names it.
//! - **Paged as History is.** Edits are read 200 at a time, from the newest each time; while
//!   older ones are kept, whatever happened before the oldest edit read is left out too, so the
//!   list never shows an older capture or comment above a gap of edits. "Older" reads 200 more.
//! - **Read again on any change to the node.** A plan change, a capture, a comment or a start
//!   reads it again, so each shows live.
//! - **A capture or a comment opens its tab.** A capture's line opens Changes at its revision, and
//!   a comment's opens Comments. Pending review comments are left out, as Claude's reads leave
//!   them out.

use crate::calls;
use crate::canvas::View;
use crate::live::use_work_changes;
use crate::side::history::OlderButton;
use crate::side::{Review, Tab, when};
use dioxus::prelude::*;
use reef_api::{
    Author, Edited, HISTORY_MAX, HistoryPage, Logged, Revision, Thread, ThreadState, WorkRecord,
};
use reef_model::NodeName;
use serde::{Deserialize, Serialize};

/// The most characters of a comment's first line the timeline shows
const EXCERPT_MAX: usize = 120;

/// What happened to a node, at one moment
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Happened {
    /// An edit of the node, or a start of its work, as its history kept it
    Edited(Logged),
    /// A capture of its changes, kept as a revision, or a draft's latest rewrite
    Captured {
        /// The revision's number
        revision: u32,
        /// Whether it is the node's draft, which the next capture rewrites
        draft: bool,
        /// How many files it changed
        files: usize,
        /// How many lines it added
        added: u32,
        /// How many lines it removed
        removed: u32,
    },
    /// A comment on the node: a thread's first, or a reply in one
    Commented {
        /// The thread's number
        thread: u32,
        /// Whether it replies in the thread, rather than starting it
        reply: bool,
        /// Its body's first line, cut short if it is long
        excerpt: String,
    },
    /// A thread marked open or resolved by hand
    Marked {
        /// The thread's number
        thread: u32,
        /// The state it was marked
        state: ThreadState,
    },
    /// A thread linked to the change that answered it
    Resolved {
        /// The thread's number
        thread: u32,
        /// The node whose revision answered it, which may be another
        node: NodeName,
        /// That revision's number
        revision: u32,
        /// The summary of the commit whose trailer named the thread, if one did
        commit: Option<String>,
    },
}

/// One thing that happened to a node: when, who did it, and what it was
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moment {
    /// When, in milliseconds since the epoch
    pub at: u64,
    /// Who did it
    pub author: Author,
    /// What happened
    pub happened: Happened,
}

impl Moment {
    /// The kind of moment it is, as its entry's `data-kind` names it
    pub fn kind(&self) -> &'static str {
        match &self.happened {
            Happened::Edited(logged) if is_start(logged) => "start",
            Happened::Edited(_) => "edit",
            Happened::Captured { .. } => "capture",
            Happened::Commented { .. } => "comment",
            Happened::Marked { .. } => "mark",
            Happened::Resolved { .. } => "resolution",
        }
    }

    /// What happened, as a reader is told
    pub fn describe(&self) -> String {
        match &self.happened {
            Happened::Edited(logged) => logged.describe(),
            Happened::Captured {
                revision,
                draft,
                files,
                added,
                removed,
            } => {
                // a draft is said to be one, then what the revision changed
                let verb = match draft {
                    true => "drafted",
                    false => "captured",
                };
                let noun = match files {
                    1 => "file",
                    _ => "files",
                };
                format!("{verb} revision {revision}: {files} {noun} +{added} −{removed}")
            }
            Happened::Commented {
                thread,
                reply,
                excerpt,
            } => match reply {
                true => format!("replied in thread #{thread}: {excerpt}"),
                false => format!("commented in thread #{thread}: {excerpt}"),
            },
            Happened::Marked { thread, state } => match state {
                ThreadState::Open => format!("reopened thread #{thread}"),
                ThreadState::Resolved => format!("resolved thread #{thread}"),
            },
            Happened::Resolved {
                thread,
                node,
                revision,
                commit,
            } => {
                // the revision that answered it, then the commit that named it, if one did
                let mut said =
                    format!("resolved thread #{thread} by revision {revision} of {node}");
                if let Some(commit) = commit {
                    said.push_str(&format!(": {commit}"));
                }
                said
            }
        }
    }
}

/// A node's timeline: what happened to it, newest first, and whether older edits are kept
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTimeline {
    /// What happened, newest first
    pub moments: Vec<Moment>,
    /// Whether edits older than these are kept, which "Older" reads
    pub older: bool,
}

/// Whether a history entry is a start of the node's work
///
/// # Arguments
///
/// * `logged` - The entry
fn is_start(logged: &Logged) -> bool {
    logged
        .edits
        .iter()
        .any(|edit| matches!(edit, Edited::Started { .. }))
}

/// A comment's first line with any text, cut short if it is long
///
/// # Arguments
///
/// * `body` - The comment's markdown
fn excerpt(body: &str) -> String {
    // the first line that says anything
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    // cut at a character, not a byte, and marked as cut
    match line.chars().count() > EXCERPT_MAX {
        true => format!("{}…", line.chars().take(EXCERPT_MAX).collect::<String>()),
        false => line.to_string(),
    }
}

/// Merges what happened to a node into one timeline, newest first
///
/// While older edits are kept, a moment older than the oldest edit read is left out, so nothing
/// shows above a gap. A start in the work record that no history entry names is added, as a
/// start a reef before C90 recorded.
///
/// # Arguments
///
/// * `page` - The node's history, as many pages of it as were read
/// * `work` - The node's work record, if it was started
/// * `revisions` - Its revisions, in any order
/// * `threads` - Its threads, pending review comments in them left out
pub fn merge(
    page: HistoryPage,
    work: Option<WorkRecord>,
    revisions: Vec<Revision>,
    threads: Vec<Thread>,
) -> NodeTimeline {
    let older = page.older.is_some();
    // the oldest moment shown, while older edits are kept, is the oldest edit read
    let cutoff = match older {
        true => page.edits.iter().map(|logged| logged.at).min(),
        false => None,
    };
    // the record's start, unless the history names it already
    let named = |started: u64| {
        page.edits.iter().any(|logged| {
            logged.edits.iter().any(
                |edit| matches!(edit, Edited::Started { started_at, .. } if *started_at == started),
            )
        })
    };
    let mut moments: Vec<Moment> = Vec::new();
    if let Some(work) = work
        && !named(work.started_at)
    {
        let started = Logged {
            at: work.started_at,
            author: work.started_by.clone(),
            version: None,
            whole_plan: false,
            edits: vec![Edited::Started {
                started_at: work.started_at,
                branch: work.start.branch.clone(),
                start: work.start.start.clone(),
                again: false,
            }],
        };
        moments.push(Moment {
            at: started.at,
            author: started.author.clone(),
            happened: Happened::Edited(started),
        });
    }
    // every edit and start the history kept
    for logged in page.edits {
        moments.push(Moment {
            at: logged.at,
            author: logged.author.clone(),
            happened: Happened::Edited(logged),
        });
    }
    // every revision captured
    for revision in revisions {
        let (added, removed) = revision.counts();
        moments.push(Moment {
            at: revision.at,
            author: revision.author.clone(),
            happened: Happened::Captured {
                revision: revision.revision,
                draft: revision.draft,
                files: revision.files.len(),
                added,
                removed,
            },
        });
    }
    // and each thread's published comments, marks and resolutions
    for thread in threads.into_iter().filter(|thread| !thread.pending) {
        for (index, comment) in thread.comments.iter().enumerate() {
            // a reply pending in a review isn't published yet
            if comment.review.is_some() {
                continue;
            }
            moments.push(Moment {
                at: comment.at,
                author: comment.author.clone(),
                happened: Happened::Commented {
                    thread: thread.number,
                    reply: index > 0,
                    excerpt: excerpt(&comment.body),
                },
            });
        }
        for mark in &thread.marks {
            moments.push(Moment {
                at: mark.at,
                author: mark.author.clone(),
                happened: Happened::Marked {
                    thread: thread.number,
                    state: mark.state,
                },
            });
        }
        for resolution in &thread.resolutions {
            moments.push(Moment {
                at: resolution.at,
                author: resolution.author.clone(),
                happened: Happened::Resolved {
                    thread: thread.number,
                    node: resolution.node.clone(),
                    revision: resolution.revision,
                    commit: resolution
                        .commit
                        .as_ref()
                        .map(|commit| commit.summary.clone()),
                },
            });
        }
    }
    // nothing older than the oldest edit read, while older edits are kept
    if let Some(cutoff) = cutoff {
        moments.retain(|moment| moment.at >= cutoff);
    }
    // newest first, the order each source gave kept among moments of one time
    moments.sort_by_key(|moment| std::cmp::Reverse(moment.at));
    NodeTimeline { moments, older }
}

/// The node's timeline, read again after each change to its plan or its work
///
/// # Arguments
///
/// * `project` - The project the node's plan belongs to
/// * `plan` - The plan's name within it
/// * `id` - The node's id
/// * `changes` - How many changes to the plan have come, which it is read again on
#[component]
pub fn Timeline(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    id: ReadSignal<String>,
    changes: ReadSignal<u64>,
) -> Element {
    let mut view = use_context::<View>();
    let review = use_context::<Review>();
    // captures, comments and starts, which change no plan
    let work = use_work_changes();
    // how many edits to show, and of which node, so another node starts at its newest again
    let mut shown = use_signal(|| (id(), HISTORY_MAX));
    let count = move || {
        let (node, count) = shown();
        if node == id() { count } else { HISTORY_MAX }
    };
    // read in the client alone, after each change, or an ask for older edits
    let timeline = use_resource(move || async move {
        let _ = (changes(), work());
        calls::node_timeline(project(), plan(), id(), count()).await
    });
    let read = timeline.read();
    match &*read {
        None => rsx! {
            p { class: "reef-app-muted", "Loading…" }
        },
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error", "Reading the timeline failed: {error}" }
        },
        Some(Ok(timeline)) => rsx! {
            TimelineList {
                moments: timeline.moments.clone(),
                on_capture: move |revision: u32| {
                    // the Changes tab, at that revision
                    let mut picked = review.revision;
                    picked.set(Some(revision));
                    view.tab.set(Tab::Changes);
                },
                on_comment: move |_thread: u32| view.tab.set(Tab::Comments),
            }
            if timeline.older {
                OlderButton { on_older: move |_| shown.set((id(), count() + HISTORY_MAX)) }
            }
        },
    }
}

/// A timeline as a list, newest first, a line a moment, or a word that nothing is kept
///
/// # Arguments
///
/// * `moments` - What happened, newest first
/// * `on_capture` - Opens a capture's revision, by its number
/// * `on_comment` - Opens a comment's thread, by its number
#[component]
pub fn TimelineList(
    moments: Vec<Moment>,
    on_capture: Option<EventHandler<u32>>,
    on_comment: Option<EventHandler<u32>>,
) -> Element {
    // nothing kept is said plainly
    if moments.is_empty() {
        return rsx! {
            p { class: "reef-app-muted reef-app-timeline-none",
                "Nothing has happened to this node that reef kept yet."
            }
        };
    }
    rsx! {
        ol { class: "reef-app-timeline",
            for moment in moments.iter() {
                {entry(moment, on_capture, on_comment)}
            }
        }
    }
}

/// One moment as a line: when, who, its kind, and what happened, a capture's and a comment's
/// opening their tab
///
/// # Arguments
///
/// * `moment` - The moment
/// * `on_capture` - Opens a capture's revision
/// * `on_comment` - Opens a comment's thread
fn entry(
    moment: &Moment,
    on_capture: Option<EventHandler<u32>>,
    on_comment: Option<EventHandler<u32>>,
) -> Element {
    // who did it, if anyone known did, and what
    let who = moment.author.describe();
    let kind = moment.kind();
    let what = moment.describe();
    // the version an edit left, if it left one
    let version = match &moment.happened {
        Happened::Edited(logged) => logged.version.map(|version| format!("v{version}")),
        _ => None,
    };
    // what a click opens: a capture's revision, or a comment's thread
    let opens = match &moment.happened {
        Happened::Captured { revision, .. } => on_capture.map(|open| (open, *revision)),
        Happened::Commented { thread, .. } => on_comment.map(|open| (open, *thread)),
        _ => None,
    };
    rsx! {
        li {
            class: "reef-app-timeline-entry {kind}",
            "data-kind": "{kind}",
            "data-at": "{moment.at}",
            div { class: "reef-app-timeline-head",
                span { class: "reef-app-muted reef-app-timeline-when", "{when(moment.at)}" }
                if let Some(who) = who {
                    " · "
                    span { class: "reef-app-author", "{who}" }
                }
                if let Some(version) = version {
                    " · "
                    span { class: "reef-app-history-version", "{version}" }
                }
                " "
                span { class: "reef-app-tag reef-app-timeline-kind", "{kind}" }
            }
            match opens {
                Some((open, number)) => rsx! {
                    button {
                        class: "reef-app-timeline-what reef-app-timeline-open",
                        r#type: "button",
                        onclick: move |_| open.call(number),
                        "{what}"
                    }
                },
                None => rsx! {
                    div { class: "reef-app-timeline-what", "{what}" }
                },
            }
        }
    }
}
