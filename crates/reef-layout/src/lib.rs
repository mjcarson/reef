//! Layered ("Sugiyama") layout of a compound directed graph.
//!
//! Groups are laid out bottom-up: each open group's members are laid out on their own, and the
//! group is then one large box in its parent's layout. An edge between boxes in different groups
//! is laid out in the innermost group holding both ends, between the two boxes there that hold
//! them, so it shapes the order at that level and nowhere else.
//!
//! Each level is laid out in four steps: cycles are broken by reversing back edges; boxes are
//! put on layers by longest path from the sources (so work that can start now is on the first
//! layer); long edges get one dummy box a layer; layers are ordered by barycenter sweeps, keeping
//! the order with fewest crossings; and boxes are placed along each layer by weighted isotonic
//! regression toward their neighbours, which keeps the order and the gaps and straightens long
//! edges.

mod level;

pub use level::Spacing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dir {
    /// Layers run top to bottom.
    #[default]
    TB,
    /// Layers run left to right.
    LR,
}

/// A box to place. A container's `w` and `h` are its minimum size (room for its title); its real
/// size comes from its members.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LNode {
    pub parent: Option<usize>,
    pub w: f64,
    pub h: f64,
    pub container: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LEdge {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub dir: Dir,
    pub nodes: Vec<LNode>,
    pub edges: Vec<LEdge>,
    pub spacing: Spacing,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn contains(&self, o: &Rect) -> bool {
        const E: f64 = 1e-6;
        o.x >= self.x - E && o.y >= self.y - E && o.right() <= self.right() + E && o.bottom() <= self.bottom() + E
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Output {
    /// Each node's box, absolute.
    pub rects: Vec<Rect>,
    /// Each edge's route, from its source to its target. Empty for an edge between a group and
    /// a box inside it, which has no place to go.
    pub edges: Vec<Vec<Point>>,
    pub width: f64,
    pub height: f64,
}

/// Lay out a compound graph.
pub fn layout(input: &Input) -> Output {
    let n = input.nodes.len();
    let sp = &input.spacing;
    let root = n; // the virtual container holding the top-level boxes
    let parent_of = |i: usize| input.nodes[i].parent.filter(|&p| p < n && input.nodes[p].container).unwrap_or(root);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for i in 0..n {
        children[parent_of(i)].push(i);
    }
    let chain = |i: usize| {
        let mut v = vec![i];
        let mut cur = i;
        while cur != root {
            cur = parent_of(cur);
            v.push(cur);
        }
        v
    };

    // Which level each edge is laid out at, and between which two boxes there.
    struct Routed {
        level: usize,
        a: usize,
        b: usize,
        bundle: usize,
    }
    let mut bundles: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n + 1];
    let mut routed: Vec<Option<Routed>> = Vec::with_capacity(input.edges.len());
    for e in &input.edges {
        if e.from >= n || e.to >= n || e.from == e.to {
            routed.push(None);
            continue;
        }
        let cu = chain(e.from);
        let cv = chain(e.to);
        let mut found = None;
        'outer: for (iu, x) in cu.iter().enumerate().skip(1) {
            for (iv, y) in cv.iter().enumerate().skip(1) {
                if x == y {
                    found = Some((*x, cu[iu - 1], cv[iv - 1]));
                    break 'outer;
                }
            }
        }
        match found {
            Some((level, a, b)) if a != b => {
                let list = &mut bundles[level];
                let bundle = match list.iter().position(|&p| p == (a, b)) {
                    Some(k) => k,
                    None => {
                        list.push((a, b));
                        list.len() - 1
                    }
                };
                routed.push(Some(Routed { level, a, b, bundle }));
            }
            _ => routed.push(None),
        }
    }

    // Bottom-up: sizes and local positions.
    let mut size: Vec<(f64, f64)> = input.nodes.iter().map(|nd| (nd.w, nd.h)).chain([(0.0, 0.0)]).collect();
    let mut local: Vec<Point> = vec![Point::default(); n + 1];
    let mut origin_off: Vec<Point> = vec![Point::default(); n + 1];
    let mut bundle_pts: Vec<Vec<level::BundleRoute>> = vec![Vec::new(); n + 1];
    for c in post_order(&children, root) {
        if c != root && !input.nodes[c].container {
            continue;
        }
        let kids = &children[c];
        let ext: Vec<(f64, f64)> = kids.iter().map(|&k| size[k]).collect();
        let local_bundles: Vec<(usize, usize)> = bundles[c]
            .iter()
            .map(|&(a, b)| (kids.iter().position(|&k| k == a).unwrap(), kids.iter().position(|&k| k == b).unwrap()))
            .collect();
        let res = level::layout_level(input.dir, &ext, &local_bundles, sp);
        for (j, &k) in kids.iter().enumerate() {
            local[k] = res.pos[j];
        }
        bundle_pts[c] = res.bundles;
        let (cw, ch) = res.size;
        if c == root {
            size[c] = (cw + 2.0 * sp.margin, ch + 2.0 * sp.margin);
            origin_off[c] = Point { x: sp.margin, y: sp.margin };
        } else {
            let min_w = input.nodes[c].w;
            let min_h = input.nodes[c].h;
            let w = (cw + 2.0 * sp.pad).max(min_w);
            let h = (ch + 2.0 * sp.pad + sp.header).max(min_h);
            size[c] = (w, h);
            origin_off[c] = Point { x: (w - cw) / 2.0, y: sp.header + sp.pad };
        }
    }

    // Top-down: absolute boxes.
    let mut rects = vec![Rect::default(); n + 1];
    rects[root] = Rect { x: 0.0, y: 0.0, w: size[root].0, h: size[root].1 };
    let mut stack = vec![root];
    while let Some(c) = stack.pop() {
        let o = Point { x: rects[c].x + origin_off[c].x, y: rects[c].y + origin_off[c].y };
        for &k in &children[c] {
            rects[k] = Rect { x: o.x + local[k].x, y: o.y + local[k].y, w: size[k].0, h: size[k].1 };
            if !children[k].is_empty() || (k < n && input.nodes[k].container) {
                stack.push(k);
            }
        }
    }

    // Routes.
    let tb = input.dir == Dir::TB;
    // Exit and entry points on a box's main-axis sides, at cross offset `c`.
    let lo = |r: &Rect, c: f64| if tb { Point { x: c, y: r.y } } else { Point { x: r.x, y: c } };
    let hi = |r: &Rect, c: f64| if tb { Point { x: c, y: r.bottom() } } else { Point { x: r.right(), y: c } };
    let cross_c = |r: &Rect| if tb { r.cx() } else { r.cy() };
    let mut edges: Vec<Vec<Point>> = Vec::with_capacity(input.edges.len());
    for (e, r) in input.edges.iter().zip(&routed) {
        let Some(r) = r else {
            edges.push(Vec::new());
            continue;
        };
        let br = &bundle_pts[r.level][r.bundle];
        let o = Point { x: rects[r.level].x + origin_off[r.level].x, y: rects[r.level].y + origin_off[r.level].y };
        let mid: Vec<Point> = br.points.iter().map(|p| Point { x: p.x + o.x, y: p.y + o.y }).collect();
        let (ru, ra, rv, rb) = (&rects[e.from], &rects[r.a], &rects[e.to], &rects[r.b]);
        let (cu, cv) = (cross_c(ru), cross_c(rv));
        let mut pts = Vec::with_capacity(mid.len() + 4);
        if !br.reversed {
            pts.push(hi(ru, cu));
            if e.from != r.a {
                pts.push(hi(ra, cu));
            }
            pts.extend(mid);
            if e.to != r.b {
                pts.push(lo(rb, cv));
            }
            pts.push(lo(rv, cv));
        } else {
            pts.push(lo(ru, cu));
            if e.from != r.a {
                pts.push(lo(ra, cu));
            }
            pts.extend(mid);
            if e.to != r.b {
                pts.push(hi(rb, cv));
            }
            pts.push(hi(rv, cv));
        }
        edges.push(pts);
    }
    spread_ports(input, &rects, &mut edges, tb);

    rects.truncate(n);
    Output { rects, edges, width: size[root].0, height: size[root].1 }
}

/// Spread the ends of the edges meeting one side of a box along that side, in the order they
/// arrive, so they do not all meet at its centre.
fn spread_ports(input: &Input, rects: &[Rect], edges: &mut [Vec<Point>], tb: bool) {
    use std::collections::HashMap;
    // (node, which end) -> edges ending there, with the cross position of the neighbouring point
    let mut ends: HashMap<(usize, bool), Vec<(usize, f64)>> = HashMap::new();
    for (k, (e, pts)) in input.edges.iter().zip(edges.iter()).enumerate() {
        if pts.len() < 2 {
            continue;
        }
        let cross = |p: &Point| if tb { p.x } else { p.y };
        ends.entry((e.from, true)).or_default().push((k, cross(&pts[1])));
        ends.entry((e.to, false)).or_default().push((k, cross(&pts[pts.len() - 2])));
    }
    for ((node, is_start), mut list) in ends {
        if list.len() < 2 {
            continue;
        }
        list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let r = &rects[node];
        let (start, len) = if tb { (r.x, r.w) } else { (r.y, r.h) };
        let span = (len * 0.6).min(14.0 * (list.len() - 1) as f64);
        let first = start + len / 2.0 - span / 2.0;
        let step = span / (list.len() - 1) as f64;
        for (j, (k, _)) in list.iter().enumerate() {
            let c = first + step * j as f64;
            let pts = &mut edges[*k];
            let (end, next) = if is_start { (0, 1) } else { (pts.len() - 1, pts.len() - 2) };
            let old = if tb { pts[end].x } else { pts[end].y };
            for idx in [end, next] {
                // Move the point on the box, and the drop to its group's border with it.
                let p = &mut pts[idx];
                let v = if tb { &mut p.x } else { &mut p.y };
                if (*v - old).abs() < 1e-9 {
                    *v = c;
                }
            }
        }
    }
}

fn post_order(children: &[Vec<usize>], root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![(root, false)];
    while let Some((c, done)) = stack.pop() {
        if done {
            out.push(c);
        } else {
            stack.push((c, true));
            for &k in &children[c] {
                stack.push((k, false));
            }
        }
    }
    out
}

/// An SVG path through a route's points: cubic curves that leave and meet each point along the
/// layer axis, so edges flow between layers.
pub fn svg_path(points: &[Point], dir: Dir) -> String {
    use std::fmt::Write;
    let mut d = String::new();
    let Some(first) = points.first() else { return d };
    let _ = write!(d, "M{:.1},{:.1}", first.x, first.y);
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        match dir {
            Dir::TB => {
                let dy = (q.y - p.y) / 2.0;
                let _ = write!(d, " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}", p.x, p.y + dy, q.x, q.y - dy, q.x, q.y);
            }
            Dir::LR => {
                let dx = (q.x - p.x) / 2.0;
                let _ = write!(d, " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}", p.x + dx, p.y, q.x - dx, q.y, q.x, q.y);
            }
        }
    }
    d
}

#[cfg(test)]
mod tests;
