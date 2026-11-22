//! A book's plan read from its files: the node files under its `src`, each node checked as part
//! of one graph and briefed from the docs it links to.
//!
//! mdbook-reef draws and checks a book through [`Plan`], and `reef import` reads the same node
//! files through [`read_nodes`] before it lifts them into a central plan's rows. Reading
//! `book.toml` is left to each caller, which already has its own way to.

use crate::markdown::ROOT;
use crate::paths;
use crate::{DocSource, FsSource, Renderer, Sections};
use reef_model::spec::parse_node_file;
use reef_model::{Diagnostic, Graph, Node, NodeSpec, Origin, Payload, Record, nodes_from_records};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// `[preprocessor.reef]` in `book.toml`
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Directories under `src` to look for nodes in. Default: all of `src`.
    pub roots: Vec<String>,
    /// The doc sections a node's why, what, design choices and measurements are taken from
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

/// A node file of a book, as written
#[derive(Debug, Clone)]
pub struct BookNode {
    /// The file, relative to `src`
    pub rel: String,
    /// Where the file falls among the book's markdown files, in the order they sort in
    pub order: u64,
    /// The file's node, its inline members still listed in it
    pub spec: NodeSpec,
    /// The file's body as markdown, its frontmatter left out
    pub body: String,
    /// The file's frontmatter, without its fences
    yaml: String,
    /// The line of the file its frontmatter starts on
    yaml_line: usize,
}

/// Every markdown file under `src`, or under each of its `roots`, relative to `src` and sorted
///
/// Dotfiles and mdbook's `SUMMARY.md` are left out. The order they sort in is the order the
/// plan was written in.
///
/// # Arguments
///
/// * `src` - The book's `src` directory
/// * `roots` - The directories under `src` to look in, or all of it when empty
pub fn markdown_files(src: &Path, roots: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    // no roots named means all of `src`
    let roots: Vec<String> = if roots.is_empty() {
        vec![String::new()]
    } else {
        roots.to_vec()
    };
    for root in roots {
        // walk each root, a directory at a time
        let mut stack = vec![src.join(&root)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                // dotfiles are never nodes
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if name.ends_with(".md")
                    && name != "SUMMARY.md"
                    && let Ok(rel) = path.strip_prefix(src)
                {
                    // the summary is mdbook's, and every other markdown file may be a node
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    // roots that overlap name a file once
    out.sort();
    out.dedup();
    out
}

/// Every node file under `src`, in the order the files sort in, with the problems found
/// reading them
///
/// A markdown file without an `id` in its frontmatter is a doc, not a node, and is left out, as
/// is a file that can't be read.
///
/// # Arguments
///
/// * `src` - The book's `src` directory
/// * `roots` - The directories under `src` to look in, or all of it when empty
pub fn read_nodes(src: &Path, roots: &[String]) -> (Vec<BookNode>, Vec<Diagnostic>) {
    let mut nodes = Vec::new();
    let mut diags = Vec::new();
    for (order, rel) in markdown_files(src, roots).into_iter().enumerate() {
        // a file that can't be read is no node
        let Ok(text) = std::fs::read_to_string(src.join(&rel)) else {
            continue;
        };
        match parse_node_file(&text) {
            // a node file keeps its frontmatter, to place its inline members at their lines
            Ok(Some((spec, fm))) => nodes.push(BookNode {
                order: order as u64,
                spec,
                body: fm.body.to_string(),
                yaml: fm.yaml.to_string(),
                yaml_line: fm.yaml_line,
                rel,
            }),
            // a doc has no `id`
            Ok(None) => {}
            Err(error) => diags.push(Diagnostic {
                source: rel,
                line: error.line,
                message: error.message,
            }),
        }
    }
    (nodes, diags)
}

impl Plan {
    /// Find and check every node under `src`, its docs read from the book's files
    ///
    /// # Arguments
    ///
    /// * `src` - The book's `src` directory
    /// * `config` - The book's `[preprocessor.reef]`
    pub fn load(src: &Path, config: &Config) -> Plan {
        Plan::load_with(src, config, Box::new(FsSource::new(src)))
    }

    /// Find and check every node under `src`, its docs read from `docs`
    ///
    /// # Arguments
    ///
    /// * `src` - The book's `src` directory
    /// * `config` - The book's `[preprocessor.reef]`
    /// * `docs` - Where the docs the nodes link to are read from
    pub fn load_with(src: &Path, config: &Config, docs: Box<dyn DocSource>) -> Plan {
        // read every node file, in the order the files sort in
        let (files, mut diags) = read_nodes(src, &config.roots);
        // each node file's frontmatter and the line it starts on, to place its inline nodes
        let mut yamls: HashMap<String, (String, usize)> = HashMap::new();
        let mut records = Vec::with_capacity(files.len());
        for file in files {
            yamls.insert(file.rel.clone(), (file.yaml, file.yaml_line));
            records.push(Record {
                spec: file.spec,
                body: file.body,
                order: file.order,
                origin: Origin::File {
                    source: file.rel,
                    line: 1,
                },
            });
        }
        // turn the records into nodes, and put each inline node at the line its id is on
        let mut nodes = nodes_from_records(records);
        for node in nodes.iter_mut().filter(|node| node.inline) {
            if let Origin::File { source, line } = &mut node.origin
                && let Some((yaml, yaml_line)) = yamls.get(source.as_str())
            {
                *line = line_of_id(yaml, &node.id).map_or(1, |l| l + yaml_line - 1);
            }
        }
        // check the nodes as one graph
        let (mut graph, graph_diags) = Graph::build(nodes);
        diags.extend(graph_diags);
        // check their doc links and fill in their brief forms from their docs
        let renderer = Renderer::new(docs, config.sections.clone());
        diags.extend(renderer.prepare(&mut graph));
        Plan {
            graph,
            diags,
            renderer,
        }
    }

    /// Every problem, one an indented line
    pub fn errors(&self) -> String {
        self.diags
            .iter()
            .map(|diag| format!("  {diag}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The viewer's data: every node, its body rendered, its links resolved, each starting
    /// with [`ROOT`]
    ///
    /// # Arguments
    ///
    /// * `chapters` - The book's chapter files, which have pages of their own
    pub fn payload(&self, chapters: &HashSet<String>) -> Payload {
        // a node written in a chapter links to the chapter's page
        let page_of = |node: &Node| {
            node.origin
                .file()
                .filter(|file| chapters.contains(*file))
                .map(|file| format!("{ROOT}{}", paths::html_of(file)))
        };
        self.renderer.payload(&self.graph, page_of)
    }

    /// The viewer's data as JSON, as a page with fences carries it
    ///
    /// # Arguments
    ///
    /// * `chapters` - The book's chapter files, which have pages of their own
    pub fn payload_json(&self, chapters: &HashSet<String>) -> String {
        // every part of a payload is a string, a number, a bool or a list or map of them
        serde_json::to_string(&self.payload(chapters)).expect("a payload serializes")
    }
}

/// The line (0-based within `yaml`, plus one) an inline node's `id:` is on
///
/// # Arguments
///
/// * `yaml` - The file's frontmatter
/// * `id` - The inline node's id
fn line_of_id(yaml: &str, id: &str) -> Option<usize> {
    yaml.lines()
        .position(|line| {
            // an id is written as a list entry's key, in block or flow style
            let trimmed = line
                .trim_start()
                .trim_start_matches("- ")
                .trim_start_matches('{')
                .trim_start();
            let Some(rest) = trimmed.strip_prefix("id:") else {
                return false;
            };
            // its value, up to the next key in a flow map, unquoted
            let value = rest
                .trim()
                .split([',', '}'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\'']);
            value == id
        })
        .map(|line| line + 1)
}

#[cfg(test)]
mod tests {
    use super::{Config, Plan, line_of_id, read_nodes};

    /// An inline node is placed at the line its `id:` is on, in block or flow style
    #[test]
    fn inline_lines() {
        let yaml = "id: g\nnodes:\n  - id: a\n  - { id: \"b\", title: x }\n";
        assert_eq!(line_of_id(yaml, "a"), Some(3));
        assert_eq!(line_of_id(yaml, "b"), Some(4));
    }

    /// Node files are read in the order they sort in, docs are left out, and a file that
    /// can't be parsed is a problem at its line
    #[test]
    fn node_files_read_in_order() {
        let dir = std::env::temp_dir().join(format!("reef-book-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plan")).unwrap();
        // a doc, two node files and one that doesn't parse
        std::fs::write(dir.join("doc.md"), "# A doc\n").unwrap();
        std::fs::write(dir.join("plan/b.md"), "---\nid: b\n---\n# B\n\nIts body.\n").unwrap();
        std::fs::write(
            dir.join("plan/a.md"),
            "---\nid: a\nnodes:\n  - { id: m }\n---\n",
        )
        .unwrap();
        std::fs::write(dir.join("plan/c.md"), "---\nid: c\noptional: maybe\n---\n").unwrap();
        std::fs::write(dir.join("SUMMARY.md"), "---\nid: s\n---\n").unwrap();
        let (nodes, diags) = read_nodes(&dir, &[]);
        // the node files in the order they sort in, each with its body and its members
        let ids: Vec<&str> = nodes.iter().map(|node| node.spec.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(nodes[0].rel, "plan/a.md");
        assert_eq!(nodes[0].spec.nodes[0].id, "m");
        assert_eq!(nodes[1].body, "# B\n\nIts body.\n");
        // their orders count every markdown file but the summary
        assert_eq!((nodes[0].order, nodes[1].order), (1, 2));
        // the broken file is a problem, the summary never a node
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].source, "plan/c.md");
        // and the plan built from them places the member at its line
        let plan = Plan::load(&dir, &Config::default());
        let member = plan.graph.node(plan.graph.index_of("m").unwrap());
        assert_eq!(member.origin.file(), Some("plan/a.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
