//! The diagram: open groups' frames, then the arrows, then the boxes, as reef-draw's markup
//! draws them for reef-view
//!
//! Each box, group and toggle says it is under the pointer through [`Hits`], since its handlers
//! run before the SVG's own as the event bubbles. The SVG takes what they said, so it never
//! looks the target up in the DOM, and nothing stops the event on its way.

use dioxus::prelude::*;
use reef_draw::markup::{HEADER_H, LINE_H, PAD_X, PAD_Y, PILL_H, status_class};
use reef_draw::{Scene, accept};
use reef_layout::{Dir, svg_path};
use reef_model::{EdgeKind, Graph, Status};
use std::collections::HashSet;
use std::rc::Rc;

/// The gap between a box's pills
const PILL_GAP: f64 = 4.0;

/// The elements rsx can make, with an SVG `<title>`
///
/// dioxus-html's `title` is the document's, made in the HTML namespace, where a browser gives
/// it no tooltip. `svgtitle` is made as SVG's own.
#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
mod dioxus_elements {
    pub use dioxus::prelude::dioxus_elements::*;

    /// An SVG `<title>`: the tooltip of the element it is in
    pub mod svgtitle {
        /// Its tag
        pub const TAG_NAME: &str = "title";
        /// SVG's namespace
        pub const NAME_SPACE: Option<&str> = Some("http://www.w3.org/2000/svg");
    }

    /// Every element rsx can make, with the SVG title
    pub mod elements {
        pub use super::svgtitle;
        pub use dioxus::prelude::dioxus_elements::elements::*;
    }
}

/// A plan laid out, with what it was laid out from
#[derive(Debug)]
pub struct Drawing {
    /// The plan
    pub graph: Rc<Graph>,
    /// What is visible, each box's size, and where each goes
    pub scene: Scene,
    /// Which way the arrows run
    pub dir: Dir,
    /// The group the view is of, if any
    pub root: Option<usize>,
    /// How long the layout took, in milliseconds
    pub ms: f64,
    /// How many layouts the view has done, this one included
    pub draws: u64,
}

/// A drawing, shared, and compared by identity, so a memo over it sees each new layout
#[derive(Debug, Clone)]
pub struct Shown(pub Rc<Drawing>);

impl PartialEq for Shown {
    /// The same layout, not an equal one
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// What a press landed on
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// A group's header or a collapsed group's toggle, which opens or closes the group
    Toggle(usize),
    /// A box or a group's frame, which selects it
    Item(usize),
}

/// What the diagram's own handlers say is under the pointer, for the SVG's handlers to take
///
/// The innermost element says first, and the first to say wins, as `closest` would find it.
#[derive(Clone, Copy, PartialEq)]
pub struct Hits {
    /// What the last press landed on
    pub press: Signal<Option<Hit>>,
    /// The box the pointer last moved over
    pub over: Signal<Option<usize>>,
}

impl Hits {
    /// Takes what the last press landed on, leaving nothing for the next
    pub fn take_press(self) -> Option<Hit> {
        let mut press = self.press;
        press.take()
    }

    /// Takes the box the pointer last moved over, leaving nothing for the next
    pub fn take_over(self) -> Option<usize> {
        let mut over = self.over;
        over.take()
    }

    /// Says a press landed on something, unless something inside it said so first
    ///
    /// # Arguments
    ///
    /// * `hit` - What it landed on
    fn pressed(mut self, hit: Hit) {
        if self.press.peek().is_none() {
            self.press.set(Some(hit));
        }
    }

    /// Says the pointer moved over a box
    ///
    /// # Arguments
    ///
    /// * `item` - The box's item
    fn came_over(mut self, item: usize) {
        if self.over.peek().is_none() {
            self.over.set(Some(item));
        }
    }
}

/// Makes what the diagram's handlers say is under the pointer, as a component's hooks
pub fn use_hits() -> Hits {
    Hits {
        press: use_signal(|| None),
        over: use_signal(|| None),
    }
}

/// The diagram's contents: frames (outer first), arrows, then boxes
///
/// # Arguments
///
/// * `shown` - What is drawn, laid out
/// * `sel` - The selected node's item, if it is drawn
/// * `lit` - The items lit by the pointer's hover, if it is over a box
/// * `uid` - The view's id, which names its arrowhead marker
/// * `hits` - Where its handlers say what the pointer is on
#[component]
pub fn Diagram(
    shown: Memo<Option<Shown>>,
    sel: Memo<Option<usize>>,
    lit: Memo<Option<HashSet<usize>>>,
    uid: String,
    hits: Hits,
) -> Element {
    let Some(Shown(d)) = shown.read().clone() else {
        return rsx! {};
    };
    let sel = *sel.read();
    let lit = lit.read().clone().unwrap_or_default();
    let (vis, out) = (&d.scene.vis, &d.scene.out);
    // outer frames first, so inner ones draw over them
    let depth = |mut i: usize| {
        let mut n = 0;
        while let Some(p) = vis.items[i].parent {
            n += 1;
            i = p;
        }
        n
    };
    let mut frames: Vec<usize> = (0..vis.items.len())
        .filter(|&i| vis.items[i].open)
        .collect();
    frames.sort_by_key(|&i| depth(i));
    let boxes: Vec<usize> = (0..vis.items.len())
        .filter(|&i| !vis.items[i].open)
        .collect();
    // each arrow drawn, highlighted with the selection and lit with the hover
    let edges: Vec<(usize, bool, bool)> = (0..vis.edges.len())
        .filter(|&k| out.edges[k].len() >= 2)
        .map(|k| {
            let e = &vis.edges[k];
            let hl = sel.is_some_and(|s| e.from == s || e.to == s);
            (k, hl, lit.contains(&e.from) && lit.contains(&e.to))
        })
        .collect();
    rsx! {
        for i in frames {
            Frame { d: Shown(d.clone()), i, lit: lit.contains(&i), hits }
        }
        for (k, hl, lit) in edges {
            Arrow { d: Shown(d.clone()), k, hl, lit, uid: uid.clone() }
        }
        for i in boxes {
            NodeBox { d: Shown(d.clone()), i, sel: sel == Some(i), lit: lit.contains(&i), hits }
        }
    }
}

/// An open group's frame, header and count
///
/// # Arguments
///
/// * `d` - The drawing
/// * `i` - The group's item
/// * `lit` - Whether the pointer's hover lights it
/// * `hits` - Where it says it was pressed
#[component]
fn Frame(d: Shown, i: usize, lit: bool, hits: Hits) -> Element {
    let (g, it, r) = (&d.0.graph, &d.0.scene.vis.items[i], d.0.scene.out.rects[i]);
    let n = g.node(it.node);
    let (done, total) = g.progress(it.node);
    let class = format!(
        "reef-group {}{}{}",
        status_class(g, it.node),
        if n.optional { " optional" } else { "" },
        if lit { " lit" } else { "" }
    );
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let (tx, ty, cx) = (r.x + PAD_X, r.y + 20.0, r.right() - PAD_X);
    let title = format!(" {}", n.title);
    rsx! {
        g {
            class: "{class}",
            "data-item": "{i}",
            onpointerdown: move |_| hits.pressed(Hit::Item(i)),
            rect {
                class: "reef-frame",
                x: "{x:.1}",
                y: "{y:.1}",
                width: "{w:.1}",
                height: "{h:.1}",
                rx: "8",
            }
            g {
                class: "reef-head",
                "data-toggle": "{i}",
                onpointerdown: move |_| hits.pressed(Hit::Toggle(i)),
                rect {
                    class: "reef-head-hit",
                    x: "{x:.1}",
                    y: "{y:.1}",
                    width: "{w:.1}",
                    height: "{HEADER_H}",
                }
                text { class: "reef-group-title", x: "{tx:.1}", y: "{ty:.1}",
                    tspan { class: "reef-chev", "▾" }
                    "{title}"
                }
            }
            text {
                class: "reef-count",
                x: "{cx:.1}",
                y: "{ty:.1}",
                text_anchor: "end",
                "{done}/{total}"
            }
        }
    }
}

/// An arrow, lit up with the selection or the pointer's hover
///
/// # Arguments
///
/// * `d` - The drawing
/// * `k` - The arrow's index among the visible ones
/// * `hl` - Whether it leads to or from the selected box
/// * `lit` - Whether the pointer's hover lights both its ends
/// * `uid` - The view's id, which names its arrowhead marker
#[component]
fn Arrow(d: Shown, k: usize, hl: bool, lit: bool, uid: String) -> Element {
    let e = &d.0.scene.vis.edges[k];
    let path = svg_path(&d.0.scene.out.edges[k], d.0.dir);
    let class = format!(
        "reef-edge {}{}{}",
        if e.kind == EdgeKind::Helps {
            "helps"
        } else {
            "needs"
        },
        if hl { " hl" } else { "" },
        if lit { " lit" } else { "" },
    );
    // several arrows drawn as one are thicker, and say how many
    let width = (e.count > 1).then(|| {
        format!(
            "stroke-width:{:.1}px",
            (1.4 + (e.count as f64).ln()).min(4.0)
        )
    });
    let (count, from, to) = (e.count, e.from, e.to);
    rsx! {
        path {
            class: "{class}",
            "data-edge": "{k}",
            "data-from": "{from}",
            "data-to": "{to}",
            d: "{path}",
            marker_end: "url(#reef-arrow-{uid})",
            style: width,
            if count > 1 {
                svgtitle { "{count} arrows" }
            }
        }
    }
}

/// An acceptance pill on a box: its class, tooltip and text, and where it sits
struct Pill {
    /// Its classes, by kind and whether every criterion is met
    class: String,
    /// Its tooltip
    title: String,
    /// Its text
    text: String,
    /// Its left edge
    x: f64,
    /// Its width
    w: f64,
    /// Its text's centre
    tx: f64,
}

/// A box: a node, or a collapsed group with its toggle and progress bar
///
/// # Arguments
///
/// * `d` - The drawing
/// * `i` - The box's item
/// * `sel` - Whether it is selected
/// * `lit` - Whether the pointer's hover lights it
/// * `hits` - Where it says it was pressed or moved over
#[component]
fn NodeBox(d: Shown, i: usize, sel: bool, lit: bool, hits: Hits) -> Element {
    let (g, it, r, sz) = (
        &d.0.graph,
        &d.0.scene.vis.items[i],
        d.0.scene.out.rects[i],
        &d.0.scene.sizes[i],
    );
    let n = g.node(it.node);
    let mut class = format!("reef-node {}", status_class(g, it.node));
    for (on, name) in [
        (n.optional, " optional"),
        (it.collapsed, " collapsed"),
        (it.ghost, " ghost"),
        (sel, " sel"),
        (lit, " lit"),
    ] {
        if on {
            class.push_str(name);
        }
    }
    let done =
        n.status == Some(Status::Done) || (it.collapsed && g.status(it.node) == Status::Done);
    let text_x = if it.collapsed { PAD_X + 16.0 } else { PAD_X };
    // each line of the title, where it sits
    let lines: Vec<(f64, String)> = sz
        .lines
        .iter()
        .enumerate()
        .map(|(l, line)| (PAD_Y + 12.5 + l as f64 * LINE_H, line.clone()))
        .collect();
    // the acceptance pills, in a row under the title
    let py = PAD_Y + sz.lines.len() as f64 * LINE_H + 3.0;
    let mut px = text_x;
    let mut pills = Vec::with_capacity(sz.pills.len());
    for &(k, met, total, pw) in &sz.pills {
        let full = if met == total { " full" } else { "" };
        let class = format!("reef-acc-pill k-{}{full}", k.as_str());
        pills.push(Pill {
            class,
            title: accept::pill_title(k, met, total),
            text: accept::pill_text(k, met, total),
            x: px,
            w: pw,
            tx: px + pw / 2.0,
        });
        px += pw + PILL_GAP;
    }
    let (pty, x, y, w, h) = (py + 11.0, r.x, r.y, r.w, r.h);
    // a collapsed group's progress bar and count
    let collapsed = it.collapsed;
    let (pdone, ptotal) = g.progress(it.node);
    let bar_y = r.h - PAD_Y - 6.0;
    let bar_w = r.w - 2.0 * PAD_X - 52.0;
    let frac = if ptotal == 0 {
        0.0
    } else {
        pdone as f64 / ptotal as f64
    };
    let (cy, fw, cx, cty) = (PAD_Y + 12.5, bar_w * frac, r.w - PAD_X, bar_y + 6.0);
    rsx! {
        g {
            class: "{class}",
            "data-item": "{i}",
            transform: "translate({x:.1},{y:.1})",
            onpointerdown: move |_| hits.pressed(Hit::Item(i)),
            onpointermove: move |_| hits.came_over(i),
            rect { class: "reef-box", width: "{w:.1}", height: "{h:.1}", rx: "6" }
            if done {
                text { class: "reef-tick", x: "-6", y: "6", "✓" }
            }
            text { class: "reef-label",
                for (y, line) in lines {
                    tspan { x: "{text_x:.1}", y: "{y:.1}", "{line}" }
                }
            }
            for pill in pills {
                g { class: "{pill.class}",
                    svgtitle { "{pill.title}" }
                    rect {
                        x: "{pill.x:.1}",
                        y: "{py:.1}",
                        width: "{pill.w:.1}",
                        height: "{PILL_H}",
                        rx: "7.5",
                    }
                    text {
                        x: "{pill.tx:.1}",
                        y: "{pty:.1}",
                        text_anchor: "middle",
                        "{pill.text}"
                    }
                }
            }
            if collapsed {
                g {
                    class: "reef-expand",
                    "data-toggle": "{i}",
                    onpointerdown: move |_| hits.pressed(Hit::Toggle(i)),
                    rect {
                        class: "reef-expand-hit",
                        x: "2",
                        y: "2",
                        width: "24",
                        height: "24",
                    }
                    text { class: "reef-chev", x: "{PAD_X}", y: "{cy:.1}", "▸" }
                }
                rect {
                    class: "reef-bar-bg",
                    x: "{PAD_X}",
                    y: "{bar_y:.1}",
                    width: "{bar_w:.1}",
                    height: "5",
                    rx: "2.5",
                }
                rect {
                    class: "reef-bar",
                    x: "{PAD_X}",
                    y: "{bar_y:.1}",
                    width: "{fw:.1}",
                    height: "5",
                    rx: "2.5",
                }
                text {
                    class: "reef-count",
                    x: "{cx:.1}",
                    y: "{cty:.1}",
                    text_anchor: "end",
                    "{pdone}/{ptotal}"
                }
            }
        }
    }
}
