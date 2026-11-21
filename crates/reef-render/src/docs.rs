//! Resolving a plan's doc links and filling in each node's brief form from them, over any
//! source of docs, and rendering the plan for the viewer.

use crate::extract::{self, has_text};
use crate::markdown::{heading_spans, inline_html, render, render_body, ROOT};
use crate::paths::{self, Target};
use crate::source::{doc_body, DocSource, Headings};
use reef_model::{Diagnostic, Graph, Link, Node, Payload, PerfDoc};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// The doc sections a node's brief form is taken from, by their titles
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Sections {
    /// The subsections of a doc whose first paragraph or list says why a node's work is needed,
    /// or the first cells of a table row (`| **Impact** | ... |`) whose second cell does.
    pub why_sections: Vec<String>,
    /// The subsections of a doc whose first paragraph or list says what finishing a node's
    /// work will do, or the first cells of a table row whose second cell does.
    #[serde(alias = "description-sections")]
    pub what_sections: Vec<String>,
    /// The subsections of a doc that hold a node's design choices.
    pub design_sections: Vec<String>,
    /// The subsections of a doc that hold the work's measurements. A linked heading without one
    /// shows its tables instead.
    pub performance_sections: Vec<String>,
}

impl Default for Sections {
    /// The section titles reef looks for unless a book names its own
    fn default() -> Self {
        // turn each list of titles into owned strings
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Sections {
            why_sections: v(&["Why", "Context", "Motivation", "Problem", "Symptom", "Question", "The question", "Impact"]),
            what_sections: v(&["What it does", "What", "Summary", "Overview", "The fix"]),
            design_sections: v(&["Design choices", "Design", "Decisions", "The fix"]),
            performance_sections: v(&["Performance", "Results", "Benchmark", "Benchmarks", "Measurements"]),
        }
    }
}

/// Renders a plan's nodes and resolves their doc links against one source of docs
///
/// Each doc is read and its headings found at most once, so a renderer is made for one build
/// of a plan and dropped after it.
pub struct Renderer<S: DocSource> {
    /// Where the docs the nodes link to are read from
    source: S,
    /// The doc sections a node's brief form is taken from
    sections: Sections,
    /// Each doc's body without its frontmatter, by path, or `None` where there is no such doc
    texts: RefCell<HashMap<String, Option<Arc<str>>>>,
    /// Each doc's headings, by path, or `None` where there is no such doc
    headings: RefCell<HashMap<String, Option<Headings>>>,
}

impl<S: DocSource> Renderer<S> {
    /// Render nodes against a source of docs
    ///
    /// # Arguments
    ///
    /// * `source` - Where the docs the nodes link to are read from
    /// * `sections` - The doc sections a node's brief form is taken from
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_render::{MemSource, Renderer, Sections};
    ///
    /// let docs = MemSource::new().with("f.md", "# F\n\n## Design\n\nOne way.\n");
    /// let renderer = Renderer::new(docs, Sections::default());
    /// let link = renderer.doc_link("plan/a.md", "../f.md#design").unwrap();
    /// assert_eq!(link.href, "reef-root:/f.html#design");
    /// ```
    pub fn new(source: S, sections: Sections) -> Self {
        Renderer { source, sections, texts: RefCell::default(), headings: RefCell::default() }
    }

    /// Where the docs are read from
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The doc sections a node's brief form is taken from
    pub fn sections(&self) -> &Sections {
        &self.sections
    }

    /// Headings of a doc, as `(id, text, level)`, or `None` if there is no such doc.
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn headings(&self, path: &str) -> Option<Headings> {
        // a doc's headings are found once
        if let Some(h) = self.headings.borrow().get(path) {
            return h.clone();
        }
        let h = self.source.headings(path);
        self.headings.borrow_mut().insert(path.to_string(), h.clone());
        h
    }

    /// A doc's body without its frontmatter, or `None` if there is no such doc
    ///
    /// # Arguments
    ///
    /// * `path` - The doc's path from the source's root
    fn text(&self, path: &str) -> Option<Arc<str>> {
        // a doc is read once
        if let Some(t) = self.texts.borrow().get(path) {
            return t.clone();
        }
        let t = self.source.read(path).map(|text| Arc::from(doc_body(&text)));
        self.texts.borrow_mut().insert(path.to_string(), t.clone());
        t
    }

    /// Resolve a doc link written in `from`, checking that its source can hold it and that its
    /// file and anchor exist.
    ///
    /// # Arguments
    ///
    /// * `from` - The path the link is written relative to
    /// * `link` - The link as written
    pub fn doc_link(&self, from: &str, link: &str) -> Result<Link, String> {
        match paths::resolve(from, link) {
            Target::External(url) => Ok(Link { label: url.clone(), href: url }),
            Target::Internal { path, anchor } => {
                // the source names where it looks, or refuses a path it can't hold, whatever it names
                let place = self.source.place(&path).map_err(|why| format!("doc `{link}` {why}"))?;
                if !path.ends_with(".md") {
                    return Ok(Link { label: path.clone(), href: format!("{ROOT}{path}") });
                }
                let Some(heads) = self.headings(&path) else {
                    return Err(format!("doc `{link}` not found (looked for {place})"));
                };
                let label = match &anchor {
                    Some(a) => match heads.iter().find(|h| &h.0 == a) {
                        Some(h) => h.1.clone(),
                        None => return Err(format!("doc `{link}`: {place} has no heading with the id `{a}`")),
                    },
                    None => heads.first().map_or_else(|| path.clone(), |h| h.1.clone()),
                };
                let href = match anchor {
                    Some(a) => format!("{ROOT}{}#{a}", paths::html_of(&path)),
                    None => format!("{ROOT}{}", paths::html_of(&path)),
                };
                Ok(Link { label, href })
            }
        }
    }

    /// Check a built plan's doc links and fill in every node's brief form, returning the
    /// problems found: each doc link that doesn't resolve, then each piece of work that does
    /// not say why it is needed or what finishing it will do
    ///
    /// # Arguments
    ///
    /// * `graph` - The plan, whose nodes are briefed in place
    pub fn prepare(&self, graph: &mut Graph) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        // every doc link must name a doc, and a heading of it if it names one
        for n in &graph.nodes {
            for d in &n.docs_raw {
                if let Err(msg) = self.doc_link(n.origin.base(), d) {
                    diags.push(n.origin.diagnostic(msg));
                }
            }
        }
        // fill in every node's brief form once, from what is written or else from its docs
        for n in &mut graph.nodes {
            self.brief(n);
        }
        // every piece of work must say why it is needed and what finishing it will do, each
        // part a problem of its own, so a fix of one part doesn't change the other's
        let missing = (0..graph.len())
            .filter(|&i| !graph.is_group(i))
            .flat_map(|i| self.missing_brief(graph.node(i)));
        diags.extend(missing);
        diags
    }

    /// The problems of a piece of work that does not say why it is needed or what finishing it
    /// will do, once its docs have been looked in: one for each part missing.
    ///
    /// Each part's problem reads the same whether or not the other part is missing, so a write
    /// that fills in one part removes its problem and leaves the other's as it was.
    ///
    /// # Arguments
    ///
    /// * `n` - The briefed node to check
    fn missing_brief(&self, n: &Node) -> Vec<Diagnostic> {
        // name the doc sections each missing part could have come from
        let sections = |names: &[String]| names.join(", ");
        let mut diags = Vec::new();
        // a why with nothing written and nothing found
        if !has_text(&n.why) {
            let message = format!(
                "`{}` does not say why it is needed: write `description: {{ why: ... }}`, or link a doc with a section named one of {}",
                n.id,
                sections(&self.sections.why_sections)
            );
            diags.push(n.origin.diagnostic(message));
        }
        // and a what the same
        if !has_text(&n.what) {
            let message = format!(
                "`{}` does not say what finishing it will do: write `description: {{ what: ... }}`, or link a doc with a section named one of {}",
                n.id,
                sections(&self.sections.what_sections)
            );
            diags.push(n.origin.diagnostic(message));
        }
        diags
    }

    /// Fill in a node's why, what, details and design choices: as written, or else from the
    /// first of its docs that has them.
    ///
    /// # Arguments
    ///
    /// * `n` - The node to brief
    fn brief(&self, n: &mut Node) {
        // the path the node's links are written relative to
        let base = n.origin.base().to_string();
        // render each part of the description that is written
        if let Some(d) = &n.description_raw {
            let part = |md: &Option<String>| md.as_deref().map_or(String::new(), |md| render(md, &base, None, false));
            n.why = part(&d.why);
            n.what = part(&d.what);
            n.details = part(&d.details);
        }
        // render the design choices that are written, as a list if there are several
        match n.decisions_raw.as_slice() {
            [] => {}
            [one] => n.design = render(one, &base, None, false),
            many => {
                let list: String = many.iter().map(|d| format!("- {}\n", d.replace('\n', "\n  "))).collect();
                n.design = render(&list, &base, None, false);
            }
        }
        // render the note under the measurements
        if let Some(p) = &mut n.performance
            && let Some(note) = &p.note
        {
            p.note = Some(render(note, &base, None, false));
        }
        // render each acceptance criterion so it sits in a line
        for c in &mut n.acceptance {
            c.text = inline_html(&render(&c.text, &base, None, false));
        }
        // take whatever is still missing from the docs, in the order they are linked
        for link in &n.docs_raw {
            if has_text(&n.why) && has_text(&n.what) && !n.design.is_empty() && n.perf_doc.is_some() {
                break;
            }
            let Target::Internal { path, anchor } = paths::resolve(&base, link) else { continue };
            if !path.ends_with(".md") {
                continue;
            }
            let Some(text) = self.text(&path) else { continue };
            let Some(sec) = extract::section(&text, anchor.as_deref()) else { continue };
            // the why from a section or table row named for it
            if !has_text(&n.why)
                && let Some((from, html)) = extract::brief_part(sec, &self.sections.why_sections, &path)
            {
                n.why = html;
                n.why_from = Some(from);
            }
            // the what likewise: never a doc's opening, which rarely says it plainly
            if !has_text(&n.what)
                && let Some((from, html)) = extract::brief_part(sec, &self.sections.what_sections, &path)
            {
                n.what = html;
                n.what_from = Some(from);
            }
            // the design choices from a section named for them
            if n.design.is_empty()
                && let Some((from, html)) = extract::design(sec, &self.sections.design_sections, &path)
            {
                n.design = html;
                n.design_from = Some(from);
            }
            // where the measurements are, linked as the doc is
            if n.perf_doc.is_none() {
                n.perf_doc = perf_doc(&text, sec, anchor.is_some(), &self.sections.performance_sections)
                    .and_then(|pd| self.doc_link(&base, link).ok().map(|l| PerfDoc { href: l.href, label: l.label, ..pd }));
            }
        }
    }

    /// The viewer's data: every node, its body rendered, its links resolved. Links start with
    /// [`ROOT`].
    ///
    /// # Arguments
    ///
    /// * `graph` - The plan, its nodes briefed by [`Renderer::prepare`]
    /// * `page_of` - The node's own page, if it has one
    pub fn payload(&self, graph: &Graph, page_of: impl Fn(&Node) -> Option<String>) -> Payload {
        // each node's brief form was filled in when the plan was prepared
        let nodes = graph
            .nodes
            .iter()
            .map(|n| {
                let mut n = n.clone();
                // render its body and resolve its doc links
                n.html = render_body(&n.body, n.origin.base(), &n.title);
                n.docs = n.docs_raw.iter().filter_map(|d| self.doc_link(n.origin.base(), d).ok()).collect();
                // link its own page
                n.page = page_of(&n);
                n
            })
            .collect();
        Payload { nodes }
    }
}

/// Where a doc section keeps its measurements: a subsection named in `names`, or failing that,
/// when the link names a heading, that section's tables. `sec` is a slice of `text`, so the
/// subsection's heading id is the one it has in the whole file.
///
/// # Arguments
///
/// * `text` - The whole doc
/// * `sec` - The linked section of it
/// * `anchored` - Whether the link names a heading
/// * `names` - The subsection titles that hold measurements
fn perf_doc(text: &str, sec: &str, anchored: bool, names: &[String]) -> Option<PerfDoc> {
    let offset = sec.as_ptr() as usize - text.as_ptr() as usize;
    if let Some((title, sub)) = extract::subsection(sec, names) {
        let sub_at = sub.as_ptr() as usize - text.as_ptr() as usize;
        let head = heading_spans(text).into_iter().rfind(|h| h.start >= offset && h.end <= sub_at)?;
        return Some(PerfDoc { href: String::new(), label: String::new(), heading: Some(head.id), heading_text: Some(title), tables: 0 });
    }
    if !anchored {
        return None;
    }
    let tables = extract::measured_tables(sec).len();
    (tables > 0).then(|| PerfDoc { href: String::new(), label: String::new(), heading: None, heading_text: None, tables })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use reef_model::{Description, Node, NodeName, Origin};

    /// A node written in `plan/a.md` that links to `docs`
    fn linked(id: &str, docs: &[&str]) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            origin: Origin::File { source: "plan/a.md".into(), line: 3 },
            docs_raw: docs.iter().map(|d| d.to_string()).collect(),
            ..Node::default()
        }
    }

    /// A design doc with a why, a what and design choices under one of its headings
    const DESIGN: &str = "---\nid: d\n---\n# Design\n\n## Rows\n\n### Context\n\nRows are slow.\n\n### What it does\n\nRows go out [together](other.md#x).\n\n### Design choices\n\n- **One write.** Atomic.\n";

    #[test]
    fn links_and_anchors() {
        let r = Renderer::new(MemSource::new().with("design.md", DESIGN), Sections::default());
        // an anchored link takes its heading's text and the page mdbook writes
        let l = r.doc_link("plan/a.md", "../design.md#rows").unwrap();
        assert_eq!((l.label.as_str(), l.href.as_str()), ("Rows", "reef-root:/design.html#rows"));
        // a link to the whole doc takes its title
        assert_eq!(r.doc_link("plan/a.md", "../design.md").unwrap().label, "Design");
        // a missing doc or heading is named the way the build names it
        assert_eq!(r.doc_link("plan/a.md", "../gone.md").unwrap_err(), "doc `../gone.md` not found (looked for src/gone.md)");
        assert_eq!(
            r.doc_link("plan/a.md", "../design.md#nope").unwrap_err(),
            "doc `../design.md#nope`: src/design.md has no heading with the id `nope`"
        );
        // links that aren't docs are left as written
        assert_eq!(r.doc_link("a.md", "https://x.y").unwrap().href, "https://x.y");
        assert_eq!(r.doc_link("plan/a.md", "img.png").unwrap().href, "reef-root:/plan/img.png");
    }

    #[test]
    fn briefs_from_a_source_in_memory() {
        let r = Renderer::new(MemSource::new().with("design.md", DESIGN), Sections::default());
        let (mut g, _) = Graph::build(vec![linked("a", &["../design.md#rows"]), linked("b", &["../gone.md"])]);
        let diags: Vec<String> = r.prepare(&mut g).iter().map(|d| d.to_string()).collect();
        // the why, the what and the design choices come from the linked section
        let a = g.node(0);
        assert_eq!((a.why.trim(), a.why_from.as_deref()), ("<p>Rows are slow.</p>", Some("Context")));
        assert!(a.what.contains("href=\"reef-root:/other.html#x\""), "links rebased from the doc: {}", a.what);
        assert_eq!(a.what_from.as_deref(), Some("What it does"));
        assert!(a.design.contains("<summary>One write.</summary>"), "{}", a.design);
        // a missing doc is a problem, and so is each part of the brief that work then lacks
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert_eq!(diags[0], "plan/a.md:3: doc `../gone.md` not found (looked for src/gone.md)");
        assert!(diags[1].starts_with("plan/a.md:3: `b` does not say why it is needed: "), "{diags:?}");
        assert!(diags[2].starts_with("plan/a.md:3: `b` does not say what finishing it will do: "), "{diags:?}");
    }

    #[test]
    fn missing_why_and_what_are_problems_apart() {
        let r = Renderer::new(MemSource::new(), Sections::default());
        // work that says neither has a problem for each part, worded as if it lacked that alone
        let (mut g, _) = Graph::build(vec![linked("a", &[])]);
        let neither: Vec<String> = r.prepare(&mut g).iter().map(|d| d.to_string()).collect();
        assert_eq!(neither.len(), 2, "{neither:?}");
        // a what written fixes its problem and leaves the why's word for word
        let mut n = linked("a", &[]);
        n.description_raw = Some(Description { what: Some("Rows are fast.".into()), ..Description::default() });
        let (mut g, _) = Graph::build(vec![n]);
        let why_only: Vec<String> = r.prepare(&mut g).iter().map(|d| d.to_string()).collect();
        assert_eq!(why_only, neither[..1]);
        // and a why written the same, the other way round
        let mut n = linked("a", &[]);
        n.description_raw = Some(Description { why: Some("Rows are slow.".into()), ..Description::default() });
        let (mut g, _) = Graph::build(vec![n]);
        let what_only: Vec<String> = r.prepare(&mut g).iter().map(|d| d.to_string()).collect();
        assert_eq!(what_only, neither[1..]);
    }

    #[test]
    fn payload_renders_bodies_and_pages() {
        let r = Renderer::new(MemSource::new().with("design.md", DESIGN), Sections::default());
        let mut n = linked("a", &["../design.md"]);
        n.body = "# a\n\nSee [d](../design.md#rows).\n".into();
        let (g, _) = Graph::build(vec![n]);
        let p = r.payload(&g, |n| n.origin.file().map(|f| format!("{ROOT}{}", paths::html_of(f))));
        let a = &p.nodes[0];
        // the body is rendered, its repeated title dropped, and its doc links resolved
        assert_eq!(a.html.trim(), "<p>See <a href=\"reef-root:/design.html#rows\">d</a>.</p>");
        assert_eq!(a.docs, vec![Link { label: "Design".into(), href: "reef-root:/design.html".into() }]);
        assert_eq!(a.page.as_deref(), Some("reef-root:/plan/a.html"));
    }

    #[test]
    fn stored_nodes_link_from_the_root() {
        let r = Renderer::new(MemSource::new().with("design.md", DESIGN), Sections::default());
        // a stored node's links are written from the root of its plan's docs
        let name: NodeName = "reef/central/a".parse().unwrap();
        let mut n = Node { id: "a".into(), origin: Origin::Stored { name }, docs_raw: vec!["design.md#rows".into(), "gone.md".into()], ..Node::default() };
        n.body = "See [d](design.md).\n".into();
        let (mut g, _) = Graph::build(vec![n]);
        let diags: Vec<String> = r.prepare(&mut g).iter().map(|d| d.to_string()).collect();
        assert_eq!(g.node(0).why_from.as_deref(), Some("Context"));
        // its problems are at its name
        assert_eq!(diags, ["reef/central/a: doc `gone.md` not found (looked for src/gone.md)"]);
        let p = r.payload(&g, |_| None);
        assert!(p.nodes[0].html.contains("href=\"reef-root:/design.html\""), "{}", p.nodes[0].html);
        assert_eq!(p.nodes[0].docs[0].href, "reef-root:/design.html#rows");
    }

    #[test]
    fn sections_from_config() {
        // the kebab-case keys a book.toml uses, and the old name for the what's sections
        let s: Sections = serde_json::from_str(r#"{"why-sections":["Why"],"description-sections":["Gist"]}"#).unwrap();
        assert_eq!(s.why_sections, ["Why"]);
        assert_eq!(s.what_sections, ["Gist"]);
        assert_eq!(s.design_sections, Sections::default().design_sections);
    }
}
