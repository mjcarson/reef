//! The SVG and HTML the viewer draws, as strings. Pure, so it is tested natively.

use reef_layout::{Dir, Output, svg_path};
use crate::accept::{self, Filter};
use reef_model::{EdgeKind, Graph, Kind, Status, Visible};
use std::fmt::Write;

pub const FONT_PX: f64 = 13.0;
pub const LINE_H: f64 = 17.0;
pub const PAD_X: f64 = 12.0;
pub const PAD_Y: f64 = 9.0;
pub const MAX_TEXT_W: f64 = 210.0;
pub const BAR_H: f64 = 20.0;
pub const HEADER_H: f64 = 30.0;
/// The row of acceptance pills under a box's title.
pub const PILL_H: f64 = 15.0;
const PILL_GAP: f64 = 4.0;
/// The pills' 10.5px text against the 13px it is measured in.
const PILL_FONT: f64 = 10.5 / FONT_PX;

/// What a box needs to show: its wrapped title and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSize {
    pub lines: Vec<String>,
    /// The acceptance pills: each kind shown, its text and its width.
    pub pills: Vec<(Kind, usize, usize, f64)>,
    pub w: f64,
    pub h: f64,
}

/// Wrap words greedily into lines no wider than `max`.
pub fn wrap(text: &str, max: f64, measure: &dyn Fn(&str) -> f64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
            continue;
        }
        let candidate = format!("{cur} {word}");
        if measure(&candidate) <= max {
            cur = candidate;
        } else {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The size of each visible item. An open group's size is its minimum: room for its title.
/// `filter` says which kinds of acceptance criteria the boxes' pills count.
pub fn sizes(g: &Graph, vis: &Visible, filter: &Filter, measure: &dyn Fn(&str) -> f64) -> Vec<BoxSize> {
    vis.items
        .iter()
        .map(|it| {
            let title = &g.node(it.node).title;
            if it.open {
                let w = measure(title) + measure("▾  ") + measure(" 000/000") + 2.0 * PAD_X + 16.0;
                return BoxSize { lines: vec![title.clone()], pills: Vec::new(), w, h: HEADER_H + 10.0 };
            }
            let lines = wrap(title, MAX_TEXT_W, measure);
            let text_w = lines.iter().map(|l| measure(l)).fold(0.0, f64::max);
            let mut w = (text_w + 2.0 * PAD_X).max(96.0);
            let mut h = lines.len() as f64 * LINE_H + 2.0 * PAD_Y;
            let pills: Vec<(Kind, usize, usize, f64)> = accept::pills(g, it.node, filter, it.collapsed)
                .into_iter()
                .map(|(k, m, t)| (k, m, t, (measure(&accept::pill_text(k, m, t)) * PILL_FONT + 12.0).ceil()))
                .collect();
            if !pills.is_empty() {
                let row: f64 = pills.iter().map(|p| p.3).sum::<f64>() + PILL_GAP * (pills.len() - 1) as f64;
                let indent = if it.collapsed { 16.0 } else { 0.0 };
                w = w.max(row + indent + 2.0 * PAD_X);
                h += PILL_H + 5.0;
            }
            if it.collapsed {
                w = w.max(150.0) + 18.0;
                h += BAR_H;
            }
            BoxSize { lines, pills, w: w.ceil(), h: h.ceil() }
        })
        .collect()
}

/// A link off this book, which a page cannot read across.
fn is_external(href: &str) -> bool {
    href.contains("://") || href.starts_with("mailto:") || href.starts_with("//")
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn status_class(g: &Graph, i: usize) -> &'static str {
    if g.is_ready(i) {
        return "s-ready";
    }
    match g.status(i) {
        Status::Todo => "s-todo",
        Status::InProgress => "s-progress",
        Status::Done => "s-done",
        Status::Blocked => "s-blocked",
    }
}

/// The diagram's contents: open groups' frames, then the arrows, then the boxes.
pub fn diagram(g: &Graph, vis: &Visible, out: &Output, sizes: &[BoxSize], dir: Dir, uid: &str, selected: Option<usize>) -> String {
    let mut s = String::with_capacity(vis.items.len() * 400);
    // Outer frames first so inner ones draw over them.
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = vis.items[i].parent {
            d += 1;
            i = p;
        }
        d
    };
    let mut frames: Vec<usize> = (0..vis.items.len()).filter(|&i| vis.items[i].open).collect();
    frames.sort_by_key(|&i| depth(i));
    for i in frames {
        let it = &vis.items[i];
        let r = out.rects[i];
        let n = g.node(it.node);
        let (done, total) = g.progress(it.node);
        let _ = write!(
            s,
            r#"<g class="reef-group {cls}{opt}" data-item="{i}"><rect class="reef-frame" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="8"/><g class="reef-head" data-toggle="{i}"><rect class="reef-head-hit" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{hh}"/><text class="reef-group-title" x="{tx:.1}" y="{ty:.1}"><tspan class="reef-chev">▾</tspan> {title}</text></g><text class="reef-count" x="{cx:.1}" y="{ty:.1}" text-anchor="end">{done}/{total}</text></g>"#,
            cls = status_class(g, it.node),
            opt = if n.optional { " optional" } else { "" },
            x = r.x,
            y = r.y,
            w = r.w,
            h = r.h,
            hh = HEADER_H,
            tx = r.x + PAD_X,
            ty = r.y + 20.0,
            cx = r.right() - PAD_X,
            title = esc(&n.title),
        );
    }
    for (k, e) in vis.edges.iter().enumerate() {
        let pts = &out.edges[k];
        if pts.len() < 2 {
            continue;
        }
        let hl = selected.is_some_and(|sel| e.from == sel || e.to == sel);
        let kind = if e.kind == EdgeKind::Helps { "helps" } else { "needs" };
        let width = if e.count > 1 { format!(r#" style="stroke-width:{:.1}px""#, (1.4 + (e.count as f64).ln()).min(4.0)) } else { String::new() };
        let _ = write!(
            s,
            r#"<path class="reef-edge {kind}{hl}" data-edge="{k}" data-from="{f}" data-to="{t}" d="{d}" marker-end="url(#reef-arrow-{uid})"{width}>"#,
            hl = if hl { " hl" } else { "" },
            f = e.from,
            t = e.to,
            d = svg_path(pts, dir),
        );
        if e.count > 1 {
            let _ = write!(s, "<title>{} arrows</title>", e.count);
        }
        s.push_str("</path>");
    }
    for (i, it) in vis.items.iter().enumerate() {
        if it.open {
            continue;
        }
        let r = out.rects[i];
        let n = g.node(it.node);
        let sz = &sizes[i];
        let mut cls = format!("reef-node {}", status_class(g, it.node));
        if n.optional {
            cls.push_str(" optional");
        }
        if it.collapsed {
            cls.push_str(" collapsed");
        }
        if it.ghost {
            cls.push_str(" ghost");
        }
        if selected == Some(i) {
            cls.push_str(" sel");
        }
        let _ = write!(
            s,
            r#"<g class="{cls}" data-item="{i}" transform="translate({x:.1},{y:.1})"><rect class="reef-box" width="{w:.1}" height="{h:.1}" rx="6"/>"#,
            x = r.x,
            y = r.y,
            w = r.w,
            h = r.h,
        );
        if n.status == Some(Status::Done) || (it.collapsed && g.status(it.node) == Status::Done) {
            s.push_str(r#"<text class="reef-tick" x="-6" y="6">✓</text>"#);
        }
        let text_x = if it.collapsed { PAD_X + 16.0 } else { PAD_X };
        s.push_str(r#"<text class="reef-label">"#);
        for (l, line) in sz.lines.iter().enumerate() {
            let _ = write!(s, r#"<tspan x="{text_x:.1}" y="{:.1}">{}</tspan>"#, PAD_Y + 12.5 + l as f64 * LINE_H, esc(line));
        }
        s.push_str("</text>");
        if !sz.pills.is_empty() {
            let mut px = text_x;
            let py = PAD_Y + sz.lines.len() as f64 * LINE_H + 3.0;
            for &(k, met, total, pw) in &sz.pills {
                let _ = write!(
                    s,
                    r#"<g class="reef-acc-pill k-{kind}{full}"><title>{title}</title><rect x="{px:.1}" y="{py:.1}" width="{pw:.1}" height="{PILL_H}" rx="7.5"/><text x="{tx:.1}" y="{ty:.1}" text-anchor="middle">{text}</text></g>"#,
                    kind = k.as_str(),
                    full = if met == total { " full" } else { "" },
                    title = esc(&accept::pill_title(k, met, total)),
                    tx = px + pw / 2.0,
                    ty = py + 11.0,
                    text = esc(&accept::pill_text(k, met, total)),
                );
                px += pw + PILL_GAP;
            }
        }
        if it.collapsed {
            let (done, total) = g.progress(it.node);
            let bar_y = r.h - PAD_Y - 6.0;
            let bar_w = r.w - 2.0 * PAD_X - 52.0;
            let frac = if total == 0 { 0.0 } else { done as f64 / total as f64 };
            let _ = write!(
                s,
                r#"<g class="reef-expand" data-toggle="{i}"><rect class="reef-expand-hit" x="2" y="2" width="24" height="24"/><text class="reef-chev" x="{PAD_X}" y="{cy:.1}">▸</text></g><rect class="reef-bar-bg" x="{PAD_X}" y="{bar_y:.1}" width="{bar_w:.1}" height="5" rx="2.5"/><rect class="reef-bar" x="{PAD_X}" y="{bar_y:.1}" width="{fw:.1}" height="5" rx="2.5"/><text class="reef-count" x="{cx:.1}" y="{ty:.1}" text-anchor="end">{done}/{total}</text>"#,
                cy = PAD_Y + 12.5,
                fw = bar_w * frac,
                cx = r.w - PAD_X,
                ty = bar_y + 6.0,
            );
        }
        s.push_str("</g>");
    }
    s
}

/// One line in a sidebar list: a node, its status, and a link that selects it.
fn entry(g: &Graph, j: usize, note: &str) -> String {
    format!(
        r#"<li><a href="javascript:void 0" data-goto="{id}"><span class="reef-dot {cls}"></span>{title}</a>{note}</li>"#,
        id = esc(&g.node(j).id),
        cls = status_class(g, j),
        title = esc(&g.node(j).title),
        note = if note.is_empty() { String::new() } else { format!(r#" <span class="reef-note">{}</span>"#, esc(note)) },
    )
}

fn status_label(g: &Graph, i: usize) -> &'static str {
    if g.is_ready(i) {
        return "ready to start";
    }
    match g.status(i) {
        Status::Todo => "to do",
        Status::InProgress => "in progress",
        Status::Done => "done",
        Status::Blocked => "blocked",
    }
}

/// A sidebar section, open, whose header folds it and sticks to the sidebar's top while the
/// section is scrolled through. `head` is markup; `class` is added to the section's own.
pub fn sec(head: &str, class: &str, body: &str) -> String {
    let class = if class.is_empty() { String::new() } else { format!(" {class}") };
    format!(r#"<details class="reef-sec{class}" open><summary class="reef-sec-head"><h3>{head}</h3></summary><div class="reef-sec-body">{body}</div></details>"#)
}

/// A note saying which doc heading a brief part was taken from, or nothing when it was written.
///
/// # Arguments
///
/// * `from` - The doc heading or table row the part was taken from, if it was
fn from_note(from: Option<&str>) -> String {
    from.map_or(String::new(), |f| format!(r#" <span class="reef-note">from <em>{}</em></span>"#, esc(f)))
}

/// The sidebar's Description section: why the work is needed and what finishing it will do,
/// each labelled, then the longer explanation. Empty when the node has none of them.
///
/// # Arguments
///
/// * `n` - The node to describe
fn description(n: &reef_model::Node) -> String {
    // the labelled parts that have text, each with where it was taken from
    let parts = [("Why", &n.why, n.why_from.as_deref()), ("What", &n.what, n.what_from.as_deref())];
    let mut body = String::new();
    let mut list = String::new();
    for (label, html, from) in parts {
        if !html.trim().is_empty() {
            let _ = write!(list, "<dt>{label}{}</dt><dd>{html}</dd>", from_note(from));
        }
    }
    if !list.is_empty() {
        let _ = write!(body, r#"<dl class="reef-ww">{list}</dl>"#);
    }
    // the longer explanation goes below them
    if !n.details.trim().is_empty() {
        let _ = write!(body, r#"<div class="reef-desc">{}</div>"#, n.details);
    }
    if body.is_empty() {
        return String::new();
    }
    sec("Description", "", &body)
}

/// The sidebar for node `i`. `open` says whether it is a group shown open; `filter`, which
/// acceptance criteria show.
pub fn sidebar(g: &Graph, i: usize, open: Option<bool>, filter: &Filter) -> String {
    let n = g.node(i);
    let mut s = String::new();
    s.push_str(r#"<button class="reef-close" data-act="close" title="Close (Esc)">×</button>"#);
    let _ = write!(s, r#"<div class="reef-badges"><span class="reef-badge {}">{}</span>"#, status_class(g, i), status_label(g, i));
    if n.optional {
        s.push_str(r#"<span class="reef-badge optional">optional</span>"#);
    }
    let _ = write!(s, r#"<code class="reef-id">{}</code></div>"#, esc(&n.id));
    let _ = write!(s, "<h2>{}</h2>", esc(&n.title));
    let ancestors = g.ancestors(i);
    if !ancestors.is_empty() {
        s.push_str(r#"<div class="reef-crumbs">"#);
        for (k, &a) in ancestors.iter().rev().enumerate() {
            if k > 0 {
                s.push_str(" › ");
            }
            let _ = write!(s, r#"<a href="javascript:void 0" data-goto="{}">{}</a>"#, esc(&g.node(a).id), esc(&g.node(a).title));
        }
        s.push_str("</div>");
    }
    s.push_str(r#"<div class="reef-actions">"#);
    if let Some(page) = &n.page {
        let _ = write!(s, r#"<a class="reef-button" href="{}">Open page</a>"#, esc(page));
    }
    if let Some(open) = open {
        let _ = write!(s, r#"<button class="reef-button" data-act="toggle-sel">{}</button>"#, if open { "Collapse" } else { "Expand" });
    }
    s.push_str(r#"<button class="reef-button" data-act="center">Show in diagram</button></div>"#);
    s.push_str(&description(n));
    s.push_str(&accept::section(g, i, filter));
    s.push_str(&crate::perf::section(g, i));
    if !n.docs.is_empty() {
        let mut body = String::new();
        for d in &n.docs {
            let href = esc(&d.href);
            let label = esc(&d.label);
            let head = format!(
                r#"<a href="{href}">{label}</a><a class="reef-button reef-open" href="{href}" title="Open this doc's page">Open page ↗</a>"#
            );
            if is_external(&d.href) {
                let _ = write!(body, r#"<div class="reef-doc reef-doc-ext"><div class="reef-doc-head">{head}</div></div>"#);
            } else {
                // The header is the toggle. It starts open: the viewer fetches its text as it shows the sidebar.
                let _ = write!(
                    body,
                    r#"<details class="reef-doc reef-doc-read" data-href="{href}" open><summary class="reef-doc-head" title="Read here">{head}</summary><div class="reef-doc-text"><p class="reef-note">Loading…</p></div></details>"#
                );
            }
        }
        s.push_str(&sec("Documentation", "", &body));
    }
    if !n.design.trim().is_empty() {
        let from = from_note(n.design_from.as_deref());
        s.push_str(&sec(&format!("Design choices{from}"), "", &format!(r#"<div class="reef-design">{}</div>"#, n.design)));
    }
    if g.is_group(i) {
        let (done, total) = g.progress(i);
        let members: String = g.children(i).iter().map(|&c| entry(g, c, if g.is_group(c) { "group" } else { "" })).collect();
        let head = format!(r#"Members <span class="reef-note">{done} of {total} done</span>"#);
        s.push_str(&sec(&head, "", &format!("<ul>{members}</ul>")));
    }
    let needs: Vec<String> = g.needs(i).iter().map(|&j| entry(g, j, "")).chain(g.helps(i).iter().map(|&j| entry(g, j, "helps"))).collect();
    if !needs.is_empty() {
        s.push_str(&sec("Needs", "", &format!("<ul>{}</ul>", needs.concat())));
    }
    let inherited: Vec<String> = ancestors
        .iter()
        .flat_map(|&a| g.needs(a).iter().map(move |&j| (a, j)))
        .map(|(a, j)| entry(g, j, &format!("via {}", g.node(a).id)))
        .collect();
    if !inherited.is_empty() {
        s.push_str(&sec("Its groups need", "", &format!("<ul>{}</ul>", inherited.concat())));
    }
    let deps: Vec<String> = g
        .dependents(i)
        .into_iter()
        .map(|(j, k)| entry(g, j, if k == EdgeKind::Helps { "helped" } else { "" }))
        .collect();
    if !deps.is_empty() {
        s.push_str(&sec("Needed by", "", &format!("<ul>{}</ul>", deps.concat())));
    }
    if !n.html.trim().is_empty() {
        let _ = write!(s, r#"<div class="reef-body">{}</div>"#, n.html);
    }
    let _ = write!(s, r#"<div class="reef-source">{}</div>"#, esc(&n.origin.to_string()));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_model::Node;
    use std::collections::HashSet;

    fn measure(s: &str) -> f64 {
        s.chars().count() as f64 * 7.0
    }

    #[test]
    fn wraps() {
        let lines = wrap("one two three four five six", 70.0, &measure);
        assert_eq!(lines, vec!["one two", "three four", "five six"]);
        assert_eq!(wrap("", 70.0, &measure), vec![""]);
    }

    #[test]
    fn draws() {
        let mk = |id: &str, parent: Option<&str>, needs: &[&str]| Node {
            id: id.into(),
            title: format!("<{id}>"),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            ..Node::default()
        };
        let (g, _) = Graph::build(vec![mk("a", None, &[]), mk("g", None, &["a"]), mk("x", Some("g"), &[])]);
        let vis = g.visible(None, false, &HashSet::new());
        let sz = sizes(&g, &vis, &Filter::default(), &measure);
        let input = reef_layout::Input {
            dir: Dir::TB,
            nodes: vis.items.iter().zip(&sz).map(|(it, s)| reef_layout::LNode { parent: it.parent, w: s.w, h: s.h, container: it.open }).collect(),
            edges: vis.edges.iter().map(|e| reef_layout::LEdge { from: e.from, to: e.to }).collect(),
            spacing: Default::default(),
        };
        let out = reef_layout::layout(&input);
        let svg = diagram(&g, &vis, &out, &sz, Dir::TB, "u", None);
        assert!(svg.contains("&lt;a&gt;"), "escaped");
        assert!(svg.contains("collapsed"));
        assert!(svg.contains("reef-edge needs"));
        let sb = sidebar(&g, g.index_of("g").unwrap(), Some(false), &Filter::default());
        assert!(sb.contains("Members"));
        assert!(sb.contains("data-goto=\"x\""));
    }

    #[test]
    fn sidebar_says_why_and_what() {
        // a node whose why was taken from a doc and whose what and details were written
        let a = Node {
            id: "a".into(),
            title: "a".into(),
            why: "<p>Slow.</p>".into(),
            why_from: Some("Context".into()),
            what: "<p>Fast.</p>".into(),
            details: "<p>More.</p>".into(),
            ..Node::default()
        };
        let b = Node { id: "b".into(), title: "b".into(), ..Node::default() };
        let (g, _) = Graph::build(vec![a, b]);
        let sb = sidebar(&g, g.index_of("a").unwrap(), None, &Filter::default());
        // each part is labelled, in order, and only the taken one says where it came from
        let ww = r#"<dl class="reef-ww"><dt>Why <span class="reef-note">from <em>Context</em></span></dt><dd><p>Slow.</p></dd><dt>What</dt><dd><p>Fast.</p></dd></dl>"#;
        assert!(sb.contains(ww), "{sb}");
        // the longer explanation follows them
        assert!(sb.contains(&format!(r#"{ww}<div class="reef-desc"><p>More.</p></div>"#)), "{sb}");
        // a node with none of them has no Description section
        assert!(!sidebar(&g, g.index_of("b").unwrap(), None, &Filter::default()).contains("Description"));
    }

    #[test]
    fn boxes_show_acceptance_pills() {
        use reef_model::Criterion;
        let crit = |kind| Criterion { kind, text: "t".into(), ..Criterion::default() };
        let a = Node { id: "a".into(), title: "a".into(), acceptance: vec![crit(Kind::Unit), crit(Kind::Unit), crit(Kind::Feature)], ..Node::default() };
        let b = Node { id: "b".into(), title: "b".into(), ..Node::default() };
        let (g, _) = Graph::build(vec![a, b]);
        let vis = g.visible(None, false, &HashSet::new());
        let all = sizes(&g, &vis, &Filter::default(), &measure);
        let (ia, ib) = (vis.shown_as[0].unwrap(), vis.shown_as[1].unwrap());
        assert_eq!(all[ia].pills.iter().map(|p| (p.0, p.2)).collect::<Vec<_>>(), vec![(Kind::Unit, 2), (Kind::Feature, 1)]);
        assert!(all[ia].h > all[ib].h, "the pills take a row");
        let some = sizes(&g, &vis, &Filter::default().toggled(Kind::Unit).toggled(Kind::Feature), &measure);
        assert!(some[ia].pills.is_empty());
        assert_eq!(some[ia].h, some[ib].h, "a box with nothing shown has no pill row");
        let input = reef_layout::Input {
            dir: Dir::TB,
            nodes: vis.items.iter().zip(&all).map(|(it, s)| reef_layout::LNode { parent: it.parent, w: s.w, h: s.h, container: it.open }).collect(),
            edges: vec![],
            spacing: Default::default(),
        };
        let out = reef_layout::layout(&input);
        let svg = diagram(&g, &vis, &out, &all, Dir::TB, "u", None);
        assert!(svg.contains(r#"class="reef-acc-pill k-unit""#), "{svg}");
        assert!(svg.contains("<title>unit tests: 0 of 2 met</title>"), "{svg}");
        assert!(svg.contains(">U 0/2<"), "{svg}");
    }
}
