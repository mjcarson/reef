//! Live refresh: a plan's changes, or a project's, followed from the JSON API's change stream
//!
//! The stream is live only, so it says that something changed, not what the plan is now. Each
//! change bumps a counter, and whatever reads the counter reads the plan again: a put or a whole
//! plan written, a gap the stream says it left, and a reconnect, since nothing is sent again
//! after a dropped connection. A project's queue spans its plans, and whether an entry is ready
//! moves with any of them, so the queue follows the whole project's stream.

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

/// The changes that mean reading a plan again: a put, a page edited, a whole plan written, or
/// a gap
const PLAN_KINDS: &[&str] = &["nodes-put", "pages-put", "plan-written", "missed"];

/// The changes that mean reading a project's queue again: an edit of it, a plan listed, or any
/// change to a plan, which can make an entry ready or waiting
const PROJECT_KINDS: &[&str] = &[
    "queue-changed",
    "plan-added",
    "nodes-put",
    "plan-written",
    "missed",
];

/// Counts the changes to a plan announced since the page opened it, following its stream again
/// when the route names another plan
///
/// A resource that reads the count reads the plan again after each change.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
pub fn use_changes(project: ReadSignal<String>, plan: ReadSignal<String>) -> ReadSignal<u64> {
    // how many changes have come, and the stream they come on
    let count = use_signal(|| 0u64);
    let mut stream = use_signal(|| None::<Follow>);
    // follow the plan the route names; the stream it replaces closes as it is dropped
    use_effect(move || {
        let url = reef_api::paths::changes(Some(&project()), Some(&plan()));
        stream.set(follow(&url, PLAN_KINDS, count));
    });
    count.into()
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
        stream.set(follow(&url, PROJECT_KINDS, count));
    });
    count.into()
}

/// Opens a change stream, bumping `count` for each change of the kinds given, and for a
/// reconnect
///
/// Each bump forgets the doc sections read first, since a change can change the page one is cut
/// from, so a section shown again after it is fetched again.
///
/// # Arguments
///
/// * `url` - The stream's path, narrowed to what is followed
/// * `kinds` - The kinds of change that mean reading again
/// * `count` - What to bump
#[cfg(feature = "web")]
fn follow(url: &str, kinds: &[&'static str], count: Signal<u64>) -> Option<Follow> {
    use std::cell::Cell;
    use std::rc::Rc;
    // the changes asked for, from the JSON API on this page's own server
    let source = web_sys::EventSource::new(url).ok()?;
    let mut follow = Follow {
        source,
        listeners: Vec::new(),
    };
    // each change of a kind that means reading again, the docs read before it forgotten first
    for kind in kinds {
        let mut count = count;
        follow.listen(kind, move |_| {
            crate::canvas::dom::forget_docs();
            *count.write() += 1;
        });
    }
    // the browser reconnects a dropped stream itself, but what changed meanwhile isn't sent again
    let dropped = Rc::new(Cell::new(false));
    let on_error = dropped.clone();
    follow.listen("error", move |_| on_error.set(true));
    let mut reconnected = count;
    follow.listen("open", move |_| {
        if dropped.replace(false) {
            crate::canvas::dom::forget_docs();
            *reconnected.write() += 1;
        }
    });
    Some(follow)
}

/// Nothing is followed on the server, where effects never run
#[cfg(not(feature = "web"))]
fn follow(_url: &str, _kinds: &[&'static str], _count: Signal<u64>) -> Option<Follow> {
    None
}
