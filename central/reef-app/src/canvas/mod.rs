//! The plan's canvas: its toolbar, its diagram and its sidebar, drawn by Dioxus
//!
//! What it draws comes from reef-draw, as reef-view's does: the scene (each box's size and the
//! layout), the camera (pan and zoom), the search and the charts. Its state is Dioxus's, held by
//! id: the open groups, the selection and the acceptance filter. A newer revision of the plan
//! is drawn where the old one was, keeping all of it, and another plan remounts the canvas,
//! which [`PlanPage`](crate::app::PlanPage) keys by the plan's name.

mod diagram;
pub mod dom;
mod menu;
mod toolbar;

pub use diagram::{Diagram, Drawing, Hit, Hits, Shown, use_hits};
pub use menu::Menu;
pub use toolbar::Chip;

use crate::side::{DiffMode, Sidebar, Tab};
use dioxus::prelude::*;
use dom::{El, Measure};
use menu::NodeMenu;
use reef_draw::{Camera, Filter, Scene};
use reef_layout::Dir;
use reef_model::{Direction, Graph, Kind, Layout, Payload, ViewSpec};
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use toolbar::{Legend, Toolbar};

/// A plan's payload, with the name of the plan it is of, as `project/plan`
pub type Keyed = (String, Payload);

/// The narrowest the sidebar can be dragged
const SIDE_MIN_W: f64 = 240.0;
/// How far the arrow keys move the sidebar's edge
const SIDE_STEP: f64 = 32.0;

/// A plan's graph, shared, and compared by identity, so a memo over it sees each new one
#[derive(Clone)]
pub struct Plan(pub Rc<Graph>);

impl PartialEq for Plan {
    /// The same graph, not an equal one
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// A tooltip to show: its markup, and the pointer it follows, in client pixels
#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    /// The tooltip's markup
    pub html: String,
    /// The pointer's x
    pub x: f64,
    /// The pointer's y
    pub y: f64,
}

/// A pointer pressed on the diagram: where, whether it has moved, and what it was pressed on
#[derive(Debug, Clone, PartialEq)]
struct Press {
    /// Where it went down, in client pixels
    x: f64,
    /// Where it went down, in client pixels
    y: f64,
    /// Whether it has moved far enough to be a pan
    moved: bool,
    /// The box, group or toggle it went down on, if any
    hit: Option<Hit>,
}

/// A view's state, shared with the toolbar and the sidebar through context
#[derive(Clone, Copy, PartialEq)]
pub struct View {
    /// The plan's name, `project/plan`, which its open groups are kept under
    name: Signal<String>,
    /// The plan drawn, once its payload has come
    pub plan: Signal<Option<Plan>>,
    /// What the view shows of it: its root or focus, its direction, and its layout
    spec: Signal<ViewSpec>,
    /// The open groups, by id
    pub expanded: Signal<HashSet<String>>,
    /// Which kinds of acceptance criteria are shown
    pub filter: Signal<Filter>,
    /// The selected node, by id
    pub selected: Signal<Option<String>>,
    /// The pan and zoom
    pub camera: Signal<Camera>,
    /// The box the pointer is over, whose lineage lights up, with the layout it is an item of:
    /// a new layout lets it go, as reef-view's does, until the pointer moves again
    pub hover: Signal<Option<(usize, u64)>>,
    /// Measures labels in the view's font, once its element is mounted
    measure: Signal<Option<Rc<Measure>>>,
    /// What is drawn now, laid out
    pub shown: Memo<Option<Shown>>,
    /// The view's element
    pub root: Signal<Option<El>>,
    /// The diagram's SVG element
    svg: Signal<Option<El>>,
    /// The stage, which holds the diagram and the sidebar
    stage: Signal<Option<El>>,
    /// The search box
    pub search_box: Signal<Option<El>>,
    /// The sidebar's width as dragged, if it was
    side_w: Signal<Option<f64>>,
    /// The pointer dragging the sidebar's edge, and whether it has moved
    side_drag: Signal<Option<(i32, bool)>>,
    /// The stage's size last laid out for, to keep the view centred when it changes
    seen: Signal<(f64, f64)>,
    /// Whether the view has been fitted for the first time
    fitted: Signal<bool>,
    /// Whether the diagram is flashing to show where a node is
    flash: Signal<bool>,
    /// The timer that hides the scroll hint, while it shows
    hint: Signal<Option<i32>>,
    /// What is typed in the search box
    pub search: Signal<String>,
    /// Whether the search results are open
    pub results: Signal<bool>,
    /// The chart tooltip shown, if any
    pub tip: Signal<Option<Tip>>,
    /// The sidebar's tab
    pub tab: Signal<Tab>,
    /// How the Changes tab draws a revision's files, kept by the browser across nodes and pages
    pub diff_mode: Signal<DiffMode>,
    /// The node menu shown, if a right-click opened one
    pub menu: Signal<Option<Menu>>,
}

impl View {
    /// Whether the canvas is still mounted, for a timer that may outlive it
    fn alive(self) -> bool {
        self.name.try_peek().is_ok()
    }

    /// The plan drawn now, if one is
    pub fn graph(self) -> Option<Rc<Graph>> {
        self.plan.read().as_ref().map(|p| p.0.clone())
    }

    /// What is drawn now, laid out afresh if anything it is drawn from changed
    pub fn drawn(self) -> Option<Rc<Drawing>> {
        self.shown.read().as_ref().map(|s| s.0.clone())
    }

    /// Whether the view fills the page, where a plain wheel zooms
    fn page_mode(self) -> bool {
        self.spec.peek().layout == Layout::Page
    }

    /// The diagram's size on the page
    fn stage_size(self) -> (f64, f64) {
        let b = self
            .svg
            .peek()
            .as_ref()
            .map(dom::bounds)
            .unwrap_or_default();
        (b.width.max(1.0), b.height.max(1.0))
    }

    /// How wide the sidebar is drawn at a stage width, as reef.css sizes it
    ///
    /// # Arguments
    ///
    /// * `w` - The stage's width
    fn side_width(self, w: f64) -> f64 {
        match *self.side_w.peek() {
            Some(px) => px.max(SIDE_MIN_W).min(w - 120.0),
            // a view filling a page has a wider sidebar, unless the window is narrow
            None if self.page_mode() && dom::viewport_width() > 900.0 => 560f64.min(w * 0.45),
            None => 400f64.min(w * 0.88),
        }
    }

    /// The part of the stage the sidebar leaves free
    fn free_width(self) -> f64 {
        let (w, _) = self.stage_size();
        // the sidebar shows while a node is selected
        if self.selected.peek().is_none() {
            w
        } else {
            (w - self.side_width(w)).max(w * 0.3)
        }
    }

    /// The sidebar's width for the stage's style, if it was dragged
    fn side_style(self) -> Option<String> {
        let px = *self.side_w.read();
        px.map(|px| {
            format!(
                "--reef-side-w: min({:.0}px, calc(100% - 120px))",
                px.max(SIDE_MIN_W)
            )
        })
    }

    /// Fits the whole plan in the free part of the stage
    pub fn fit(mut self) {
        let Some(d) = self.drawn() else { return };
        let (_, h) = self.stage_size();
        let w = self.free_width();
        self.camera.write().fit(&d.scene.out, w, h);
    }

    /// Fits the plan, or starts it at a readable size if fitting would make it too small
    fn initial_view(mut self) {
        let Some(d) = self.drawn() else { return };
        let (_, h) = self.stage_size();
        let w = self.free_width();
        self.camera.write().initial(&d.scene.out, w, h);
    }

    /// Zooms about the middle of the free part of the stage
    ///
    /// # Arguments
    ///
    /// * `factor` - How much to scale by
    pub fn zoom_center(mut self, factor: f64) {
        let (_, h) = self.stage_size();
        let w = self.free_width();
        self.camera.write().zoom_at(factor, w / 2.0, h / 2.0);
    }

    /// The stage changed size: keeps what was in the middle there, or fits for the first time
    fn resized(mut self) {
        let (w, h) = self.stage_size();
        let (pw, ph) = *self.seen.peek();
        if (w - pw).abs() < 1.0 && (h - ph).abs() < 1.0 {
            return;
        }
        // a stage with no size before is fitted as a new view is, once there is a plan to fit
        if !*self.fitted.peek() {
            if self.drawn().is_some() && w > 1.0 && h > 1.0 {
                self.seen.set((w, h));
                self.initial_view();
                self.fitted.set(true);
            }
            return;
        }
        self.seen.set((w, h));
        self.camera.write().recentre((pw, ph), (w, h));
    }

    /// Keeps the open groups, under the plan's name
    fn save(self) {
        let ids: Vec<String> = self.expanded.peek().iter().cloned().collect();
        dom::store(
            &reef_draw::plan_key(&self.name.peek()),
            &reef_draw::groups_to_stored(ids.iter().map(String::as_str)),
        );
    }

    /// Opens or closes the group shown as an item, keeping it where it is on screen
    ///
    /// # Arguments
    ///
    /// * `item` - The group's item
    pub fn toggle(mut self, item: usize) {
        let Some(d) = self.drawn() else { return };
        let Some(it) = d.scene.vis.items.get(item) else {
            return;
        };
        if !d.graph.is_group(it.node) {
            return;
        }
        // where it is now, then flip it
        let before = d.scene.out.rects[item];
        let id = d.graph.node(it.node).id.clone();
        {
            let mut open = self.expanded.write();
            if !open.remove(&id) {
                open.insert(id);
            }
        }
        self.save();
        // laid out again, the group stays where it was
        let Some(now) = self.drawn() else { return };
        if let Some(after) = now.scene.item_of(it.node).map(|i| now.scene.out.rects[i]) {
            self.camera.write().keep(before, after);
        }
    }

    /// Opens every group, or closes them all, then fits the plan
    ///
    /// # Arguments
    ///
    /// * `open` - Whether to open them
    pub fn set_all(mut self, open: bool) {
        let Some(g) = self.graph() else { return };
        let ids = if open {
            (0..g.len())
                .filter(|&i| g.is_group(i))
                .map(|i| g.node(i).id.clone())
                .collect()
        } else {
            HashSet::new()
        };
        self.expanded.set(ids);
        self.save();
        self.fit();
    }

    /// Closes the node menu, giving the keyboard back to the view
    pub fn close_menu(mut self) {
        if self.menu.peek().is_some() {
            self.menu.set(None);
        }
        if let Some(el) = self.root.peek().as_ref() {
            dom::focus(el);
        }
    }

    /// The id of the node an item of the drawing shows
    ///
    /// # Arguments
    ///
    /// * `item` - The item
    fn item_id(self, item: usize) -> Option<String> {
        self.drawn().and_then(|d| {
            d.scene
                .vis
                .items
                .get(item)
                .map(|it| d.graph.node(it.node).id.clone())
        })
    }

    /// Selects a node, or none
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn select(mut self, id: Option<String>) {
        if *self.selected.peek() != id {
            self.selected.set(id);
        }
    }

    /// Shows a node: opens the groups hiding it, selects it and pans to it
    ///
    /// # Arguments
    ///
    /// * `id` - The node's id
    pub fn goto(mut self, id: &str) {
        let (Some(g), Some(d)) = (self.graph(), self.drawn()) else {
            return;
        };
        let Some(node) = g.index_of(id) else { return };
        // a node hidden in a collapsed group opens it, up to the view's root
        if d.scene.own_item(node).is_none() {
            {
                let mut open = self.expanded.write();
                for a in g.ancestors(node) {
                    if Some(a) == d.root {
                        break;
                    }
                    open.insert(g.node(a).id.clone());
                }
            }
            self.save();
        }
        self.results.set(false);
        self.select(Some(id.to_string()));
        self.center_on(node);
    }

    /// Centres a node in the free part of the stage, and flashes the diagram
    ///
    /// # Arguments
    ///
    /// * `node` - The node's index
    fn center_on(mut self, node: usize) {
        let Some(d) = self.drawn() else { return };
        let Some(item) = d.scene.item_of(node) else {
            return;
        };
        let (_, h) = self.stage_size();
        let w = self.free_width();
        self.camera.write().center_on(d.scene.out.rects[item], w, h);
        // the flash fades once its animation is done
        self.flash.set(true);
        let mut flash = self.flash;
        dom::later(900, move || {
            let _ = flash.try_write().map(|mut f| *f = false);
        });
    }

    /// Opens or closes the selected group, opening the groups hiding it first
    pub fn toggle_selected(self) {
        let (Some(g), Some(d), Some(id)) =
            (self.graph(), self.drawn(), self.selected.peek().clone())
        else {
            return;
        };
        let Some(node) = g.index_of(&id) else { return };
        match d.scene.own_item(node) {
            Some(item) => self.toggle(item),
            None => {
                self.goto(&id);
                if let Some(item) = self.drawn().and_then(|d| d.scene.item_of(node)) {
                    self.toggle(item);
                }
            }
        }
    }

    /// Shows or hides a kind of acceptance criteria, in every view
    ///
    /// # Arguments
    ///
    /// * `k` - The kind
    pub fn toggle_kind(mut self, k: Kind) {
        let f = self.filter.peek().toggled(k);
        dom::store(Filter::STORAGE_KEY, &f.to_stored());
        self.filter.set(f);
    }

    /// Draws the Changes tab's files as the semantic view or the classic one, and keeps the pick
    ///
    /// # Arguments
    ///
    /// * `mode` - How to draw them
    pub fn set_diff_mode(mut self, mode: DiffMode) {
        dom::store(DiffMode::STORAGE_KEY, mode.key());
        self.diff_mode.set(mode);
    }

    /// Shows or leaves full screen
    pub fn fullscreen(self) {
        if let Some(el) = self.root.peek().as_ref() {
            dom::toggle_fullscreen(el);
        }
    }

    /// Shows the scroll hint for a moment
    fn flash_hint(mut self) {
        if let Some(t) = self.hint.take() {
            dom::cancel(t);
        }
        // shown until the timer hides it
        let mut hint = self.hint;
        let timer = dom::later(1400, move || {
            let _ = hint.try_write().map(|mut h| *h = None);
        });
        self.hint.set(Some(timer.unwrap_or(0)));
    }

    /// Where a client point is on the stage
    ///
    /// # Arguments
    ///
    /// * `x` - The point's client x
    /// * `y` - The point's client y
    fn local(self, x: f64, y: f64) -> (f64, f64) {
        let b = self
            .svg
            .peek()
            .as_ref()
            .map(dom::bounds)
            .unwrap_or_default();
        (x - b.left, y - b.top)
    }

    /// Sets the sidebar's width from where its edge was dragged to, as reef.css bounds it
    ///
    /// # Arguments
    ///
    /// * `px` - The width, or `None` for the stylesheet's
    fn set_side_w(mut self, px: Option<f64>) {
        self.side_w.set(px.map(|px| px.max(SIDE_MIN_W)));
    }

    /// Keeps the sidebar's width as drawn now, for the next view
    fn save_side_w(self) {
        let (w, _) = self.stage_size();
        let px = self.side_width(w);
        dom::store(reef_draw::side_w_key(self.page_mode()), &format!("{px:.0}"));
    }
}

/// What the app shows of every plan: all of it, filling the stage
fn whole_plan() -> ViewSpec {
    ViewSpec {
        layout: Layout::Page,
        ..ViewSpec::default()
    }
}

/// The plan's canvas: its toolbar, diagram, legend and sidebar, and a node's menu
///
/// Keyed by the plan's name, so another plan starts it afresh, while each newer revision of the
/// same plan is drawn in place. It shows the whole plan filling the stage, or what a
/// ```` ```reef ```` fence asks for, inline in a chapter's text at the fence's height.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `payload` - The plan's payload, as it is read and read again, named for its plan
/// * `changes` - How many changes to the plan have come, which the editor reads the node again on
/// * `spec` - What a fence asks the view to show, or `None` for the whole plan filling the stage
/// * `select` - A node to show once the plan is first drawn, as a link to it asks
#[component]
pub fn Canvas(
    project: String,
    plan: String,
    payload: Resource<Result<Keyed, ServerFnError>>,
    changes: ReadSignal<u64>,
    #[props(default)] spec: Option<ViewSpec>,
    #[props(default)] select: Option<String>,
) -> Element {
    let name = format!("{project}/{plan}");
    let view = use_view(name.clone(), spec);
    // the plan's comment threads, which the toolbar counts and the sidebar shows
    let (threads_project, threads_plan) = (project.clone(), plan.clone());
    crate::side::threads::use_threads(
        use_signal(move || threads_project).into(),
        use_signal(move || threads_plan).into(),
    );
    // take each payload of this plan as it comes, built into its graph
    use_effect(move || take(view, &payload.read()));
    // fit the view once there is a plan and a stage to fit it in
    use_effect(move || {
        let _ = view.shown.read();
        if !*view.fitted.peek() {
            view.resized();
        }
    });
    // and once it is fitted, show the node a link asked for, if the plan has it
    let mut shown_asked = use_signal(|| false);
    use_effect(move || {
        let fitted = *view.fitted.read();
        let Some(id) = select.as_deref() else { return };
        if !fitted || *shown_asked.peek() {
            return;
        }
        if view.graph().is_some_and(|g| g.index_of(id).is_some()) {
            shown_asked.set(true);
            view.goto(id);
        }
    });
    // leaving full screen, or entering it, fits the view once the new size settles
    use_hook(move || {
        Rc::new(dom::on_document("fullscreenchange", move || {
            dom::later(60, move || {
                if view.alive() {
                    view.fit();
                }
            });
        }))
    });
    // the drawing and the toolbar's state, a fence's view drawn at its height in the text
    let drawn = view.drawn();
    let page = view.spec.read().layout == Layout::Page;
    let height = match page {
        true => None,
        false => Some(format!(
            "height: {}",
            view.spec.read().height.as_deref().unwrap_or("70vh")
        )),
    };
    let classes = format!(
        "reef{} reef-mounted reef-app-graph{}{}",
        if page { " reef-page" } else { "" },
        if view.selected.read().is_some() {
            " reef-has-side"
        } else {
            ""
        },
        if view.side_drag.read().is_some() {
            " reef-resizing"
        } else {
            ""
        },
    );
    let boxes = drawn.as_ref().map(|d| d.scene.vis.items.len().to_string());
    let layout_ms = drawn.as_ref().map(|d| format!("{:.1}", d.ms));
    let draws = drawn.as_ref().map(|d| d.draws.to_string());
    let has_acc = view.graph().is_some_and(|g| g.has_acceptance());
    // what is said over the view until its plan is drawn: over the stage for the whole plan,
    // and inside the view for a fence's, which sits in a chapter's text
    let note = match &*payload.read() {
        Some(Err(error)) => rsx! {
            p { class: "reef-app-error reef-app-over", "Reading the plan failed: {error}" }
        },
        None => rsx! {
            p { class: "reef-app-muted reef-app-over", "Loading the plan…" }
        },
        Some(Ok(_)) => rsx! {},
    };
    rsx! {
        div {
            class: "{classes}",
            style: height,
            tabindex: "0",
            "data-boxes": boxes,
            "data-layout-ms": layout_ms,
            "data-draws": draws,
            onmounted: move |event| mounted(view, &event.data()),
            onkeydown: move |event| keyed(view, event),
            Toolbar { has_acc }
            Stage { changes, project: project.clone(), plan: plan.clone() }
            ChartTip {}
            NodeMenu { project: project.clone(), plan: plan.clone() }
            if !page {
                {note.clone()}
            }
        }
        if page {
            {note}
        }
    }
}

/// Makes a view's state, provided to everything the canvas holds
///
/// # Arguments
///
/// * `name` - The plan's name, `project/plan`
/// * `spec` - What a fence asks the view to show, or `None` for the whole plan
fn use_view(name: String, spec: Option<ViewSpec>) -> View {
    // the signals, each starting as a newly mounted view's does
    let name = use_signal(|| name);
    let plan = use_signal(|| None::<Plan>);
    let spec = use_signal(move || spec.unwrap_or_else(whole_plan));
    let expanded = use_signal(HashSet::<String>::new);
    let mut filter = use_signal(Filter::default);
    let measure = use_signal(|| None::<Rc<Measure>>);
    let mut side_w = use_signal(|| None);
    let mut diff_mode = use_signal(|| DiffMode::Semantic);
    // what the browser keeps is read once the page is live, since the server's HTML had none of it
    use_effect(move || {
        if let Some(stored) = dom::load(Filter::STORAGE_KEY) {
            filter.set(Filter::from_stored(&stored));
        }
        if let Some(px) = dom::load(reef_draw::side_w_key(true)).and_then(|w| w.parse().ok()) {
            side_w.set(Some(px));
        }
        if let Some(stored) = dom::load(DiffMode::STORAGE_KEY) {
            diff_mode.set(DiffMode::from_stored(&stored));
        }
    });
    // how many layouts the view has done, which the page shows as `data-draws`
    let draws = use_hook(|| Rc::new(Cell::new(0u64)));
    // what is drawn: laid out again when the plan, the open groups, the filter or the font change
    let shown = use_memo(move || {
        let plan = plan.read().clone()?;
        let measure = measure.read().clone()?;
        let spec = spec.read();
        let g = &plan.0;
        let root = spec.root_id().and_then(|id| g.index_of(id));
        let dir = if spec.direction == Direction::LR {
            Dir::LR
        } else {
            Dir::TB
        };
        let open: HashSet<usize> = expanded
            .read()
            .iter()
            .filter_map(|id| g.index_of(id))
            .collect();
        // time the layout, as reef-view does, and count it
        let started = dom::now();
        let scene = Scene::build(
            g,
            root,
            spec.focus.is_some(),
            &open,
            &filter.read(),
            dir,
            &|s| measure.width(s),
        );
        draws.set(draws.get() + 1);
        Some(Shown(Rc::new(Drawing {
            graph: g.clone(),
            scene,
            dir,
            root,
            ms: dom::now() - started,
            draws: draws.get(),
        })))
    });
    let view = View {
        name,
        plan,
        spec,
        expanded,
        filter,
        selected: use_signal(|| None),
        camera: use_signal(Camera::default),
        hover: use_signal(|| None),
        measure,
        shown,
        root: use_signal(|| None),
        svg: use_signal(|| None),
        stage: use_signal(|| None),
        search_box: use_signal(|| None),
        side_w,
        side_drag: use_signal(|| None),
        seen: use_signal(|| (0.0, 0.0)),
        fitted: use_signal(|| false),
        flash: use_signal(|| false),
        hint: use_signal(|| None),
        search: use_signal(String::new),
        results: use_signal(|| false),
        tip: use_signal(|| None),
        tab: use_signal(|| Tab::Details),
        menu: use_signal(|| None),
        diff_mode,
    };
    use_context_provider(|| view);
    view
}

/// Takes a payload of the view's plan into it, built into its graph
///
/// The first one opens the groups saved for the plan, or the view's own. A selected node a
/// newer revision lacks is let go.
///
/// # Arguments
///
/// * `view` - The view
/// * `payload` - The payload read last, if one has come
fn take(mut view: View, payload: &Option<Result<Keyed, ServerFnError>>) {
    // a payload that failed or hasn't come yet, or is still another plan's, leaves the view be
    let Some(Ok((name, payload))) = payload else {
        return;
    };
    if *name != *view.name.peek() {
        return;
    }
    // build it, warning of anything wrong, which doesn't stop it drawing
    let (g, problems) = Graph::build(payload.nodes.clone());
    for problem in problems {
        dom::warn(&format!("reef: {problem}"));
    }
    let g = Rc::new(g);
    // the first revision opens the groups saved for the plan, else its view's
    if view.plan.peek().is_none() {
        let saved = dom::load(&reef_draw::plan_key(&view.name.peek()))
            .and_then(|s| reef_draw::groups_from_stored(&s));
        let ids: HashSet<String> = match saved {
            Some(ids) => ids.into_iter().collect(),
            None => view
                .spec
                .peek()
                .initial_expanded(&g)
                .into_iter()
                .map(|i| g.node(i).id.clone())
                .collect(),
        };
        view.expanded.set(ids);
    }
    // a node the revision lacks can't stay selected
    let gone = view
        .selected
        .peek()
        .as_ref()
        .is_some_and(|id| g.index_of(id).is_none());
    if gone {
        view.selected.set(None);
    }
    view.plan.set(Some(Plan(g)));
}

/// The view's element is in the page: measure its font, from now on
///
/// # Arguments
///
/// * `view` - The view
/// * `data` - The element as Dioxus mounted it
fn mounted(mut view: View, data: &MountedData) {
    let Some(el) = dom::mounted(data) else { return };
    view.measure.set(Some(Rc::new(Measure::new(&el))));
    view.root.set(Some(el));
}

/// What keeps the keys typed in it: a text box, a text area, a select or an editable element
const FIELDS: &str = "input, textarea, select, [contenteditable]:not([contenteditable='false'])";

/// The view's keys, while it has the focus: zoom, fit, full screen, search, and letting go
///
/// A key typed in a field inside the view, such as the sidebar's comment box, is left to it.
///
/// # Arguments
///
/// * `view` - The view
/// * `event` - The key pressed
fn keyed(view: View, event: KeyboardEvent) {
    // a key typed in a field is the field's, so the view's keys answer only outside one
    if dom::target(&*event.data()).is_some_and(|el| dom::closest(&el, FIELDS).is_some()) {
        return;
    }
    // a key held with Ctrl, Cmd or Alt is the browser's
    let mods = event.modifiers();
    if mods.ctrl() || mods.meta() || mods.alt() {
        return;
    }
    let handled = match event.key() {
        Key::Character(c) => match c.as_str() {
            "+" | "=" => {
                view.zoom_center(1.25);
                true
            }
            "-" | "_" => {
                view.zoom_center(0.8);
                true
            }
            "0" => {
                view.fit();
                true
            }
            "f" => {
                view.fullscreen();
                true
            }
            "/" => {
                if let Some(el) = view.search_box.peek().as_ref() {
                    dom::focus(el);
                }
                true
            }
            _ => false,
        },
        Key::Escape => {
            // an open menu closes first, and the selection goes after
            if view.menu.peek().is_some() {
                view.close_menu();
            } else {
                view.select(None);
            }
            true
        }
        _ => false,
    };
    if handled {
        event.prevent_default();
        event.stop_propagation();
    }
}

/// The stage: the diagram, the sidebar and its edge, the legend and the scroll hint
///
/// # Arguments
///
/// * `changes` - How many changes to the plan have come
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
fn Stage(changes: ReadSignal<u64>, project: String, plan: String) -> Element {
    let view = use_context::<View>();
    let style = view.side_style();
    rsx! {
        div {
            class: "reef-stage",
            style,
            onmounted: move |event| {
                let mut stage = view.stage;
                stage.set(dom::mounted(&event.data()));
            },
            Board {}
            if let Some(id) = view.selected.read().clone() {
                Sidebar { id, project: project.clone(), plan: plan.clone(), changes }
            }
            Grip {}
            Legend {}
            div { class: if view.hint.read().is_some() { "reef-hint on" } else { "reef-hint" },
                if view.page_mode() {
                    "Scroll to zoom · drag to pan"
                } else {
                    "Ctrl + scroll to zoom · drag to pan"
                }
            }
        }
    }
}

/// The diagram's SVG: pan, pinch, wheel, clicks, right-clicks and hover, over the diagram itself
#[component]
fn Board() -> Element {
    let mut view = use_context::<View>();
    // what each box's own handlers say the pointer is on, and the pointers down
    let hits = use_hits();
    let mut press = use_signal(|| None::<Press>);
    // what the last press landed on: the SVG holds the pointer from the press, so the browser
    // sends the click and double-click that follow it to the SVG, not to the box
    let mut last_hit = use_signal(|| None::<Hit>);
    let mut pointers = use_signal(Vec::<(i32, f64, f64)>::new);
    let mut pinch = use_signal(|| None::<f64>);
    let mut panning = use_signal(|| false);
    // the arrows' marker, named as reef-view names a view's; the page holds one canvas, and the
    // name must be the server's too, or the arrows would point at a marker hydration never made
    let uid = "app".to_string();
    let sel = use_memo(move || {
        let id = view.selected.read().clone()?;
        let d = view.shown.read().clone()?;
        d.0.graph.index_of(&id).and_then(|n| d.0.scene.item_of(n))
    });
    let lit = use_memo(move || {
        let (item, at) = (*view.hover.read())?;
        let d = view.shown.read().clone()?;
        (d.0.draws == at && item < d.0.scene.vis.items.len()).then(|| d.0.scene.lineage(item))
    });
    let class = format!(
        "reef-svg{}{}{}",
        if lit.read().is_some() {
            " reef-dim"
        } else {
            ""
        },
        if panning() { " reef-panning" } else { "" },
        if (view.flash)() { " reef-flash" } else { "" },
    );
    let transform = view.camera.read().transform();
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            role: "img",
            "aria-label": "Plan diagram",
            onmounted: move |event| {
                let mut svg = view.svg;
                svg.set(dom::mounted(&event.data()));
            },
            onresize: move |_| view.resized(),
            onpointerdown: move |event| {
                let (x, y) = point(&event);
                let id = event.pointer_id();
                // a mouse presses with its main button alone, though what another button
                // landed on is kept for the node menu a right-click opens
                if event.pointer_type() == "mouse" && event.trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                    last_hit.set(hits.take_press());
                    return;
                }
                if let Some(svg) = view.svg.peek().as_ref() {
                    dom::capture(svg, id);
                }
                pointers.with_mut(|p| {
                    p.retain(|q| q.0 != id);
                    p.push((id, x, y));
                });
                let hit = hits.take_press();
                last_hit.set(hit);
                if pointers.peek().len() == 1 {
                    press.set(Some(Press { x, y, moved: false, hit }));
                } else {
                    press.set(None);
                    pinch.set(None);
                }
            },
            onpointermove: move |event| {
                let (x, y) = point(&event);
                let id = event.pointer_id();
                // the box the pointer moved over, if it said so
                let over = hits.take_over();
                let Some(at) = pointers.peek().iter().position(|p| p.0 == id) else {
                    // with nothing pressed, the box under the pointer lights its lineage
                    let now = over.zip(view.drawn().map(|d| d.draws));
                    if *view.hover.peek() != now {
                        view.hover.set(now);
                    }
                    return;
                };
                let (_, px, py) = pointers.peek()[at];
                pointers.write()[at] = (id, x, y);
                // two pointers pinch to zoom about their middle
                if pointers.peek().len() >= 2 {
                    let (a, b) = (pointers.peek()[0], pointers.peek()[1]);
                    let d = ((a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt();
                    let (mx, my) = view.local((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
                    if let Some(prev) = *pinch.peek()
                        && prev > 0.0
                    {
                        view.camera.write().zoom_at(d / prev, mx, my);
                    }
                    pinch.set(Some(d));
                    return;
                }
                // one pointer pans, once it has moved past a click's wobble
                let Some(mut p) = press.peek().clone() else {
                    return;
                };
                if !p.moved && ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt() > 4.0 {
                    p.moved = true;
                    press.set(Some(p.clone()));
                    panning.set(true);
                }
                if p.moved {
                    let mut cam = view.camera.write();
                    cam.tx += x - px;
                    cam.ty += y - py;
                }
            },
            onpointerup: move |event| released(view, event.pointer_id(), false, press, pointers, pinch, panning),
            onpointercancel: move |event| released(view, event.pointer_id(), true, press, pointers, pinch, panning),
            onpointerleave: move |_| {
                if view.hover.peek().is_some() {
                    view.hover.set(None);
                }
            },
            oncontextmenu: move |event| {
                // a right-click on a box, a group or its head opens its node's menu in place
                // of the browser's, while one on nothing leaves the browser's alone
                let Some(Hit::Item(item) | Hit::Toggle(item)) = *last_hit.peek() else {
                    return;
                };
                let Some(id) = view.item_id(item) else {
                    return;
                };
                event.prevent_default();
                let c = event.client_coordinates();
                view.menu.set(Some(Menu { id, x: c.x, y: c.y }));
            },
            ondoubleclick: move |_| {
                // a collapsed box opens; its toggle already did on the press
                if let Some(Hit::Item(item)) = *last_hit.peek()
                    && view.drawn().is_some_and(|d| d.scene.vis.items.get(item).is_some_and(|it| it.collapsed))
                {
                    view.toggle(item);
                }
            },
            onwheel: move |event| {
                let mods = event.modifiers();
                // in a page of text, a plain wheel scrolls the page, and says how to zoom
                let fullscreen = view.root.peek().as_ref().is_some_and(dom::is_fullscreen);
                if !(mods.ctrl() || mods.meta() || view.page_mode() || fullscreen) {
                    view.flash_hint();
                    return;
                }
                event.prevent_default();
                let dy = match event.delta() {
                    dioxus::html::geometry::WheelDelta::Pixels(v) => v.y,
                    dioxus::html::geometry::WheelDelta::Lines(v) => v.y * 16.0,
                    dioxus::html::geometry::WheelDelta::Pages(v) => v.y * 400.0,
                }
                .clamp(-120.0, 120.0);
                let c = event.client_coordinates();
                let (sx, sy) = view.local(c.x, c.y);
                view.camera.write().zoom_at((-dy * 0.004).exp(), sx, sy);
            },
            defs {
                marker {
                    id: "reef-arrow-{uid}",
                    view_box: "0 0 10 10",
                    ref_x: "9",
                    ref_y: "5",
                    marker_units: "userSpaceOnUse",
                    marker_width: "9",
                    marker_height: "9",
                    orient: "auto-start-reverse",
                    path { class: "reef-arrowhead", d: "M0,0 L10,5 L0,10 z" }
                }
            }
            g { class: "reef-viewport", transform: "{transform}",
                Diagram { shown: view.shown, sel, lit, uid: uid.clone(), hits }
            }
        }
    }
}

/// Where a pointer event happened, in client pixels
///
/// # Arguments
///
/// * `event` - The event
fn point(event: &PointerEvent) -> (f64, f64) {
    let c = event.client_coordinates();
    (c.x, c.y)
}

/// A pointer let go of the diagram: a press that didn't move selects, toggles or lets go
///
/// # Arguments
///
/// * `view` - The view
/// * `id` - The pointer's id
/// * `cancel` - Whether the browser took the pointer away, which is never a click
/// * `press` - The press, if one pointer was down
/// * `pointers` - The pointers down
/// * `pinch` - The pinch's last span
/// * `panning` - Whether the diagram is being panned
fn released(
    view: View,
    id: i32,
    cancel: bool,
    mut press: Signal<Option<Press>>,
    mut pointers: Signal<Vec<(i32, f64, f64)>>,
    mut pinch: Signal<Option<f64>>,
    mut panning: Signal<bool>,
) {
    // the pointer is up, and a pinch with one finger left is over
    pointers.write().retain(|p| p.0 != id);
    if pointers.peek().len() < 2 {
        pinch.set(None);
    }
    if *panning.peek() {
        panning.set(false);
    }
    let Some(p) = press.take() else { return };
    if p.moved || cancel {
        return;
    }
    // a click: on a toggle, a box or a group, or on nothing
    match p.hit {
        Some(Hit::Toggle(item)) => view.toggle(item),
        Some(Hit::Item(item)) => view.select(view.item_id(item)),
        None => view.select(None),
    }
}

/// The sidebar's edge: dragged to resize, double-clicked to reset, moved by the arrow keys
#[component]
fn Grip() -> Element {
    let view = use_context::<View>();
    let mut grip = use_signal(|| None::<El>);
    let mut drag = view.side_drag;
    rsx! {
        div {
            class: "reef-side-grip",
            role: "separator",
            "aria-orientation": "vertical",
            "aria-label": "Resize sidebar",
            tabindex: "0",
            title: "Drag to resize · double-click to reset",
            onmounted: move |event| grip.set(dom::mounted(&event.data())),
            onpointerdown: move |event| {
                if event.trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                    return;
                }
                event.prevent_default();
                if let Some(el) = grip.peek().as_ref() {
                    dom::capture(el, event.pointer_id());
                    dom::focus(el);
                }
                drag.set(Some((event.pointer_id(), false)));
            },
            onpointermove: move |event| {
                let Some((id, _)) = *drag.peek() else { return };
                if id != event.pointer_id() {
                    return;
                }
                drag.set(Some((id, true)));
                // the sidebar reaches from the pointer to the stage's right edge
                let right = view.stage.peek().as_ref().map(dom::bounds).unwrap_or_default().right();
                view.set_side_w(Some(right - event.client_coordinates().x));
            },
            onpointerup: move |event| grip_released(view, grip, drag, event.pointer_id()),
            onpointercancel: move |event| grip_released(view, grip, drag, event.pointer_id()),
            ondoubleclick: move |_| {
                view.set_side_w(None);
                dom::forget(reef_draw::side_w_key(view.page_mode()));
            },
            onkeydown: move |event| {
                let step = match event.key() {
                    Key::ArrowLeft => SIDE_STEP,
                    Key::ArrowRight => -SIDE_STEP,
                    _ => return,
                };
                event.prevent_default();
                event.stop_propagation();
                let (w, _) = view.stage_size();
                view.set_side_w(Some(view.side_width(w) + step));
                view.save_side_w();
            },
        }
    }
}

/// The sidebar's edge let go: its width is kept if it moved
///
/// # Arguments
///
/// * `view` - The view
/// * `grip` - The edge's element
/// * `drag` - The drag
/// * `id` - The pointer let go
fn grip_released(
    view: View,
    grip: Signal<Option<El>>,
    mut drag: Signal<Option<(i32, bool)>>,
    id: i32,
) {
    let Some((held, moved)) = *drag.peek() else {
        return;
    };
    if held != id {
        return;
    }
    drag.set(None);
    if let Some(el) = grip.peek().as_ref() {
        dom::release(el, id);
    }
    if moved {
        view.save_side_w();
    }
}

/// The charts' tooltip, kept inside the view and clear of the pointer
#[component]
fn ChartTip() -> Element {
    let view = use_context::<View>();
    // the tooltip's own size, to keep it inside the view
    let mut size = use_signal(|| (0.0, 0.0));
    let Some(tip) = view.tip.read().clone() else {
        return rsx! {
            div { class: "reef-tip", role: "tooltip", hidden: true }
        };
    };
    // beside the pointer, or before it where the view ends
    let c = view
        .root
        .peek()
        .as_ref()
        .map(dom::bounds)
        .unwrap_or_default();
    let (w, h) = size();
    let mut x = tip.x - c.left + 14.0;
    let mut y = tip.y - c.top + 14.0;
    if x + w > c.width - 4.0 {
        x = (tip.x - c.left - w - 14.0).max(4.0);
    }
    if y + h > c.height - 4.0 {
        y = (tip.y - c.top - h - 14.0).max(4.0);
    }
    rsx! {
        div {
            class: "reef-tip",
            role: "tooltip",
            style: "left: {x:.0}px; top: {y:.0}px",
            onresize: move |event| {
                if let Ok(s) = event.data().get_border_box_size() {
                    size.set((s.width, s.height));
                }
            },
            dangerous_inner_html: "{tip.html}",
        }
    }
}
