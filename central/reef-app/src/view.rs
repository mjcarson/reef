//! The plan's graph: reef-view, linked into the app's wasm and mounted into an element Dioxus made
//!
//! The viewer draws the first payload through `reef_view::mount`. Each later payload of the same
//! plan is swapped in where the old one was, keeping its open groups and its selection, and a
//! payload of another plan is drawn from that plan's own view. The handle lives in a signal, so
//! the page going away drops it, which tears the viewer down.

use dioxus::prelude::*;
use reef_model::Payload;
use std::rc::Rc;

/// A plan's payload, with the name of the plan it is of, as `project/plan`
pub type Keyed = (String, Payload);

/// A view reef-view drew, in the client, and the plan it draws now
#[cfg(feature = "web")]
type Drawn = (reef_view::Handle, String);

/// Nothing is drawn on the server
#[cfg(not(feature = "web"))]
type Drawn = ();

/// The plan drawn by reef-view, mounted once and swapped after that
///
/// # Arguments
///
/// * `payload` - The plan's payload, as it is read and read again
/// * `selected` - Set to the id of the node the viewer selects
#[component]
pub fn PlanGraph(
    payload: Resource<Result<Keyed, ServerFnError>>,
    selected: Signal<Option<String>>,
) -> Element {
    // the element Dioxus made for the view, once it is in the page
    let mut element = use_signal(|| None::<Rc<MountedData>>);
    // the drawn view, once the first payload has come
    let handle = use_signal(|| None::<Drawn>);
    // draw each payload as it comes, once the element is there
    use_effect(move || {
        // nothing can be drawn until Dioxus has put the element in the page
        let Some(mounted) = element() else {
            return;
        };
        // a payload that failed or hasn't come yet leaves the view as it is
        if let Some(Ok((plan, payload))) = &*payload.read() {
            draw(&mounted, plan, payload.clone(), handle, selected);
        }
    });
    rsx! {
        div {
            class: "reef reef-page reef-app-graph",
            onmounted: move |event| element.set(Some(event.data())),
        }
        match &*payload.read() {
            Some(Err(error)) => rsx! {
                p { class: "reef-app-error reef-app-over", "Reading the plan failed: {error}" }
            },
            None => rsx! {
                p { class: "reef-app-muted reef-app-over", "Loading the plan…" }
            },
            Some(Ok(_)) => rsx! {},
        }
    }
}

/// What the app shows of every plan: all of it, filling the stage
#[cfg(feature = "web")]
fn view() -> reef_model::ViewSpec {
    reef_model::ViewSpec {
        layout: reef_model::Layout::Page,
        ..reef_model::ViewSpec::default()
    }
}

/// Mounts the viewer the first time, and swaps each payload in after that
///
/// # Arguments
///
/// * `mounted` - The element Dioxus made for the view
/// * `plan` - The plan the payload is of, as `project/plan`
/// * `payload` - The plan to draw
/// * `handle` - The view drawn so far, if any
/// * `selected` - Set to the id of the node the viewer selects
#[cfg(feature = "web")]
fn draw(
    mounted: &MountedData,
    plan: &str,
    payload: Payload,
    mut handle: Signal<Option<Drawn>>,
    mut selected: Signal<Option<String>>,
) {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    // a view drawn already takes the new payload in place, with no reload
    if let Some((drawn, drawing)) = &mut *handle.write() {
        if drawing == plan {
            // a newer revision of the plan it draws, where the old one was
            drawn.swap(payload);
        } else {
            // another plan, from its own view and the groups saved for it
            drawn.swap_plan(payload, view(), plan);
            *drawing = plan.to_string();
        }
        return;
    }
    // the first payload mounts the viewer into Dioxus's element
    let Some(element) = mounted.try_as_web_event() else {
        return;
    };
    let drawn = reef_view::mount(element.unchecked_into(), payload, view(), plan);
    // the app hears whatever the viewer selects
    drawn.on_select(move |id| selected.set(id));
    handle.set(Some((drawn, plan.to_string())));
}

/// Nothing is drawn on the server, where effects never run
#[cfg(not(feature = "web"))]
fn draw(
    _mounted: &MountedData,
    _plan: &str,
    _payload: Payload,
    _handle: Signal<Option<Drawn>>,
    _selected: Signal<Option<String>>,
) {
}
