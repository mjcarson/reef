//! Live refresh: a plan's changes, or a project's, followed from the JSON API's change stream
//!
//! The stream is live only, so it says that something changed, not what the plan is now. Each
//! change bumps a counter, and whatever reads the counter reads the plan again: a put or a whole
//! plan written, a gap the stream says it left, and a reconnect, since nothing is sent again
//! after a dropped connection. Each plan has a queue of its own, and whether its entries are
//! ready moves with that plan alone, so the queue follows the plan's stream too.
//!
//! A node's work, its captured revisions and its comments, changes apart from its plan. The plan's stream bumps
//! a counter of its own for those, [`WorkChanges`], so a capture reads the Changes tab again
//! without reading the plan's payload again.
//!
//! A project's runners, their sessions and loops, follow its stream on a counter of their own,
//! so a transcript is read again as each entry comes, without reading the queue or the plan.
//!
//! The front page follows one stream for every project, and counts each change by the project
//! its data names, so a change reads only its own project's plans again ([`ExplorerChanges`]).

use dioxus::prelude::*;
use std::collections::HashMap;

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
    "plan-branched",
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
    /// Whether the stream's first opening bumps it too, for a count whose first read came before
    /// the stream was followed, as a server's render does, so no change between them is lost
    opened: bool,
}

/// The changes that mean reading a plan's queue again: an edit of it, or any change to the plan
/// or the docs it reads, which can make an entry ready or waiting
const QUEUE_KINDS: &[&str] = &[
    "queue-changed",
    "nodes-put",
    "plan-written",
    "plan-branched",
    "project-fetched",
    "missed",
];

/// The changes that mean reading a project's runners again: a session's report or an act on it,
/// a loop changed, or a gap
const SESSION_KINDS: &[&str] = &["session-changed", "loop-changed", "missed"];

/// The changes that mean reading every project again: one registered, or a gap, which may have
/// dropped any project's changes
const EXPLORER_KINDS: &[&str] = &["project-added", "missed"];

/// The changes that mean reading one project's plans summed up again: a plan listed or moved, a
/// fetch, a node or page written, a start, or a loop, session or test run that changed
const SUMMARY_KINDS: &[&str] = &[
    "plan-added",
    "plan-branched",
    "project-fetched",
    "nodes-put",
    "pages-put",
    "plan-written",
    "work-recorded",
    "loop-changed",
    "session-changed",
    "test-run-changed",
];

/// The changes the front page follows on its one stream: those that mean reading every project
/// again, and those that mean reading one project's plans again, by project
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExplorerChanges {
    /// How many changes meant reading every project again: one registered, a gap, the stream's
    /// first opening, since the server read the page before it, and each reconnect
    pub all: ReadSignal<u64>,
    /// How many changes each project has had, by its name
    projects: ReadSignal<HashMap<String, u64>>,
}

impl ExplorerChanges {
    /// How many changes mean reading one project's plans again, its own alone, since a change
    /// that reaches every project reads every one again at once
    ///
    /// # Arguments
    ///
    /// * `project` - The project's name
    pub fn of(&self, project: &str) -> u64 {
        // its own changes, by its name
        self.projects.read().get(project).copied().unwrap_or(0)
    }
}

/// Counts the changes every project and each one has had since the front page opened, on one
/// change stream that names no project, since plain HTTP/1.1 opens six connections a host
///
/// A resource that reads [`ExplorerChanges::of`] reads that project's plans again after each of
/// its changes, without reading any other project's.
pub fn use_explorer_changes() -> ExplorerChanges {
    // how many changes have come to every project and to each, and the stream they come on
    let all = use_signal(|| 0u64);
    let projects = use_signal(HashMap::new);
    let mut stream = use_signal(|| None::<Follow>);
    // follow every project, once
    use_effect(move || {
        let url = reef_api::paths::changes(None, None);
        let counters = [Counter {
            kinds: EXPLORER_KINDS,
            count: all,
            docs: false,
            opened: true,
        }];
        let mut followed = follow(&url, &counters);
        // and count each project's own changes by the project each names
        by_project(followed.as_mut(), SUMMARY_KINDS, projects, all);
        stream.set(followed);
    });
    ExplorerChanges {
        all: all.into(),
        projects: projects.into(),
    }
}

/// Listens on a stream for changes of some kinds, bumping the count of the project each names,
/// or every project's for a change whose data says none
///
/// # Arguments
///
/// * `follow` - The stream, if it opened
/// * `kinds` - The kinds of change to count by project
/// * `projects` - Each project's count, by its name
/// * `all` - Every project's count
#[cfg(feature = "web")]
fn by_project(
    follow: Option<&mut Follow>,
    kinds: &'static [&'static str],
    projects: Signal<HashMap<String, u64>>,
    all: Signal<u64>,
) {
    // nothing to listen on if the stream didn't open
    let Some(follow) = follow else {
        return;
    };
    for kind in kinds {
        let (mut projects, mut all) = (projects, all);
        follow.listen(kind, move |event| match changed_project(&event) {
            // the project the change names reads its plans again
            Some(project) => *projects.write().entry(project).or_default() += 1,
            // and a change that can't be read reaches every project
            None => *all.write() += 1,
        });
    }
}

/// Nothing is followed on the server, where effects never run
#[cfg(not(feature = "web"))]
fn by_project(
    _follow: Option<&mut Follow>,
    _kinds: &'static [&'static str],
    _projects: Signal<HashMap<String, u64>>,
    _all: Signal<u64>,
) {
}

/// The project a change event names, from its data, the change as JSON
///
/// # Arguments
///
/// * `event` - The stream's event
#[cfg(feature = "web")]
fn changed_project(event: &web_sys::Event) -> Option<String> {
    use wasm_bindgen::JsCast;
    // a server-sent event's data is its change, as JSON
    let data = event
        .dyn_ref::<web_sys::MessageEvent>()?
        .data()
        .as_string()?;
    let change: reef_api::Change = serde_json::from_str(&data).ok()?;
    change.project().map(str::to_string)
}

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
            opened: false,
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
                opened: false,
            },
            Counter {
                kinds: WORK_KINDS,
                count: work,
                docs: false,
                opened: false,
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

/// Counts the changes to a plan's queue announced since the page opened, and to the plan and
/// the docs it reads, following its stream again when the route names another plan
///
/// A resource that reads the count reads the plan's queue again after each change.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
pub fn use_queue_changes(project: ReadSignal<String>, plan: ReadSignal<String>) -> ReadSignal<u64> {
    // how many changes have come, and the stream they come on
    let count = use_signal(|| 0u64);
    let mut stream = use_signal(|| None::<Follow>);
    // follow the plan the route names, its queue's changes among them
    use_effect(move || {
        let url = reef_api::paths::changes(Some(&project()), Some(&plan()));
        let counters = [Counter {
            kinds: QUEUE_KINDS,
            count,
            docs: true,
            opened: false,
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
    let first = Rc::new(Cell::new(true));
    follow.listen("open", move |_| {
        // a reconnect bumps every counter, and the first opening those read before it
        let opening = first.replace(false);
        let dropped = dropped.replace(false);
        if dropped {
            crate::canvas::dom::forget_docs();
        }
        for counter in &reconnected {
            if dropped || (opening && counter.opened) {
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
