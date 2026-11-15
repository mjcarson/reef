//! Finding a book's nodes, checking them, and rendering them for the viewer.

use crate::extract;
use crate::paths::{self, Target};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use reef_model::spec::{frontmatter_is_yaml, parse_node_file};
use reef_model::{Diagnostic, Graph, Layout, Link, Node, Payload, PerfDoc, ViewSpec};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Where links in rendered node bodies start. Each page replaces it with its way back to the
/// book's root, so a body is rendered once however many pages show it.
pub const ROOT: &str = "reef-root:/";

/// `[preprocessor.reef]` in `book.toml`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Directories under `src` to look for nodes in. Default: all of `src`.
    pub roots: Vec<String>,
    /// The subsections of a doc whose first paragraph is a node's description, or the first
    /// cells of a table row (`| **Impact** | ... |`) whose second cell is.
    pub description_sections: Vec<String>,
    /// The subsections of a doc that hold a node's design choices.
    pub design_sections: Vec<String>,
    /// The subsections of a doc that hold the work's measurements. A linked heading without one
    /// shows its tables instead.
    pub performance_sections: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Config {
            roots: Vec::new(),
            description_sections: v(&["What it does", "Symptom", "Summary", "Overview", "Impact"]),
            design_sections: v(&["Design choices", "Design", "Decisions", "The fix"]),
            performance_sections: v(&["Performance", "Results", "Benchmark", "Benchmarks", "Measurements"]),
        }
    }
}

pub struct Plan {
    pub src: PathBuf,
    pub config: Config,
    pub graph: Graph,
    pub diags: Vec<Diagnostic>,
    headings: RefCell<HashMap<String, Option<Vec<(String, String, u8)>>>>,
}

pub fn md_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Every markdown file under `dir`, relative to `src`, sorted.
pub fn markdown_files(src: &Path, roots: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let roots: Vec<String> = if roots.is_empty() { vec![String::new()] } else { roots.to_vec() };
    for r in roots {
        let mut stack = vec![src.join(&r)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if name.ends_with(".md") && name != "SUMMARY.md"
                    && let Ok(rel) = p.strip_prefix(src) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

impl Plan {
    /// Find and check every node under `src`.
    pub fn load(src: &Path, config: &Config) -> Plan {
        let mut diags = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        for rel in markdown_files(src, &config.roots) {
            let Ok(text) = std::fs::read_to_string(src.join(&rel)) else { continue };
            match parse_node_file(&text) {
                Ok(Some((spec, fm))) => {
                    let start = nodes.len();
                    Node::from_spec(spec, &rel, 1, fm.body.to_string(), &mut nodes);
                    for n in &mut nodes[start..] {
                        if n.inline {
                            n.line = line_of_id(fm.yaml, &n.id).map_or(1, |l| l + fm.yaml_line - 1);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => diags.push(Diagnostic { source: rel.clone(), line: e.line, message: e.message }),
            }
        }
        let (graph, gd) = Graph::build(nodes);
        diags.extend(gd);
        let plan = Plan { src: src.to_path_buf(), config: config.clone(), graph, diags, headings: RefCell::default() };
        let mut doc_diags = Vec::new();
        for n in &plan.graph.nodes {
            for d in &n.docs_raw {
                if let Err(msg) = plan.doc_link(&n.source, d) {
                    doc_diags.push(Diagnostic { source: n.source.clone(), line: Some(n.line), message: msg });
                }
            }
        }
        let mut plan = plan;
        plan.diags.extend(doc_diags);
        plan
    }

    /// Headings of a book file, as `(id, text, level)`, or `None` if there is no such file.
    fn headings(&self, path: &str) -> Option<Vec<(String, String, u8)>> {
        if let Some(h) = self.headings.borrow().get(path) {
            return h.clone();
        }
        let h = std::fs::read_to_string(self.src.join(path)).ok().map(|text| {
            let body = frontmatter_is_yaml(&text).map_or(text.as_str(), |fm| fm.body).to_string();
            headings_of(&body)
        });
        self.headings.borrow_mut().insert(path.to_string(), h.clone());
        h
    }

    /// Resolve a doc link written in `from`, checking that its file and anchor exist.
    pub fn doc_link(&self, from: &str, link: &str) -> Result<Link, String> {
        match paths::resolve(from, link) {
            Target::External(url) => Ok(Link { label: url.clone(), href: url }),
            Target::Internal { path, anchor } => {
                if !path.ends_with(".md") {
                    return Ok(Link { label: path.clone(), href: format!("{ROOT}{path}") });
                }
                let Some(heads) = self.headings(&path) else {
                    return Err(format!("doc `{link}` not found (looked for src/{path})"));
                };
                let label = match &anchor {
                    Some(a) => match heads.iter().find(|h| &h.0 == a) {
                        Some(h) => h.1.clone(),
                        None => return Err(format!("doc `{link}`: src/{path} has no heading with the id `{a}`")),
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

    /// Fill in a node's description and design choices: as written, or else from the first of
    /// its docs that has them. `texts` caches doc files by path.
    fn brief(&self, n: &mut Node, texts: &mut HashMap<String, Option<String>>) {
        if let Some(d) = &n.description_raw {
            n.description = render(d, &n.source, None, false);
        }
        match n.decisions_raw.as_slice() {
            [] => {}
            [one] => n.design = render(one, &n.source, None, false),
            many => {
                let list: String = many.iter().map(|d| format!("- {}\n", d.replace('\n', "\n  "))).collect();
                n.design = render(&list, &n.source, None, false);
            }
        }
        if let Some(p) = &mut n.performance
            && let Some(note) = &p.note
        {
            p.note = Some(render(note, &n.source, None, false));
        }
        for link in &n.docs_raw {
            if !n.description.is_empty() && !n.design.is_empty() && n.perf_doc.is_some() {
                break;
            }
            let Target::Internal { path, anchor } = paths::resolve(&n.source, link) else { continue };
            if !path.ends_with(".md") {
                continue;
            }
            let text = texts.entry(path.clone()).or_insert_with(|| {
                std::fs::read_to_string(self.src.join(&path))
                    .ok()
                    .map(|t| frontmatter_is_yaml(&t).map_or(t.as_str(), |fm| fm.body).to_string())
            });
            let Some(text) = text.as_deref() else { continue };
            let Some(sec) = extract::section(text, anchor.as_deref()) else { continue };
            if n.description.is_empty()
                && let Some(d) = extract::description(sec, &self.config.description_sections, &path)
            {
                n.description = d;
            }
            if n.design.is_empty()
                && let Some((from, html)) = extract::design(sec, &self.config.design_sections, &path)
            {
                n.design = html;
                n.design_from = Some(from);
            }
            if n.perf_doc.is_none() {
                n.perf_doc = perf_doc(text, sec, anchor.is_some(), &self.config.performance_sections)
                    .and_then(|pd| self.doc_link(&n.source, link).ok().map(|l| PerfDoc { href: l.href, label: l.label, ..pd }));
            }
        }
    }

    pub fn errors(&self) -> String {
        self.diags.iter().map(|d| format!("  {d}")).collect::<Vec<_>>().join("\n")
    }

    /// The viewer's data: every node, its body rendered, its links resolved. Links start with
    /// [`ROOT`]. `chapters` are the book's chapter files, which have pages of their own.
    pub fn payload_json(&self, chapters: &HashSet<String>) -> String {
        let mut texts: HashMap<String, Option<String>> = HashMap::new();
        let nodes = self
            .graph
            .nodes
            .iter()
            .map(|n| {
                let mut n = n.clone();
                n.html = render_body(&n.body, &n.source, &n.title);
                n.docs = n.docs_raw.iter().filter_map(|d| self.doc_link(&n.source, d).ok()).collect();
                self.brief(&mut n, &mut texts);
                if chapters.contains(&n.source) {
                    n.page = Some(format!("{ROOT}{}", paths::html_of(&n.source)));
                }
                n
            })
            .collect();
        serde_json::to_string(&Payload { nodes }).expect("serializable")
    }
}

/// Where a doc section keeps its measurements: a subsection named in `names`, or failing that,
/// when the link names a heading, that section's tables. `sec` is a slice of `text`, so the
/// subsection's heading id is the one it has in the whole file.
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

/// The line (0-based within `yaml`, plus one) an inline node's `id:` is on.
fn line_of_id(yaml: &str, id: &str) -> Option<usize> {
    yaml.lines().position(|l| {
        let t = l.trim_start().trim_start_matches("- ").trim_start_matches('{').trim_start();
        let Some(rest) = t.strip_prefix("id:") else { return false };
        let v = rest.trim().split([',', '}']).next().unwrap_or("").trim().trim_matches(['"', '\'']);
        v == id
    }).map(|l| l + 1)
}

/// `(id, text, level)` of each heading, with the ids mdbook gives them.
pub fn headings_of(md: &str) -> Vec<(String, String, u8)> {
    heading_spans(md).into_iter().map(|h| (h.id, h.text, h.level)).collect()
}

/// A heading, with where it lies in its markdown.
#[derive(Debug, Clone)]
pub struct HeadingSpan {
    pub id: String,
    pub text: String,
    pub level: u8,
    /// Where the heading starts.
    pub start: usize,
    /// Where the heading ends and its section's text begins.
    pub end: usize,
}

/// Each heading, with the id mdbook gives it and its byte range.
pub fn heading_spans(md: &str) -> Vec<HeadingSpan> {
    let mut out = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut cur: Option<(Option<String>, String, u8, usize)> = None;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Heading { level, id, .. }) => cur = Some((id.map(|s| s.to_string()), String::new(), level as u8, range.start)),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = &mut cur {
                    c.1.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((id, text, level, start)) = cur.take() {
                    let id = match id {
                        Some(id) => id,
                        None => {
                            let base = paths::heading_id(&text);
                            let mut id = base.clone();
                            let mut k = 1;
                            while used.contains(&id) {
                                id = format!("{base}-{k}");
                                k += 1;
                            }
                            used.insert(id.clone());
                            id
                        }
                    };
                    out.push(HeadingSpan { id, text, level, start, end: range.end });
                }
            }
            _ => {}
        }
    }
    out
}

/// Render a node's body for the sidebar: links made book-relative, its `reef` fences and a
/// first heading that repeats its title left out.
pub fn render_body(md: &str, source: &str, title: &str) -> String {
    render(md, source, Some(title), false)
}

/// Render markdown written in `source` for the sidebar. With `title`, a first heading that
/// repeats it is left out. With `drop_struck`, struck-through text is left out.
pub fn render(md: &str, source: &str, title: Option<&str>, drop_struck: bool) -> String {
    let title = title.unwrap_or("\u{0}");
    let mut struck = 0usize;
    let mut events = Vec::new();
    let mut skip_fence = false;
    let mut first_heading: Option<Vec<Event<'_>>> = None;
    let mut seen_block = false;
    let rewrite = |dest: CowStr<'_>| -> CowStr<'static> {
        match paths::resolve(source, &dest) {
            Target::External(u) => CowStr::from(u),
            Target::Internal { path, anchor } => {
                let base = format!("{ROOT}{}", paths::html_of(&path));
                CowStr::from(match anchor {
                    Some(a) => format!("{base}#{a}"),
                    None => base,
                })
            }
        }
    };
    for ev in Parser::new_ext(md, md_options()) {
        match &ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if is_reef(info) => {
                skip_fence = true;
                continue;
            }
            Event::End(TagEnd::CodeBlock) if skip_fence => {
                skip_fence = false;
                continue;
            }
            _ if skip_fence => continue,
            Event::Start(Tag::Strikethrough) if drop_struck => {
                struck += 1;
                continue;
            }
            Event::End(TagEnd::Strikethrough) if drop_struck => {
                struck = struck.saturating_sub(1);
                continue;
            }
            _ if struck > 0 => continue,
            _ => {}
        }
        if !seen_block {
            if let Event::Start(Tag::Heading { .. }) = ev {
                first_heading = Some(vec![]);
            }
            if let Some(h) = &mut first_heading {
                let end = matches!(ev, Event::End(TagEnd::Heading(_)));
                h.push(ev.clone().into_static());
                if end {
                    seen_block = true;
                    let text: String = h.iter().filter_map(|e| if let Event::Text(t) | Event::Code(t) = e { Some(t.to_string()) } else { None }).collect();
                    if text.trim() != title.trim() {
                        events.extend(first_heading.take().unwrap());
                    }
                    first_heading = None;
                }
                continue;
            }
            seen_block = true;
        }
        let ev = match ev {
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Link { link_type, dest_url: rewrite(dest_url), title, id })
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Image { link_type, dest_url: rewrite(dest_url), title, id })
            }
            e => e.into_static(),
        };
        events.push(ev.into_static());
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

pub fn is_reef(info: &str) -> bool {
    info.split(|c: char| c.is_whitespace() || c == ',' || c == '{').next() == Some("reef")
}

/// A ```` ```reef ```` fence in a chapter.
pub struct Fence {
    pub range: Range<usize>,
    pub line: usize,
    pub spec: Result<ViewSpec, String>,
}

pub fn find_fences(md: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let mut cur: Option<(Range<usize>, String)> = None;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if is_reef(&info) => cur = Some((range, String::new())),
            Event::Text(t) => {
                if let Some(c) = &mut cur {
                    c.1.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((range, body)) = cur.take() {
                    let line = md[..range.start].matches('\n').count() + 1;
                    let spec = toml::from_str::<ViewSpec>(&body).map_err(|e| e.message().to_string());
                    out.push(Fence { range, line, spec });
                }
            }
            _ => {}
        }
    }
    out
}

/// The HTML a fence becomes.
pub fn fence_html(spec: &ViewSpec) -> String {
    let mut spec = spec.clone();
    if let Some(link) = &spec.page {
        spec.page = Some(page_href(link));
    }
    let json = serde_json::to_string(&spec).expect("serializable");
    let (class, style) = match spec.layout {
        Layout::Page => ("reef reef-page", String::new()),
        Layout::Inline => {
            let height = spec.height.clone().unwrap_or_else(|| "70vh".into());
            ("reef", format!(" style=\"height: {}\"", paths::escape(&height)))
        }
    };
    format!(
        "<div class=\"{class}\" data-view=\"{}\"{style}><p class=\"reef-noscript\">This plan is drawn by reef.js, which needs JavaScript.</p></div>",
        paths::escape(&json),
    )
}

/// The href a fence's `page` link becomes: the same relative path, to the `.html` mdbook writes.
pub fn page_href(link: &str) -> String {
    let (path, anchor) = link.split_once('#').map_or((link, None), |(p, a)| (p, Some(a)));
    let html = paths::html_of(path);
    match anchor {
        Some(a) => format!("{html}#{a}"),
        None => html,
    }
}

/// The checks that look at all of a file's fences: at most one fills the page, and each `page`
/// link names a chapter of the book (`is_chapter` takes a `src`-relative path). `from` is the
/// file, relative to `src`. Returns `(line, problem)`.
pub fn check_fences_together(fences: &[Fence], from: &str, is_chapter: impl Fn(&str) -> bool) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut first_page: Option<usize> = None;
    for f in fences {
        let Ok(spec) = &f.spec else { continue };
        if spec.layout == Layout::Page {
            match first_page {
                Some(l) => out.push((f.line, format!("a page has one layout = \"page\" view, and line {l} has it already"))),
                None => first_page = Some(f.line),
            }
        }
        if let Some(link) = &spec.page {
            match paths::resolve(from, link) {
                paths::Target::Internal { path, .. } if is_chapter(&path) => {}
                paths::Target::Internal { path, .. } => out.push((f.line, format!("page `{link}`: `{path}` is not a chapter of this book"))),
                paths::Target::External(_) => out.push((f.line, format!("page `{link}`: expected a chapter of this book"))),
            }
        }
    }
    out
}

/// The data a page with fences carries, once.
pub fn data_script(payload_json: &str, prefix: &str) -> String {
    let json = payload_json.replace(ROOT, prefix).replace("</", "<\\/");
    format!("<script type=\"application/json\" class=\"reef-data\">{json}</script>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_links_book_relative() {
        let html = render_body("# T\n\nSee [b](../b.md#x) and [w](https://w.org).\n", "a/n.md", "T");
        assert!(html.contains("href=\"reef-root:/b.html#x\""), "{html}");
        assert!(html.contains("https://w.org"));
        assert!(!html.contains("<h1>"), "repeated title dropped: {html}");
    }

    #[test]
    fn briefs_written_or_taken_from_docs() {
        let dir = std::env::temp_dir().join(format!("reef-brief-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plan")).unwrap();
        std::fs::write(
            dir.join("f.md"),
            "# F\n\n## What it does\n\nTaken.\n\n## Design choices\n\n- **Chosen.** Why.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("plan/a.md"),
            "---\nid: a\ndocs: [../f.md]\nnodes:\n  - { id: b, docs: [../f.md], description: Mine, decisions: [x, y] }\n---\n",
        )
        .unwrap();
        let plan = Plan::load(&dir, &Config::default());
        assert!(plan.diags.is_empty(), "{}", plan.errors());
        let json: serde_json::Value = serde_json::from_str(&plan.payload_json(&HashSet::new())).unwrap();
        let node = |id: &str| json["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap().clone();
        let a = node("a");
        assert_eq!(a["description"].as_str().unwrap().trim(), "<p>Taken.</p>");
        assert!(a["design"].as_str().unwrap().contains("<summary>Chosen.</summary>"));
        assert_eq!(a["design_from"], "Design choices");
        let b = node("b");
        assert_eq!(b["description"].as_str().unwrap().trim(), "<p>Mine</p>");
        assert!(b["design"].as_str().unwrap().contains("<li>x</li>"), "{b}");
        assert!(b.get("design_from").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn performance_from_frontmatter_and_docs() {
        let dir = std::env::temp_dir().join(format!("reef-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plan")).unwrap();
        std::fs::write(
            dir.join("opt.md"),
            "# Opts\n\n## Results\n\nNot this one.\n\n### O1. One\n\n| | |\n| --- | --- |\n| **Impact** | big |\n\n**Probed.** Run.\n\n| Way | n |\n| --- | --- |\n| a | 1 |\n\n### O2. Two\n\nWords.\n\n#### Results\n\nMeasured.\n\n### O3. Three\n\nNone here.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("plan/a.md"),
            "---\nid: a\nnodes:\n  - id: o1\n    docs: [../opt.md#o1-one]\n    performance:\n      note: Medians of **eight**.\n      metrics:\n        - { name: n, before: 2, after: 1 }\n  - { id: o2, docs: [../opt.md#o2-two] }\n  - { id: o3, docs: [../opt.md#o3-three] }\n---\n",
        )
        .unwrap();
        let plan = Plan::load(&dir, &Config::default());
        assert!(plan.diags.is_empty(), "{}", plan.errors());
        let json: serde_json::Value = serde_json::from_str(&plan.payload_json(&HashSet::new())).unwrap();
        let node = |id: &str| json["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap().clone();
        let o1 = node("o1");
        assert_eq!(o1["perf_doc"]["tables"], 1, "{o1}");
        assert_eq!(o1["perf_doc"]["href"], "reef-root:/opt.html#o1-one");
        assert!(o1["perf_doc"].get("heading").is_none());
        assert!(o1["performance"]["note"].as_str().unwrap().contains("<strong>eight</strong>"), "{o1}");
        assert_eq!(o1["performance"]["metrics"][0]["after"], 1.0);
        let o2 = node("o2");
        assert_eq!(o2["perf_doc"]["heading"], "results-1", "the file's own id for the second Evidence: {o2}");
        assert_eq!(o2["perf_doc"]["heading_text"], "Results");
        assert!(node("o3").get("perf_doc").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_fences() {
        let md = "```reef\nlayout = \"page\"\n```\n\n```reef\nlayout = \"page\"\npage = \"../p.md#x\"\n```\n";
        let f = find_fences(md);
        let html = fence_html(f[1].spec.as_ref().unwrap());
        assert!(html.contains("class=\"reef reef-page\""), "{html}");
        assert!(!html.contains("style="), "{html}");
        assert!(html.contains("../p.html#x"), "{html}");
        let probs = check_fences_together(&f, "a/b.md", |p| p == "p.md");
        assert_eq!(probs.len(), 1, "{probs:?}");
        assert_eq!(probs[0].0, 5);
        assert_eq!(check_fences_together(&f, "a/b.md", |_| false).len(), 2);
        assert!(fence_html(&ViewSpec::default()).contains("style=\"height: 70vh\""));
    }

    #[test]
    fn finds_fences() {
        let md = "x\n\n```reef\nroot = \"g\"\ndirection = \"LR\"\n```\n\n```rust\n```\n";
        let f = find_fences(md);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 3);
        let spec = f[0].spec.as_ref().unwrap();
        assert_eq!(spec.root.as_deref(), Some("g"));
        assert!(find_fences("```reef\nnope = 1\n```\n")[0].spec.is_err());
    }

    #[test]
    fn heading_ids_match_mdbook() {
        let h = headings_of("# A b\n## A b\n### X {#custom}\n");
        assert_eq!(h[0].0, "a-b");
        assert_eq!(h[1].0, "a-b-1");
        assert_eq!(h[2].0, "custom");
    }

    #[test]
    fn inline_lines() {
        let y = "id: g\nnodes:\n  - id: a\n  - { id: \"b\", title: x }\n";
        assert_eq!(line_of_id(y, "a"), Some(3));
        assert_eq!(line_of_id(y, "b"), Some(4));
    }
}
