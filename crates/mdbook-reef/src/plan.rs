//! Finding a book's nodes, checking them, and rendering them for the viewer.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use reef_model::spec::parse_node_file;
use reef_model::{Diagnostic, Graph, Layout, Node, ViewSpec};
use reef_render::paths;
use reef_render::{DocSource, FsSource, ROOT, Renderer, Sections, is_reef, md_options};
use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

/// `[preprocessor.reef]` in `book.toml`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Directories under `src` to look for nodes in. Default: all of `src`.
    pub roots: Vec<String>,
    /// The doc sections a node's why, what, design choices and measurements are taken from.
    #[serde(flatten)]
    pub sections: Sections,
}

/// A book's plan: its nodes checked as one graph, with the problems found in them
pub struct Plan {
    /// Every node, checked and briefed
    pub graph: Graph,
    /// The problems found, each with where it was written
    pub diags: Vec<Diagnostic>,
    /// Renders the nodes against the book's docs
    pub renderer: Renderer<Box<dyn DocSource>>,
}

/// Every markdown file under `dir`, relative to `src`, sorted.
///
/// # Arguments
///
/// * `src` - The book's `src` directory
/// * `roots` - The directories under `src` to look in, or all of it when empty
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
    /// Find and check every node under `src`, its docs read from the book's files.
    ///
    /// # Arguments
    ///
    /// * `src` - The book's `src` directory
    /// * `config` - The book's `[preprocessor.reef]`
    pub fn load(src: &Path, config: &Config) -> Plan {
        Plan::load_with(src, config, Box::new(FsSource::new(src)))
    }

    /// Find and check every node under `src`, its docs read from `docs`.
    ///
    /// # Arguments
    ///
    /// * `src` - The book's `src` directory
    /// * `config` - The book's `[preprocessor.reef]`
    /// * `docs` - Where the docs the nodes link to are read from
    pub fn load_with(src: &Path, config: &Config, docs: Box<dyn DocSource>) -> Plan {
        let mut diags = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        // parse every node file, and put each inline node at the line its id is on
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
        // check the nodes as one graph
        let (mut graph, gd) = Graph::build(nodes);
        diags.extend(gd);
        // check their doc links and fill in their brief forms from their docs
        let renderer = Renderer::new(docs, config.sections.clone());
        diags.extend(renderer.prepare(&mut graph));
        Plan { graph, diags, renderer }
    }

    /// Every problem, one an indented line
    pub fn errors(&self) -> String {
        self.diags.iter().map(|d| format!("  {d}")).collect::<Vec<_>>().join("\n")
    }

    /// The viewer's data: every node, its body rendered, its links resolved. Links start with
    /// [`ROOT`].
    ///
    /// # Arguments
    ///
    /// * `chapters` - The book's chapter files, which have pages of their own
    pub fn payload_json(&self, chapters: &HashSet<String>) -> String {
        // a node written in a chapter links to the chapter's page
        let page_of = |n: &Node| chapters.contains(&n.source).then(|| format!("{ROOT}{}", paths::html_of(&n.source)));
        serde_json::to_string(&self.renderer.payload(&self.graph, page_of)).expect("serializable")
    }
}

/// The line (0-based within `yaml`, plus one) an inline node's `id:` is on.
///
/// # Arguments
///
/// * `yaml` - The file's frontmatter
/// * `id` - The inline node's id
fn line_of_id(yaml: &str, id: &str) -> Option<usize> {
    yaml.lines().position(|l| {
        let t = l.trim_start().trim_start_matches("- ").trim_start_matches('{').trim_start();
        let Some(rest) = t.strip_prefix("id:") else { return false };
        let v = rest.trim().split([',', '}']).next().unwrap_or("").trim().trim_matches(['"', '\'']);
        v == id
    }).map(|l| l + 1)
}

/// A ```` ```reef ```` fence in a chapter.
pub struct Fence {
    /// Where the fence lies in the chapter's markdown
    pub range: Range<usize>,
    /// The line the fence opens on
    pub line: usize,
    /// The view the fence draws, or why it can't be read
    pub spec: Result<ViewSpec, String>,
}

/// Every ```` ```reef ```` fence in a chapter's markdown.
///
/// # Arguments
///
/// * `md` - The chapter's markdown, its frontmatter left out
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
///
/// # Arguments
///
/// * `spec` - The view the fence draws
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
///
/// # Arguments
///
/// * `link` - The `page` link as written
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
///
/// # Arguments
///
/// * `fences` - The file's fences
/// * `from` - The file, relative to `src`
/// * `is_chapter` - Whether a `src`-relative path is a chapter of the book
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
///
/// # Arguments
///
/// * `payload_json` - The viewer's data, its links starting with [`ROOT`]
/// * `prefix` - The page's way back to the book's root
pub fn data_script(payload_json: &str, prefix: &str) -> String {
    let json = payload_json.replace(ROOT, prefix).replace("</", "<\\/");
    format!("<script type=\"application/json\" class=\"reef-data\">{json}</script>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn briefs_written_or_taken_from_docs() {
        let dir = std::env::temp_dir().join(format!("reef-brief-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plan")).unwrap();
        std::fs::write(
            dir.join("f.md"),
            "# F\n\n## Context\n\nNeeded.\n\n## What it does\n\nTaken.\n\n## Design choices\n\n- **Chosen.** Why.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("plan/a.md"),
            "---\nid: a\ndocs: [../f.md]\nnodes:\n  - { id: b, docs: [../f.md], description: { what: Mine, details: Longer }, decisions: [x, y] }\n---\n",
        )
        .unwrap();
        let plan = Plan::load(&dir, &Config::default());
        assert!(plan.diags.is_empty(), "{}", plan.errors());
        let json: serde_json::Value = serde_json::from_str(&plan.payload_json(&HashSet::new())).unwrap();
        let node = |id: &str| json["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap().clone();
        // a node with nothing written takes each part from its doc, and says where from
        let a = node("a");
        assert_eq!(a["why"].as_str().unwrap().trim(), "<p>Needed.</p>");
        assert_eq!(a["why_from"], "Context");
        assert_eq!(a["what"].as_str().unwrap().trim(), "<p>Taken.</p>");
        assert_eq!(a["what_from"], "What it does");
        assert!(a.get("details").is_none());
        assert!(a["design"].as_str().unwrap().contains("<summary>Chosen.</summary>"));
        assert_eq!(a["design_from"], "Design choices");
        // what is written wins, and only what is missing is taken from the doc
        let b = node("b");
        assert_eq!(b["why"].as_str().unwrap().trim(), "<p>Needed.</p>");
        assert_eq!(b["why_from"], "Context");
        assert_eq!(b["what"].as_str().unwrap().trim(), "<p>Mine</p>");
        assert!(b.get("what_from").is_none(), "{b}");
        assert_eq!(b["details"].as_str().unwrap().trim(), "<p>Longer</p>");
        assert!(b["design"].as_str().unwrap().contains("<li>x</li>"), "{b}");
        assert!(b.get("design_from").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_must_say_why_and_what() {
        let dir = std::env::temp_dir().join(format!("reef-why-what-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // an optimization register entry: its why is its Impact row, its what its own opening
        std::fs::write(
            dir.join("opt.md"),
            "# Opts\n\n### O1. One\n\n| | |\n| --- | --- |\n| **Impact** | Quadratic |\n| **Summary** | Counts once |\n\nAn opening.\n\n### O2. Two\n\nNo why or what here.\n",
        )
        .unwrap();
        // a group, which needs neither, and members that have, lack or half-have them
        std::fs::write(
            dir.join("g.md"),
            "---\nid: g\nnodes:\n  - { id: o1, docs: [opt.md#o1-one] }\n  - id: o2\n    docs: [opt.md#o2-two]\n  - id: plain\n    description: Only details.\n  - { id: whole, description: { why: Slow., what: Fast. } }\n---\n",
        )
        .unwrap();
        let plan = Plan::load(&dir, &Config::default());
        // only the two members missing a part are problems, each at its own line
        let diags: Vec<String> = plan.diags.iter().map(|d| d.to_string()).collect();
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert!(diags[0].starts_with("g.md:5: `o2` does not say why it is needed or what finishing it will do"), "{diags:?}");
        assert!(diags[1].starts_with("g.md:7: `plain` does not say why it is needed or what finishing it will do"), "{diags:?}");
        // the register entry's parts, labelled by the rows they came from, not its opening
        let o1 = plan.graph.node(plan.graph.index_of("o1").unwrap());
        assert_eq!((o1.why.trim(), o1.why_from.as_deref()), ("<p>Quadratic</p>", Some("Impact")));
        assert_eq!((o1.what.trim(), o1.what_from.as_deref()), ("<p>Counts once</p>", Some("Summary")));
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
            "---\nid: a\nnodes:\n  - id: o1\n    docs: [../opt.md#o1-one]\n    description: { what: X }\n    performance:\n      note: Medians of **eight**.\n      metrics:\n        - { name: n, before: 2, after: 1 }\n  - { id: o2, docs: [../opt.md#o2-two], description: { why: W, what: X } }\n  - { id: o3, docs: [../opt.md#o3-three], description: { why: W, what: X } }\n---\n",
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
    fn acceptance_in_the_payload() {
        let dir = std::env::temp_dir().join(format!("reef-accept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.md"),
            "---\nid: a\ndescription: { why: W, what: X }\nperformance:\n  metrics:\n    - { name: n, before: 2, after: 1 }\nacceptance:\n  - unit: Answers **at once**\n  - perf: { text: Halves, metric: n, at-most: 1 }\n---\n",
        )
        .unwrap();
        let plan = Plan::load(&dir, &Config::default());
        assert!(plan.diags.is_empty(), "{}", plan.errors());
        let json: serde_json::Value = serde_json::from_str(&plan.payload_json(&HashSet::new())).unwrap();
        let acc = &json["nodes"][0]["acceptance"];
        assert_eq!(acc[0]["kind"], "unit");
        assert_eq!(acc[0]["text"], "Answers <strong>at once</strong>");
        assert_eq!(acc[1]["at-most"], 1.0);
        let nodes: Vec<reef_model::Node> = serde_json::from_value(json["nodes"].clone()).unwrap();
        assert_eq!(nodes[0].acceptance[1].metric.as_deref(), Some("n"));
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
    fn inline_lines() {
        let y = "id: g\nnodes:\n  - id: a\n  - { id: \"b\", title: x }\n";
        assert_eq!(line_of_id(y, "a"), Some(3));
        assert_eq!(line_of_id(y, "b"), Some(4));
    }

    #[test]
    fn config_from_book_toml() {
        // the roots beside the section names, the old name for the what's sections too
        let c: Config = toml::from_str("command = \"mdbook-reef\"\nroots = [\"plan\"]\nwhy-sections = [\"Why\"]\ndescription-sections = [\"Gist\"]\n").unwrap();
        assert_eq!(c.roots, ["plan"]);
        assert_eq!(c.sections.why_sections, ["Why"]);
        assert_eq!(c.sections.what_sections, ["Gist"]);
        assert_eq!(c.sections.design_sections, Sections::default().design_sections);
        // nothing written is the defaults
        let c: Config = toml::from_str("").unwrap();
        assert!(c.roots.is_empty());
        assert_eq!(c.sections, Sections::default());
    }

    #[test]
    fn example_book_from_memory() {
        let book = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../example-book");
        let (_, src, config) = crate::check::open_book(&book).unwrap();
        // every file of the book, read into memory
        let mut docs = reef_render::MemSource::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(text) = std::fs::read_to_string(&p) {
                    let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
                    docs.insert(rel, text);
                }
            }
        }
        // the plan built with its docs read from disk, and from memory
        let from_disk = Plan::load(&src, &config);
        let from_memory = Plan::load_with(&src, &config, Box::new(docs));
        assert!(from_disk.diags.is_empty(), "{}", from_disk.errors());
        assert!(from_memory.diags.is_empty(), "{}", from_memory.errors());
        // the same links, anchors, whys and whats either way
        let chapters: HashSet<String> = markdown_files(&src, &[]).into_iter().collect();
        let json = from_disk.payload_json(&chapters);
        assert_eq!(json, from_memory.payload_json(&chapters));
        // and the payload reads back as the viewer reads it, writing out the same
        let payload: reef_model::Payload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload.nodes.len(), from_disk.graph.len());
        assert_eq!(serde_json::to_string(&payload).unwrap(), json);
    }
}
