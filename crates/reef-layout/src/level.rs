//! The layout of one level: the direct members of one group.

use crate::{Dir, Point};

/// Gaps and paddings, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    /// Between boxes on one layer.
    pub node_gap: f64,
    /// Between layers.
    pub rank_gap: f64,
    /// Between an edge passing through a layer and its neighbours.
    pub edge_gap: f64,
    /// Inside a group's frame.
    pub pad: f64,
    /// A group frame's title bar.
    pub header: f64,
    /// Around the whole drawing.
    pub margin: f64,
    /// A group with this many members and no edges among them is packed in a grid rather
    /// than one long layer.
    pub grid_min: usize,
}

impl Default for Spacing {
    fn default() -> Self {
        Spacing { node_gap: 22.0, rank_gap: 46.0, edge_gap: 10.0, pad: 14.0, header: 30.0, margin: 16.0, grid_min: 5 }
    }
}

/// Where an edge between two boxes of a level runs through the layers between them, in that
/// level's coordinates, ordered from the edge's source side to its target side.
#[derive(Debug, Clone, Default)]
pub struct BundleRoute {
    pub points: Vec<Point>,
    /// The edge points against the layers (it closed a cycle).
    pub reversed: bool,
}

pub(crate) struct LevelResult {
    /// Each box's top-left corner.
    pub pos: Vec<Point>,
    pub size: (f64, f64),
    pub bundles: Vec<BundleRoute>,
}

/// Lay out boxes of the given `(w, h)` with edges `(a, b)` between them.
pub(crate) fn layout_level(dir: Dir, ext: &[(f64, f64)], edges: &[(usize, usize)], sp: &Spacing) -> LevelResult {
    let n = ext.len();
    if n == 0 {
        return LevelResult { pos: Vec::new(), size: (0.0, 0.0), bundles: Vec::new() };
    }
    if edges.is_empty() && n >= sp.grid_min {
        return grid(ext, sp);
    }
    let tb = dir == Dir::TB;
    let cext: Vec<f64> = ext.iter().map(|e| if tb { e.0 } else { e.1 }).collect();
    let mext: Vec<f64> = ext.iter().map(|e| if tb { e.1 } else { e.0 }).collect();

    // 1. Break cycles: reverse the edges a depth-first walk finds going back.
    let mut out_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, &(a, b)) in edges.iter().enumerate() {
        if a != b {
            out_adj[a].push(k);
        }
    }
    let mut reversed = vec![false; edges.len()];
    let mut state = vec![0u8; n];
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        state[s] = 1;
        let mut stack = vec![(s, 0usize)];
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if *next < out_adj[v].len() {
                let k = out_adj[v][*next];
                *next += 1;
                let w = edges[k].1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[k] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let dag: Vec<(usize, usize)> = edges
        .iter()
        .zip(&reversed)
        .map(|(&(a, b), &r)| if r { (b, a) } else { (a, b) })
        .collect();

    // 2. Layers by longest path from the sources.
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &dag {
        if a != b {
            succ[a].push(b);
            indeg[b] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for &w in &succ[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push(w);
            }
        }
    }
    // The longest path puts every source on the first layer, which makes that layer as wide as
    // the plan has starting points. Pull a box down to just above the first thing it feeds when
    // that shortens at least as many edges as it lengthens: sources always, and the head of a
    // chain hanging off one of them. Readiness is drawn on the box, so a box needs no layer to
    // say it can start.
    let mut preds = vec![0usize; n];
    for &(a, b) in &dag {
        if a != b {
            preds[b] += 1;
        }
    }
    for &v in queue.iter().rev() {
        if succ[v].is_empty() || preds[v] > succ[v].len() {
            continue;
        }
        let target = succ[v].iter().map(|&s| layer[s]).min().unwrap() - 1;
        if target > layer[v] {
            layer[v] = target;
        }
    }

    // 3. Dummy boxes along long edges, each in the same connected part as its edge.
    let mut part = parts(n, edges);
    let mut vlayer = layer.clone();
    let mut vcext = cext.clone();
    let mut dummy = vec![false; n];
    let mut chains: Vec<Vec<usize>> = Vec::with_capacity(dag.len());
    for &(a, b) in &dag {
        let mut ch = vec![a];
        for l in layer[a] + 1..layer[b] {
            vlayer.push(l);
            vcext.push(0.0);
            dummy.push(true);
            part.push(part[a]);
            ch.push(vlayer.len() - 1);
        }
        ch.push(b);
        chains.push(ch);
    }
    let vn = vlayer.len();
    let nlayers = vlayer.iter().max().map_or(1, |m| m + 1);
    let mut up: Vec<Vec<(usize, f64)>> = vec![Vec::new(); vn];
    let mut down: Vec<Vec<(usize, f64)>> = vec![Vec::new(); vn];
    for ch in &chains {
        for w in ch.windows(2) {
            let weight = match (dummy[w[0]], dummy[w[1]]) {
                (true, true) => 8.0,
                (false, false) => 1.0,
                _ => 2.0,
            };
            down[w[0]].push((w[1], weight));
            up[w[1]].push((w[0], weight));
        }
    }

    // 4. Order each layer.
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); nlayers];
    {
        let mut seen = vec![false; vn];
        let mut starts: Vec<usize> = (0..vn).filter(|&v| up[v].is_empty()).collect();
        starts.sort_by_key(|&v| (vlayer[v], v));
        for s in starts {
            let mut stack = vec![s];
            while let Some(v) = stack.pop() {
                if seen[v] {
                    continue;
                }
                seen[v] = true;
                layers[vlayer[v]].push(v);
                for &(w, _) in down[v].iter().rev() {
                    if !seen[w] {
                        stack.push(w);
                    }
                }
            }
        }
        for v in 0..vn {
            if !seen[v] {
                layers[vlayer[v]].push(v);
            }
        }
    }
    let mut pos = vec![0usize; vn];
    let set_pos = |layers: &Vec<Vec<usize>>, pos: &mut Vec<usize>| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                pos[v] = i;
            }
        }
    };
    set_pos(&layers, &mut pos);
    let mut best = layers.clone();
    let mut best_cross = crossings(&layers, &down, &pos);
    for iter in 0..24 {
        if best_cross == 0 {
            break;
        }
        let downward = iter % 2 == 0;
        let range: Vec<usize> = if downward { (1..nlayers).collect() } else { (0..nlayers.saturating_sub(1)).rev().collect() };
        for l in range {
            let nb = if downward { &up } else { &down };
            let mut keyed: Vec<(f64, usize, usize)> = layers[l]
                .iter()
                .map(|&v| {
                    let list = &nb[v];
                    let bary = if list.is_empty() {
                        pos[v] as f64
                    } else {
                        list.iter().map(|&(w, _)| pos[w] as f64).sum::<f64>() / list.len() as f64
                    };
                    (bary, pos[v], v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            layers[l] = keyed.into_iter().map(|k| k.2).collect();
            for (i, &v) in layers[l].iter().enumerate() {
                pos[v] = i;
            }
        }
        let c = crossings(&layers, &down, &pos);
        if c < best_cross {
            best_cross = c;
            best = layers.clone();
        }
    }
    // Keep each connected part together on every layer. The sort is stable, so the order
    // within a part, and so its crossings, stay as the sweeps left them.
    let mut layers = best;
    for l in &mut layers {
        l.sort_by_key(|&v| part[v]);
    }
    set_pos(&layers, &mut pos);

    // 5. Place boxes along each layer.
    let gap = |a: usize, b: usize| if dummy[a] || dummy[b] { sp.edge_gap } else { sp.node_gap };
    let mut center = vec![0.0f64; vn];
    for l in &layers {
        let mut x = 0.0;
        for (i, &v) in l.iter().enumerate() {
            if i > 0 {
                x += vcext[l[i - 1]] / 2.0 + gap(l[i - 1], v) + vcext[v] / 2.0;
            }
            center[v] = x;
        }
    }
    let passes: [(bool, bool); 4] = [(true, false), (false, true), (true, false), (false, true)];
    for round in 0..6 {
        let mut this_round = passes.to_vec();
        if round == 5 {
            this_round.push((true, true));
        }
        for (use_up, use_down) in this_round {
            let order: Vec<usize> = if use_up && !use_down { (0..nlayers).collect() } else { (0..nlayers).rev().collect() };
            // each part of each layer is placed on its own, since no arrow ties it to the others
            for ids in order.into_iter().flat_map(|l| layers[l].chunk_by(|&a, &b| part[a] == part[b])) {
                let mut desired = Vec::with_capacity(ids.len());
                let mut weights = Vec::with_capacity(ids.len());
                for &v in ids {
                    let mut sw = 0.0;
                    let mut sx = 0.0;
                    if use_up {
                        for &(w, wt) in &up[v] {
                            sw += wt;
                            sx += wt * center[w];
                        }
                    }
                    if use_down {
                        for &(w, wt) in &down[v] {
                            sw += wt;
                            sx += wt * center[w];
                        }
                    }
                    if sw == 0.0 {
                        desired.push(center[v]);
                        weights.push(0.05);
                    } else {
                        desired.push(sx / sw);
                        weights.push(sw);
                    }
                }
                let seps: Vec<f64> = (1..ids.len())
                    .map(|i| vcext[ids[i - 1]] / 2.0 + gap(ids[i - 1], ids[i]) + vcext[ids[i]] / 2.0)
                    .collect();
                let placed = place_in_order(&desired, &weights, &seps);
                for (&v, x) in ids.iter().zip(placed) {
                    center[v] = x;
                }
            }
        }
    }
    // set the parts side by side, each just past the ones before it
    pack_parts(&mut center, &vcext, &vlayer, &part, nlayers, sp.node_gap);
    let min_c = (0..vn).map(|v| center[v] - vcext[v] / 2.0).fold(f64::INFINITY, f64::min);
    let max_c = (0..vn).map(|v| center[v] + vcext[v] / 2.0).fold(f64::NEG_INFINITY, f64::max);
    for c in &mut center {
        *c -= min_c;
    }

    // 6. Layers along the main axis.
    let mut lext = vec![0.0f64; nlayers];
    for v in 0..n {
        lext[layer[v]] = lext[layer[v]].max(mext[v]);
    }
    let mut lstart = vec![0.0f64; nlayers];
    for l in 1..nlayers {
        lstart[l] = lstart[l - 1] + lext[l - 1] + sp.rank_gap;
    }
    let main_total = lstart[nlayers - 1] + lext[nlayers - 1];
    let cross_total = max_c - min_c;
    let to_real = |c: f64, m: f64| if tb { Point { x: c, y: m } } else { Point { x: m, y: c } };

    let pos_out: Vec<Point> = (0..n)
        .map(|v| {
            let m = lstart[layer[v]] + (lext[layer[v]] - mext[v]) / 2.0;
            to_real(center[v] - cext[v] / 2.0, m)
        })
        .collect();
    let bundles = chains
        .iter()
        .zip(&reversed)
        .map(|(ch, &rev)| {
            let mut points = Vec::new();
            for &d in &ch[1..ch.len() - 1] {
                let l = vlayer[d];
                points.push(to_real(center[d], lstart[l]));
                points.push(to_real(center[d], lstart[l] + lext[l]));
            }
            if rev {
                points.reverse();
            }
            BundleRoute { points, reversed: rev }
        })
        .collect();
    let size = if tb { (cross_total, main_total) } else { (main_total, cross_total) };
    LevelResult { pos: pos_out, size, bundles }
}

/// Count edge crossings between adjacent layers.
fn crossings(layers: &[Vec<usize>], down: &[Vec<(usize, f64)>], pos: &[usize]) -> usize {
    let mut total = 0;
    for l in layers {
        let mut segs: Vec<(usize, usize)> = Vec::new();
        for &v in l {
            for &(w, _) in &down[v] {
                segs.push((pos[v], pos[w]));
            }
        }
        segs.sort_unstable();
        // Inversions in the second coordinate.
        for i in 0..segs.len() {
            for j in i + 1..segs.len() {
                if segs[i].0 != segs[j].0 && segs[j].1 < segs[i].1 {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Place boxes in a fixed order as close as they can be to where they want to be, weighted,
/// keeping at least `seps[i]` between box `i` and box `i + 1`: weighted isotonic regression by
/// pool-adjacent-violators on positions shifted by the running separation.
fn place_in_order(desired: &[f64], weights: &[f64], seps: &[f64]) -> Vec<f64> {
    let n = desired.len();
    let mut offset = vec![0.0; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + seps[i - 1];
    }
    // blocks: (weight sum, weighted target sum, length)
    let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let w = weights[i].max(1e-9);
        blocks.push((w, w * (desired[i] - offset[i]), 1));
        while blocks.len() > 1 {
            let (w2, s2, _) = blocks[blocks.len() - 1];
            let (w1, s1, _) = blocks[blocks.len() - 2];
            if s1 / w1 <= s2 / w2 {
                break;
            }
            let b2 = blocks.pop().unwrap();
            let b1 = blocks.last_mut().unwrap();
            b1.0 += b2.0;
            b1.1 += b2.1;
            b1.2 += b2.2;
        }
    }
    let mut out = Vec::with_capacity(n);
    for (w, s, len) in blocks {
        let y = s / w;
        for _ in 0..len {
            let i = out.len();
            out.push(y + offset[i]);
        }
    }
    out
}

/// Which connected part of a level each box is in, ignoring which way the edges point
///
/// A part is named by the smallest index of a box in it, so parts sort in the order their first
/// boxes come in.
///
/// # Arguments
///
/// * `n` - How many boxes the level has
/// * `edges` - The edges `(a, b)` between them
fn parts(n: usize, edges: &[(usize, usize)]) -> Vec<usize> {
    /// Follows a box's links up to the box that names its part
    ///
    /// # Arguments
    ///
    /// * `link` - Each box's link toward the box naming its part
    /// * `v` - The box to find the part of
    fn find(link: &mut [usize], mut v: usize) -> usize {
        while link[v] != v {
            // halve the path on the way, so later finds are short
            link[v] = link[link[v]];
            v = link[v];
        }
        v
    }
    // every box starts as a part of its own
    let mut link: Vec<usize> = (0..n).collect();
    // join the two parts each edge spans under the smaller of their names
    for &(a, b) in edges {
        let (ra, rb) = (find(&mut link, a), find(&mut link, b));
        if ra != rb {
            link[ra.max(rb)] = ra.min(rb);
        }
    }
    // name each box by its part
    (0..n).map(|v| find(&mut link, v)).collect()
}

/// Set a level's connected parts side by side across its layers
///
/// Each part was placed on its own, so nothing ties where one part sits to where another does.
/// In the order of their names, each part moves the least it can to sit `gap` past the parts
/// before it on every layer they share, so a part fits beside the others' shape rather than
/// past their widest layer.
///
/// # Arguments
///
/// * `center` - Each box's centre across the layers, dummies included, moved in place
/// * `cext` - Each box's extent across the layers
/// * `vlayer` - Each box's layer
/// * `part` - The connected part each box is in
/// * `nlayers` - How many layers the level has
/// * `gap` - The room to keep between two parts
fn pack_parts(center: &mut [f64], cext: &[f64], vlayer: &[usize], part: &[usize], nlayers: usize, gap: f64) {
    // the boxes of each part, the parts in the order of their names
    let mut members: Vec<(usize, usize)> = part.iter().copied().zip(0..).collect();
    members.sort_unstable();
    let groups: Vec<&[(usize, usize)]> = members.chunk_by(|a, b| a.0 == b.0).collect();
    // one part has nothing to sit beside
    if groups.len() < 2 {
        return;
    }
    // how far the parts already set reach on each layer
    let mut reach = vec![f64::NEG_INFINITY; nlayers];
    for group in groups {
        // this part's extent on each layer
        let mut lo = vec![f64::INFINITY; nlayers];
        let mut hi = vec![f64::NEG_INFINITY; nlayers];
        for &(_, v) in group {
            let l = vlayer[v];
            lo[l] = lo[l].min(center[v] - cext[v] / 2.0);
            hi[l] = hi[l].max(center[v] + cext[v] / 2.0);
        }
        // the least shift that clears the parts before it on every layer they share
        let shared = (0..nlayers)
            .filter(|&l| lo[l].is_finite() && reach[l].is_finite())
            .map(|l| reach[l] + gap - lo[l])
            .fold(f64::NEG_INFINITY, f64::max);
        let shift = if shared.is_finite() {
            shared
        } else {
            // a part that shares no layer with them goes past all of them, or stays if it is first
            let furthest = reach.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let nearest = lo.iter().copied().fold(f64::INFINITY, f64::min);
            if furthest.is_finite() { furthest + gap - nearest } else { 0.0 }
        };
        // move it there, and add it to what the next part must clear
        for &(_, v) in group {
            center[v] += shift;
        }
        for l in 0..nlayers {
            reach[l] = reach[l].max(hi[l] + shift);
        }
    }
}

/// Boxes with no edges among them, packed in rows.
fn grid(ext: &[(f64, f64)], sp: &Spacing) -> LevelResult {
    let n = ext.len();
    let cols = ((n as f64 * 1.6).sqrt().ceil() as usize).clamp(1, n);
    let rows = n.div_ceil(cols);
    let mut colw = vec![0.0f64; cols];
    let mut rowh = vec![0.0f64; rows];
    for (i, &(w, h)) in ext.iter().enumerate() {
        colw[i % cols] = colw[i % cols].max(w);
        rowh[i / cols] = rowh[i / cols].max(h);
    }
    let mut colx = vec![0.0; cols];
    for c in 1..cols {
        colx[c] = colx[c - 1] + colw[c - 1] + sp.node_gap;
    }
    let mut rowy = vec![0.0; rows];
    for r in 1..rows {
        rowy[r] = rowy[r - 1] + rowh[r - 1] + sp.node_gap;
    }
    let pos = ext
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            let (c, r) = (i % cols, i / cols);
            Point { x: colx[c] + (colw[c] - w) / 2.0, y: rowy[r] + (rowh[r] - h) / 2.0 }
        })
        .collect();
    let size = (colx[cols - 1] + colw[cols - 1], rowy[rows - 1] + rowh[rows - 1]);
    LevelResult { pos, size, bundles: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isotonic_keeps_order_and_gaps() {
        let x = place_in_order(&[10.0, 0.0, 5.0], &[1.0, 1.0, 1.0], &[4.0, 4.0]);
        assert!(x[1] - x[0] >= 4.0 - 1e-9 && x[2] - x[1] >= 4.0 - 1e-9, "{x:?}");
        let x = place_in_order(&[0.0, 100.0], &[1.0, 1.0], &[4.0]);
        assert_eq!(x, vec![0.0, 100.0]);
    }
}
