//! The canvas's toolbar and legend: the search, the acceptance filter, the plan's open comment
//! threads, and the view's buttons

use super::{View, dom};
use crate::side::Tab;
use crate::side::threads::{Threads, excerpt, place_label};
use dioxus::prelude::*;
use reef_draw::accept;
use reef_draw::markup::status_class;
use reef_model::Kind;

/// The toolbar: the search, the criteria chips when the plan has criteria, and the buttons
///
/// # Arguments
///
/// * `has_acc` - Whether the plan has acceptance criteria, which the chips filter
#[component]
pub fn Toolbar(has_acc: bool) -> Element {
    let view = use_context::<View>();
    let page = view.spec.read().page.clone();
    rsx! {
        div { class: "reef-toolbar",
            Search {}
            span { class: "reef-spacer" }
            if has_acc {
                AccFilter {}
            }
            OpenComments {}
            button {
                "data-act": "expand",
                title: "Open every group",
                onclick: move |_| view.set_all(true),
                "Expand all"
            }
            button {
                "data-act": "collapse",
                title: "Close every group",
                onclick: move |_| view.set_all(false),
                "Collapse all"
            }
            button {
                "data-act": "out",
                title: "Zoom out ( - )",
                onclick: move |_| view.zoom_center(0.8),
                "−"
            }
            button {
                "data-act": "in",
                title: "Zoom in ( + )",
                onclick: move |_| view.zoom_center(1.25),
                "+"
            }
            button {
                "data-act": "fit",
                title: "Fit to view ( 0 )",
                onclick: move |_| view.fit(),
                "Fit"
            }
            if let Some(href) = page {
                a {
                    class: "reef-button",
                    href: "{href}",
                    title: "Open this plan on a page of its own",
                    "Full page ↗"
                }
            }
            button {
                "data-act": "full",
                title: "Full screen ( f )",
                onclick: move |_| view.fullscreen(),
                "⛶"
            }
        }
    }
}

/// The search box and the nodes it finds
#[component]
fn Search() -> Element {
    let mut view = use_context::<View>();
    // what the text finds, best first, in or out of the view's root
    let hits = use_memo(move || {
        let text = view.search.read();
        let g = view.graph()?;
        let root = view.drawn().and_then(|d| d.root);
        let found = reef_draw::search(&g, root, &text);
        (!found.is_empty()).then(|| {
            found
                .into_iter()
                .map(|i| {
                    let n = g.node(i);
                    (n.id.clone(), n.title.clone(), status_class(&g, i))
                })
                .collect::<Vec<_>>()
        })
    });
    let shown = *view.results.read() && hits.read().is_some();
    rsx! {
        div { class: "reef-searchbox",
            input {
                class: "reef-search",
                r#type: "search",
                placeholder: "Search nodes ( / )",
                aria_label: "Search nodes",
                autocomplete: "off",
                value: "{view.search}",
                onmounted: move |event| view.search_box.set(dom::mounted(&event.data())),
                oninput: move |event| {
                    view.search.set(event.value());
                    view.results.set(true);
                },
                onkeydown: move |event| {
                    // the view's own keys don't hear what is typed here
                    event.stop_propagation();
                    match event.key() {
                        Key::Enter => {
                            let first = hits.read().as_ref().and_then(|h| h.first().map(|(id, _, _)| id.clone()));
                            if let Some(id) = first {
                                view.goto(&id);
                            }
                        }
                        Key::Escape => {
                            view.search.set(String::new());
                            view.results.set(false);
                            if let Some(el) = view.root.peek().as_ref() {
                                dom::focus(el);
                            }
                        }
                        _ => {}
                    }
                },
            }
            ul { class: "reef-results", hidden: !shown,
                if let Some(found) = hits.read().clone() {
                    for (id, title, status) in found {
                        li { key: "{id}",
                            a {
                                href: "javascript:void 0",
                                "data-goto": "{id}",
                                onclick: move |event| {
                                    event.prevent_default();
                                    view.goto(&id);
                                },
                                span { class: "reef-dot {status}" }
                                "{title} "
                                code { "{id}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The plan's open comment threads: how many, and a list of them that goes to each one
///
/// A canvas drawn outside a plan's page reads no threads, and shows nothing here.
#[component]
fn OpenComments() -> Element {
    let mut view = use_context::<View>();
    let Some(mut threads) = try_use_context::<Threads>() else {
        return rsx! {};
    };
    // whether the list is open, and the threads still open, oldest first
    let mut shown = use_signal(|| false);
    let mut open = threads.now();
    open.retain(reef_api::Thread::is_open_published);
    let count = open.len();
    rsx! {
        div { class: "reef-app-open-box",
            button {
                class: if count > 0 { "reef-app-open-comments some" } else { "reef-app-open-comments" },
                "data-act": "comments",
                title: "The plan's open comment threads",
                "aria-expanded": "{shown}",
                onclick: move |_| {
                    let now = *shown.peek();
                    shown.set(!now);
                },
                "Comments "
                span { class: "reef-app-count", "{count}" }
                " ▾"
            }
            ul { class: "reef-results reef-app-open-list", hidden: !*shown.read(),
                if open.is_empty() {
                    li { class: "reef-app-muted reef-app-open-none", "No open threads." }
                }
                for thread in open {
                    li { key: "{thread.number}",
                        a {
                            href: "javascript:void 0",
                            "data-thread": "{thread.number}",
                            onclick: {
                                let (id, number) = (thread.node.id.clone(), thread.number);
                                move |event: MouseEvent| {
                                    // its node selected and shown, its thread in the Comments tab
                                    event.prevent_default();
                                    shown.set(false);
                                    view.goto(&id);
                                    view.tab.set(Tab::Comments);
                                    threads.focus.set(Some(number));
                                }
                            },
                            div { class: "reef-app-open-where",
                                code { "{thread.node.id}" }
                                " {place_label(&thread)}"
                            }
                            div { class: "reef-app-open-text", "{excerpt(&thread)}" }
                        }
                    }
                }
            }
        }
    }
}

/// The toolbar's criteria chips: each kind shown or hidden, in every view
#[component]
fn AccFilter() -> Element {
    let view = use_context::<View>();
    let f = *view.filter.read();
    rsx! {
        span {
            class: "reef-acc-filter",
            role: "group",
            "aria-label": "Acceptance criteria shown",
            for k in Kind::ALL {
                Chip { k, on: f.shows(k), count: None }
            }
        }
    }
}

/// A chip that shows or hides a kind of criteria, with its counts in a sidebar's section
///
/// # Arguments
///
/// * `k` - The kind
/// * `on` - Whether the kind is shown now
/// * `count` - The node's criteria of the kind, met and in all, if the chip counts them
#[component]
pub fn Chip(k: Kind, on: bool, count: Option<(usize, usize)>) -> Element {
    let view = use_context::<View>();
    let kind = k.as_str();
    let class = if on {
        format!("reef-acc-chip k-{kind} on")
    } else {
        format!("reef-acc-chip k-{kind}")
    };
    let title = format!(
        "{} {}",
        if on { "Hide" } else { "Show" },
        accept::plural_label(k)
    );
    rsx! {
        button {
            class: "{class}",
            "data-act": "acc:{kind}",
            "aria-pressed": "{on}",
            title: "{title}",
            onclick: move |_| view.toggle_kind(k),
            "{k.label()}"
            if let Some((met, total)) = count {
                " "
                span { class: "reef-acc-count", "{met}/{total}" }
            }
        }
    }
}

/// The legend: each status's dot, and each kind of criteria's when the plan has any
#[component]
pub fn Legend() -> Element {
    let view = use_context::<View>();
    let has_acc = view.graph().is_some_and(|g| g.has_acceptance());
    rsx! {
        div { class: "reef-legend",
            for (class, label) in [
                ("reef-dot s-done", "done"),
                ("reef-dot s-progress", "in progress"),
                ("reef-dot s-ready", "ready"),
                ("reef-dot s-todo", "to do"),
                ("reef-dot s-blocked", "blocked"),
                ("reef-dot optional", "optional"),
            ]
            {
                span {
                    i { class: "{class}" }
                    "{label}"
                }
            }
            if has_acc {
                for k in Kind::ALL {
                    span {
                        i { class: "reef-acc-dot k-{k.as_str()}" }
                        "{k.label()}"
                    }
                }
            }
        }
    }
}
