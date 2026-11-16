//! The in-browser viewer for reef planning graphs.
//!
//! The preprocessor turns each ```` ```reef ```` fence into a `div.reef` carrying its view, and
//! puts the plan's nodes in one `script.reef-data` on the page. This finds them, lays each view
//! out, and draws it as SVG with pan, zoom, collapsible groups, search and a sidebar.

mod accept;
mod chartui;
mod docs;
mod markup;
mod perf;
mod viewer;

use reef_model::{Graph, Payload, ViewSpec};
use std::rc::Rc;
use wasm_bindgen::prelude::*;

/// The plan as the viewer holds it.
pub struct Plan {
    pub graph: Graph,
}

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
        viewer::Viewer::mount(el, plan.clone(), spec, k as usize);
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
    let (graph, diags) = Graph::build(payload.nodes);
    for d in diags {
        web_sys::console::warn_1(&format!("reef: {d}").into());
    }
    Ok(Plan { graph })
}
