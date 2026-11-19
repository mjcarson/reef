//! What a node's author writes: the frontmatter of a markdown file, or one entry in a group's
//! `nodes:` list.

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
///
/// It serializes as canonical frontmatter: its keys in the order below, each left out at its
/// default, and acceptance criteria in their short form (`- unit: <text>`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Unique across the book. The only required key.
    pub id: String,
    /// Shown on the box. Defaults to the file's first heading, then to the id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Where the work stands, as its author says. Default: to do, or for a group, what its
    /// members add up to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    /// Drawn dashed. Optional work helps rather than gates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// The group this node belongs to. Inline nodes belong to the node that lists them.
    #[serde(default, alias = "parent", skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Work that must be done before this can start: a solid arrow from each.
    #[serde(default, deserialize_with = "one_or_many", alias = "after", alias = "requires", skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Work that helps this but does not gate it: a dashed arrow from each.
    #[serde(default, deserialize_with = "one_or_many", skip_serializing_if = "Vec::is_empty")]
    pub helps: Vec<String>,
    /// Links to the docs that describe this work, relative to the file that names them.
    #[serde(default, deserialize_with = "one_or_many", alias = "doc", alias = "links", skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
    /// Whether this group starts collapsed. Groups start collapsed unless this says `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// Why the work is needed and what finishing it will do, shown first in the sidebar, with
    /// a longer explanation below them. A plain string is the longer explanation alone. A part
    /// not written is taken from the first doc that has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Description>,
    /// The design choices, as markdown: one string, or a list of them drawn as bullets.
    /// Default: taken from the first doc that has them.
    #[serde(default, deserialize_with = "one_or_many", alias = "design", skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<String>,
    /// How the work changed performance: metrics measured before and after it, drawn in the
    /// sidebar as a table and charts.
    #[serde(default, alias = "perf", skip_serializing_if = "Option::is_none")]
    pub performance: Option<crate::perf::Performance>,
    /// How the work will be known to be done: unit and integration tests, performance figures
    /// and features that must work.
    #[serde(default, alias = "criteria", serialize_with = "short_criteria", skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<crate::accept::Criterion>,
    /// Markdown shown in the sidebar for an inline node. A file node's body is its file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Members with no file of their own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeSpec>,
}

#[cfg(feature = "parse")]
impl NodeSpec {
    /// Print this spec as canonical YAML frontmatter, without its `---` fences
    ///
    /// Reading what it prints with [`NodeSpec::from_yaml`] gives back the same spec.
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::NodeSpec;
    ///
    /// let spec = NodeSpec::from_yaml("id: C4\nneeds: C1\nacceptance:\n  - { unit: Round-trips }\n").unwrap();
    /// assert_eq!(spec.to_yaml(), "id: C4\nneeds:\n- C1\nacceptance:\n- unit: Round-trips\n");
    /// ```
    pub fn to_yaml(&self) -> String {
        // every key is a string, a number, a bool or a list or map of them, which YAML can
        // always hold
        serde_yaml_ng::to_string(self).expect("a node spec prints as YAML")
    }

    /// Read a spec from YAML frontmatter, without its `---` fences
    ///
    /// # Arguments
    ///
    /// * `yaml` - The frontmatter, as `reef node put` or a store keeps it
    pub fn from_yaml(yaml: &str) -> Result<NodeSpec, SpecError> {
        serde_yaml_ng::from_str(yaml).map_err(|e| SpecError { line: e.location().map(|l| l.line()), message: e.to_string() })
    }
}

/// Write acceptance criteria in the short form a book writes them in: each criterion's kind
/// as its key, holding its text, with any of its other keys beside it
///
/// # Arguments
///
/// * `criteria` - The criteria to write
/// * `s` - The serializer to write them to
fn short_criteria<S: Serializer>(criteria: &[crate::accept::Criterion], s: S) -> Result<S::Ok, S::Error> {
    /// One criterion in its short form
    struct Short<'a>(&'a crate::accept::Criterion);

    impl Serialize for Short<'_> {
        /// Write the criterion's kind and text, then each other key it has
        ///
        /// # Arguments
        ///
        /// * `s` - The serializer to write it to
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let c = self.0;
            let mut map = s.serialize_map(None)?;
            // its kind's key holds its text
            map.serialize_entry(c.kind.as_str(), &c.text)?;
            // then each other key it has, in the order the reference gives them
            if let Some(status) = &c.status {
                map.serialize_entry("status", status)?;
            }
            for (key, value) in [("test", &c.test), ("metric", &c.metric), ("node", &c.node), ("chart", &c.chart)] {
                if let Some(value) = value {
                    map.serialize_entry(key, value)?;
                }
            }
            for (key, value) in [("at-least", &c.at_least), ("at-most", &c.at_most)] {
                if let Some(value) = value {
                    map.serialize_entry(key, value)?;
                }
            }
            map.end()
        }
    }

    // write each criterion in turn
    s.collect_seq(criteria.iter().map(Short))
}

/// Why a piece of work is needed, what finishing it will do, and a longer explanation, each as
/// markdown.
///
/// Written as a map of `why`, `what` and `details`, or as a plain string, which is `details`
/// alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Description {
    /// Why the plan needs this work: the problem it solves or the gap it fills
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// What will be true, or possible, once this work is done
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    /// A longer explanation, shown below the why and the what
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// The keys a description's map form may hold.
const DESCRIPTION_KEYS: [&str; 3] = ["why", "what", "details"];

impl<'de> Deserialize<'de> for Description {
    /// Read a description from a plain string or from a map of its parts
    ///
    /// # Arguments
    ///
    /// * `d` - The deserializer to read the description from
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        use serde_json::Value as J;
        // read the description as a generic value so both of its forms can be told apart
        let map = match J::deserialize(d)? {
            // a plain string is the longer explanation alone
            J::String(s) => return Ok(Description { details: Some(s), ..Description::default() }),
            // an empty description says nothing
            J::Null => return Ok(Description::default()),
            J::Object(m) => m,
            _ => return Err(D::Error::custom("a description is a string, or a map of `why`, `what` and `details`")),
        };
        // refuse a key that is not one of the parts, so a typo is not silently dropped
        if let Some(key) = map.keys().find(|k| !DESCRIPTION_KEYS.contains(&k.as_str())) {
            return Err(D::Error::custom(format!("description has no key `{key}`: its keys are `why`, `what` and `details`")));
        }
        // take each part, which must be a string when it is given
        let part = |key: &str| -> Result<Option<String>, D::Error> {
            match map.get(key) {
                None | Some(J::Null) => Ok(None),
                Some(J::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(D::Error::custom(format!("description `{key}:` is markdown text"))),
            }
        };
        Ok(Description { why: part("why")?, what: part("what")?, details: part("details")? })
    }
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
        // a plain string is the longer explanation alone
        let (spec, _) = parse_node_file("---\nid: a\ndescription: Short.\ndecisions: One.\n---\n").unwrap().unwrap();
        let d = spec.description.unwrap();
        assert_eq!((d.why, d.what, d.details.as_deref()), (None, None, Some("Short.")));
        assert_eq!(spec.decisions, vec!["One."]);
        // the map form holds each part
        let text = "---\nid: a\ndescription:\n  why: Slow.\n  what: Fast.\n  details: |\n    More.\n---\n";
        let (spec, _) = parse_node_file(text).unwrap().unwrap();
        let d = spec.description.unwrap();
        assert_eq!((d.why.as_deref(), d.what.as_deref(), d.details.as_deref()), (Some("Slow."), Some("Fast."), Some("More.\n")));
        // the flow form an inline node uses
        let (spec, _) = parse_node_file("---\nid: g\nnodes:\n  - { id: b, description: { what: Done. } }\n---\n").unwrap().unwrap();
        assert_eq!(spec.nodes[0].description.as_ref().unwrap().what.as_deref(), Some("Done."));
        // a key that is not a part is refused, at the description's line
        let err = parse_node_file("---\nid: a\ndescription:\n  wy: Typo.\n---\n").unwrap_err();
        assert!(err.message.contains("`wy`") && err.message.contains("`why`, `what` and `details`"), "{err:?}");
        assert!(err.line.is_some(), "{err:?}");
        // a part that is not text is refused
        assert!(parse_node_file("---\nid: a\ndescription: { why: [x] }\n---\n").is_err());
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
    fn acceptance() {
        let text = "---\nid: a\nacceptance:\n  - unit: Empty answers at once\n  - integration: { text: Ends, test: tests/scan.rs }\n  - feature: Survives a restart\n    status: met\n  - kind: performance\n    text: Doubles\n    metric: inserts\n    at-least: 6000\n---\n";
        let (spec, _) = parse_node_file(text).unwrap().unwrap();
        let a = &spec.acceptance;
        assert_eq!(a.len(), 4);
        assert_eq!(a[0].kind, crate::accept::Kind::Unit);
        assert_eq!(a[1].test.as_deref(), Some("tests/scan.rs"));
        assert_eq!(a[2].status, Some(crate::accept::Met::Met));
        assert_eq!((a[3].kind, a[3].at_least), (crate::accept::Kind::Performance, Some(6000.0)));
        assert!(parse_node_file("---\nid: a\nacceptance:\n  - Just text\n---\n").is_err());
    }

    #[test]
    fn prints_canonical_yaml() {
        let text = "---\nnodes:\n  - { id: m, body: \"Line one.\\n\\nLine two.\\n\" }\nid: g\ntitle: \"G: a group\"\nstatus: wip\noptional: true\nparent: top\nneeds: a\nhelps: [b]\ndoc: x.md#y\ncollapsed: false\ndescription: { why: Slow., what: Fast. }\ndesign: One way.\nperf:\n  metrics:\n    - { name: n, better: higher, before: 2, after: [4, 3, 5] }\ncriteria:\n  - unit: Answers at once\n  - kind: performance\n    text: Doubles\n    metric: n\n    at-least: 4\n  - feature: { text: Survives, status: met, test: t.rs }\ntags: [ignored]\n---\n";
        let (spec, _) = parse_node_file(text).unwrap().unwrap();
        // every key in order under its own name, aliases and unknown keys gone, criteria short
        let yaml = spec.to_yaml();
        let expected = "id: g\ntitle: 'G: a group'\nstatus: in-progress\noptional: true\ngroup: top\nneeds:\n- a\nhelps:\n- b\ndocs:\n- x.md#y\ncollapsed: false\ndescription:\n  why: Slow.\n  what: Fast.\ndecisions:\n- One way.\nperformance:\n  metrics:\n  - name: n\n    better: higher\n    before: 2.0\n    after:\n    - 4.0\n    - 3.0\n    - 5.0\nacceptance:\n- unit: Answers at once\n- perf: Doubles\n  metric: n\n  at-least: 4.0\n- feature: Survives\n  status: met\n  test: t.rs\nnodes:\n- id: m\n  body: |\n    Line one.\n\n    Line two.\n";
        assert_eq!(yaml, expected);
        // and it reads back as the same spec
        assert_eq!(NodeSpec::from_yaml(&yaml).unwrap(), spec);
        // a spec with nothing but its id prints just that
        assert_eq!(NodeSpec { id: "a".into(), ..NodeSpec::default() }.to_yaml(), "id: a\n");
        // a value that would read as another type is quoted
        let odd = NodeSpec { id: "123".into(), title: Some("yes".into()), ..NodeSpec::default() };
        assert_eq!(NodeSpec::from_yaml(&odd.to_yaml()).unwrap(), odd);
        // a problem in what is read names its line
        assert_eq!(NodeSpec::from_yaml("id: a\nstatus: nope\n").unwrap_err().line, Some(2));
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
