//! A doc read in the sidebar. When a "Read here" opens, reef-draw reads the section its link
//! points at, and it is put in the sidebar.

use reef_draw::web::docs::{self, Part};
use web_sys::HtmlDetailsElement;

/// Fill a "Read here", or a Performance section's doc, that has just opened, once.
pub fn open(details: HtmlDetailsElement) {
    if !details.open() || details.has_attribute("data-loaded") {
        return;
    }
    let _ = details.set_attribute("data-loaded", "");
    let Some(href) = details.get_attribute("data-href") else { return };
    let part = if !details.class_list().contains("reef-perf-doc") {
        Part::Whole
    } else {
        Part::measurements(details.get_attribute("data-heading"))
    };
    wasm_bindgen_futures::spawn_local(async move {
        let html = match docs::read(&href, part).await {
            Ok(html) => html,
            Err(e) => {
                web_sys::console::warn_2(&"reef: reading a doc:".into(), &e);
                docs::UNREADABLE.to_string()
            }
        };
        if let Ok(Some(text)) = details.query_selector(".reef-doc-text") {
            text.set_inner_html(&html);
        }
    });
}
