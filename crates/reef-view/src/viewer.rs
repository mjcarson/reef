//! One drawn view: its DOM, its state, and its event handlers.

use crate::Plan;
use crate::accept::{self, Filter};
use crate::markup::{self, BoxSize, FONT_PX, HEADER_H};
use reef_layout::{Dir, Input, LEdge, LNode, Output, Spacing};
use reef_model::{Direction, Layout, ViewSpec, Visible};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, PointerEvent, WheelEvent};

const MIN_K: f64 = 0.05;
const MAX_K: f64 = 4.0;
/// The narrowest the sidebar can be dragged.
const SIDE_MIN_W: f64 = 240.0;

struct Press {
    x: f64,
    y: f64,
    moved: bool,
    target: Option<Element>,
}

pub struct Viewer {
    el: HtmlElement,
    svg: Element,
    vp: Element,
    side: HtmlElement,
    stage: HtmlElement,
    grip: HtmlElement,
    /// The pointer dragging the sidebar's edge, and whether it has moved.
    side_drag: Option<(i32, bool)>,
    search: HtmlInputElement,
    results: HtmlElement,
    hint: HtmlElement,
    plan: Rc<Plan>,
    dir: Dir,
    root: Option<usize>,
    ghosts: bool,
    expanded: HashSet<usize>,
    vis: Visible,
    out: Output,
    sizes: Vec<BoxSize>,
    k: f64,
    tx: f64,
    ty: f64,
    selected: Option<usize>,
    hover: Option<usize>,
    uid: String,
    storage_key: String,
    ctx: Option<CanvasRenderingContext2d>,
    pointers: Vec<(i32, f64, f64)>,
    press: Option<Press>,
    pinch: Option<f64>,
    hint_timer: Option<i32>,
    /// Filling the page beside the book's sidebar: a plain wheel zooms, as in full screen.
    page_mode: bool,
    /// The stage size last laid out for, to keep the view centred when it changes.
    seen: (f64, f64),
    /// Which kinds of acceptance criteria are shown, shared by every view in the book.
    acc: Filter,
    /// What the view shows, kept to start it again when its plan is swapped
    spec: ViewSpec,
    /// The sidebar's charts, told of each swapped plan
    charts: Option<crate::chartui::Charts>,
    /// Who hears when the selection changes, given the selected node's id
    on_select: Option<OnSelect>,
}

/// A callback told the selected node's id, or `None`, each time the selection changes
pub type OnSelect = Rc<RefCell<Box<dyn FnMut(Option<String>)>>>;

/// Tells a selection callback what is selected now, unless it is running already
///
/// # Arguments
///
/// * `callback` - Who to tell
/// * `id` - The id of the node selected now, or `None`
pub fn tell(callback: &OnSelect, id: Option<String>) {
    // a callback that changes the selection itself isn't told of that change again
    if let Ok(mut callback) = callback.try_borrow_mut() {
        callback(id);
    }
}

/// The key a view's open groups are saved under, given the name of the plan it draws
///
/// # Arguments
///
/// * `plan` - The plan's name, such as `reef/central`
pub fn plan_key(plan: &str) -> String {
    format!("reef:plan:{plan}")
}

/// The groups a view opens with: those saved under its key, else its spec's
///
/// # Arguments
///
/// * `key` - Where the view's open groups are saved
/// * `spec` - What the view shows
/// * `g` - The plan it draws
fn open_groups(key: &str, spec: &ViewSpec, g: &reef_model::Graph) -> HashSet<usize> {
    // the groups opened last time, by id, as far as the plan still has them
    let saved = storage()
        .and_then(|s| s.get_item(key).ok().flatten())
        .and_then(|saved| serde_json::from_str::<Vec<String>>(&saved).ok());
    match saved {
        Some(ids) => ids.iter().filter_map(|id| g.index_of(id)).collect(),
        None => spec.initial_expanded(g),
    }
}

/// The legend's dots for each kind of acceptance criterion
fn acc_legend() -> String {
    reef_model::Kind::ALL
        .iter()
        .map(|k| format!(r#"<span><i class="reef-acc-dot k-{}"></i>{}</span>"#, k.as_str(), k.label()))
        .collect()
}

/// The event every view on the page hears when the acceptance filter changes.
const ACC_EVENT: &str = "reef-acceptance";

fn load_filter() -> Filter {
    storage().and_then(|s| s.get_item(Filter::STORAGE_KEY).ok().flatten()).map_or_else(Filter::default, |v| Filter::from_stored(&v))
}

fn window() -> web_sys::Window {
    web_sys::window().expect("a window")
}

fn q(el: &Element, sel: &str) -> Element {
    el.query_selector(sel).ok().flatten().unwrap_or_else(|| panic!("reef: no {sel}"))
}

/// A listener a view added: what it listens on, for what, whether as the event goes down, and
/// the closure it calls
type Listener = (web_sys::EventTarget, &'static str, bool, Closure<dyn FnMut(JsValue)>);

/// Everything a view hooked into the page: each listener with its closure, and its observers
///
/// The closures hold the view, so the view lives as long as they do. Disconnecting takes each
/// listener off and each observer away, then drops the closures, which frees the view. Kept
/// apart from the view, which they hold, so neither keeps the other alive.
#[derive(Default)]
pub struct Wiring {
    /// Every listener the view and its charts added
    listeners: Vec<Listener>,
    /// The observers watching the view's element and its sidebar
    observers: Vec<Observer>,
    /// The observers' callbacks, kept while they observe
    callbacks: Vec<Closure<dyn FnMut()>>,
}

/// An observer a view started
pub enum Observer {
    /// Watches the view's size
    Resize(web_sys::ResizeObserver),
    /// Watches the sidebar's contents
    Mutation(web_sys::MutationObserver),
}

impl Wiring {
    /// Adds a listener, keeping its closure until the view is torn down
    ///
    /// # Arguments
    ///
    /// * `target` - What to listen on
    /// * `kind` - The event's name
    /// * `capture` - Whether to hear it as it goes down, before its target does
    /// * `passive` - Whether the listener never prevents the default
    /// * `f` - What to do with each event
    pub fn listen<E: JsCast + 'static>(&mut self, target: &web_sys::EventTarget, kind: &'static str, capture: bool, passive: bool, mut f: impl FnMut(E) + 'static) {
        // one closure type for every event, each cast to the type its listener expects
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| f(e.unchecked_into::<E>()));
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_capture(capture);
        opts.set_passive(passive);
        let _ = target.add_event_listener_with_callback_and_add_event_listener_options(kind, cb.as_ref().unchecked_ref(), &opts);
        self.listeners.push((target.clone(), kind, capture, cb));
    }

    /// Keeps an observer, and the callback it calls, until the view is torn down
    ///
    /// # Arguments
    ///
    /// * `observer` - The observer, already observing
    /// * `callback` - What it calls
    pub fn observe(&mut self, observer: Observer, callback: Closure<dyn FnMut()>) {
        self.observers.push(observer);
        self.callbacks.push(callback);
    }

    /// Takes every listener off and every observer away, then drops their closures
    ///
    /// A closure running now finishes first: wasm-bindgen frees it once it returns.
    pub fn disconnect(self) {
        // nothing calls into the view after this
        for (target, kind, capture, cb) in &self.listeners {
            let _ = target.remove_event_listener_with_callback_and_bool(kind, cb.as_ref().unchecked_ref(), *capture);
        }
        for observer in &self.observers {
            match observer {
                Observer::Resize(ro) => ro.disconnect(),
                Observer::Mutation(mo) => mo.disconnect(),
            }
        }
        // the closures, and the view each holds, go with self
    }
}

/// Runs `f` on the viewer unless it is already busy (an event fired from inside a handler), then
/// tells the selection callback if `f` changed what is selected
///
/// # Arguments
///
/// * `v` - The viewer
/// * `f` - What to do with it
fn with(v: &Rc<RefCell<Viewer>>, f: impl FnOnce(&mut Viewer)) {
    // run it, noting what was selected before
    let told = match v.try_borrow_mut() {
        Ok(mut v) => {
            let before = v.selected;
            f(&mut v);
            // the plan is the same, so its indexes compare
            match (v.selected != before, &v.on_select) {
                (true, Some(callback)) => Some((callback.clone(), v.selected_id())),
                _ => None,
            }
        }
        Err(_) => None,
    };
    // the callback runs with the viewer free, so it may call the view's handle
    if let Some((callback, id)) = told {
        tell(&callback, id);
    }
}

fn closest_item(target: &Option<Element>, sel: &str, attr: &str) -> Option<usize> {
    target.as_ref()?.closest(sel).ok()??.get_attribute(attr)?.parse().ok()
}

impl Viewer {
    /// Draws a view of a plan into an element, and returns it with what it hooked into the page
    ///
    /// # Arguments
    ///
    /// * `el` - The element to draw into, in place of whatever it holds
    /// * `plan` - The plan to draw
    /// * `spec` - What the view shows
    /// * `index` - Which view on the page this is, for its SVG ids
    /// * `storage_key` - Where the view's open groups are saved
    pub fn mount(el: HtmlElement, plan: Rc<Plan>, spec: ViewSpec, index: usize, storage_key: String) -> (Rc<RefCell<Viewer>>, Wiring) {
        let uid = format!("{index}-{}", (js_sys::Math::random() * 1e9) as u64);
        let _ = el.class_list().add_1("reef-mounted");
        let _ = el.set_attribute("tabindex", "0");
        let page_mode = spec.layout == Layout::Page;
        let open_page = spec.page.as_deref().map_or(String::new(), |href| {
            format!(r#"<a class="reef-button" href="{}" title="Open this plan on a page of its own">Full page ↗</a>"#, markup::esc(href))
        });
        let acc = load_filter();
        let has_acc = plan.graph.has_acceptance();
        let acc_bar = if has_acc { accept::toolbar(&acc) } else { String::new() };
        let acc_legend = if has_acc { acc_legend() } else { String::new() };
        let hint_text = if page_mode { "Scroll to zoom · drag to pan" } else { "Ctrl + scroll to zoom · drag to pan" };
        el.set_inner_html(&format!(
            r##"<div class="reef-toolbar"><div class="reef-searchbox"><input class="reef-search" type="search" placeholder="Search nodes ( / )" aria-label="Search nodes" autocomplete="off"><ul class="reef-results" hidden></ul></div><span class="reef-spacer"></span>{acc_bar}<button data-act="expand" title="Open every group">Expand all</button><button data-act="collapse" title="Close every group">Collapse all</button><button data-act="out" title="Zoom out ( - )">−</button><button data-act="in" title="Zoom in ( + )">+</button><button data-act="fit" title="Fit to view ( 0 )">Fit</button>{open_page}<button data-act="full" title="Full screen ( f )">⛶</button></div><div class="reef-stage"><svg class="reef-svg" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Plan diagram"><defs><marker id="reef-arrow-{uid}" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="9" markerHeight="9" orient="auto-start-reverse"><path class="reef-arrowhead" d="M0,0 L10,5 L0,10 z"/></marker></defs><g class="reef-viewport"></g></svg><aside class="reef-side" hidden></aside><div class="reef-side-grip" role="separator" aria-orientation="vertical" aria-label="Resize sidebar" tabindex="0" title="Drag to resize · double-click to reset"></div><div class="reef-legend"><span><i class="reef-dot s-done"></i>done</span><span><i class="reef-dot s-progress"></i>in progress</span><span><i class="reef-dot s-ready"></i>ready</span><span><i class="reef-dot s-todo"></i>to do</span><span><i class="reef-dot s-blocked"></i>blocked</span><span><i class="reef-dot optional"></i>optional</span>{acc_legend}</div><div class="reef-hint">{hint_text}</div></div>"##
        ));
        let svg = q(&el, ".reef-svg");
        let vp = q(&el, ".reef-viewport");
        let side: HtmlElement = q(&el, ".reef-side").unchecked_into();
        let stage: HtmlElement = q(&el, ".reef-stage").unchecked_into();
        let grip: HtmlElement = q(&el, ".reef-side-grip").unchecked_into();
        let search: HtmlInputElement = q(&el, ".reef-search").unchecked_into();
        let results: HtmlElement = q(&el, ".reef-results").unchecked_into();
        let hint: HtmlElement = q(&el, ".reef-hint").unchecked_into();
        let g = &plan.graph;
        let root = spec.root_id().and_then(|id| g.index_of(id));
        let ghosts = spec.focus.is_some();
        let expanded = open_groups(&storage_key, &spec, g);
        let ctx = measure_ctx(&el);
        let v = Viewer {
            el,
            svg,
            vp,
            side,
            stage,
            grip,
            side_drag: None,
            search,
            results,
            hint,
            plan,
            dir: if spec.direction == Direction::LR { Dir::LR } else { Dir::TB },
            root,
            ghosts,
            expanded,
            vis: Visible::default(),
            out: Output::default(),
            sizes: Vec::new(),
            k: 1.0,
            tx: 0.0,
            ty: 0.0,
            selected: None,
            hover: None,
            uid,
            storage_key,
            ctx,
            pointers: Vec::new(),
            press: None,
            pinch: None,
            hint_timer: None,
            page_mode,
            seen: (0.0, 0.0),
            acc,
            spec,
            charts: None,
            on_select: None,
        };
        let v = Rc::new(RefCell::new(v));
        {
            let mut b = v.borrow_mut();
            let w = storage().and_then(|s| s.get_item(b.side_w_key()).ok().flatten()).and_then(|w| w.parse().ok());
            b.set_side_w(w);
            b.render();
            b.initial_view();
            b.seen = b.stage_size();
        }
        let wiring = Viewer::attach(&v);
        (v, wiring)
    }

    /// Empties the view's element, once its wiring is gone, so the element can be drawn into
    /// again
    ///
    /// The selection callback and the charts are let go too, since they may hold the caller's
    /// state.
    pub fn unmounted(&mut self) {
        // a hint still showing would clear itself on a detached element
        if let Some(t) = self.hint_timer.take() {
            window().clear_timeout_with_handle(t);
        }
        // the element as it was before the view drew into it
        self.el.set_inner_html("");
        let classes = self.el.class_list();
        let _ = classes.remove_3("reef-mounted", "reef-has-side", "reef-resizing");
        for attr in ["tabindex", "data-boxes", "data-layout-ms"] {
            let _ = self.el.remove_attribute(attr);
        }
        // nothing outside the view is told of it again
        self.on_select = None;
        self.charts = None;
    }

    /// Draws a newer revision of this view's plan where the old one was
    ///
    /// The open groups, the selection, the root and the search are found again by id, and the
    /// pan, the zoom and the sidebar's scroll stay as they were. A selected node the new plan
    /// lacks is let go, and its sidebar closed.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to draw now
    pub fn swap(&mut self, plan: Rc<Plan>) {
        // what is open and selected, by id, as the old plan knows it
        let old = self.plan.clone();
        let open: Vec<&str> = self.expanded.iter().map(|&i| old.graph.node(i).id.as_str()).collect();
        let selected = self.selected_id();
        // take the new plan, and find each of them in it
        self.plan = plan;
        let plan = self.plan.clone();
        let g = &plan.graph;
        self.root = self.spec.root_id().and_then(|id| g.index_of(id));
        self.expanded = open.iter().filter_map(|id| g.index_of(id)).collect();
        self.selected = selected.and_then(|id| g.index_of(&id));
        // the charts and the toolbar's criteria follow the new plan
        self.replanned();
        // draw it at the same pan and zoom
        self.render();
        // the sidebar shows the selected node as it is now, at the scroll it had, or closes
        match self.selected {
            Some(n) => {
                let top = self.side.scroll_top();
                self.show_side(n);
                self.side.set_scroll_top(top);
            }
            None => {
                self.side.set_hidden(true);
                let _ = self.el.class_list().remove_1("reef-has-side");
            }
        }
        self.refresh_selection();
        // open search results are found again in the new plan
        if !self.results.hidden() {
            self.update_results();
        }
    }

    /// Draws another plan in this view, started from its own view and its own saved groups
    ///
    /// Nothing is selected, the search is cleared, and the view is fitted afresh. The layout and
    /// height stay as the element was mounted.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan to draw now
    /// * `spec` - What the view shows of it
    /// * `storage_key` - Where its open groups are saved
    pub fn swap_plan(&mut self, plan: Rc<Plan>, spec: ViewSpec, storage_key: String) {
        // take the plan and its view, and find the view's root and open groups in it
        self.plan = plan;
        let plan = self.plan.clone();
        let g = &plan.graph;
        self.root = spec.root_id().and_then(|id| g.index_of(id));
        self.ghosts = spec.focus.is_some();
        self.dir = if spec.direction == Direction::LR { Dir::LR } else { Dir::TB };
        self.expanded = open_groups(&storage_key, &spec, g);
        self.storage_key = storage_key;
        self.spec = spec;
        // indexes into the old plan mean nothing now: drop the selection and any search results
        self.selected = None;
        self.side.set_hidden(true);
        let _ = self.el.class_list().remove_1("reef-has-side");
        self.search.set_value("");
        self.results.set_hidden(true);
        // the charts and the toolbar's criteria follow the new plan
        self.replanned();
        // draw it, and fit it as a new view would be
        self.render();
        self.initial_view();
    }

    /// The id of the node selected now, if one is
    pub fn selected_id(&self) -> Option<String> {
        self.selected.map(|i| self.graph().node(i).id.clone())
    }

    /// Sets who hears when the selection changes
    ///
    /// # Arguments
    ///
    /// * `callback` - Told the selected node's id, or `None`
    pub fn set_on_select(&mut self, callback: OnSelect) {
        self.on_select = Some(callback);
    }

    /// The selection callback and what to tell it, if the selected node's id moved off `before`
    ///
    /// # Arguments
    ///
    /// * `before` - The id of the node that was selected
    pub fn moved_from(&self, before: Option<String>) -> Option<(OnSelect, Option<String>)> {
        // ids, since a swap's indexes are the new plan's
        let now = self.selected_id();
        match (now != before, &self.on_select) {
            (true, Some(callback)) => Some((callback.clone(), now)),
            _ => None,
        }
    }

    /// Points what draws from the plan outside `render` at the plan just taken: the charts'
    /// handlers, and the toolbar's criteria chips and legend dots, which only a plan with
    /// acceptance criteria shows
    fn replanned(&mut self) {
        // the sidebar's charts redraw from the new plan
        if let Some(charts) = &self.charts {
            charts.set_plan(self.plan.clone());
        }
        // the chips and dots come and go with whether the plan has criteria
        let has = self.graph().has_acceptance();
        let bar = self.el.query_selector(".reef-toolbar .reef-acc-filter").ok().flatten();
        match (has, bar) {
            (true, None) => {
                // the chips sit before the toolbar's buttons, the dots after the legend's statuses
                if let Ok(Some(expand)) = self.el.query_selector(".reef-toolbar [data-act=expand]") {
                    let _ = expand.insert_adjacent_html("beforebegin", &accept::toolbar(&self.acc));
                }
                if let Ok(Some(legend)) = self.el.query_selector(".reef-legend") {
                    let _ = legend.insert_adjacent_html("beforeend", &acc_legend());
                }
            }
            (false, Some(bar)) => {
                // each dot is its legend entry's only mark, so the entry goes with it
                bar.remove();
                if let Ok(dots) = self.el.query_selector_all(".reef-legend .reef-acc-dot") {
                    for k in 0..dots.length() {
                        if let Some(entry) = dots.item(k).and_then(|dot| dot.parent_element()) {
                            entry.remove();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn graph(&self) -> &reef_model::Graph {
        &self.plan.graph
    }

    fn measure(&self, s: &str) -> f64 {
        match &self.ctx {
            Some(c) => c.measure_text(s).map(|m| m.width()).unwrap_or(s.len() as f64 * 7.0),
            None => s.chars().count() as f64 * FONT_PX * 0.55,
        }
    }

    /// Lay out and draw what the view shows now.
    fn render(&mut self) {
        let t0 = window().performance().map(|p| p.now());
        let plan = self.plan.clone();
        let g = &plan.graph;
        self.vis = g.visible(self.root, self.ghosts, &self.expanded);
        let sizes = markup::sizes(g, &self.vis, &self.acc, &|s| self.measure(s));
        let input = Input {
            dir: self.dir,
            nodes: self.vis.items.iter().zip(&sizes).map(|(it, s)| LNode { parent: it.parent, w: s.w, h: s.h, container: it.open }).collect(),
            edges: self.vis.edges.iter().map(|e| LEdge { from: e.from, to: e.to }).collect(),
            spacing: Spacing { header: HEADER_H, ..Spacing::default() },
        };
        self.out = reef_layout::layout(&input);
        self.sizes = sizes;
        let sel = self.selected.and_then(|s| self.vis.shown_as[s]);
        let markup = markup::diagram(g, &self.vis, &self.out, &self.sizes, self.dir, &self.uid, sel);
        self.vp.set_inner_html(&markup);
        self.hover = None;
        let _ = self.svg.class_list().remove_1("reef-dim");
        if let (Some(t0), Some(p)) = (t0, window().performance()) {
            let _ = self.el.set_attribute("data-layout-ms", &format!("{:.1}", p.now() - t0));
        }
        let _ = self.el.set_attribute("data-boxes", &self.vis.items.len().to_string());
        self.apply();
    }

    fn apply(&self) {
        let _ = self.vp.set_attribute("transform", &format!("translate({:.2},{:.2}) scale({:.4})", self.tx, self.ty, self.k));
    }

    fn stage_size(&self) -> (f64, f64) {
        let r = self.svg.get_bounding_client_rect();
        (r.width().max(1.0), r.height().max(1.0))
    }

    /// The part of the stage the sidebar leaves free.
    fn free_width(&self) -> f64 {
        let (w, _) = self.stage_size();
        if self.side.hidden() { w } else { (w - self.side.get_bounding_client_rect().width()).max(w * 0.3) }
    }

    /// The stage changed size (the window, or the book's sidebar opening or closing): keep what
    /// was in the middle in the middle, or lay out afresh if it had no size before.
    fn resized(&mut self) {
        let (w, h) = self.stage_size();
        let (pw, ph) = self.seen;
        if (w - pw).abs() < 1.0 && (h - ph).abs() < 1.0 {
            return;
        }
        self.seen = (w, h);
        if pw <= 1.0 || ph <= 1.0 {
            self.initial_view();
        } else {
            self.tx += (w - pw) / 2.0;
            self.ty += (h - ph) / 2.0;
            self.apply();
        }
    }

    fn fit(&mut self) {
        let (_, h) = self.stage_size();
        let w = self.free_width();
        let (cw, ch) = (self.out.width.max(1.0), self.out.height.max(1.0));
        self.k = (w / cw).min(h / ch).clamp(MIN_K, 1.0);
        self.tx = (w - cw * self.k) / 2.0;
        self.ty = ((h - ch * self.k) / 2.0).max(0.0);
        self.apply();
    }

    /// Fit, unless that would make labels too small to read: then start at a readable size,
    /// at the top, and leave the rest to panning, zooming out, or Fit.
    fn initial_view(&mut self) {
        self.fit();
        // Below this a 13px label is unreadable; start at READABLE instead.
        const TOO_SMALL: f64 = 0.35;
        const READABLE: f64 = 0.55;
        if self.k < TOO_SMALL {
            let w = self.free_width();
            self.k = READABLE;
            self.tx = (w - self.out.width * self.k) / 2.0;
            self.ty = 0.0;
            self.apply();
        }
    }

    fn zoom_at(&mut self, factor: f64, sx: f64, sy: f64) {
        let k = (self.k * factor).clamp(MIN_K, MAX_K);
        self.tx = sx - (sx - self.tx) * (k / self.k);
        self.ty = sy - (sy - self.ty) * (k / self.k);
        self.k = k;
        self.apply();
    }

    fn zoom_center(&mut self, factor: f64) {
        let (_, h) = self.stage_size();
        self.zoom_at(factor, self.free_width() / 2.0, h / 2.0);
    }

    fn save(&self) {
        let g = self.graph();
        let ids: Vec<&str> = self.expanded.iter().map(|&i| g.node(i).id.as_str()).collect();
        if let Some(s) = storage() {
            let _ = s.set_item(&self.storage_key, &serde_json::to_string(&ids).unwrap_or_default());
        }
    }

    /// Open or close the group shown as item `item`, keeping it where it is on screen.
    fn toggle(&mut self, item: usize) {
        let Some(it) = self.vis.items.get(item) else { return };
        let node = it.node;
        if !self.graph().is_group(node) {
            return;
        }
        let before = self.out.rects[item];
        if !self.expanded.remove(&node) {
            self.expanded.insert(node);
        }
        self.save();
        self.render();
        if let Some(after) = self.vis.shown_as[node].map(|i| self.out.rects[i]) {
            self.tx += (before.x - after.x) * self.k;
            self.ty += (before.y - after.y) * self.k;
            self.apply();
        }
        if self.selected == Some(node) {
            self.show_side(node);
        }
    }

    fn set_all(&mut self, open: bool) {
        let g = self.plan.clone();
        let g = &g.graph;
        self.expanded = if open { (0..g.len()).filter(|&i| g.is_group(i)).collect() } else { HashSet::new() };
        self.save();
        self.render();
        self.fit();
    }

    fn show_side(&mut self, node: usize) {
        let g = self.graph();
        let open = g.is_group(node).then(|| self.expanded.contains(&node));
        self.side.set_inner_html(&markup::sidebar(g, node, open, &self.acc));
        // Docs start open, and a details put in already open fires no `toggle` everywhere: fetch them now.
        if let Ok(list) = self.side.query_selector_all("details.reef-doc-read[open], details.reef-perf-doc[open]") {
            for k in 0..list.length() {
                if let Some(d) = list.item(k).and_then(|d| d.dyn_into::<web_sys::HtmlDetailsElement>().ok()) {
                    crate::docs::open(d);
                }
            }
        }
        self.side.set_hidden(false);
        self.side.set_scroll_top(0);
        let _ = self.el.class_list().add_1("reef-has-side");
    }

    /// Where the sidebar's width is kept: one for views filling a page, one for views in the text.
    fn side_w_key(&self) -> &'static str {
        if self.page_mode { "reef:side-w:page" } else { "reef:side-w:inline" }
    }

    /// Set the sidebar's width in px, or `None` for the stylesheet's. It never takes more than
    /// the stage less a strip of the diagram, however the stage shrinks later.
    fn set_side_w(&self, px: Option<f64>) {
        let style = self.stage.style();
        match px {
            Some(px) => {
                let _ = style.set_property("--reef-side-w", &format!("min({:.0}px, calc(100% - 120px))", px.max(SIDE_MIN_W)));
            }
            None => {
                let _ = style.remove_property("--reef-side-w");
            }
        }
    }

    /// Keep the sidebar's width as drawn now, for the next view of this kind.
    fn save_side_w(&self) {
        if let Some(s) = storage() {
            let w = self.side.get_bounding_client_rect().width();
            let _ = s.set_item(self.side_w_key(), &format!("{w:.0}"));
        }
    }

    fn select(&mut self, node: Option<usize>) {
        self.selected = node;
        match node {
            Some(n) => self.show_side(n),
            None => {
                self.side.set_hidden(true);
                let _ = self.el.class_list().remove_1("reef-has-side");
            }
        }
        self.refresh_selection();
    }

    fn refresh_selection(&self) {
        let sel = self.selected.and_then(|s| self.vis.shown_as[s]);
        let Ok(list) = self.vp.query_selector_all(".reef-node, .reef-edge") else { return };
        for k in 0..list.length() {
            let Some(e) = list.item(k).and_then(|n| n.dyn_into::<Element>().ok()) else { continue };
            let attr = |a: &str| e.get_attribute(a).and_then(|v| v.parse::<usize>().ok());
            if let Some(i) = attr("data-item") {
                let _ = e.class_list().toggle_with_force("sel", Some(i) == sel);
            } else {
                let on = sel.is_some() && (attr("data-from") == sel || attr("data-to") == sel);
                let _ = e.class_list().toggle_with_force("hl", on);
            }
        }
    }

    /// Light up everything upstream and downstream of item `item`.
    fn light(&mut self, item: Option<usize>) {
        if self.hover == item {
            return;
        }
        self.hover = item;
        let Ok(list) = self.vp.query_selector_all(".reef-node, .reef-group, .reef-edge") else { return };
        let lit: HashSet<usize> = match item {
            Some(i) => self.lineage(i),
            None => HashSet::new(),
        };
        let _ = self.svg.class_list().toggle_with_force("reef-dim", item.is_some());
        for k in 0..list.length() {
            let Some(e) = list.item(k).and_then(|n| n.dyn_into::<Element>().ok()) else { continue };
            let attr = |a: &str| e.get_attribute(a).and_then(|v| v.parse::<usize>().ok());
            let on = if let Some(i) = attr("data-item") {
                lit.contains(&i)
            } else {
                matches!((attr("data-from"), attr("data-to")), (Some(f), Some(t)) if lit.contains(&f) && lit.contains(&t))
            };
            let _ = e.class_list().toggle_with_force("lit", on);
        }
    }

    fn lineage(&self, start: usize) -> HashSet<usize> {
        let mut out = HashSet::from([start]);
        for forward in [true, false] {
            let mut stack = vec![start];
            let mut seen = HashSet::from([start]);
            while let Some(v) = stack.pop() {
                for e in &self.vis.edges {
                    let (a, b) = if forward { (e.from, e.to) } else { (e.to, e.from) };
                    if a == v && seen.insert(b) {
                        stack.push(b);
                    }
                }
            }
            out.extend(seen);
        }
        // Open groups holding a lit box stay lit too.
        let boxes: Vec<usize> = out.iter().copied().collect();
        for i in boxes {
            let mut p = self.vis.items[i].parent;
            while let Some(c) = p {
                out.insert(c);
                p = self.vis.items[c].parent;
            }
        }
        out
    }

    /// Show node `node`: open the groups hiding it, pan to it and select it.
    fn goto(&mut self, node: usize) {
        let in_view = |v: &Viewer| v.vis.shown_as[node].is_some_and(|i| v.vis.items[i].node == node);
        if !in_view(self) {
            let g = self.plan.clone();
            for a in g.graph.ancestors(node) {
                if Some(a) == self.root {
                    break;
                }
                self.expanded.insert(a);
            }
            self.save();
            self.render();
        }
        self.select(Some(node));
        self.center_on(node);
    }

    fn center_on(&mut self, node: usize) {
        let Some(item) = self.vis.shown_as[node] else { return };
        let r = self.out.rects[item];
        let (_, h) = self.stage_size();
        let w = self.free_width();
        if self.k < 0.7 {
            self.k = 0.9;
        }
        self.tx = w / 2.0 - r.cx() * self.k;
        self.ty = h / 2.0 - r.cy() * self.k;
        self.apply();
        let _ = self.svg.class_list().add_1("reef-flash");
        let svg = self.svg.clone();
        let cb = Closure::once_into_js(move || {
            let _ = svg.class_list().remove_1("reef-flash");
        });
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 900);
    }

    fn search_matches(&self, text: &str) -> Vec<usize> {
        let g = self.graph();
        let t = text.trim().to_lowercase();
        if t.is_empty() {
            return Vec::new();
        }
        let in_scope = |i: usize| self.root.is_none_or(|r| g.is_ancestor(r, i));
        let mut hits: Vec<(u8, usize)> = (0..g.len())
            .filter_map(|i| {
                let n = g.node(i);
                let id = n.id.to_lowercase();
                let title = n.title.to_lowercase();
                let rank = if id == t {
                    0
                } else if id.starts_with(&t) || title.starts_with(&t) {
                    1
                } else if title.contains(&t) || id.contains(&t) {
                    2
                } else {
                    return None;
                };
                Some((rank + if in_scope(i) { 0 } else { 3 }, i))
            })
            .collect();
        hits.sort();
        hits.into_iter().take(12).map(|h| h.1).collect()
    }

    fn update_results(&self) {
        let hits = self.search_matches(&self.search.value());
        if hits.is_empty() {
            self.results.set_hidden(true);
            return;
        }
        let g = self.graph();
        let html: String = hits
            .iter()
            .map(|&i| {
                let n = g.node(i);
                format!(
                    r#"<li><a href="javascript:void 0" data-goto="{}"><span class="reef-dot {}"></span>{} <code>{}</code></a></li>"#,
                    markup::esc(&n.id),
                    if g.is_ready(i) { "s-ready" } else { status_dot(g.status(i)) },
                    markup::esc(&n.title),
                    markup::esc(&n.id)
                )
            })
            .collect();
        self.results.set_inner_html(&html);
        self.results.set_hidden(false);
    }

    fn flash_hint(&mut self) {
        let _ = self.hint.class_list().add_1("on");
        if let Some(t) = self.hint_timer.take() {
            window().clear_timeout_with_handle(t);
        }
        let hint = self.hint.clone();
        let cb = Closure::once_into_js(move || {
            let _ = hint.class_list().remove_1("on");
        });
        self.hint_timer = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 1400).ok();
    }

    fn fullscreen(&self) {
        let doc = window().document().expect("a document");
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else {
            let _ = self.el.request_fullscreen();
        }
    }

    fn is_fullscreen(&self) -> bool {
        window().document().and_then(|d| d.fullscreen_element()).is_some_and(|e| e == *self.el)
    }

    /// Show or hide kind `k` of acceptance criteria, in every view in the book.
    fn toggle_kind(&mut self, k: reef_model::Kind) {
        let f = self.acc.toggled(k);
        if let Some(s) = storage() {
            let _ = s.set_item(Filter::STORAGE_KEY, &f.to_stored());
        }
        self.set_filter(f);
        // The other views on this page follow.
        if let Some(doc) = window().document()
            && let Ok(e) = Event::new(ACC_EVENT)
        {
            let _ = doc.dispatch_event(&e);
        }
    }

    /// Draw again with filter `f`: the boxes' pills, the toolbar's chips, and the sidebar's
    /// Acceptance section, in place so the sidebar keeps its scroll.
    fn set_filter(&mut self, f: Filter) {
        if f == self.acc {
            return;
        }
        self.acc = f;
        self.render();
        self.refresh_selection();
        if let Ok(Some(bar)) = self.el.query_selector(".reef-toolbar .reef-acc-filter") {
            bar.set_outer_html(&accept::toolbar(&f));
        }
        if let (Some(n), false) = (self.selected, self.side.hidden())
            && let Ok(Some(sec)) = self.side.query_selector(".reef-acc")
        {
            let html = accept::section(self.graph(), n, &f);
            sec.set_outer_html(&html);
        }
    }

    fn act(&mut self, act: &str) {
        if let Some(k) = accept::act_kind(act) {
            self.toggle_kind(k);
            return;
        }
        match act {
            "fit" => self.fit(),
            "in" => self.zoom_center(1.25),
            "out" => self.zoom_center(0.8),
            "expand" => self.set_all(true),
            "collapse" => self.set_all(false),
            "full" => self.fullscreen(),
            "close" => self.select(None),
            "toggle-sel" => {
                if let Some(item) = self.selected.and_then(|s| self.vis.shown_as[s]).filter(|&i| Some(self.vis.items[i].node) == self.selected) {
                    self.toggle(item);
                } else if let Some(s) = self.selected {
                    self.goto(s);
                    if let Some(item) = self.vis.shown_as[s] {
                        self.toggle(item);
                    }
                }
            }
            "center" => {
                if let Some(s) = self.selected {
                    self.goto(s);
                }
            }
            _ => {}
        }
    }

    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        let r = self.svg.get_bounding_client_rect();
        (x - r.left(), y - r.top())
    }

    /// Hooks the view into the page: its listeners, its charts' and its observers, returned
    /// so the caller can take them away again
    ///
    /// # Arguments
    ///
    /// * `v` - The view
    fn attach(v: &Rc<RefCell<Viewer>>) -> Wiring {
        // everything added here, to take off again when the view is torn down
        let mut w = Wiring::default();
        let (el, svg, search): (web_sys::EventTarget, web_sys::EventTarget, web_sys::EventTarget) = {
            let b = v.borrow();
            (b.el.clone().into(), b.svg.clone().into(), b.search.clone().into())
        };

        // Toolbar, sidebar and search results.
        let vc = v.clone();
        w.listen(&el, "click", false, true, move |e: Event| {
            let t: Option<Element> = e.target().and_then(|t| t.dyn_into().ok());
            let Some(t) = t else { return };
            if let Ok(Some(a)) = t.closest("[data-act]") {
                let act = a.get_attribute("data-act").unwrap_or_default();
                with(&vc, |v| v.act(&act));
            } else if let Ok(Some(a)) = t.closest("[data-goto]") {
                e.prevent_default();
                let id = a.get_attribute("data-goto").unwrap_or_default();
                with(&vc, |v| {
                    if let Some(n) = v.graph().index_of(&id) {
                        v.results.set_hidden(true);
                        v.goto(n);
                    }
                });
            }
        });

        let vc = v.clone();
        w.listen(&search, "input", false, true, move |_: Event| with(&vc, |v| v.update_results()));
        let vc = v.clone();
        w.listen(&search, "keydown", false, false, move |e: KeyboardEvent| {
            e.stop_propagation();
            match e.key().as_str() {
                "Enter" => with(&vc, |v| {
                    if let Some(&n) = v.search_matches(&v.search.value()).first() {
                        v.results.set_hidden(true);
                        v.goto(n);
                    }
                }),
                "Escape" => with(&vc, |v| {
                    v.search.set_value("");
                    v.results.set_hidden(true);
                    let _ = v.el.focus();
                }),
                _ => {}
            }
        });

        // Pan, pinch and click on the diagram.
        let vc = v.clone();
        w.listen(&svg, "pointerdown", false, true, move |e: PointerEvent| {
            if e.button() != 0 && e.pointer_type() == "mouse" {
                return;
            }
            with(&vc, |v| {
                let _ = v.svg.set_pointer_capture(e.pointer_id());
                v.pointers.retain(|p| p.0 != e.pointer_id());
                v.pointers.push((e.pointer_id(), e.client_x() as f64, e.client_y() as f64));
                if v.pointers.len() == 1 {
                    v.press = Some(Press { x: e.client_x() as f64, y: e.client_y() as f64, moved: false, target: e.target().and_then(|t| t.dyn_into().ok()) });
                } else {
                    v.press = None;
                    v.pinch = None;
                }
            });
        });
        let vc = v.clone();
        w.listen(&svg, "pointermove", false, true, move |e: PointerEvent| {
            with(&vc, |v| {
                let (x, y) = (e.client_x() as f64, e.client_y() as f64);
                let Some(idx) = v.pointers.iter().position(|p| p.0 == e.pointer_id()) else {
                    // Hovering.
                    let t: Option<Element> = e.target().and_then(|t| t.dyn_into().ok());
                    let item = closest_item(&t, ".reef-node", "data-item");
                    v.light(item);
                    return;
                };
                let (_, px, py) = v.pointers[idx];
                v.pointers[idx] = (e.pointer_id(), x, y);
                if v.pointers.len() >= 2 {
                    let (a, b) = (v.pointers[0], v.pointers[1]);
                    let d = ((a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt();
                    let (mx, my) = v.local((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
                    if let Some(prev) = v.pinch
                        && prev > 0.0 {
                            v.zoom_at(d / prev, mx, my);
                        }
                    v.pinch = Some(d);
                    return;
                }
                if let Some(p) = &mut v.press {
                    if !p.moved && ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt() > 4.0 {
                        p.moved = true;
                        let _ = v.svg.class_list().add_1("reef-panning");
                    }
                    if p.moved {
                        v.tx += x - px;
                        v.ty += y - py;
                        v.apply();
                    }
                }
            });
        });
        for kind in ["pointerup", "pointercancel"] {
            let vc = v.clone();
            let cancel = kind == "pointercancel";
            w.listen(&svg, kind, false, true, move |e: PointerEvent| {
                with(&vc, |v| {
                    v.pointers.retain(|p| p.0 != e.pointer_id());
                    if v.pointers.len() < 2 {
                        v.pinch = None;
                    }
                    let _ = v.svg.class_list().remove_1("reef-panning");
                    let Some(press) = v.press.take() else { return };
                    if press.moved || cancel {
                        return;
                    }
                    let t = press.target;
                    if let Some(item) = closest_item(&t, "[data-toggle]", "data-toggle") {
                        v.toggle(item);
                    } else if let Some(item) = closest_item(&t, ".reef-node", "data-item").or_else(|| closest_item(&t, ".reef-group", "data-item")) {
                        let node = v.vis.items[item].node;
                        v.select(Some(node));
                    } else {
                        v.select(None);
                    }
                });
            });
        }
        let vc = v.clone();
        w.listen(&svg, "pointerleave", false, true, move |_: PointerEvent| with(&vc, |v| v.light(None)));
        let vc = v.clone();
        w.listen(&svg, "dblclick", false, true, move |e: Event| {
            let t: Option<Element> = e.target().and_then(|t| t.dyn_into().ok());
            if closest_item(&t, "[data-toggle]", "data-toggle").is_some() {
                return;
            }
            if let Some(item) = closest_item(&t, ".reef-node.collapsed", "data-item") {
                with(&vc, |v| v.toggle(item));
            }
        });
        let vc = v.clone();
        w.listen(&svg, "wheel", false, false, move |e: WheelEvent| {
            with(&vc, |v| {
                if !(e.ctrl_key() || e.meta_key() || v.page_mode || v.is_fullscreen()) {
                    v.flash_hint();
                    return;
                }
                e.prevent_default();
                let unit = match e.delta_mode() {
                    1 => 16.0,
                    2 => 400.0,
                    _ => 1.0,
                };
                let dy = (e.delta_y() * unit).clamp(-120.0, 120.0);
                let (sx, sy) = v.local(e.client_x() as f64, e.client_y() as f64);
                v.zoom_at((-dy * 0.004).exp(), sx, sy);
            });
        });

        // Keys, when the diagram has focus.
        let vc = v.clone();
        w.listen(&el, "keydown", false, false, move |e: KeyboardEvent| {
            if e.ctrl_key() || e.meta_key() || e.alt_key() {
                return;
            }
            let handled = match e.key().as_str() {
                "+" | "=" => { with(&vc, |v| v.zoom_center(1.25)); true }
                "-" | "_" => { with(&vc, |v| v.zoom_center(0.8)); true }
                "0" => { with(&vc, |v| v.fit()); true }
                "f" => { with(&vc, |v| v.fullscreen()); true }
                "/" => { with(&vc, |v| { let _ = v.search.focus(); }); true }
                "Escape" => { with(&vc, |v| v.select(None)); true }
                _ => false,
            };
            if handled {
                e.prevent_default();
                e.stop_propagation();
            }
        });

        // The sidebar's edge: drag to resize, double-click to reset, arrows when focused.
        let grip: web_sys::EventTarget = v.borrow().grip.clone().into();
        let vc = v.clone();
        w.listen(&grip, "pointerdown", false, false, move |e: PointerEvent| {
            if e.button() != 0 {
                return;
            }
            e.prevent_default();
            with(&vc, |v| {
                let _ = v.grip.set_pointer_capture(e.pointer_id());
                let _ = v.grip.focus();
                v.side_drag = Some((e.pointer_id(), false));
                let _ = v.el.class_list().add_1("reef-resizing");
            });
        });
        let vc = v.clone();
        w.listen(&grip, "pointermove", false, true, move |e: PointerEvent| {
            with(&vc, |v| {
                if let Some((id, _)) = v.side_drag
                    && id == e.pointer_id()
                {
                    v.side_drag = Some((id, true));
                    let right = v.stage.get_bounding_client_rect().right();
                    v.set_side_w(Some(right - e.client_x() as f64));
                }
            });
        });
        for kind in ["pointerup", "pointercancel"] {
            let vc = v.clone();
            w.listen(&grip, kind, false, true, move |e: PointerEvent| {
                with(&vc, |v| {
                    if let Some((id, moved)) = v.side_drag
                        && id == e.pointer_id()
                    {
                        v.side_drag = None;
                        let _ = v.grip.release_pointer_capture(id);
                        let _ = v.el.class_list().remove_1("reef-resizing");
                        if moved {
                            v.save_side_w();
                        }
                    }
                });
            });
        }
        let vc = v.clone();
        w.listen(&grip, "dblclick", false, true, move |_: Event| {
            with(&vc, |v| {
                v.set_side_w(None);
                if let Some(s) = storage() {
                    let _ = s.remove_item(v.side_w_key());
                }
            });
        });
        let vc = v.clone();
        w.listen(&grip, "keydown", false, false, move |e: KeyboardEvent| {
            let step = match e.key().as_str() {
                "ArrowLeft" => 32.0,
                "ArrowRight" => -32.0,
                _ => return,
            };
            e.prevent_default();
            e.stop_propagation();
            with(&vc, |v| {
                v.set_side_w(Some(v.side.get_bounding_client_rect().width() + step));
                v.save_side_w();
            });
        });

        // The Performance section's charts, kept to tell them of a swapped plan.
        {
            let mut b = v.borrow_mut();
            let charts = crate::chartui::attach(&b.el, &b.side, b.plan.clone(), &mut w);
            b.charts = Some(charts);
        }

        // A doc's "Read here", or a Performance section's doc, opening. `toggle` does not bubble, so listen as it goes down.
        {
            let side: web_sys::EventTarget = v.borrow().side.clone().into();
            w.listen(&side, "toggle", true, false, move |e: Event| {
                if let Some(d) = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlDetailsElement>().ok())
                    && (d.class_list().contains("reef-doc-read") || d.class_list().contains("reef-perf-doc"))
                {
                    crate::docs::open(d);
                }
            });
        }

        // Size changes, including the book's sidebar sliding in or out, which is no window resize.
        let vc = v.clone();
        let observer = Closure::<dyn FnMut()>::new(move || with(&vc, |v| v.resized()));
        if let Ok(ro) = web_sys::ResizeObserver::new(observer.as_ref().unchecked_ref()) {
            ro.observe(&v.borrow().el);
            w.observe(Observer::Resize(ro), observer);
        }

        let doc: web_sys::EventTarget = window().document().expect("a document").into();
        // Another view changed the acceptance filter.
        let vc = v.clone();
        w.listen(&doc, ACC_EVENT, false, true, move |_: Event| with(&vc, |v| v.set_filter(load_filter())));

        let vc = v.clone();
        w.listen(&doc, "fullscreenchange", false, true, move |_: Event| {
            let vc = vc.clone();
            // Let the new size settle first.
            let cb = Closure::once_into_js(move || with(&vc, |v| v.fit()));
            let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 60);
        });
        w
    }
}

fn status_dot(s: reef_model::Status) -> &'static str {
    match s {
        reef_model::Status::Todo => "s-todo",
        reef_model::Status::InProgress => "s-progress",
        reef_model::Status::Done => "s-done",
        reef_model::Status::Blocked => "s-blocked",
    }
}

fn storage() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

/// A canvas context set to the font the diagram's labels are drawn in.
fn measure_ctx(el: &Element) -> Option<CanvasRenderingContext2d> {
    let doc = window().document()?;
    let canvas: web_sys::HtmlCanvasElement = doc.create_element("canvas").ok()?.dyn_into().ok()?;
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let family = window()
        .get_computed_style(el)
        .ok()
        .flatten()
        .and_then(|s| s.get_property_value("font-family").ok())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "sans-serif".into());
    ctx.set_font(&format!("{FONT_PX}px {family}"));
    Some(ctx)
}
