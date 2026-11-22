//! What a view shows and where: its laid-out boxes, its pan and zoom, and its search

use crate::accept::Filter;
use crate::markup::{self, BoxSize, HEADER_H};
use reef_layout::{Dir, Input, LEdge, LNode, Output, Rect, Spacing};
use reef_model::{Graph, Visible};
use std::collections::HashSet;

/// The furthest a view zooms out
pub const MIN_K: f64 = 0.05;
/// The furthest a view zooms in
pub const MAX_K: f64 = 4.0;
/// Below this scale a 13px label is unreadable, so a view starts at [`READABLE`] instead
const TOO_SMALL: f64 = 0.35;
/// The scale a view too big to fit readably starts at
const READABLE: f64 = 0.55;
/// The most search results shown
const RESULTS: usize = 12;

/// What a view shows now, laid out: the items visible, each one's size, and where each goes
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// The boxes, open groups and arrows shown
    pub vis: Visible,
    /// Each visible item's wrapped title, pills and size, in `vis.items` order
    pub sizes: Vec<BoxSize>,
    /// Where the layout put each item and arrow
    pub out: Output,
}

impl Scene {
    /// Lays out what a view of a plan shows
    ///
    /// # Arguments
    ///
    /// * `g` - The plan
    /// * `root` - The group the view is of, or `None` for the whole plan
    /// * `ghosts` - Whether the root's outside neighbours are drawn faded around it
    /// * `expanded` - The groups open, by index
    /// * `filter` - Which kinds of acceptance criteria the boxes' pills count
    /// * `dir` - Which way the arrows run
    /// * `measure` - How wide a string is in the boxes' 13px font
    pub fn build(
        g: &Graph,
        root: Option<usize>,
        ghosts: bool,
        expanded: &HashSet<usize>,
        filter: &Filter,
        dir: Dir,
        measure: &dyn Fn(&str) -> f64,
    ) -> Scene {
        // what is visible, then how big each box is
        let vis = g.visible(root, ghosts, expanded);
        let sizes = markup::sizes(g, &vis, filter, measure);
        // lay them out with room for each open group's header
        let input = Input {
            dir,
            nodes: vis
                .items
                .iter()
                .zip(&sizes)
                .map(|(it, s)| LNode {
                    parent: it.parent,
                    w: s.w,
                    h: s.h,
                    container: it.open,
                })
                .collect(),
            edges: vis
                .edges
                .iter()
                .map(|e| LEdge {
                    from: e.from,
                    to: e.to,
                })
                .collect(),
            spacing: Spacing {
                header: HEADER_H,
                ..Spacing::default()
            },
        };
        let out = reef_layout::layout(&input);
        Scene { vis, sizes, out }
    }

    /// The item a node is shown as, if it is visible: itself, or the collapsed group holding it
    ///
    /// # Arguments
    ///
    /// * `node` - The node's index in the plan
    pub fn item_of(&self, node: usize) -> Option<usize> {
        self.vis.shown_as.get(node).copied().flatten()
    }

    /// The item a node is drawn as itself, if it is, rather than inside a collapsed group
    ///
    /// # Arguments
    ///
    /// * `node` - The node's index in the plan
    pub fn own_item(&self, node: usize) -> Option<usize> {
        self.item_of(node)
            .filter(|&i| self.vis.items[i].node == node)
    }

    /// Everything upstream and downstream of an item, and the open groups holding any of it
    ///
    /// # Arguments
    ///
    /// * `start` - The item to light up from
    pub fn lineage(&self, start: usize) -> HashSet<usize> {
        let mut out = HashSet::from([start]);
        // follow the arrows forward, then back
        for forward in [true, false] {
            let mut stack = vec![start];
            let mut seen = HashSet::from([start]);
            while let Some(v) = stack.pop() {
                for e in &self.vis.edges {
                    let (a, b) = if forward {
                        (e.from, e.to)
                    } else {
                        (e.to, e.from)
                    };
                    if a == v && seen.insert(b) {
                        stack.push(b);
                    }
                }
            }
            out.extend(seen);
        }
        // open groups holding a lit box stay lit too
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
}

/// A view's pan and zoom: content at `(x, y)` is drawn at `(x * k + tx, y * k + ty)`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The scale
    pub k: f64,
    /// The horizontal offset, in screen pixels
    pub tx: f64,
    /// The vertical offset, in screen pixels
    pub ty: f64,
}

impl Default for Camera {
    /// Unscaled, at the origin
    fn default() -> Self {
        Camera {
            k: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

impl Camera {
    /// The SVG transform that draws the content where the camera says
    pub fn transform(&self) -> String {
        format!(
            "translate({:.2},{:.2}) scale({:.4})",
            self.tx, self.ty, self.k
        )
    }

    /// Fits the whole layout in the part of the stage the sidebar leaves free, never zooming in
    ///
    /// # Arguments
    ///
    /// * `out` - The layout
    /// * `w` - The stage's free width
    /// * `h` - The stage's height
    pub fn fit(&mut self, out: &Output, w: f64, h: f64) {
        // the scale that fits both ways, centred across and at the top or middle down
        let (cw, ch) = (out.width.max(1.0), out.height.max(1.0));
        self.k = (w / cw).min(h / ch).clamp(MIN_K, 1.0);
        self.tx = (w - cw * self.k) / 2.0;
        self.ty = ((h - ch * self.k) / 2.0).max(0.0);
    }

    /// Fits the layout, unless that would make labels too small to read: then starts at a
    /// readable size, at the top, and leaves the rest to panning, zooming out, or Fit
    ///
    /// # Arguments
    ///
    /// * `out` - The layout
    /// * `w` - The stage's free width
    /// * `h` - The stage's height
    pub fn initial(&mut self, out: &Output, w: f64, h: f64) {
        self.fit(out, w, h);
        // too small to read: readable, centred across, from the top
        if self.k < TOO_SMALL {
            self.k = READABLE;
            self.tx = (w - out.width * self.k) / 2.0;
            self.ty = 0.0;
        }
    }

    /// Zooms by a factor, keeping the point under `(sx, sy)` where it is
    ///
    /// # Arguments
    ///
    /// * `factor` - How much to scale by
    /// * `sx` - The fixed point's x, from the stage's left
    /// * `sy` - The fixed point's y, from the stage's top
    pub fn zoom_at(&mut self, factor: f64, sx: f64, sy: f64) {
        // the new scale, within bounds, with the offset moved to keep the point still
        let k = (self.k * factor).clamp(MIN_K, MAX_K);
        self.tx = sx - (sx - self.tx) * (k / self.k);
        self.ty = sy - (sy - self.ty) * (k / self.k);
        self.k = k;
    }

    /// Centres a box in the free part of the stage, zooming in to read it if it is small
    ///
    /// # Arguments
    ///
    /// * `r` - The box, in layout units
    /// * `w` - The stage's free width
    /// * `h` - The stage's height
    pub fn center_on(&mut self, r: Rect, w: f64, h: f64) {
        // far out, come in to a readable scale first
        if self.k < 0.7 {
            self.k = 0.9;
        }
        self.tx = w / 2.0 - r.cx() * self.k;
        self.ty = h / 2.0 - r.cy() * self.k;
    }

    /// Pans so a box that moved in the layout stays where it was on screen
    ///
    /// # Arguments
    ///
    /// * `before` - Where the box was laid out
    /// * `after` - Where it is laid out now
    pub fn keep(&mut self, before: Rect, after: Rect) {
        self.tx += (before.x - after.x) * self.k;
        self.ty += (before.y - after.y) * self.k;
    }

    /// Pans so what was in the middle of the stage stays there when the stage changes size
    ///
    /// # Arguments
    ///
    /// * `was` - The stage's size before
    /// * `now` - Its size now
    pub fn recentre(&mut self, was: (f64, f64), now: (f64, f64)) {
        self.tx += (now.0 - was.0) / 2.0;
        self.ty += (now.1 - was.1) / 2.0;
    }
}

/// The nodes a search finds, best first: an exact id, then ids or titles that start with the
/// text, then ones holding it, each within the view's root before those outside it
///
/// # Arguments
///
/// * `g` - The plan
/// * `root` - The group the view is of, if any
/// * `text` - What was typed
pub fn search(g: &Graph, root: Option<usize>, text: &str) -> Vec<usize> {
    // nothing typed finds nothing
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Vec::new();
    }
    let in_scope = |i: usize| root.is_none_or(|r| g.is_ancestor(r, i));
    // rank every node that matches at all
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
    // the best few, in plan order within a rank
    hits.sort();
    hits.into_iter().take(RESULTS).map(|h| h.1).collect()
}

/// The key a view's open groups are saved under, given the name of the plan it draws
///
/// # Arguments
///
/// * `plan` - The plan's name, such as `reef/central`
pub fn plan_key(plan: &str) -> String {
    format!("reef:plan:{plan}")
}

/// Where a view's sidebar width is kept: one for views filling a page, one for views in the text
///
/// # Arguments
///
/// * `page` - Whether the view fills the page
pub fn side_w_key(page: bool) -> &'static str {
    if page {
        "reef:side-w:page"
    } else {
        "reef:side-w:inline"
    }
}

/// The open groups' ids as they are saved
///
/// # Arguments
///
/// * `ids` - The open groups' ids
pub fn groups_to_stored<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    serde_json::to_string(&ids.into_iter().collect::<Vec<_>>()).unwrap_or_default()
}

/// The open groups' ids saved, if what is saved reads as them
///
/// # Arguments
///
/// * `stored` - What was saved
pub fn groups_from_stored(stored: &str) -> Option<Vec<String>> {
    serde_json::from_str(stored).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_model::Node;

    /// A node with an id, a title, a group and needs
    fn mk(id: &str, parent: Option<&str>, needs: &[&str]) -> Node {
        Node {
            id: id.into(),
            title: format!("{id} title"),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            ..Node::default()
        }
    }

    /// Search ranks an exact id first, and keeps what is outside the root last
    #[test]
    fn search_ranks() {
        let (g, _) = Graph::build(vec![
            mk("g", None, &[]),
            mk("a1", Some("g"), &[]),
            mk("a", None, &[]),
            mk("ba", None, &[]),
        ]);
        let ids = |hits: Vec<usize>| {
            hits.into_iter()
                .map(|i| g.node(i).id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(search(&g, None, "a")), ["a", "a1", "ba"]);
        assert_eq!(ids(search(&g, g.index_of("g"), "a")), ["a1", "a", "ba"]);
        assert!(search(&g, None, "  ").is_empty());
    }

    /// Lineage follows the arrows both ways from an item, and lights the groups holding it
    #[test]
    fn lineage_follows_arrows() {
        let (g, _) = Graph::build(vec![
            mk("a", None, &[]),
            mk("b", None, &["a"]),
            mk("c", None, &[]),
        ]);
        let scene = Scene::build(
            &g,
            None,
            false,
            &HashSet::new(),
            &Filter::default(),
            Dir::TB,
            &|s| s.len() as f64 * 7.0,
        );
        let item = |id: &str| scene.item_of(g.index_of(id).unwrap()).unwrap();
        assert_eq!(
            scene.lineage(item("a")),
            HashSet::from([item("a"), item("b")])
        );
        assert_eq!(scene.lineage(item("c")), HashSet::from([item("c")]));
    }

    /// Zooming keeps the point under the pointer still, and fitting never zooms in
    #[test]
    fn camera_zooms_and_fits() {
        let mut cam = Camera::default();
        cam.zoom_at(2.0, 100.0, 50.0);
        assert_eq!((cam.k, cam.tx, cam.ty), (2.0, -100.0, -50.0));
        let out = Output {
            width: 100.0,
            height: 50.0,
            ..Output::default()
        };
        cam.fit(&out, 400.0, 400.0);
        assert_eq!((cam.k, cam.tx, cam.ty), (1.0, 150.0, 175.0));
        // a layout too big to read when fitted starts readable, at the top
        let big = Output {
            width: 10_000.0,
            height: 100.0,
            ..Output::default()
        };
        cam.initial(&big, 400.0, 400.0);
        assert_eq!((cam.k, cam.ty), (READABLE, 0.0));
    }

    /// Saved groups read back as they were written
    #[test]
    fn groups_round_trip() {
        assert_eq!(
            groups_from_stored(&groups_to_stored(["a", "b"])),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(groups_from_stored("nope"), None);
    }
}
