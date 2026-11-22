//! The Performance section and its charts, which answer the pointer as reef-view's do
//!
//! Each chart is a component that keeps its view (series hidden, an x range zoomed to) in a
//! signal and draws reef-draw's SVG for it. What the pointer shows over a chart, its crosshair
//! and the bars and rows it lights, is written into that SVG as the pointer moves, so a hover
//! never draws the chart again. The tooltip is the canvas's.
//!
//! - **Hover:** a crosshair snapped to the nearest x on line charts, the row under the pointer
//!   on bar charts, and a tooltip with every series' figure there.
//! - **Legend:** a click shows that series alone; ctrl- or cmd-click hides or shows just it.
//! - **Zoom:** a drag across a line chart zooms to that x range; a double-click or *Reset zoom*
//!   goes back.
//! - **Trends:** a click on a member's point goes to the member.

use super::Sec;
use super::docs::DocText;
use crate::canvas::dom::{self, El, Part};
use crate::canvas::{Tip, View};
use dioxus::prelude::*;
use reef_draw::perf::{self, ChartRef, ChartView, Hover, PerfPart};

/// How far, in SVG units, a drag must go to zoom rather than click
const MIN_DRAG: f64 = 6.0;

/// The Performance section, or nothing when the node has no measurements
///
/// # Arguments
///
/// * `i` - The node
#[component]
pub fn PerfSection(i: usize) -> Element {
    let view = use_context::<View>();
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    let Some(sec) = perf::section_parts(&g, i) else {
        return rsx! {};
    };
    let mut body = use_signal(|| None::<El>);
    rsx! {
        Sec { head: sec.head, class: "reef-perf",
            div {
                class: "reef-perf-body",
                onmounted: move |event| body.set(dom::mounted(&event.data())),
                // a table row lights its metric's bars, when the pointer isn't over a chart
                onpointermove: move |event| {
                    let (Some(root), Some(t)) = (view.root.peek().clone(), dom::target(&*event.data())) else { return };
                    if dom::closest(&t, "svg.reef-perf-chart").is_some() {
                        return;
                    }
                    let row = dom::closest(&t, ".reef-perf tr[data-metric]");
                    let k = row.as_ref().and_then(|r| dom::attr(r, "data-metric")).and_then(|k| k.parse().ok());
                    dom::light(&root, row.as_ref().unwrap_or(&t), k);
                },
                onpointerleave: move |_| {
                    if let (Some(root), Some(b)) = (view.root.peek().clone(), body.peek().clone()) {
                        dom::light(&root, &b, None);
                    }
                },
                for part in sec.parts {
                    match part {
                        PerfPart::Html(html) => rsx! {
                            div { class: "reef-app-html", dangerous_inner_html: "{html}" }
                        },
                        PerfPart::Chart(chart) => rsx! {
                            PerfChart { node: i, chart }
                        },
                        PerfPart::Doc(d) => rsx! {
                            DocText {
                                href: d.href.clone(),
                                class: "reef-perf-doc",
                                head: rsx! {
                                    summary { dangerous_inner_html: perf::doc_summary(&d) }
                                },
                                part: match d.heading.clone() {
                                    Some(h) => Part::Sub(h),
                                    None => Part::Tables,
                                },
                            }
                        },
                    }
                }
            }
        }
    }
}

/// A drag across a zoomable chart: the pointer, and where it went from and is now, in SVG units
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    /// The pointer's id
    pointer: i32,
    /// Where it went down
    from: f64,
    /// Where it is now
    to: f64,
}

/// One chart of a node's section, or of one of its criteria
///
/// # Arguments
///
/// * `node` - The node whose section it is in
/// * `chart` - Which chart
#[component]
pub fn PerfChart(node: usize, chart: ChartRef) -> Element {
    let view = use_context::<View>();
    // what the reader has done to it, what the pointer is over, and a drag across it
    let mut cv = use_signal(ChartView::default);
    let mut hover = use_signal(|| None::<Hover>);
    let mut drag = use_signal(|| None::<Drag>);
    let mut eat_click = use_signal(|| false);
    let mut wrap = use_signal(|| None::<El>);
    let Some(g) = view.graph() else {
        return rsx! {};
    };
    if node >= g.len() {
        return rsx! {};
    }
    let svg = perf::chart(&g, node, &chart, &cv.read());
    if svg.is_empty() {
        return rsx! {};
    }
    let can_open = hover.read().as_ref().is_some_and(|h| h.goto.is_some());
    let (c1, c2, c3, c4) = (chart.clone(), chart.clone(), chart.clone(), chart.clone());
    rsx! {
        div {
            class: if can_open { "reef-app-chart reef-perf-can-open" } else { "reef-app-chart" },
            onmounted: move |event| wrap.set(dom::mounted(&event.data())),
            onpointermove: move |event| {
                let Some(el) = wrap.peek().clone() else { return };
                let Some(g) = view.graph() else { return };
                let c = event.client_coordinates();
                let Some((svg, x, y)) = to_svg(&el, c.x, c.y) else { return };
                let v = cv.peek().clone();
                // a drag draws the range it would zoom to
                let dragging = *drag.peek();
                if let Some(mut d) = dragging {
                    d.to = x;
                    drag.set(Some(d));
                    if let Some((l, t, r, b)) = perf::plot_area(&g, node, &c1, &v) {
                        let (a, z) = (d.from.min(d.to).clamp(l, r), d.from.max(d.to).clamp(l, r));
                        dom::overlay(&svg, &format!(r#"<rect class="reef-perf-select" x="{a:.1}" y="{t:.1}" width="{:.1}" height="{:.1}"/>"#, z - a, b - t));
                    }
                    set_tip(view, None);
                    return;
                }
                // otherwise what is under the pointer, shown over the chart and in the tooltip
                let h = perf::hover(&g, node, &c1, &v, x, y);
                if let Some(root) = view.root.peek().as_ref() {
                    dom::light(root, &el, h.as_ref().and_then(|h| h.metric));
                }
                dom::overlay(&svg, h.as_ref().map_or("", |h| h.overlay.as_str()));
                set_tip(view, h.as_ref().map(|h| Tip { html: h.tip.clone(), x: c.x, y: c.y }));
                if *hover.peek() != h {
                    hover.set(h);
                }
            },
            onpointerleave: move |_| {
                if drag.peek().is_none() {
                    left(view, wrap, hover);
                }
            },
            onpointerdown: move |event| {
                eat_click.set(false);
                if event.trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                    return;
                }
                let (Some(el), Some(g)) = (wrap.peek().clone(), view.graph()) else { return };
                // the legend and the reset button are clicked, not dragged from
                let on_control = dom::target(&*event.data()).is_some_and(|t| dom::closest(&t, ".reef-perf-legend-item, .reef-perf-reset").is_some());
                if on_control || !perf::zoomable(&g, node, &c2) {
                    return;
                }
                let c = event.client_coordinates();
                let Some((_, x, y)) = to_svg(&el, c.x, c.y) else { return };
                let Some((l, t, r, b)) = perf::plot_area(&g, node, &c2, &cv.peek()) else { return };
                if x < l || x > r || y < t || y > b {
                    return;
                }
                event.prevent_default();
                dom::capture(&el, event.pointer_id());
                drag.set(Some(Drag { pointer: event.pointer_id(), from: x, to: x }));
            },
            onpointerup: move |event| released(view, wrap, cv, drag, eat_click, hover, node, &c3, event.pointer_id()),
            onpointercancel: move |event| released(view, wrap, cv, drag, eat_click, hover, node, &c4, event.pointer_id()),
            onclick: {
                move |event: MouseEvent| {
                    // the click that ends a drag is no click
                    if eat_click.take() {
                        event.prevent_default();
                        event.stop_propagation();
                        return;
                    }
                    let Some(t) = dom::target(&*event.data()) else { return };
                    // a legend item shows its series alone, or with ctrl or cmd, toggles it
                    if let Some(item) = dom::closest(&t, ".reef-perf-legend-item") {
                        event.prevent_default();
                        event.stop_propagation();
                        let (Some(s), Some(el)) = (dom::attr(&item, "data-series").and_then(|s| s.parse().ok()), wrap.peek().clone()) else { return };
                        let n = dom::count(&el, ".reef-perf-legend-item");
                        let mods = event.modifiers();
                        let next = cv.peek().legend_click(s, n, mods.ctrl() || mods.meta());
                        left(view, wrap, hover);
                        cv.set(next);
                        return;
                    }
                    // reset zoom
                    if dom::closest(&t, "[data-chart-act=\"reset\"]").is_some() {
                        event.prevent_default();
                        event.stop_propagation();
                        left(view, wrap, hover);
                        cv.write().x = None;
                        return;
                    }
                    // a trend's member opens
                    let goto = hover.peek().as_ref().and_then(|h| h.goto.clone());
                    if let Some(id) = goto {
                        event.prevent_default();
                        event.stop_propagation();
                        view.goto(&id);
                    }
                }
            },
            ondoubleclick: move |event| {
                // a zoomed chart goes back to its whole range
                if cv.peek().x.is_some() {
                    event.prevent_default();
                    left(view, wrap, hover);
                    cv.write().x = None;
                }
            },
            dangerous_inner_html: "{svg}",
        }
    }
}

/// A client point in a chart's SVG units, with the chart's SVG element
///
/// The charts keep their aspect, so one scale serves both axes.
///
/// # Arguments
///
/// * `wrap` - What holds the chart's SVG
/// * `x` - The point's client x
/// * `y` - The point's client y
fn to_svg(wrap: &El, x: f64, y: f64) -> Option<(El, f64, f64)> {
    let svg = dom::query(wrap, "svg.reef-perf-chart")?;
    let b = dom::bounds(&svg);
    let k = if b.width > 0.0 {
        perf::W / b.width
    } else {
        1.0
    };
    Some((svg, (x - b.left) * k, (y - b.top) * k))
}

/// Shows the canvas's tooltip, or hides it, unless it shows that already
///
/// # Arguments
///
/// * `view` - The view
/// * `tip` - What to show, or `None` to hide it
fn set_tip(view: View, tip: Option<Tip>) {
    let mut shown = view.tip;
    if *shown.peek() != tip {
        shown.set(tip);
    }
}

/// The pointer left a chart: its overlay, its lit metric and the tooltip go
///
/// # Arguments
///
/// * `view` - The view
/// * `wrap` - What holds the chart's SVG
/// * `hover` - What the pointer was over
fn left(view: View, wrap: Signal<Option<El>>, mut hover: Signal<Option<Hover>>) {
    if let Some(el) = wrap.peek().as_ref() {
        if let Some(svg) = dom::query(el, "svg.reef-perf-chart") {
            dom::overlay(&svg, "");
        }
        if let Some(root) = view.root.peek().as_ref() {
            dom::light(root, el, None);
        }
    }
    set_tip(view, None);
    if hover.peek().is_some() {
        hover.set(None);
    }
}

/// A pointer let go of a chart: a drag far enough across it zooms to the range it covered
///
/// # Arguments
///
/// * `view` - The view
/// * `wrap` - What holds the chart's SVG
/// * `cv` - The chart's view
/// * `drag` - The drag, if one is under way
/// * `eat_click` - Set when the drag zoomed, so the click that follows isn't one
/// * `hover` - What the pointer is over
/// * `node` - The node whose chart it is
/// * `chart` - Which chart
/// * `pointer` - The pointer let go
#[allow(clippy::too_many_arguments)]
fn released(
    view: View,
    wrap: Signal<Option<El>>,
    mut cv: Signal<ChartView>,
    mut drag: Signal<Option<Drag>>,
    mut eat_click: Signal<bool>,
    hover: Signal<Option<Hover>>,
    node: usize,
    chart: &ChartRef,
    pointer: i32,
) {
    // only the pointer that started the drag ends it
    let Some(d) = *drag.peek() else { return };
    if d.pointer != pointer {
        return;
    }
    drag.set(None);
    if let Some(el) = wrap.peek().as_ref() {
        dom::release(el, pointer);
        if let Some(svg) = dom::query(el, "svg.reef-perf-chart") {
            dom::overlay(&svg, "");
        }
    }
    if (d.to - d.from).abs() < MIN_DRAG {
        return;
    }
    // the x values at both ends, as the chart is drawn now
    let Some(g) = view.graph() else { return };
    let v = cv.peek().clone();
    let (Some(a), Some(b)) = (
        perf::x_at(&g, node, chart, &v, d.from.min(d.to)),
        perf::x_at(&g, node, chart, &v, d.from.max(d.to)),
    ) else {
        return;
    };
    if b > a {
        eat_click.set(true);
        left(view, wrap, hover);
        cv.set(ChartView {
            x: Some((a, b)),
            ..v
        });
    }
}
