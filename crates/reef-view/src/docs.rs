//! A doc read in the sidebar. When a "Read here" opens, its page is fetched, and the section
//! the link points at is cut out of the page mdbook rendered and put in the sidebar. Nothing is
//! embedded in the page ahead of time, so the plan's data stays small however long its docs are.

use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{DomParser, Element, HtmlDetailsElement, Response, SupportedType, Url};

thread_local! {
    /// Sections already read, by absolute link.
    static CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// What part of a doc's section to show.
#[derive(Clone, PartialEq)]
enum Part {
    /// All of it: a "Read here".
    Whole,
    /// The subsection with this heading id: a node's measurements.
    Sub(String),
    /// Only its tables with a header row, each with its caption: a node's measurements when
    /// they have no subsection of their own.
    Tables,
}

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
        details.get_attribute("data-heading").map_or(Part::Tables, Part::Sub)
    };
    wasm_bindgen_futures::spawn_local(async move {
        let html = match load(&href, part).await {
            Ok(html) => html,
            Err(e) => {
                web_sys::console::warn_2(&"reef: reading a doc:".into(), &e);
                r#"<p class="reef-note">This doc can't be read here (a book opened from a file can't fetch its pages). Use <em>Open page ↗</em>.</p>"#.to_string()
            }
        };
        if let Ok(Some(text)) = details.query_selector(".reef-doc-text") {
            text.set_inner_html(&html);
        }
    });
}

async fn load(href: &str, part: Part) -> Result<String, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let url = Url::new_with_base(href, &window.location().href()?)?;
    let key = match &part {
        Part::Whole => url.href(),
        Part::Sub(h) => format!("{} sub:{h}", url.href()),
        Part::Tables => format!("{} tables", url.href()),
    };
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(hit);
    }
    let anchor = js_sys::decode_uri_component(url.hash().trim_start_matches('#'))?.as_string().unwrap_or_default();
    url.set_hash("");
    let page = url.href();
    let resp: Response = JsFuture::from(window.fetch_with_str(&page)).await?.dyn_into()?;
    if !resp.ok() {
        return Err(format!("{page}: {}", resp.status()).into());
    }
    let text = JsFuture::from(resp.text()?).await?.as_string().unwrap_or_default();
    let doc = DomParser::new()?.parse_from_string(&text, SupportedType::TextHtml)?;
    let main = doc.query_selector("main")?.ok_or("the page has no <main>")?;
    let parts = match &part {
        Part::Sub(h) => section(&doc, h)?,
        _ if anchor.is_empty() => whole(&main),
        _ => section(&doc, &anchor)?,
    };
    let mut html = String::new();
    for el in parts {
        if part == Part::Tables {
            if !is_measured_table(&el) {
                continue;
            }
            if let Some(c) = caption(&el) {
                html.push_str(&format!(r#"<p class="reef-perf-caption">{c}</p>"#));
            }
        }
        rebase(&el, &page);
        if part != Part::Whole {
            drop_struck(&el);
        }
        html.push_str(&el.outer_html());
    }
    if html.trim().is_empty() {
        html = r#"<p class="reef-note">The section is empty.</p>"#.into();
    }
    CACHE.with(|c| c.borrow_mut().insert(key, html.clone()));
    Ok(html)
}

/// The heading level of an element, if it is a heading.
fn level(el: &Element) -> Option<u8> {
    let tag = el.tag_name();
    let b = tag.as_bytes();
    (b.len() == 2 && (b[0] == b'H' || b[0] == b'h') && (b'1'..=b'6').contains(&b[1])).then(|| b[1] - b'0')
}

/// What follows the heading `id`, up to the next heading at its level or above.
fn section(doc: &web_sys::Document, id: &str) -> Result<Vec<Element>, JsValue> {
    let head = doc.get_element_by_id(id).ok_or_else(|| format!("no heading `{id}`"))?;
    let top = level(&head).unwrap_or(1);
    let mut out = Vec::new();
    let mut cur = head.next_element_sibling();
    while let Some(el) = cur {
        if level(&el).is_some_and(|l| l <= top) {
            break;
        }
        cur = el.next_element_sibling();
        if keep(&el) {
            out.push(el);
        }
    }
    Ok(out)
}

/// A whole page's text, without its title.
fn whole(main: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    let mut cur = main.first_element_child();
    let mut titled = false;
    while let Some(el) = cur {
        cur = el.next_element_sibling();
        if !titled && level(&el) == Some(1) {
            titled = true;
            continue;
        }
        if keep(&el) {
            out.push(el);
        }
    }
    out
}

/// Leave out scripts, and plans drawn on the doc's own page, which would not be drawn here.
fn keep(el: &Element) -> bool {
    !el.tag_name().eq_ignore_ascii_case("script") && !el.class_list().contains("reef")
}

/// A table, or mdbook's wrapper around one, whose header row has text: a register entry's
/// opening `| | |` table has none.
fn is_measured_table(el: &Element) -> bool {
    let is_table = el.tag_name().eq_ignore_ascii_case("table") || el.query_selector(":scope > table").ok().flatten().is_some();
    is_table
        && el
            .query_selector("thead")
            .ok()
            .flatten()
            .and_then(|h| h.text_content())
            .is_some_and(|t| !t.trim().is_empty())
}

/// The bold text that opens the paragraph before a table, as in "**The cell alone** (…)".
fn caption(el: &Element) -> Option<String> {
    let p = el.previous_element_sibling().filter(|p| p.tag_name().eq_ignore_ascii_case("p"))?;
    let lead = p.first_element_child().filter(|b| b.tag_name().eq_ignore_ascii_case("strong"))?;
    let text = lead.text_content()?;
    let starts = p.text_content().is_some_and(|t| t.trim_start().starts_with(text.trim()));
    let text = text.trim().trim_end_matches(['.', ':']).trim();
    (starts && !text.is_empty()).then(|| crate::markup::esc(text))
}

/// Leave out struck-through text, which says what is no longer true.
fn drop_struck(el: &Element) {
    let Ok(list) = el.query_selector_all("del, s") else { return };
    for k in 0..list.length() {
        if let Some(e) = list.item(k).and_then(|n| n.dyn_into::<Element>().ok()) {
            e.remove();
        }
    }
}

/// Make the section's links and images point where they did on its own page.
fn rebase(el: &Element, page: &str) {
    for (sel, attr) in [("a[href]", "href"), ("img[src]", "src")] {
        let Ok(list) = el.query_selector_all(sel) else { continue };
        for k in 0..list.length() {
            let Some(e) = list.item(k).and_then(|n| n.dyn_into::<Element>().ok()) else { continue };
            if let Some(v) = e.get_attribute(attr)
                && let Ok(u) = Url::new_with_base(&v, page)
            {
                let _ = e.set_attribute(attr, &u.href());
            }
        }
    }
}
