//! The in-browser viewer for reef planning graphs.
//!
//! The preprocessor turns each ```` ```reef ```` fence into a `div.reef` carrying its view, and
//! puts the plan's nodes in one `script.reef-data` on the page. This finds them, lays each view
//! out, and draws it as SVG with pan, zoom, collapsible groups, search and a sidebar.
//!
//! An app that links the viewer into its own wasm turns off the `autostart` feature and draws a
//! plan into an element of its own with [`mount`], swapping in new revisions through the
//! [`Handle`] it returns.
//!
//! What it draws (box sizes, the layout, the markup, the criteria and the charts) comes from
//! reef-draw, which reef-app's Dioxus canvas draws from too; this crate is the DOM around it.

mod chartui;
mod docs;
mod viewer;

use reef_model::{Graph, Payload, ViewSpec};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

/// The plan as the viewer holds it.
pub struct Plan {
    pub graph: Graph,
}

#[cfg(feature = "autostart")]
#[wasm_bindgen(start)]
pub fn start() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    if doc.ready_state() == "loading" {
        let cb = Closure::<dyn FnMut()>::new(mount_all);
        let _ = doc.add_event_listener_with_callback("DOMContentLoaded", cb.as_ref().unchecked_ref());
        cb.forget();
    } else {
        mount_all();
    }
}

/// Draw every `div.reef` on the page that is not drawn yet.
#[wasm_bindgen]
pub fn mount_all() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let Ok(divs) = doc.query_selector_all("div.reef:not(.reef-mounted)") else { return };
    if divs.length() == 0 {
        return;
    }
    let plan = match load_plan(&doc) {
        Ok(p) => Rc::new(p),
        Err(e) => {
            web_sys::console::error_1(&format!("reef: {e}").into());
            return;
        }
    };
    // each view's open groups are kept under its page and its place on it
    let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_default();
    for k in 0..divs.length() {
        let Some(el) = divs.item(k).and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok()) else { continue };
        let spec: ViewSpec = match el.get_attribute("data-view").map(|v| serde_json::from_str(&v)) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                el.set_inner_text(&format!("reef: bad view: {e}"));
                continue;
            }
            None => ViewSpec::default(),
        };
        // a book's views last as long as its page, so their wiring is never taken away
        let (_, wiring) = viewer::Viewer::mount(el, plan.clone(), spec, k as usize, format!("reef:{path}:{k}"));
        std::mem::forget(wiring);
    }
}

fn load_plan(doc: &web_sys::Document) -> Result<Plan, String> {
    let script = doc
        .query_selector("script.reef-data")
        .ok()
        .flatten()
        .ok_or("this page has no reef data (is mdbook-reef in [preprocessor]?)")?;
    let text = script.text_content().unwrap_or_default();
    let payload: Payload = serde_json::from_str(&text).map_err(|e| format!("reading the plan: {e}"))?;
    Ok(plan_of(payload))
}

/// Builds the plan a payload holds, warning in the console about anything wrong with it
///
/// # Arguments
///
/// * `payload` - The plan's nodes, as the preprocessor or a server wrote them
fn plan_of(payload: Payload) -> Plan {
    // check the nodes as one graph
    let (graph, diags) = Graph::build(payload.nodes);
    // a problem doesn't stop the plan drawing, so report it and go on
    for d in diags {
        web_sys::console::warn_1(&format!("reef: {d}").into());
    }
    Plan { graph }
}

/// Draws a plan into an element the caller owns, in place of whatever it holds
///
/// The element should be a `div` with the `reef` class (and `reef-page` for a view that fills
/// the page), and a height, as the preprocessor writes a fence. The view's open groups are kept
/// in local storage under the plan's name, so each plan drawn on a page keeps its own.
///
/// # Arguments
///
/// * `el` - The element to draw into
/// * `payload` - The plan's nodes, their doc links absolute or relative to the page
/// * `view` - What the view shows: its root or focus, direction and which groups start open
/// * `plan` - The plan's name, such as `reef/central`, which its saved open groups are kept under
///
/// # Examples
///
/// ```no_run
/// use reef_model::{Payload, ViewSpec};
/// use wasm_bindgen::JsCast;
///
/// let doc = web_sys::window().unwrap().document().unwrap();
/// let el = doc.get_element_by_id("plan").unwrap().unchecked_into();
/// let handle = reef_view::mount(el, Payload::default(), ViewSpec::default(), "reef/central");
/// // hear which node is selected
/// handle.on_select(|id| web_sys::console::log_1(&format!("{id:?}").into()));
/// // a newer revision of the plan, drawn without a reload where the old one was
/// handle.swap(Payload::default());
/// // another plan, drawn from its own view
/// handle.swap_plan(Payload::default(), ViewSpec::default(), "reef/app");
/// ```
pub fn mount(el: web_sys::HtmlElement, payload: Payload, view: ViewSpec, plan: &str) -> Handle {
    // build the plan, then draw it as the only view this caller tracks
    let built = Rc::new(plan_of(payload));
    let (viewer, wiring) = viewer::Viewer::mount(el, built, view, 0, viewer::plan_key(plan));
    Handle { viewer, wiring: Some(wiring) }
}

/// A view drawn by [`mount`], for swapping in new revisions of its plan, or another plan
///
/// Dropping the handle, or [`Handle::unmount`], tears the view down: its listeners and
/// observers are taken away, its element is emptied, and the view is freed.
pub struct Handle {
    /// The drawn view
    viewer: Rc<RefCell<viewer::Viewer>>,
    /// What the view hooked into the page, until it is torn down
    wiring: Option<viewer::Wiring>,
}

impl Handle {
    /// Draws a newer revision of the plan where the old one was, without a reload
    ///
    /// Open groups, the selection, the search and the root are kept by id, and so are the pan,
    /// the zoom and the sidebar's scroll. A selected node the new revision lacks is let go.
    ///
    /// # Arguments
    ///
    /// * `payload` - The plan's nodes to draw now
    pub fn swap(&self, payload: Payload) {
        // build the new plan before touching the view
        let plan = Rc::new(plan_of(payload));
        // a swap made from inside one of the view's own handlers waits for none, so it is dropped
        let told = match self.viewer.try_borrow_mut() {
            Ok(mut viewer) => {
                let before = viewer.selected_id();
                viewer.swap(plan);
                viewer.moved_from(before)
            }
            Err(_) => {
                web_sys::console::warn_1(&"reef: a swap made while the view was busy was dropped".into());
                None
            }
        };
        // a selected node the new revision lacks is let go, which the callback hears of
        if let Some((callback, id)) = told {
            viewer::tell(&callback, id);
        }
    }

    /// Draws another plan in the same view, from its own view and the groups saved for it
    ///
    /// Nothing is selected, and the view is fitted afresh. The layout and height stay as the
    /// element was mounted.
    ///
    /// # Arguments
    ///
    /// * `payload` - The other plan's nodes
    /// * `view` - What the view shows of it
    /// * `plan` - Its name, which its saved open groups are kept under
    pub fn swap_plan(&self, payload: Payload, view: ViewSpec, plan: &str) {
        // build the new plan before touching the view
        let built = Rc::new(plan_of(payload));
        // a swap made from inside one of the view's own handlers waits for none, so it is dropped
        let told = match self.viewer.try_borrow_mut() {
            Ok(mut viewer) => {
                let before = viewer.selected_id();
                viewer.swap_plan(built, view, viewer::plan_key(plan));
                viewer.moved_from(before)
            }
            Err(_) => {
                web_sys::console::warn_1(&"reef: a swap made while the view was busy was dropped".into());
                None
            }
        };
        // the selection is let go, which the callback hears of
        if let Some((callback, id)) = told {
            viewer::tell(&callback, id);
        }
    }

    /// Calls `f` with the selected node's id, or `None`, each time the selection changes
    ///
    /// It is called once the view is done with the event, so it may call this handle. Setting
    /// it again replaces the one set before.
    ///
    /// # Arguments
    ///
    /// * `f` - Told the id of the node selected now
    pub fn on_select(&self, f: impl FnMut(Option<String>) + 'static) {
        // the view holds it, to call after each change to the selection
        let callback: viewer::OnSelect = Rc::new(RefCell::new(Box::new(f)));
        match self.viewer.try_borrow_mut() {
            Ok(mut viewer) => viewer.set_on_select(callback),
            Err(_) => web_sys::console::warn_1(&"reef: a selection callback set while the view was busy was dropped".into()),
        }
    }

    /// The id of the node selected now, if one is
    pub fn selected(&self) -> Option<String> {
        // a view busy with an event is asked again after it
        self.viewer.try_borrow().ok().and_then(|viewer| viewer.selected_id())
    }

    /// Tears the view down: its listeners and observers taken away, its element emptied, and the
    /// view freed, as dropping the handle does
    pub fn unmount(self) {
        // dropping does it
    }
}

impl Drop for Handle {
    /// Takes the view's listeners and observers away, then empties its element
    fn drop(&mut self) {
        // nothing calls into the view once its wiring is gone, which drops the closures holding it
        let Some(wiring) = self.wiring.take() else { return };
        wiring.disconnect();
        // a view busy with an event, which dropped its handle, is emptied once the event is done
        match self.viewer.try_borrow_mut() {
            Ok(mut viewer) => viewer.unmounted(),
            Err(_) => {
                let viewer = self.viewer.clone();
                let later = Closure::once_into_js(move || {
                    if let Ok(mut viewer) = viewer.try_borrow_mut() {
                        viewer.unmounted();
                    }
                });
                if let Some(window) = web_sys::window() {
                    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), 0);
                }
            }
        }
    }
}
