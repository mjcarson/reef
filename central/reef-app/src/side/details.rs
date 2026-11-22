//! The Details tab: what the plan says about the selected node, section by section, as
//! reef-draw's sidebar markup lays it out for reef-view

use super::Sec;
use super::docs::DocText;
use super::perf::{PerfChart, PerfSection};
use crate::canvas::View;
use crate::canvas::dom::Part;
use dioxus::prelude::*;
use reef_draw::accept;
use reef_draw::markup::{from_note, is_external, status_class};
use reef_draw::perf::ChartRef;
use reef_model::{EdgeKind, Graph, Kind, Met};

/// A node in one of the sidebar's lists: its id, title, status and a note
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// Its id
    id: String,
    /// Its title
    title: String,
    /// Its status's class
    status: &'static str,
    /// What to say after it, if anything
    note: String,
}

impl Entry {
    /// A node as a list entry
    ///
    /// # Arguments
    ///
    /// * `g` - The plan
    /// * `j` - The node
    /// * `note` - What to say after it
    fn of(g: &Graph, j: usize, note: impl Into<String>) -> Entry {
        let n = g.node(j);
        Entry {
            id: n.id.clone(),
            title: n.title.clone(),
            status: status_class(g, j),
            note: note.into(),
        }
    }
}

/// The Details tab for a node
///
/// # Arguments
///
/// * `i` - The node's index in the plan drawn
#[component]
pub fn Details(i: usize) -> Element {
    let view = use_context::<View>();
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    if i >= g.len() {
        return rsx! {};
    }
    let n = g.node(i).clone();
    // the nodes around it: its members, what it needs, what its groups need, what needs it
    let members: Vec<Entry> = g
        .children(i)
        .iter()
        .map(|&c| Entry::of(&g, c, if g.is_group(c) { "group" } else { "" }))
        .collect();
    let (done, total) = g.progress(i);
    let needs: Vec<Entry> = g
        .needs(i)
        .iter()
        .map(|&j| Entry::of(&g, j, ""))
        .chain(g.helps(i).iter().map(|&j| Entry::of(&g, j, "helps")))
        .collect();
    let inherited: Vec<Entry> = g
        .ancestors(i)
        .iter()
        .flat_map(|&a| g.needs(a).iter().map(move |&j| (a, j)))
        .map(|(a, j)| Entry::of(&g, j, format!("via {}", g.node(a).id)))
        .collect();
    let deps: Vec<Entry> = g
        .dependents(i)
        .into_iter()
        .map(|(j, k)| Entry::of(&g, j, if k == EdgeKind::Helps { "helped" } else { "" }))
        .collect();
    let is_group = g.is_group(i);
    let design_head = format!("Design choices{}", from_note(n.design_from.as_deref()));
    let members_head = format!(r#"Members <span class="reef-note">{done} of {total} done</span>"#);
    let origin = n.origin.to_string();
    rsx! {
        Description { i }
        Acceptance { i }
        PerfSection { i }
        if !n.docs.is_empty() {
            Sec { head: "Documentation".to_string(),
                for d in n.docs.clone() {
                    if is_external(&d.href) {
                        div { class: "reef-doc reef-doc-ext",
                            div { class: "reef-doc-head",
                                a { href: "{d.href}", "{d.label}" }
                                a {
                                    class: "reef-button reef-open",
                                    href: "{d.href}",
                                    title: "Open this doc's page",
                                    "Open page ↗"
                                }
                            }
                        }
                    } else {
                        DocText {
                            href: d.href.clone(),
                            class: "reef-doc reef-doc-read",
                            head: rsx! {
                                summary { class: "reef-doc-head", title: "Read here",
                                    a { href: "{d.href}", "{d.label}" }
                                    a {
                                        class: "reef-button reef-open",
                                        href: "{d.href}",
                                        title: "Open this doc's page",
                                        "Open page ↗"
                                    }
                                }
                            },
                            part: Part::Whole,
                        }
                    }
                }
            }
        }
        if !n.design.trim().is_empty() {
            Sec { head: design_head,
                div { class: "reef-design", dangerous_inner_html: "{n.design}" }
            }
        }
        if is_group {
            Sec { head: members_head,
                Entries { entries: members }
            }
        }
        if !needs.is_empty() {
            Sec { head: "Needs".to_string(),
                Entries { entries: needs }
            }
        }
        if !inherited.is_empty() {
            Sec { head: "Its groups need".to_string(),
                Entries { entries: inherited }
            }
        }
        if !deps.is_empty() {
            Sec { head: "Needed by".to_string(),
                Entries { entries: deps }
            }
        }
        if !n.html.trim().is_empty() {
            div { class: "reef-body", dangerous_inner_html: "{n.html}" }
        }
        div { class: "reef-source", "{origin}" }
    }
}

/// A list of nodes, each a link that goes to it
///
/// # Arguments
///
/// * `entries` - The nodes
#[component]
fn Entries(entries: Vec<Entry>) -> Element {
    let view = use_context::<View>();
    rsx! {
        ul {
            for e in entries {
                li {
                    a {
                        href: "javascript:void 0",
                        "data-goto": "{e.id}",
                        onclick: {
                            let id = e.id.clone();
                            move |event: MouseEvent| {
                                event.prevent_default();
                                event.stop_propagation();
                                view.goto(&id);
                            }
                        },
                        span { class: "reef-dot {e.status}" }
                        "{e.title}"
                    }
                    if !e.note.is_empty() {
                        " "
                        span { class: "reef-note", "{e.note}" }
                    }
                }
            }
        }
    }
}

/// The Description section: why the work is needed and what finishing it will do, each
/// labelled, then the longer explanation; nothing when the node has none of them
///
/// # Arguments
///
/// * `i` - The node
#[component]
fn Description(i: usize) -> Element {
    let view = use_context::<View>();
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    let n = g.node(i);
    // the labelled parts that have text, each with where it was taken from
    let parts: Vec<(&'static str, String, Option<String>)> = [
        ("Why", &n.why, &n.why_from),
        ("What", &n.what, &n.what_from),
    ]
    .into_iter()
    .filter(|(_, html, _)| !html.trim().is_empty())
    .map(|(label, html, from)| (label, html.clone(), from.clone()))
    .collect();
    let details = n.details.clone();
    if parts.is_empty() && details.trim().is_empty() {
        return rsx! {};
    }
    rsx! {
        Sec { head: "Description".to_string(),
            if !parts.is_empty() {
                dl { class: "reef-ww",
                    for (label, html, from) in parts {
                        dt {
                            "{label}"
                            if let Some(from) = from {
                                " "
                                span { class: "reef-note",
                                    "from "
                                    em { "{from}" }
                                }
                            }
                        }
                        dd { dangerous_inner_html: "{html}" }
                    }
                }
            }
            if !details.trim().is_empty() {
                div { class: "reef-desc", dangerous_inner_html: "{details}" }
            }
        }
    }
}

/// One acceptance criterion as the section lists it
#[derive(Debug, Clone, PartialEq)]
struct Criterion {
    /// Its index among the node's
    c: usize,
    /// Its kind
    kind: Kind,
    /// Whether it is met
    met: Met,
    /// Its text's markup
    text: String,
    /// Its meta line's markup: its test, chart and target
    meta: String,
    /// Whether it draws a chart
    chart: bool,
}

/// The Acceptance section: each kind's chip with its counts, the criteria the filter shows,
/// and how many it hides; nothing when the node has no criteria
///
/// # Arguments
///
/// * `i` - The node
#[component]
fn Acceptance(i: usize) -> Element {
    let view = use_context::<View>();
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    let n = g.node(i);
    if n.acceptance.is_empty() {
        return rsx! {};
    }
    let f = *view.filter.read();
    // every kind's counts, filtered or not, for the chips
    let all = g.acceptance_counts(i, &[true; 4], false);
    let chips: Vec<(Kind, (usize, usize))> = Kind::ALL
        .into_iter()
        .map(|k| (k, all[k.index()]))
        .filter(|(_, c)| c.1 > 0)
        .collect();
    // the criteria the filter shows
    let (mut met, mut hidden) = (0, 0);
    let mut shown = Vec::new();
    for (c, (crit, r)) in n.acceptance.iter().zip(g.acceptance(i)).enumerate() {
        if !f.shows(crit.kind) {
            hidden += 1;
            continue;
        }
        met += usize::from(r.status == Met::Met);
        shown.push(Criterion {
            c,
            kind: crit.kind,
            met: r.status,
            text: crit.text.clone(),
            meta: accept::meta(&g, i, c),
            chart: r.chart.is_some(),
        });
    }
    let head = format!(
        r#"Acceptance <span class="reef-note">{met} of {} met</span>"#,
        shown.len()
    );
    rsx! {
        Sec { head, class: "reef-acc",
            div { class: "reef-acc-chips",
                for (k, count) in chips {
                    crate::canvas::Chip { k, on: f.shows(k), count: Some(count) }
                }
            }
            ul { class: "reef-acc-list",
                for crit in shown {
                    CriterionItem { i, crit }
                }
            }
            if hidden > 0 {
                p { class: "reef-note", "{hidden} hidden by the filter." }
            }
        }
    }
}

/// One criterion: its state, kind and text, its meta line, and its chart
///
/// # Arguments
///
/// * `i` - The node it is of
/// * `crit` - The criterion
#[component]
fn CriterionItem(i: usize, crit: Criterion) -> Element {
    let kind = crit.kind.as_str();
    let (mark, word) = accept::state(crit.met);
    let class = format!("reef-acc-item k-{kind} reef-acc-{}", crit.met.as_str());
    let label = crit.kind.label();
    rsx! {
        li { class: "{class}",
            div { class: "reef-acc-line",
                span { class: "reef-acc-state", title: "{word}", aria_label: "{word}", "{mark}" }
                span { class: "reef-acc-badge k-{kind}", "{label}" }
                span { class: "reef-acc-text", dangerous_inner_html: "{crit.text}" }
            }
            if !crit.meta.is_empty() {
                div { class: "reef-acc-meta", dangerous_inner_html: "{crit.meta}" }
            }
            if crit.chart {
                PerfChart { node: i, chart: ChartRef::Accept(crit.c) }
            }
        }
    }
}
