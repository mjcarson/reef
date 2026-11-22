//! The canvas's diagram draws what reef-view draws
//!
//! For one plan and each kind of view (collapsed, expanded, a root, a focus with its faded
//! neighbours, arrows left to right, a selection), the Dioxus `Diagram` is rendered natively and
//! compared with reef-draw's markup for the same scene, which reef-view writes into the page.
//! Both are parsed, so the comparison is of elements, attributes and text, not of how each
//! escapes or closes its tags.

use dioxus::prelude::*;
use reef_app::canvas::{Diagram, Drawing, Shown, use_hits};
use reef_draw::{Filter, Scene, markup};
use reef_layout::Dir;
use reef_model::{Criterion, Direction, Expand, Graph, Kind, Met, Node, Status, ViewSpec};
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

/// The uid both drawings name their arrowhead marker with
const UID: &str = "test";

/// A node with an id, a title, a group and needs
///
/// # Arguments
///
/// * `id` - Its id
/// * `parent` - Its group, if any
/// * `needs` - What it needs
fn node(id: &str, parent: Option<&str>, needs: &[&str]) -> Node {
    Node {
        id: id.into(),
        title: format!("{id} does <its> \"work\" & more than fits on one line of a box"),
        parent: parent.map(Into::into),
        needs: needs.iter().map(|s| s.to_string()).collect(),
        ..Node::default()
    }
}

/// A criterion of a kind, met or not
///
/// # Arguments
///
/// * `kind` - Its kind
/// * `met` - Whether it is met
fn criterion(kind: Kind, met: bool) -> Criterion {
    Criterion {
        kind,
        text: "it holds".into(),
        status: Some(if met { Met::Met } else { Met::Unmet }),
        ..Criterion::default()
    }
}

/// A plan with most of what a diagram draws: a group in a group, needs and helps, several
/// arrows into one collapsed group, an optional node, done work, and criteria
fn plan() -> Graph {
    let mut a = node("a", Some("g1"), &[]);
    a.status = Some(Status::Done);
    a.acceptance = vec![criterion(Kind::Unit, true), criterion(Kind::Feature, false)];
    let mut b = node("b", Some("g1"), &["a"]);
    b.optional = true;
    b.helps = vec!["d".into()];
    let mut d = node("d", Some("g2"), &["a"]);
    d.acceptance = vec![criterion(Kind::Integration, true)];
    let nodes = vec![
        node("g1", None, &[]),
        a,
        b,
        node("g2", Some("g1"), &[]),
        d,
        node("x", None, &["a", "b", "d"]),
        node("y", None, &["g1"]),
        node("z", None, &["x"]),
    ];
    let (g, problems) = Graph::build(nodes);
    assert!(problems.is_empty(), "{problems:?}");
    g
}

/// A label's width, the same for both drawings
///
/// # Arguments
///
/// * `s` - The label
fn measure(s: &str) -> f64 {
    s.chars().count() as f64 * 7.0
}

/// What a view of the plan draws, laid out, with the selected node's item
///
/// # Arguments
///
/// * `g` - The plan
/// * `spec` - The view
/// * `selected` - The selected node's id, if any
fn drawing(g: &Rc<Graph>, spec: &ViewSpec, selected: Option<&str>) -> (Rc<Drawing>, Option<usize>) {
    let root = spec.root_id().and_then(|id| g.index_of(id));
    let dir = if spec.direction == Direction::LR {
        Dir::LR
    } else {
        Dir::TB
    };
    let open = spec.initial_expanded(g);
    let scene = Scene::build(
        g,
        root,
        spec.focus.is_some(),
        &open,
        &Filter::default(),
        dir,
        &measure,
    );
    let sel = selected
        .and_then(|id| g.index_of(id))
        .and_then(|n| scene.item_of(n));
    let d = Drawing {
        graph: g.clone(),
        scene,
        dir,
        root,
        ms: 0.0,
        draws: 1,
    };
    (Rc::new(d), sel)
}

/// Props for the harness: the drawing and the selection
#[derive(Props, Clone)]
struct HarnessProps {
    /// What to draw
    d: Rc<Drawing>,
    /// The selected item
    sel: Option<usize>,
}

impl PartialEq for HarnessProps {
    /// The same drawing and selection
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.d, &other.d) && self.sel == other.sel
    }
}

/// Draws the diagram as the canvas does, with nothing under the pointer
///
/// # Arguments
///
/// * `props` - The drawing and the selection
#[allow(non_snake_case)]
fn Harness(props: HarnessProps) -> Element {
    let d = props.d.clone();
    let shown = use_memo(move || Some(Shown(d.clone())));
    let at = props.sel;
    let sel = use_memo(move || at);
    let lit = use_memo(|| None::<HashSet<usize>>);
    let hits = use_hits();
    rsx! {
        Diagram { shown, sel, lit, uid: UID.to_string(), hits }
    }
}

/// The Dioxus diagram, rendered natively
///
/// # Arguments
///
/// * `d` - What to draw
/// * `sel` - The selected item
fn native(d: Rc<Drawing>, sel: Option<usize>) -> String {
    let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { d, sel });
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// An element as compared: its name, its attributes, and its children, text included
#[derive(Debug, PartialEq)]
enum Tree {
    /// An element
    El(String, BTreeMap<String, String>, Vec<Tree>),
    /// A run of text
    Text(String),
}

/// Parses SVG content into the trees compared
///
/// # Arguments
///
/// * `content` - The elements, as they go inside an `<svg>`
fn parse(content: &str) -> Vec<Tree> {
    let doc = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{content}</svg>"#);
    let parsed = roxmltree::Document::parse(&doc).unwrap_or_else(|e| panic!("{e}: {doc}"));
    parsed.root_element().children().filter_map(tree).collect()
}

/// One parsed node as compared, with neighbouring text merged
///
/// # Arguments
///
/// * `n` - The node
fn tree(n: roxmltree::Node) -> Option<Tree> {
    if n.is_text() {
        return n.text().map(|t| Tree::Text(t.to_string()));
    }
    if !n.is_element() {
        return None;
    }
    let attrs = n
        .attributes()
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    // dioxus writes each text node apart, so runs of text are joined before comparing
    let mut kids: Vec<Tree> = Vec::new();
    for kid in n.children().filter_map(tree) {
        match (kids.last_mut(), kid) {
            (Some(Tree::Text(a)), Tree::Text(b)) => a.push_str(&b),
            (_, kid) => kids.push(kid),
        }
    }
    kids.retain(|k| !matches!(k, Tree::Text(t) if t.is_empty()));
    Some(Tree::El(n.tag_name().name().to_string(), attrs, kids))
}

/// Each view draws in Dioxus as it does in reef-draw's markup
#[test]
fn every_view_draws_as_reef_view_does() {
    let g = Rc::new(plan());
    let views: Vec<(&str, ViewSpec, Option<&str>)> = vec![
        ("the whole plan, collapsed", ViewSpec::default(), None),
        (
            "every group open",
            ViewSpec {
                expand: Expand::Keyword("all".into()),
                ..ViewSpec::default()
            },
            None,
        ),
        (
            "one group's members",
            ViewSpec {
                root: Some("g1".into()),
                ..ViewSpec::default()
            },
            None,
        ),
        (
            "a focus with its neighbours",
            ViewSpec {
                focus: Some("g1".into()),
                ..ViewSpec::default()
            },
            None,
        ),
        (
            "left to right, open",
            ViewSpec {
                direction: Direction::LR,
                expand: Expand::Keyword("all".into()),
                ..ViewSpec::default()
            },
            None,
        ),
        (
            "a selected node",
            ViewSpec {
                expand: Expand::Keyword("all".into()),
                ..ViewSpec::default()
            },
            Some("b"),
        ),
        (
            "a selected collapsed group",
            ViewSpec::default(),
            Some("g1"),
        ),
    ];
    for (what, spec, selected) in views {
        let (d, sel) = drawing(&g, &spec, selected);
        let reference = markup::diagram(
            &g,
            &d.scene.vis,
            &d.scene.out,
            &d.scene.sizes,
            d.dir,
            UID,
            sel,
        );
        let drawn = native(d, sel);
        // the view drew something worth comparing
        assert!(reference.contains("reef-node"), "{what}: {reference}");
        let (ours, theirs) = (parse(&drawn), parse(&reference));
        // and the parse found its elements
        assert!(ours.len() > 5, "{what}: {drawn}");
        assert_eq!(
            ours, theirs,
            "{what}\n--- dioxus\n{drawn}\n--- markup\n{reference}"
        );
    }
}

/// The plan exercises what the comparison is for: pills, a merged arrow and faded neighbours
#[test]
fn the_plan_draws_pills_merged_arrows_and_ghosts() {
    let g = Rc::new(plan());
    let (collapsed, _) = drawing(&g, &ViewSpec::default(), None);
    let markup = |d: &Drawing| {
        markup::diagram(
            &g,
            &d.scene.vis,
            &d.scene.out,
            &d.scene.sizes,
            d.dir,
            UID,
            None,
        )
    };
    assert!(
        markup(&collapsed).contains(" arrows</title>"),
        "a merged arrow says how many"
    );
    let (open, _) = drawing(
        &g,
        &ViewSpec {
            expand: Expand::Keyword("all".into()),
            ..ViewSpec::default()
        },
        None,
    );
    assert!(markup(&open).contains("reef-acc-pill"), "pills");
    let (focus, _) = drawing(
        &g,
        &ViewSpec {
            focus: Some("g1".into()),
            ..ViewSpec::default()
        },
        None,
    );
    assert!(markup(&focus).contains(" ghost"), "ghosts");
}
