//! The sidebar's charts, made interactive: what the pointer does to the charts `perf` draws.
//!
//! - **Hover:** `perf::hover` gives a tooltip, a crosshair to draw over the chart, the metric to
//!   light up in the table and the other charts, and the member a click on a trend opens.
//! - **Drag** across a line chart: zooms to that x range. A double-click or *Reset zoom* goes
//!   back.
//! - **Legend:** a click shows that series alone, and ctrl- or cmd-click hides or shows just
//!   that one.
//!
//! A chart keeps its view in its own `data-` attributes and is drawn again with each change, so
//! nothing here outlives the sidebar's contents.

use crate::Plan;
use crate::perf::{self, ChartRef, ChartView};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::{Element, Event, HtmlElement, MouseEvent, PointerEvent};

/// A drag across a line chart, in its SVG units.
struct Drag {
    svg: Element,
    pointer: i32,
    from: f64,
    to: f64,
}

struct State {
    plan: Rc<Plan>,
    container: HtmlElement,
    tip: HtmlElement,
    drag: Option<Drag>,
    /// A drag just ended: the click that follows it is not a click.
    eat_click: bool,
    /// The chart the pointer was last over, whose overlay to clear.
    over: Option<Element>,
}

/// A chart under an element: its `<svg>`, its node, which chart it is, and its view.
fn chart_at(t: &Element) -> Option<(Element, usize, ChartRef, ChartView)> {
    let svg = t.closest("svg.reef-perf-chart").ok()??;
    let node = svg.get_attribute("data-node")?.parse().ok()?;
    let r = ChartRef::parse(&svg.get_attribute("data-chart")?, &svg.get_attribute("data-arg").unwrap_or_default())?;
    let v = ChartView::parse(
        svg.get_attribute("data-hidden").as_deref(),
        svg.get_attribute("data-x0").as_deref(),
        svg.get_attribute("data-x1").as_deref(),
    );
    Some((svg, node, r, v))
}

/// A point on the page in a chart's SVG units. The charts keep their aspect, so one scale
/// serves both axes.
fn to_svg(svg: &Element, cx: f64, cy: f64) -> (f64, f64) {
    let r = svg.get_bounding_client_rect();
    let k = if r.width() > 0.0 { perf::W / r.width() } else { 1.0 };
    ((cx - r.left()) * k, (cy - r.top()) * k)
}

fn overlay(svg: &Element) -> Option<Element> {
    svg.query_selector(".reef-perf-overlay").ok().flatten()
}

fn target(e: &Event) -> Option<Element> {
    e.target().and_then(|t| t.dyn_into().ok())
}

impl State {
    fn graph(&self) -> &reef_model::Graph {
        &self.plan.graph
    }

    /// Draw a chart again in a new view.
    fn redraw(&self, svg: &Element, node: usize, r: &ChartRef, v: &ChartView) {
        let html = perf::chart(self.graph(), node, r, v);
        if !html.is_empty() {
            svg.set_outer_html(&html);
        }
    }

    fn hide_tip(&self) {
        self.tip.set_hidden(true);
    }

    fn show_tip(&self, html: &str, cx: f64, cy: f64) {
        self.tip.set_inner_html(html);
        self.tip.set_hidden(false);
        let c = self.container.get_bounding_client_rect();
        let (w, h) = (self.tip.offset_width() as f64, self.tip.offset_height() as f64);
        let mut x = cx - c.left() + 14.0;
        let mut y = cy - c.top() + 14.0;
        if x + w > c.width() - 4.0 {
            x = (cx - c.left() - w - 14.0).max(4.0);
        }
        if y + h > c.height() - 4.0 {
            y = (cy - c.top() - h - 14.0).max(4.0);
        }
        let st = self.tip.style();
        let _ = st.set_property("left", &format!("{x:.0}px"));
        let _ = st.set_property("top", &format!("{y:.0}px"));
    }

    /// Light up metric `k`'s table row and bars in the section around `el`, and nothing else.
    fn light(&self, el: &Element, k: Option<usize>) {
        if let Ok(list) = self.container.query_selector_all(".reef-perf-hl") {
            for j in 0..list.length() {
                if let Some(e) = list.item(j).and_then(|n| n.dyn_into::<Element>().ok()) {
                    let _ = e.class_list().remove_1("reef-perf-hl");
                }
            }
        }
        let (Some(k), Ok(Some(sec))) = (k, el.closest(".reef-perf")) else { return };
        if let Ok(list) = sec.query_selector_all(&format!(r#"[data-metric="{k}"]"#)) {
            for j in 0..list.length() {
                if let Some(e) = list.item(j).and_then(|n| n.dyn_into::<Element>().ok()) {
                    let _ = e.class_list().add_1("reef-perf-hl");
                }
            }
        }
    }

    /// The pointer left whatever chart it was over.
    fn leave(&mut self) {
        if let Some(svg) = self.over.take() {
            if let Some(o) = overlay(&svg) {
                o.set_inner_html("");
            }
            let _ = svg.remove_attribute("data-goto");
            let _ = svg.class_list().remove_1("reef-perf-can-open");
        }
        self.hide_tip();
        self.light(&self.container.clone().into(), None);
    }

    fn moved(&mut self, e: &PointerEvent) {
        let (cx, cy) = (e.client_x() as f64, e.client_y() as f64);
        if let Some(d) = &mut self.drag {
            let (x, _) = to_svg(&d.svg, cx, cy);
            d.to = x;
            let svg = d.svg.clone();
            let (from, to) = (d.from, d.to);
            if let Some((_, node, r, v)) = chart_at(&svg)
                && let Some((l, t, rr, b)) = perf::plot_area(self.graph(), node, &r, &v)
                && let Some(o) = overlay(&svg)
            {
                let (a, z) = (from.min(to).clamp(l, rr), from.max(to).clamp(l, rr));
                o.set_inner_html(&format!(r#"<rect class="reef-perf-select" x="{a:.1}" y="{t:.1}" width="{:.1}" height="{:.1}"/>"#, z - a, b - t));
            }
            self.hide_tip();
            return;
        }
        let Some(t) = target(e) else { return self.leave() };
        if let Some((svg, node, r, v)) = chart_at(&t) {
            if self.over.as_ref() != Some(&svg) {
                self.leave();
            }
            self.over = Some(svg.clone());
            let (x, y) = to_svg(&svg, cx, cy);
            match perf::hover(self.graph(), node, &r, &v, x, y) {
                Some(h) => {
                    if let Some(o) = overlay(&svg) {
                        o.set_inner_html(&h.overlay);
                    }
                    match &h.goto {
                        Some(id) => {
                            let _ = svg.set_attribute("data-goto", id);
                            let _ = svg.class_list().add_1("reef-perf-can-open");
                        }
                        None => {
                            let _ = svg.remove_attribute("data-goto");
                            let _ = svg.class_list().remove_1("reef-perf-can-open");
                        }
                    }
                    self.show_tip(&h.tip, cx, cy);
                    self.light(&svg, h.metric);
                }
                None => {
                    if let Some(o) = overlay(&svg) {
                        o.set_inner_html("");
                    }
                    let _ = svg.remove_attribute("data-goto");
                    self.hide_tip();
                    self.light(&svg, None);
                }
            }
            return;
        }
        self.leave();
        // A row of the metrics table lights up its bars.
        if let Ok(Some(row)) = t.closest(".reef-perf tr[data-metric]") {
            let k = row.get_attribute("data-metric").and_then(|k| k.parse().ok());
            self.light(&row, k);
        }
    }

    fn pressed(&mut self, e: &PointerEvent) {
        self.eat_click = false;
        if e.button() != 0 {
            return;
        }
        let Some(t) = target(e) else { return };
        if t.closest(".reef-perf-legend-item, .reef-perf-reset").ok().flatten().is_some() {
            return;
        }
        let Some((svg, node, r, v)) = chart_at(&t) else { return };
        if !r.zoomable() {
            return;
        }
        let (x, y) = to_svg(&svg, e.client_x() as f64, e.client_y() as f64);
        let Some((l, top, rr, b)) = perf::plot_area(self.graph(), node, &r, &v) else { return };
        if x < l || x > rr || y < top || y > b {
            return;
        }
        e.prevent_default();
        let _ = svg.set_pointer_capture(e.pointer_id());
        self.drag = Some(Drag { svg, pointer: e.pointer_id(), from: x, to: x });
    }

    fn released(&mut self, e: &PointerEvent) {
        let Some(d) = self.drag.take() else { return };
        if d.pointer != e.pointer_id() {
            self.drag = Some(d);
            return;
        }
        let _ = d.svg.release_pointer_capture(d.pointer);
        if let Some(o) = overlay(&d.svg) {
            o.set_inner_html("");
        }
        if (d.to - d.from).abs() < 6.0 {
            return;
        }
        let Some((svg, node, r, v)) = chart_at(&d.svg) else { return };
        let g = self.graph();
        let (Some(a), Some(b)) = (perf::x_at(g, node, &r, &v, d.from.min(d.to)), perf::x_at(g, node, &r, &v, d.from.max(d.to))) else {
            return;
        };
        if b > a {
            self.eat_click = true;
            self.leave();
            self.redraw(&svg, node, &r, &ChartView { x: Some((a, b)), ..v });
        }
    }

    /// A click on a legend item or *Reset zoom*. True when it was one.
    fn clicked(&mut self, e: &MouseEvent) -> bool {
        let Some(t) = target(e) else { return false };
        let Some((svg, node, r, v)) = chart_at(&t) else { return false };
        if let Ok(Some(item)) = t.closest(".reef-perf-legend-item") {
            let Some(s) = item.get_attribute("data-series").and_then(|s| s.parse().ok()) else { return true };
            let n = svg.query_selector_all(".reef-perf-legend-item").map_or(0, |l| l.length() as usize);
            let v = v.legend_click(s, n, e.ctrl_key() || e.meta_key());
            self.leave();
            self.redraw(&svg, node, &r, &v);
            return true;
        }
        if t.closest("[data-chart-act=\"reset\"]").ok().flatten().is_some() {
            self.leave();
            self.redraw(&svg, node, &r, &ChartView { x: None, ..v });
            return true;
        }
        false
    }

    fn double_clicked(&mut self, e: &MouseEvent) {
        let Some(t) = target(e) else { return };
        let Some((svg, node, r, v)) = chart_at(&t) else { return };
        if v.x.is_some() {
            e.prevent_default();
            self.leave();
            self.redraw(&svg, node, &r, &ChartView { x: None, ..v });
        }
    }
}

fn listen<E: wasm_bindgen::convert::FromWasmAbi + 'static>(target: &web_sys::EventTarget, kind: &str, capture: bool, passive: bool, f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(E)>::new(f);
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_capture(capture);
    opts.set_passive(passive);
    let _ = target.add_event_listener_with_callback_and_add_event_listener_options(kind, cb.as_ref().unchecked_ref(), &opts);
    cb.forget();
}

/// Make the charts in `side` interactive. The tooltip goes in `container`, the viewer, so it
/// can show past the sidebar's edge.
pub fn attach(container: &HtmlElement, side: &HtmlElement, plan: Rc<Plan>) {
    let doc = web_sys::window().and_then(|w| w.document()).expect("a document");
    let tip: HtmlElement = doc.create_element("div").expect("a div").dyn_into().expect("an HtmlElement");
    tip.set_class_name("reef-tip");
    tip.set_hidden(true);
    let _ = tip.set_attribute("role", "tooltip");
    let _ = container.append_child(&tip);
    let st = Rc::new(RefCell::new(State { plan, container: container.clone(), tip, drag: None, eat_click: false, over: None }));
    let side: web_sys::EventTarget = side.clone().into();

    let s = st.clone();
    listen(&side, "pointermove", false, true, move |e: PointerEvent| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.moved(&e);
        }
    });
    let s = st.clone();
    listen(&side, "pointerleave", false, true, move |_: PointerEvent| {
        if let Ok(mut s) = s.try_borrow_mut()
            && s.drag.is_none()
        {
            s.leave();
        }
    });
    let s = st.clone();
    listen(&side, "pointerdown", false, false, move |e: PointerEvent| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.pressed(&e);
        }
    });
    for kind in ["pointerup", "pointercancel"] {
        let s = st.clone();
        listen(&side, kind, false, true, move |e: PointerEvent| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.released(&e);
            }
        });
    }
    // Clicks are taken as they go down, before the viewer's own handler sees a trend's
    // `data-goto`: the click that ends a drag, and legend and reset clicks, stop here.
    let s = st.clone();
    listen(&side, "click", true, false, move |e: MouseEvent| {
        let Ok(mut s) = s.try_borrow_mut() else { return };
        if std::mem::take(&mut s.eat_click) || s.clicked(&e) {
            e.stop_propagation();
            e.prevent_default();
        }
    });
    let s = st.clone();
    listen(&side, "dblclick", false, false, move |e: MouseEvent| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.double_clicked(&e);
        }
    });
    // A new node in the sidebar replaces the charts: forget the old ones.
    let s = st;
    let side_el: Element = side.clone().dyn_into().expect("an element");
    let cb = Closure::<dyn FnMut()>::new(move || {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.drag = None;
            if s.over.as_ref().is_some_and(|o| !o.is_connected()) {
                s.over = None;
                s.hide_tip();
            }
        }
    });
    if let Ok(mo) = web_sys::MutationObserver::new(cb.as_ref().unchecked_ref()) {
        let opts = web_sys::MutationObserverInit::new();
        opts.set_child_list(true);
        let _ = mo.observe_with_options(&side_el, &opts);
    }
    cb.forget();
}
