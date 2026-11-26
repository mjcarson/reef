//! Live refresh: a plan's changes, or a project's, followed from the JSON API's change stream
//!
//! The stream is live only, so it says that something changed, not what the plan is now. Each
//! change bumps a counter, and whatever reads the counter reads the plan again: a put or a whole
//! plan written, a gap the stream says it left, and a reconnect, since nothing is sent again
//! after a dropped connection. A project's queue spans its plans, and whether an entry is ready
//! moves with any of them, so the queue follows the whole project's stream.
//!
//! A node's work, its captured revisions and its comments, changes apart from its plan. The plan's stream bumps
//! a counter of its own for those, [`WorkChanges`], so a capture reads the Changes tab again
//! without reading the plan's payload again.
//!
//! A project's runners, their sessions and loops, follow its stream on a counter of their own,
//! so a transcript is read again as each entry comes, without reading the queue or the plan.

use dioxus::prelude::*;

/// The plan's change stream, followed in the client
#[cfg(feature = "web")]
struct Follow {
    /// The stream
    source: web_sys::EventSource,
    /// Each event listened for, and the closure it calls
    listeners: Vec<(
        &'static str,
        wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
    )>,
}

#[cfg(feature = "web")]
impl Follow {
    /// Listens for one kind of event on the stream, keeping its closure while the stream is
    /// followed
    ///
    /// # Arguments
    ///
    /// * `kind` - The event's name: a change's kind, or `open` or `error`
    /// * `f` - What to do when it comes
    fn listen(&mut self, kind: &'static str, f: impl FnMut(web_sys::Event) + 'static) {
        use wasm_bindgen::JsCast;
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(f);
        let _ = self
            .source
            .add_event_listener_with_callback(kind, closure.as_ref().unchecked_ref());
        self.listeners.push((kind, closure));
    }
}

#[cfg(feature = "web")]
impl Drop for Follow {
    /// Closes the stream and takes its listeners off, so a page left or a plan changed stops
    /// following it
    fn drop(&mut self) {
        use wasm_bindgen::JsCast;
        // no event comes once the stream is closed, and none calls a dropped closure
        self.source.close();
        for (kind, closure) in &self.listeners {
            let _ = self
                .source
                .remove_event_listener_with_callback(kind, closure.as_ref().unchecked_ref());
        }
    }
}

/// Nothing is followed on the server
#[cfg(not(feature = "web"))]
type Follow = ();

/// The changes that mean reading a plan again: a put, a page edited, a whole plan written, its
/// project's remote fetched to a new commit, or a gap
const PLAN_KINDS: &[&str] = &[
    "nodes-put",
    "pages-put",
    "plan-written",
    "project-fetched",
    "missed",
];

/// The changes that mean reading a node's work again: a start or a capture recorded, a revision
/// captured, a comment kept, a thread marked, threads followed to a new revision, a review
/// marked, decisions kept, or a gap
const WORK_KINDS: &[&str] = &[
    "work-recorded",
    "revision-captured",
    "comment-added",
    "thread-marked",
    "threads-followed",
    "review-marked",
    "decisions-recorded",
    "missed",
];

/// How many changes to the work of a plan's nodes have come since its page opened it, which the
/// plan's page provides to everything inside it
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkChanges(pub ReadSignal<u64>);

/// One counter a stream bumps, and the kinds of change that bump it
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
struct Counter {
    /// The kinds of change that bump it
    kinds: &'static [&'static str],
    /// What to bump
    count: Signal<u64>,
    /// Whether a bump forgets the doc sections read first, since the plan's pages may have
    /// changed under them
    docs: bool,
}

/// The changes that mean reading a project's queue again: an edit of it, a plan listed, or any
/// change to a plan or the docs its plans read, which can make an entry ready or waiting
const PROJECT_KINDS: &[&str] = &[
    "queue-changed",
    "plan-added",
    "nodes-put",
    "plan-written",
    "project-fetched",
    "missed",
];

/// The changes that mean reading a project's runners again: a session's report or an act on it,
/// a loop changed, or a gap
const SESSION_KINDS: &[&str] = &["session-changed", "loop-changed", "missed"];

/// Counts the changes to a project's runners and loops announced since the page opened, following
/// its stream again when the route names another project
///
/// A resource that reads the count reads the sessions, or a transcript, again after each change.
///
/// # Arguments
///
/// * `project` - The project
pub fn use_session_changes(project: ReadSignal<String>) -> ReadSignal<u64> {
    // how many changes have come, and the stream they come on
    let count = use_signal(|| 0u64);
    let mut stream = use_signal(|| None::<Follow>);
    // follow the project the route names, its runners' sessions and loops
    use_effect(move || {
        let url = reef_api::paths::changes(Some(&project()), None);
        let counters = [Counter {
            kinds: SESSION_KINDS,
            count,
            docs: false,
        }];
        stream.set(follow(&url, &counters));
    });
    count.into()
}

/// Counts the changes to a plan announced since the page opened it, following its stream again
/// when the route names another plan
///
/// A resource that reads the count reads the plan again after each change. The changes to its
/// nodes' work are counted apart, and provided to everything inside the page as
/// [`WorkChanges`].
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
pub fn use_changes(project: ReadSignal<String>, plan: ReadSignal<String>) -> ReadSignal<u64> {
    // how many changes have come to the plan and to its nodes' work, and the stream they come on
    let count = use_signal(|| 0u64);
    let work = use_signal(|| 0u64);
    use_context_provider(|| WorkChanges(work.into()));
    let mut stream = use_signal(|| None::<Follow>);
    // follow the plan the route names; the stream it replaces closes as it is dropped
    use_effect(move || {
        let url = reef_api::paths::changes(Some(&project()), Some(&plan()));
        let counters = [
            Counter {
                kinds: PLAN_KINDS,
                count,
                docs: true,
            },
            Counter {
                kinds: WORK_KINDS,
                count: work,
                docs: false,
            },
        ];
        stream.set(follow(&url, &counters));
    });
    count.into()
}

/// How many changes to the work of the plan's nodes have come since its page opened it
///
/// A resource that reads the count reads a node's revisions, or the plan's comments, again after
/// each capture, comment or mark. Outside a plan's page, nothing is followed, and the count
/// stays at 0.
pub fn use_work_changes() -> ReadSignal<u64> {
    // the plan's page's counter, else one nothing bumps
    let none = use_signal(|| 0u64);
    match try_use_context::<WorkChanges>() {
        Some(WorkChanges(work)) => work,
        None => none.into(),
    }
}

/// Counts the changes to a project announced since the page opened, its queue's and every
/// plan's, following its stream again when the route names another project
///
/// A resource that reads the count reads the project's queue again after each change.
///
/// # Arguments
///
/// * `project` - The project
pub fn use_project_changes(project: ReadSignal<String>) -> ReadSignal<u64> {
    // how many changes have come, and the stream they come on
    let count = use_signal(|| 0u64);
    let mut stream = use_signal(|| None::<Follow>);
    // follow the project the route names, every plan of it
    use_effect(move || {
        let url = reef_api::paths::changes(Some(&project()), None);
        let counters = [Counter {
            kinds: PROJECT_KINDS,
            count,
            docs: true,
        }];
        stream.set(follow(&url, &counters));
    });
    count.into()
}

/// Opens a change stream, bumping each counter for each change of its kinds, and every counter
/// for a reconnect
///
/// A counter that follows the plan forgets the doc sections read first, since a change can
/// change the page one is cut from, so a section shown again after it is fetched again.
///
/// # Arguments
///
/// * `url` - The stream's path, narrowed to what is followed
/// * `counters` - What to bump, and the kinds of change that bump each
#[cfg(feature = "web")]
fn follow(url: &str, counters: &[Counter]) -> Option<Follow> {
    use std::cell::Cell;
    use std::rc::Rc;
    // the changes asked for, from the JSON API on this page's own server
    let source = web_sys::EventSource::new(url).ok()?;
    let mut follow = Follow {
        source,
        listeners: Vec::new(),
    };
    // each change of a kind that means reading again, the docs read before it forgotten first
    // where the counter follows the plan
    for counter in counters {
        for kind in counter.kinds {
            let Counter {
                mut count, docs, ..
            } = *counter;
            follow.listen(kind, move |_| {
                if docs {
                    crate::canvas::dom::forget_docs();
                }
                *count.write() += 1;
            });
        }
    }
    // the browser reconnects a dropped stream itself, but what changed meanwhile isn't sent again
    let dropped = Rc::new(Cell::new(false));
    let on_error = dropped.clone();
    follow.listen("error", move |event| {
        use wasm_bindgen::JsCast;
        on_error.set(true);
        // a stream closed for good, rather than reconnecting, was refused, as a lapsed sign-in is
        let closed = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::EventSource>().ok())
            .is_some_and(|source| source.ready_state() == web_sys::EventSource::CLOSED);
        if closed {
            *crate::sign_in::SIGN_IN_LOST.write() = true;
        }
    });
    let reconnected: Vec<Counter> = counters.to_vec();
    follow.listen("open", move |_| {
        if dropped.replace(false) {
            crate::canvas::dom::forget_docs();
            for counter in &reconnected {
                let mut count = counter.count;
                *count.write() += 1;
            }
        }
    });
    Some(follow)
}

/// Nothing is followed on the server, where effects never run
#[cfg(not(feature = "web"))]
fn follow(_url: &str, _counters: &[Counter]) -> Option<Follow> {
    None
}
