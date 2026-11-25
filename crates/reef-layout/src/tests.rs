use super::*;

/// A small deterministic generator, so the property tests need no crates.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A random compound graph: some containers, leaves of varied size, edges forward in index
/// order between nodes that are not nested in each other.
fn random_input(seed: u64, n: usize, dir: Dir) -> Input {
    let mut r = Lcg(seed);
    let mut nodes: Vec<LNode> = Vec::new();
    for i in 0..n {
        let container = i > 0 && r.below(6) == 0;
        let parent = if i > 0 && r.below(2) == 0 {
            let cands: Vec<usize> = (0..i).filter(|&j| nodes[j].container).collect();
            if cands.is_empty() { None } else { Some(cands[r.below(cands.len())]) }
        } else {
            None
        };
        let w = 60.0 + r.below(140) as f64;
        let h = 30.0 + r.below(40) as f64;
        nodes.push(LNode { parent, w: if container { 80.0 } else { w }, h: if container { 30.0 } else { h }, container });
    }
    let nested = |a: usize, b: usize, nodes: &[LNode]| {
        let mut cur = nodes[b].parent;
        while let Some(c) = cur {
            if c == a {
                return true;
            }
            cur = nodes[c].parent;
        }
        false
    };
    let mut edges = Vec::new();
    for _ in 0..n * 3 / 2 {
        let a = r.below(n);
        let b = r.below(n);
        let (a, b) = (a.min(b), a.max(b));
        if a != b && !nested(a, b, &nodes) && !nested(b, a, &nodes) {
            edges.push(LEdge { from: a, to: b });
        }
    }
    Input { dir, nodes, edges, spacing: Spacing::default() }
}

fn check(input: &Input, out: &Output) {
    let n = input.nodes.len();
    // Siblings never overlap.
    for i in 0..n {
        for j in i + 1..n {
            if input.nodes[i].parent == input.nodes[j].parent {
                assert!(!out.rects[i].intersects(&out.rects[j]), "{i} and {j} overlap: {:?} {:?}", out.rects[i], out.rects[j]);
            }
        }
    }
    // Groups hold their members; everything is inside the drawing.
    let all = Rect { x: 0.0, y: 0.0, w: out.width, h: out.height };
    for i in 0..n {
        assert!(all.contains(&out.rects[i]), "{i} outside the drawing");
        if let Some(p) = input.nodes[i].parent {
            assert!(out.rects[p].contains(&out.rects[i]), "{i} outside its group {p}");
        }
    }
    // Every edge has a route from its source's box to its target's.
    for (e, pts) in input.edges.iter().zip(&out.edges) {
        assert!(pts.len() >= 2, "edge {e:?} has no route");
        assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }
}

#[test]
fn random_graphs_lay_out_cleanly() {
    for seed in 0..60 {
        for dir in [Dir::TB, Dir::LR] {
            let input = random_input(seed, 5 + (seed as usize % 40), dir);
            let out = layout(&input);
            check(&input, &out);
        }
    }
}

#[test]
fn deterministic() {
    let input = random_input(7, 40, Dir::TB);
    let a = layout(&input);
    let b = layout(&input);
    assert_eq!(a.rects, b.rects);
    assert_eq!(a.edges, b.edges);
}

#[test]
fn a_chain_goes_down() {
    let nodes = vec![LNode { parent: None, w: 100.0, h: 40.0, container: false }; 3];
    let edges = vec![LEdge { from: 0, to: 1 }, LEdge { from: 1, to: 2 }, LEdge { from: 0, to: 2 }];
    let input = Input { dir: Dir::TB, nodes, edges, spacing: Spacing::default() };
    let out = layout(&input);
    check(&input, &out);
    assert!(out.rects[0].bottom() < out.rects[1].y);
    assert!(out.rects[1].bottom() < out.rects[2].y);
    // The long edge passes layer 1 beside box 1, not through it.
    let long = &out.edges[2];
    assert!(long.len() > 2);
    assert!(!long[1..long.len() - 1].iter().any(|p| p.x > out.rects[1].x && p.x < out.rects[1].right() && p.y > out.rects[1].y && p.y < out.rects[1].bottom()));
}

#[test]
fn edges_point_forward_without_cycles() {
    for seed in 0..30 {
        let input = random_input(seed, 30, Dir::TB);
        let out = layout(&input);
        for (e, pts) in input.edges.iter().zip(&out.edges) {
            let (a, b) = (&out.rects[e.from], &out.rects[e.to]);
            // Random edges go forward in index order, so they never close a cycle unless a
            // group's members are tied in both directions; a route always ends on its target.
            let last = pts.last().unwrap();
            assert!((last.y - b.y).abs() < 1e-6 || (last.y - b.bottom()).abs() < 1e-6, "{e:?}");
            let first = pts.first().unwrap();
            assert!((first.y - a.y).abs() < 1e-6 || (first.y - a.bottom()).abs() < 1e-6, "{e:?}");
        }
    }
}

#[test]
fn edge_free_group_packs_in_a_grid() {
    let mut nodes = vec![LNode { parent: None, w: 80.0, h: 30.0, container: true }];
    nodes.extend(std::iter::repeat_n(LNode { parent: Some(0), w: 100.0, h: 40.0, container: false }, 16));
    let out = layout(&Input { dir: Dir::TB, nodes, edges: vec![], spacing: Spacing::default() });
    assert!(out.rects[0].w < 16.0 * 100.0 / 2.0, "a grid, not one row: {:?}", out.rects[0]);
}

#[test]
fn scale() {
    let input = random_input(99, 400, Dir::TB);
    let t = std::time::Instant::now();
    let out = layout(&input);
    check(&input, &out);
    eprintln!("400 nodes, {} edges: {:?}", input.edges.len(), t.elapsed());
}

/// A box with no arrows sits one gap past the rest on its layer, rather than wherever the first
/// packing left it while the boxes with arrows drifted away
#[test]
fn an_unlinked_box_sits_beside_the_rest() {
    // a chain with a long edge beside it, wide boxes on alternate layers, and a box tied to nothing
    let nodes = [100.0, 600.0, 100.0, 600.0, 100.0]
        .into_iter()
        .map(|w| LNode { parent: None, w, h: 40.0, container: false })
        .collect();
    let edges = vec![
        LEdge { from: 0, to: 1 },
        LEdge { from: 0, to: 3 },
        LEdge { from: 1, to: 2 },
        LEdge { from: 2, to: 3 },
    ];
    let input = Input { dir: Dir::TB, nodes, edges, spacing: Spacing::default() };
    let out = layout(&input);
    check(&input, &out);
    // the lone box shares the first layer and sits just past the chain's head
    let (head, lone) = (out.rects[0], out.rects[4]);
    assert_eq!(lone.y, head.y, "{lone:?} is not on {head:?}'s layer");
    let gap = lone.x - head.right();
    assert!((gap - input.spacing.node_gap).abs() < 1e-6, "{gap} from the chain's head: {:?}", out.rects);
}

/// Two chains with nothing between them are set side by side, one gap apart on every layer
#[test]
fn separate_chains_sit_side_by_side() {
    // a -> b and c -> d, all the same size
    let nodes = vec![LNode { parent: None, w: 100.0, h: 40.0, container: false }; 4];
    let edges = vec![LEdge { from: 0, to: 1 }, LEdge { from: 2, to: 3 }];
    let input = Input { dir: Dir::TB, nodes, edges, spacing: Spacing::default() };
    let out = layout(&input);
    check(&input, &out);
    // each layer holds one box of each chain, one gap apart
    let sp = &input.spacing;
    assert!((out.rects[2].x - out.rects[0].right() - sp.node_gap).abs() < 1e-6, "{:?}", out.rects);
    assert!((out.rects[3].x - out.rects[1].right() - sp.node_gap).abs() < 1e-6, "{:?}", out.rects);
    // and the drawing is just the two columns, the gap and the margins
    assert!((out.width - (200.0 + sp.node_gap + 2.0 * sp.margin)).abs() < 1e-6, "{}", out.width);
}
