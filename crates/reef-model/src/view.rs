//! A view of the plan: the part of the group tree a fence shows, with collapsed groups standing
//! in for their members.

use crate::graph::{EdgeKind, Graph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Direction {
    /// Top to bottom.
    #[default]
    #[serde(alias = "tb", alias = "TD", alias = "td")]
    TB,
    /// Left to right.
    #[serde(alias = "lr")]
    LR,
}

/// Which groups start open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expand {
    /// `"all"` or `"none"`.
    Keyword(String),
    List(Vec<String>),
}

impl Default for Expand {
    fn default() -> Self {
        Expand::List(Vec::new())
    }
}

/// Where a view is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// In the page's text column, like an image.
    #[default]
    Inline,
    /// Filling the page to the right of the book's sidebar.
    Page,
}

impl Layout {
    fn is_inline(&self) -> bool {
        *self == Layout::Inline
    }
}

/// What a ```` ```reef ```` fence asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSpec {
    /// Show this group's members. Default: the whole plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Show this group's members with their outside neighbours faded beside them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub expand: Expand,
    /// CSS height of the diagram. Default `"70vh"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    /// `"page"` fills the page beside the book's sidebar. Default `"inline"`.
    #[serde(default, skip_serializing_if = "Layout::is_inline")]
    pub layout: Layout,
    /// A chapter (relative to this file) with a wider view of the same plan, linked from the
    /// toolbar. The preprocessor rewrites it to that chapter's `.html`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

impl ViewSpec {
    /// The group whose members are shown, if any.
    pub fn root_id(&self) -> Option<&str> {
        self.focus.as_deref().or(self.root.as_deref())
    }

    /// Check the view's ids against the graph.
    pub fn check(&self, g: &Graph) -> Vec<String> {
        let mut out = Vec::new();
        for (key, id) in [("root", &self.root), ("focus", &self.focus)] {
            if let Some(id) = id {
                match g.index_of(id) {
                    None => out.push(format!("{key} `{id}` is not a node")),
                    Some(i) if !g.is_group(i) => out.push(format!("{key} `{id}` has no members")),
                    _ => {}
                }
            }
        }
        if self.root.is_some() && self.focus.is_some() {
            out.push("a view names a root or a focus, not both".into());
        }
        match &self.expand {
            Expand::Keyword(k) if k != "all" && k != "none" => {
                out.push(format!("expand = \"{k}\": expected \"all\", \"none\" or a list of ids"));
            }
            Expand::List(ids) => {
                for id in ids {
                    if g.index_of(id).is_none() {
                        out.push(format!("expand names `{id}`, which is not a node"));
                    }
                }
            }
            _ => {}
        }
        if self.layout == Layout::Page && self.height.is_some() {
            out.push("height does not apply to layout = \"page\", which fills the page".into());
        }
        out
    }

    /// The groups open when the view first loads.
    pub fn initial_expanded(&self, g: &Graph) -> HashSet<usize> {
        let mut out: HashSet<usize> = (0..g.len()).filter(|&i| g.is_group(i) && g.node(i).collapsed == Some(false)).collect();
        match &self.expand {
            Expand::Keyword(k) if k == "all" => out.extend((0..g.len()).filter(|&i| g.is_group(i))),
            Expand::Keyword(_) => out.clear(),
            Expand::List(ids) => out.extend(ids.iter().filter_map(|id| g.index_of(id))),
        }
        out
    }
}

/// One box on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisItem {
    /// Index of the graph node.
    pub node: usize,
    /// Index into [`Visible::items`] of the open group this box is drawn in.
    pub parent: Option<usize>,
    /// An open group, drawn as a frame around its members.
    pub open: bool,
    /// A collapsed group, drawn as one box standing in for its members.
    pub collapsed: bool,
    /// A neighbour from outside a focused view.
    pub ghost: bool,
}

/// One arrow on screen, standing for `count` arrows of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisEdge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    pub count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Visible {
    pub items: Vec<VisItem>,
    pub edges: Vec<VisEdge>,
    /// The box each graph node is shown as, if any: itself, or the collapsed group hiding it.
    pub shown_as: Vec<Option<usize>>,
}

impl Graph {
    /// What a view shows with the given groups open.
    pub fn visible(&self, root: Option<usize>, ghosts: bool, expanded: &HashSet<usize>) -> Visible {
        let mut vis = Visible { shown_as: vec![None; self.len()], ..Visible::default() };
        // A focused group is drawn as an open frame, so arrows to the group itself have a place
        // to land; a root's members are drawn on their own.
        let tops: Vec<usize> = match root {
            Some(r) if ghosts => vec![r],
            Some(r) => self.children(r).to_vec(),
            None => self.roots().to_vec(),
        };
        let focus = root.filter(|_| ghosts);
        let mut stack: Vec<(usize, Option<usize>)> = tops.into_iter().rev().map(|i| (i, None)).collect();
        while let Some((i, parent)) = stack.pop() {
            let group = self.is_group(i);
            let open = group && (expanded.contains(&i) || Some(i) == focus);
            let item = vis.items.len();
            vis.items.push(VisItem { node: i, parent, open, collapsed: group && !open, ghost: false });
            vis.shown_as[i] = Some(item);
            if open {
                for &c in self.children(i).iter().rev() {
                    stack.push((c, Some(item)));
                }
            } else if group {
                for d in self.descendants(i) {
                    vis.shown_as[d] = Some(item);
                }
            }
        }
        let mut edges: BTreeMap<(usize, usize, EdgeKind), usize> = BTreeMap::new();
        for to in 0..self.len() {
            for (kind, deps) in [(EdgeKind::Needs, self.needs(to)), (EdgeKind::Helps, self.helps(to))] {
                for &from in deps {
                    let (f, t) = match (vis.shown_as[from], vis.shown_as[to]) {
                        // Only direct neighbours of what the view shows, not neighbours of ghosts.
                        (None, Some(t)) if ghosts && !vis.items[t].ghost => (ghost(self, &mut vis, from, root), t),
                        (Some(f), None) if ghosts && !vis.items[f].ghost => (f, ghost(self, &mut vis, to, root)),
                        (Some(f), Some(t)) if vis.items[f].ghost && vis.items[t].ghost => continue,
                        (Some(f), Some(t)) => (f, t),
                        _ => continue,
                    };
                    if f == t || vis.is_inside(f, t) || vis.is_inside(t, f) {
                        continue;
                    }
                    *edges.entry((f, t, kind)).or_default() += 1;
                }
            }
        }
        // A solid arrow already says what a dashed one beside it would.
        let solid: HashSet<(usize, usize)> =
            edges.keys().filter(|k| k.2 == EdgeKind::Needs).map(|k| (k.0, k.1)).collect();
        vis.edges = edges
            .into_iter()
            .filter(|((f, t, k), _)| *k == EdgeKind::Needs || !solid.contains(&(*f, *t)))
            .map(|((from, to, kind), count)| VisEdge { from, to, kind, count })
            .collect();
        vis
    }
}

/// Show a node from outside the view as a faded box: its outermost group that does not also
/// hold the focus, so a neighbour deep inside another feature reads as that feature, and a
/// neighbour beside the focus reads as itself.
fn ghost(g: &Graph, vis: &mut Visible, i: usize, focus: Option<usize>) -> usize {
    let holds_focus = |a: usize| focus.is_some_and(|f| a == f || g.is_ancestor(a, f));
    let shown = g.ancestors(i).into_iter().rev().find(|&a| !holds_focus(a)).unwrap_or(i);
    if let Some(item) = vis.shown_as[shown] {
        vis.shown_as[i] = Some(item);
        return item;
    }
    let item = vis.items.len();
    vis.items.push(VisItem { node: shown, parent: None, open: false, collapsed: g.is_group(shown), ghost: true });
    vis.shown_as[shown] = Some(item);
    vis.shown_as[i] = Some(item);
    item
}

impl Visible {
    /// Whether item `inner` is drawn inside open group `outer`.
    pub fn is_inside(&self, inner: usize, outer: usize) -> bool {
        let mut cur = self.items[inner].parent;
        while let Some(c) = cur {
            if c == outer {
                return true;
            }
            cur = self.items[c].parent;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Node;

    fn node(id: &str, parent: Option<&str>, needs: &[&str]) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            origin: crate::Origin::File { source: "t.md".into(), line: 0 },
            ..Node::default()
        }
    }

    fn graph() -> Graph {
        let (g, d) = Graph::build(vec![
            node("a", None, &[]),
            node("g", None, &["a"]),
            node("x", Some("g"), &[]),
            node("y", Some("g"), &["x"]),
            node("b", None, &["x", "y"]),
        ]);
        assert!(d.is_empty());
        g
    }

    fn named(g: &Graph, v: &Visible) -> Vec<(String, String, usize)> {
        v.edges
            .iter()
            .map(|e| (g.node(v.items[e.from].node).id.clone(), g.node(v.items[e.to].node).id.clone(), e.count))
            .collect()
    }

    #[test]
    fn page_layout() {
        let g = graph();
        let spec: ViewSpec = serde_json::from_str(r#"{"layout":"page","page":"x.md"}"#).unwrap();
        assert_eq!(spec.layout, Layout::Page);
        assert!(spec.check(&g).is_empty());
        let back: ViewSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);
        assert!(!serde_json::to_string(&ViewSpec::default()).unwrap().contains("layout"));
        let tall = ViewSpec { height: Some("50vh".into()), ..spec };
        assert_eq!(tall.check(&g).len(), 1);
    }

    #[test]
    fn collapsed_group_stands_in() {
        let g = graph();
        let v = g.visible(None, false, &HashSet::new());
        assert_eq!(v.items.len(), 3);
        assert_eq!(named(&g, &v), vec![("a".into(), "g".into(), 1), ("g".into(), "b".into(), 2)]);
    }

    #[test]
    fn open_group_shows_members() {
        let g = graph();
        let open = HashSet::from([g.index_of("g").unwrap()]);
        let v = g.visible(None, false, &open);
        assert_eq!(v.items.len(), 5);
        let e = named(&g, &v);
        assert!(e.contains(&("x".into(), "y".into(), 1)));
        assert!(e.contains(&("x".into(), "b".into(), 1)));
    }

    #[test]
    fn focus_adds_ghosts() {
        let g = graph();
        let gi = g.index_of("g").unwrap();
        let v = g.visible(Some(gi), true, &HashSet::new());
        let ghosts: Vec<&str> = v.items.iter().filter(|i| i.ghost).map(|i| g.node(i.node).id.as_str()).collect();
        assert_eq!(ghosts, vec!["a", "b"]);
        assert!(v.items[0].open, "the focused group is an open frame");
        let v = g.visible(Some(gi), false, &HashSet::new());
        assert_eq!(v.items.len(), 2);
        assert_eq!(v.edges.len(), 1);
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::graph::Node;

    #[test]
    fn ghost_is_the_outermost_group_beside_the_focus() {
        let n = |id: &str, parent: Option<&str>, needs: &[&str]| Node {
            id: id.into(),
            title: id.into(),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            ..Node::default()
        };
        let (g, d) = Graph::build(vec![
            n("plan", None, &[]),
            n("schema", Some("plan"), &[]),
            n("search", Some("plan"), &["schema"]),
            n("x", Some("search"), &[]),
            n("sync", Some("plan"), &[]),
            n("merge", Some("sync"), &["x"]),
        ]);
        assert!(d.is_empty(), "{d:?}");
        let v = g.visible(g.index_of("search"), true, &HashSet::new());
        let mut ghosts: Vec<&str> = v.items.iter().filter(|i| i.ghost).map(|i| g.node(i.node).id.as_str()).collect();
        ghosts.sort();
        assert_eq!(ghosts, vec!["schema", "sync"]);
    }
}
