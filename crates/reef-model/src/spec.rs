//! What a node's author writes: the frontmatter of a markdown file, or one entry in a group's
//! `nodes:` list.

use serde::{Deserialize, Deserializer, Serialize};

/// Where a piece of work stands. A group's status is derived from its members unless it names
/// one itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    #[serde(alias = "todo", alias = "to-do", alias = "open")]
    Todo,
    #[serde(alias = "doing", alias = "wip", alias = "started", alias = "in_progress")]
    InProgress,
    #[serde(alias = "complete", alias = "completed", alias = "finished")]
    Done,
    Blocked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in-progress",
            Status::Done => "done",
            Status::Blocked => "blocked",
        }
    }
}

/// One node as written. Unknown keys are ignored so a file's frontmatter can carry other
/// tools' keys (an Obsidian `tags:` list, say) beside reef's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Unique across the book. The only required key.
    pub id: String,
    /// Shown on the box. Defaults to the file's first heading, then to the id.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
    /// Drawn dashed. Optional work helps rather than gates.
    #[serde(default)]
    pub optional: bool,
    /// The group this node belongs to. Inline nodes belong to the node that lists them.
    #[serde(default, alias = "parent")]
    pub group: Option<String>,
    /// Work that must be done before this can start: a solid arrow from each.
    #[serde(default, deserialize_with = "one_or_many", alias = "after", alias = "requires")]
    pub needs: Vec<String>,
    /// Work that helps this but does not gate it: a dashed arrow from each.
    #[serde(default, deserialize_with = "one_or_many")]
    pub helps: Vec<String>,
    /// Links to the docs that describe this work, relative to the file that names them.
    #[serde(default, deserialize_with = "one_or_many", alias = "doc", alias = "links")]
    pub docs: Vec<String>,
    /// Whether this group starts collapsed. Groups start collapsed unless this says `false`.
    #[serde(default)]
    pub collapsed: Option<bool>,
    /// A brief form of the work, as markdown, shown first in the sidebar. Default: taken from
    /// the first doc that has one.
    #[serde(default)]
    pub description: Option<String>,
    /// The design choices, as markdown: one string, or a list of them drawn as bullets.
    /// Default: taken from the first doc that has them.
    #[serde(default, deserialize_with = "one_or_many", alias = "design")]
    pub decisions: Vec<String>,
    /// How the work changed performance: metrics measured before and after it, drawn in the
    /// sidebar as a table and charts.
    #[serde(default, alias = "perf")]
    pub performance: Option<crate::perf::Performance>,
    /// Markdown shown in the sidebar for an inline node. A file node's body is its file.
    #[serde(default)]
    pub body: Option<String>,
    /// Members with no file of their own.
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
        Null(()),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
        OneOrMany::Null(()) => Vec::new(),
    })
}

/// A file's frontmatter split from its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frontmatter<'a> {
    /// The YAML between the fences.
    pub yaml: &'a str,
    /// Everything after the closing fence.
    pub body: &'a str,
    /// The 1-based line the YAML starts on.
    pub yaml_line: usize,
    /// The byte offset in the file where the body starts.
    pub body_offset: usize,
}

/// Split a leading `---` fenced YAML block from a markdown file. The block must open on the
/// first line and close with a line of `---` or `...`.
pub fn split_frontmatter(text: &str) -> Option<Frontmatter<'_>> {
    let text_no_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - text_no_bom.len();
    let first_end = text_no_bom.find('\n')?;
    if text_no_bom[..first_end].trim_end() != "---" {
        return None;
    }
    let yaml_start = first_end + 1;
    let mut pos = yaml_start;
    while pos <= text_no_bom.len() {
        let end = text_no_bom[pos..].find('\n').map_or(text_no_bom.len(), |i| pos + i);
        let line = text_no_bom[pos..end].trim_end();
        if line == "---" || line == "..." {
            let body_start = (end + 1).min(text_no_bom.len());
            return Some(Frontmatter {
                yaml: &text_no_bom[yaml_start..pos],
                body: &text_no_bom[body_start..],
                yaml_line: 2,
                body_offset: bom + body_start,
            });
        }
        if end == text_no_bom.len() {
            break;
        }
        pos = end + 1;
    }
    None
}

/// An error in a file's frontmatter, with the file line it was found on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    pub line: Option<usize>,
    pub message: String,
}

/// Parse a file's frontmatter as a node. A file whose frontmatter has no `id` is not a node and
/// gives `Ok(None)`, as does a file with no frontmatter.
#[cfg(feature = "parse")]
pub fn parse_node_file(text: &str) -> Result<Option<(NodeSpec, Frontmatter<'_>)>, SpecError> {
    let Some(fm) = split_frontmatter(text) else {
        return Ok(None);
    };
    let value: serde_yaml_ng::Value = match serde_yaml_ng::from_str(fm.yaml) {
        Ok(v) => v,
        // Not YAML at all: not ours either.
        Err(_) => return Ok(None),
    };
    let has_id = value
        .as_mapping()
        .is_some_and(|m| m.contains_key(serde_yaml_ng::Value::String("id".into())));
    if !has_id {
        return Ok(None);
    }
    match serde_yaml_ng::from_str::<NodeSpec>(fm.yaml) {
        Ok(spec) => Ok(Some((spec, fm))),
        Err(e) => Err(SpecError {
            line: e.location().map(|l| l.line() + fm.yaml_line - 1),
            message: e.to_string(),
        }),
    }
}

/// Whether a file's frontmatter is a YAML mapping, so that it is safe to strip before mdbook
/// renders the page.
#[cfg(feature = "parse")]
pub fn frontmatter_is_yaml(text: &str) -> Option<Frontmatter<'_>> {
    let fm = split_frontmatter(text)?;
    match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(fm.yaml) {
        Ok(serde_yaml_ng::Value::Mapping(_)) => Some(fm),
        _ => None,
    }
}

/// The text of a markdown file's first ATX heading, if it has one.
pub fn first_heading(body: &str) -> Option<String> {
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim_start_matches('#');
            if rest.starts_with(' ') || rest.is_empty() {
                let h = rest.trim().trim_end_matches('#').trim();
                if !h.is_empty() {
                    return Some(h.to_string());
                }
            }
        }
    }
    None
}

#[cfg(all(test, feature = "parse"))]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter() {
        let fm = split_frontmatter("---\nid: a\n---\n# Hi\n").unwrap();
        assert_eq!(fm.yaml, "id: a\n");
        assert_eq!(fm.body, "# Hi\n");
        assert!(split_frontmatter("# no\n---\n").is_none());
        assert!(split_frontmatter("---\nid: a\n").is_none());
    }

    #[test]
    fn parses_a_node() {
        let text = "---\nid: m23\nstatus: done\nneeds: m22\nnodes:\n  - { id: x, needs: [m23a, m23b] }\n---\nbody";
        let (spec, fm) = parse_node_file(text).unwrap().unwrap();
        assert_eq!(spec.id, "m23");
        assert_eq!(spec.status, Some(Status::Done));
        assert_eq!(spec.needs, vec!["m22"]);
        assert_eq!(spec.nodes[0].needs, vec!["m23a", "m23b"]);
        assert_eq!(fm.body, "body");
    }

    #[test]
    fn description_and_decisions() {
        let (spec, _) = parse_node_file("---\nid: a\ndescription: Short.\ndecisions: One.\n---\n").unwrap().unwrap();
        assert_eq!(spec.description.as_deref(), Some("Short."));
        assert_eq!(spec.decisions, vec!["One."]);
        let (spec, _) = parse_node_file("---\nid: a\ndesign: [x, y]\n---\n").unwrap().unwrap();
        assert_eq!(spec.decisions, vec!["x", "y"]);
    }

    #[test]
    fn performance() {
        let text = "---\nid: a\nperformance:\n  labels: [Buffered, Shared]\n  metrics:\n    - name: inserts\n      better: higher\n      before: 3523\n      after: [6837, 6783, 6915]\n    - name: p99\n      points: [[1, 2, 3], [4, null, 5]]\n---\n";
        let (spec, _) = parse_node_file(text).unwrap().unwrap();
        let p = spec.performance.unwrap();
        assert_eq!(p.after_label(), "Shared");
        assert_eq!(p.metrics[0].better, crate::perf::Better::Higher);
        assert_eq!(p.metrics[0].after, Some(crate::perf::Value::Range([6837.0, 6783.0, 6915.0])));
        assert_eq!(p.metrics[1].points[1], (4.0, None, Some(5.0)));
        let err = parse_node_file("---\nid: a\nperformance:\n  metrics:\n    - name: x\n      before: [1, 2]\n---\n").unwrap_err();
        assert!(err.line.is_some(), "{err:?}");
    }

    #[test]
    fn not_a_node_without_id() {
        assert!(parse_node_file("---\ntitle: x\n---\n").unwrap().is_none());
        assert!(parse_node_file("no frontmatter").unwrap().is_none());
    }

    #[test]
    fn reports_the_file_line() {
        let err = parse_node_file("---\nid: a\nstatus: nope\n---\n").unwrap_err();
        assert_eq!(err.line, Some(3), "{err:?}");
    }

    #[test]
    fn heading() {
        assert_eq!(first_heading("```\n# no\n```\n## F1. Yes ##\n").as_deref(), Some("F1. Yes"));
    }
}
