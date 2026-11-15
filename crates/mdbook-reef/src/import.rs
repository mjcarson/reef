//! `mdbook-reef import`: turn a mermaid flowchart into reef nodes.
//!
//! Each subgraph becomes a group file in the output directory, with its plain boxes as inline
//! nodes in its `nodes:` list. The boxes outside any subgraph go in a root group file, which also
//! holds a fence that draws the whole thing. `A --> B` becomes `needs: [A]` on B, and `-.->`
//! becomes `helps`. `:::done` or a ✅ in the label means done, and `:::optional` or "(optional)"
//! means optional. A label that starts with an id the book has a heading for (F125, O147, X17,
//! Item 346 and so on) gets that heading as its doc.

use crate::paths;
use crate::plan::{headings_of, markdown_files};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct Args {
    /// The markdown file holding the flowchart.
    file: PathBuf,
    /// Which mermaid fence in the file, counting from 1. Default: the largest.
    #[arg(long)]
    fence: Option<usize>,
    /// The directory to write the nodes to. It should be inside the book's `src`.
    #[arg(long)]
    out: PathBuf,
    /// The root group's id. Default: the output directory's name.
    #[arg(long)]
    root: Option<String>,
    /// The root group's title. Default: the file's first heading.
    #[arg(long)]
    title: Option<String>,
    /// The book's `src` directory, for matching labels to docs. Default: found from book.toml
    /// above the input file.
    #[arg(long)]
    src: Option<PathBuf>,
    /// Do not match labels to docs.
    #[arg(long)]
    no_docs: bool,
    /// Overwrite files that already exist.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MNode {
    pub id: String,
    pub title: String,
    pub labeled: bool,
    pub done: bool,
    pub optional: bool,
    pub group: Option<String>,
    pub needs: Vec<String>,
    pub helps: Vec<String>,
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MGroup {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub needs: Vec<String>,
    pub helps: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Chart {
    pub nodes: Vec<MNode>,
    pub groups: Vec<MGroup>,
    pub report: Vec<String>,
    pub edges: usize,
}

/// The mermaid fences of a markdown file: (first line, body).
fn mermaid_fences(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, String, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        match &mut cur {
            None => {
                if let Some(rest) = t.strip_prefix("```").or_else(|| t.strip_prefix("~~~"))
                    && rest.trim().starts_with("mermaid") {
                        cur = Some((i + 1, String::new(), t[..3].to_string()));
                    }
            }
            Some((start, body, fence)) => {
                if t.starts_with(fence.as_str()) {
                    out.push((*start, std::mem::take(body)));
                    cur = None;
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    out
}

/// Parse a mermaid flowchart.
pub(crate) fn parse(src: &str) -> Chart {
    let mut chart = Chart::default();
    let mut by_id: HashMap<String, usize> = HashMap::new(); // final id -> node index
    let mut alias: HashMap<String, String> = HashMap::new(); // id as written -> final id
    let mut stack: Vec<String> = Vec::new();
    let mut raw_edges: Vec<(String, String, bool, usize)> = Vec::new();
    let mut class_lines: Vec<(Vec<String>, String)> = Vec::new();
    let mut clicks: Vec<(String, String)> = Vec::new();
    for (ln, line) in src.lines().enumerate() {
        let ln = ln + 1;
        let t = line.trim().trim_end_matches(';');
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        let word = t.split_whitespace().next().unwrap_or("");
        match word {
            "flowchart" | "graph" | "classDef" | "direction" | "style" | "linkStyle" => continue,
            "subgraph" => {
                let rest = t["subgraph".len()..].trim();
                let (id, title) = parse_subgraph_head(rest);
                let id = unique(&id, |c| chart.groups.iter().any(|g| g.id == c));
                chart.groups.push(MGroup { id: id.clone(), title, parent: stack.last().cloned(), ..MGroup::default() });
                alias.insert(id.clone(), id.clone());
                stack.push(id);
                continue;
            }
            "end" if t == "end" => {
                stack.pop();
                continue;
            }
            "class" => {
                let mut parts = t.split_whitespace().skip(1);
                if let (Some(ids), Some(cls)) = (parts.next(), parts.next()) {
                    class_lines.push((ids.split(',').map(|s| s.trim().to_string()).collect(), cls.to_string()));
                }
                continue;
            }
            "click" => {
                let mut parts = t.split_whitespace().skip(1);
                if let Some(id) = parts.next() {
                    let rest: String = parts.collect::<Vec<_>>().join(" ");
                    let rest = rest.trim_start_matches("href").trim();
                    if let Some(url) = rest.split('"').nth(1) {
                        clicks.push((id.to_string(), url.to_string()));
                    }
                }
                continue;
            }
            _ => {}
        }
        let stmt = match parse_statement(t) {
            Ok(s) => s,
            Err(e) => {
                chart.report.push(format!("line {ln}: skipped `{t}`: {e}"));
                continue;
            }
        };
        let group = stack.last().cloned();
        // Resolve each reference to a node, defining or redefining it.
        let mut resolved: Vec<Vec<String>> = Vec::new();
        for refs in &stmt.groups {
            let mut ids = Vec::new();
            for r in refs {
                let is_group = chart.groups.iter().any(|g| g.id == r.id);
                let current = alias.get(&r.id).cloned();
                let final_id = match (&r.label, current) {
                    (None, Some(f)) => f,
                    (None, None) if is_group => r.id.clone(),
                    (None, None) => {
                        let idx = chart.nodes.len();
                        chart.nodes.push(MNode { id: r.id.clone(), title: r.id.clone(), group: group.clone(), ..MNode::default() });
                        by_id.insert(r.id.clone(), idx);
                        alias.insert(r.id.clone(), r.id.clone());
                        r.id.clone()
                    }
                    (Some(label), current) => {
                        let (title, done, optional) = clean_label(label);
                        let existing = current.as_ref().and_then(|f| by_id.get(f).copied());
                        match existing {
                            Some(idx) if !chart.nodes[idx].labeled => {
                                let n = &mut chart.nodes[idx];
                                n.title = title;
                                n.labeled = true;
                                n.done |= done;
                                n.optional |= optional;
                                n.group = group.clone();
                                n.id.clone()
                            }
                            Some(idx) if chart.nodes[idx].title == title => chart.nodes[idx].id.clone(),
                            prev => {
                                let id = unique(&r.id, |c| by_id.contains_key(c) || chart.groups.iter().any(|g| g.id == c));
                                if let Some(p) = prev {
                                    chart.report.push(format!(
                                        "line {ln}: `{}` is defined twice (\"{}\" and \"{}\"); the second is `{id}`, and references after line {ln} mean it",
                                        r.id, chart.nodes[p].title, title
                                    ));
                                }
                                let idx = chart.nodes.len();
                                chart.nodes.push(MNode { id: id.clone(), title, labeled: true, done, optional, group: group.clone(), ..MNode::default() });
                                by_id.insert(id.clone(), idx);
                                alias.insert(r.id.clone(), id.clone());
                                id
                            }
                        }
                    }
                };
                if let Some(&idx) = by_id.get(&final_id) {
                    for c in &r.classes {
                        apply_class(&mut chart.nodes[idx], c);
                    }
                }
                ids.push(final_id);
            }
            resolved.push(ids);
        }
        for (k, helps) in stmt.links.iter().enumerate() {
            for a in &resolved[k] {
                for b in &resolved[k + 1] {
                    raw_edges.push((a.clone(), b.clone(), *helps, ln));
                }
            }
        }
    }
    // A bare reference made before its subgraph was defined is that subgraph.
    let group_ids: Vec<String> = chart.groups.iter().map(|g| g.id.clone()).collect();
    chart.nodes.retain(|n| n.labeled || !group_ids.contains(&n.id));
    let by_id: HashMap<String, usize> = chart.nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i)).collect();
    for (ids, cls) in class_lines {
        for id in ids {
            if let Some(f) = alias.get(&id).and_then(|f| by_id.get(f)) {
                apply_class(&mut chart.nodes[*f], &cls);
            }
        }
    }
    for (id, url) in clicks {
        if let Some(f) = alias.get(&id).and_then(|f| by_id.get(f)) {
            chart.nodes[*f].docs.push(url);
        }
    }
    for (a, b, helps, ln) in raw_edges {
        chart.edges += 1;
        let target_node = by_id.get(&b).copied();
        let target_group = chart.groups.iter().position(|g| g.id == b);
        let list = match (target_node, target_group) {
            (Some(i), _) => if helps { &mut chart.nodes[i].helps } else { &mut chart.nodes[i].needs },
            (None, Some(g)) => if helps { &mut chart.groups[g].helps } else { &mut chart.groups[g].needs },
            _ => {
                chart.report.push(format!("line {ln}: an arrow to `{b}`, which is neither a box nor a subgraph"));
                continue;
            }
        };
        if !list.contains(&a) {
            list.push(a);
        }
    }
    chart
}

fn apply_class(n: &mut MNode, cls: &str) {
    match cls {
        "done" => n.done = true,
        "optional" => n.optional = true,
        _ => {}
    }
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|k| format!("{base}_{k}")).find(|c| !taken(c)).unwrap()
}

fn parse_subgraph_head(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if let Some(stripped) = rest.strip_prefix('"') {
        let title = stripped.split('"').next().unwrap_or("").to_string();
        return (slug(&title), clean_label(&title).0);
    }
    let id_end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-')).unwrap_or(rest.len());
    let id = rest[..id_end].to_string();
    let after = rest[id_end..].trim();
    let title = if after.is_empty() {
        id.clone()
    } else {
        let inner = after.trim_start_matches('[').trim_end_matches(']').trim();
        clean_label(inner.trim_matches('"')).0
    };
    if id.is_empty() { (slug(&title), title) } else { (id, title) }
}

fn slug(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// A label's title, and whether it says done or optional.
pub(crate) fn clean_label(label: &str) -> (String, bool, bool) {
    let mut s = label.to_string();
    for br in ["<br>", "<br/>", "<br />", "<BR>"] {
        s = s.replace(br, " ");
    }
    s = s.replace("\\'", "'").replace("&quot;", "\"").replace("#quot;", "\"").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
    let done = s.contains('✅');
    s = s.replace('✅', "");
    let optional = s.contains("(optional)");
    s = s.replace("(optional)", "");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (s, done, optional)
}

struct Ref {
    id: String,
    label: Option<String>,
    classes: Vec<String>,
}

struct Statement {
    /// `A & B --> C`: [[A, B], [C]]
    groups: Vec<Vec<Ref>>,
    /// One per link between groups: whether it is dashed.
    links: Vec<bool>,
}

fn parse_statement(t: &str) -> Result<Statement, String> {
    let b = t.as_bytes();
    let mut i = 0;
    let mut groups = Vec::new();
    let mut links = Vec::new();
    loop {
        let mut refs = Vec::new();
        loop {
            skip_ws(b, &mut i);
            refs.push(parse_ref(t, &mut i)?);
            skip_ws(b, &mut i);
            if i < b.len() && b[i] == b'&' {
                i += 1;
                continue;
            }
            break;
        }
        groups.push(refs);
        skip_ws(b, &mut i);
        if i >= b.len() {
            break;
        }
        links.push(parse_link(t, &mut i)?);
    }
    if groups.len() != links.len() + 1 {
        return Err("a link with nothing after it".into());
    }
    Ok(Statement { groups, links })
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn parse_ref(t: &str, i: &mut usize) -> Result<Ref, String> {
    let b = t.as_bytes();
    let start = *i;
    while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_') {
        *i += 1;
    }
    if *i == start {
        return Err(format!("expected a box id at `{}`", &t[start..]));
    }
    let id = t[start..*i].to_string();
    let mut label = None;
    if *i < b.len() && matches!(b[*i], b'[' | b'(' | b'{' | b'>') {
        // Opening brackets: one to three of them.
        let open_start = *i;
        while *i < b.len() && matches!(b[*i], b'[' | b'(' | b'{' | b'>' | b'/' | b'\\') && *i - open_start < 3 {
            *i += 1;
        }
        let text = if *i < b.len() && b[*i] == b'"' {
            let s = *i + 1;
            let e = t[s..].find('"').map(|k| s + k).ok_or("an unclosed quote")?;
            *i = e + 1;
            t[s..e].to_string()
        } else {
            let s = *i;
            let e = t[s..].find([']', ')', '}']).map(|k| s + k).ok_or("an unclosed label")?;
            *i = e;
            t[s..e].trim_end_matches(['/', '\\']).to_string()
        };
        while *i < b.len() && matches!(b[*i], b']' | b')' | b'}' | b'/' | b'\\') {
            *i += 1;
        }
        label = Some(text);
    }
    let mut classes = Vec::new();
    while t[*i..].starts_with(":::") {
        *i += 3;
        let s = *i;
        while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_' || b[*i] == b'-') {
            *i += 1;
        }
        classes.push(t[s..*i].to_string());
    }
    Ok(Ref { id, label, classes })
}

/// A link: `-->`, `--->`, `==>`, `-.->`, `---`, with an optional `|text|`. Returns whether it is
/// dashed.
fn parse_link(t: &str, i: &mut usize) -> Result<bool, String> {
    let b = t.as_bytes();
    let start = *i;
    if *i < b.len() && b[*i] == b'<' {
        *i += 1;
    }
    while *i < b.len() && matches!(b[*i], b'-' | b'.' | b'=') {
        *i += 1;
    }
    if *i < b.len() && b[*i] == b'>' {
        *i += 1;
    }
    let link = &t[start..*i];
    if link.len() < 2 || !(link.contains("--") || link.contains("==") || link.contains("-.")) {
        return Err(format!("expected a link at `{}`", &t[start..]));
    }
    skip_ws(b, i);
    if *i < b.len() && b[*i] == b'|' {
        let e = t[*i + 1..].find('|').ok_or("an unclosed link label")?;
        *i += e + 2;
    }
    Ok(link.contains('.'))
}

/// Ids at the start of labels that a book's headings are numbered by: `F125`, `O147`, `X17`,
/// `Q62`, `M23`, and `Item 346` (numbered `346.`).
fn label_key(title: &str) -> Option<String> {
    let first = title.split_whitespace().next()?;
    if first == "Item" || first == "Resolved" {
        let num = title.split_whitespace().nth(1)?.trim_end_matches([':', ',', '.']);
        return num.chars().all(|c| c.is_ascii_digit()).then(|| num.to_string());
    }
    let first = first.trim_end_matches([':', ',', '.']);
    let letters = first.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let digits = first[letters..].chars().take_while(|c| c.is_ascii_digit()).count();
    ((1..=2).contains(&letters) && digits >= 1 && letters + digits == first.len()).then(|| first.to_string())
}

/// For each key, the best heading numbered by it: a page's title over a section in a longer
/// page.
fn doc_index(src: &Path) -> HashMap<String, (u8, String, Option<String>)> {
    let mut idx: HashMap<String, (u8, String, Option<String>)> = HashMap::new();
    for rel in markdown_files(src, &[]) {
        let Ok(text) = std::fs::read_to_string(src.join(&rel)) else { continue };
        let body = reef_model::spec::frontmatter_is_yaml(&text).map_or(text.as_str(), |fm| fm.body);
        for (id, heading, level) in headings_of(body) {
            let Some(key) = heading.split_whitespace().next().and_then(|w| w.strip_suffix('.')) else { continue };
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let anchor = (level > 1).then_some(id);
            let better = idx.get(key).is_none_or(|(l, _, _)| level < *l);
            if better {
                idx.insert(key.to_string(), (level, rel.clone(), anchor));
            }
        }
    }
    idx
}

pub fn run(args: Args) -> Result<()> {
    let text = std::fs::read_to_string(&args.file).with_context(|| format!("reading {}", args.file.display()))?;
    let fences = mermaid_fences(&text);
    if fences.is_empty() {
        bail!("{} has no mermaid fences", args.file.display());
    }
    let (line, body) = match args.fence {
        Some(k) if k >= 1 && k <= fences.len() => fences[k - 1].clone(),
        Some(k) => bail!("{} has {} mermaid fences, not {k}", args.file.display(), fences.len()),
        None => fences.iter().max_by_key(|f| f.1.len()).cloned().unwrap(),
    };
    let mut chart = parse(&body);
    eprintln!(
        "fence at line {line}: {} boxes, {} subgraphs, {} arrows",
        chart.nodes.len(),
        chart.groups.len(),
        chart.edges
    );

    let src = match &args.src {
        Some(s) => Some(s.clone()),
        None => find_src(&args.file),
    };
    let out_abs = std::path::absolute(&args.out)?;
    let out_rel = src.as_ref().and_then(|s| std::path::absolute(s).ok()).and_then(|s| out_abs.strip_prefix(&s).ok().map(|p| p.to_string_lossy().replace('\\', "/")));
    if !args.no_docs {
        match (&src, &out_rel) {
            (Some(s), Some(out_rel)) => {
                let idx = doc_index(s);
                let mut linked = 0;
                for n in &mut chart.nodes {
                    if let Some((_, file, anchor)) = label_key(&n.title).and_then(|k| idx.get(&k)) {
                        let mut link = paths::relative(out_rel, file);
                        if let Some(a) = anchor {
                            link = format!("{link}#{a}");
                        }
                        n.docs.push(link);
                        linked += 1;
                    }
                }
                eprintln!("linked {linked} boxes to the headings they are numbered by");
            }
            _ => eprintln!("not linking docs: --out is not inside the book's src (pass --src, or --no-docs)"),
        }
    }

    let root_id = args.root.clone().unwrap_or_else(|| out_abs.file_name().map_or("plan".into(), |n| n.to_string_lossy().to_string()));
    let title = args.title.clone().or_else(|| reef_model::spec::first_heading(&text)).unwrap_or_else(|| root_id.clone());
    let source_name = args.file.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string());

    let files = render_files(&chart, &root_id, &title, &source_name, line);
    std::fs::create_dir_all(&args.out)?;
    for (name, content) in &files {
        let path = args.out.join(name);
        if path.exists() && !args.force {
            bail!("{} exists; pass --force to overwrite", path.display());
        }
        std::fs::write(&path, content)?;
    }
    eprintln!("wrote {} files to {}", files.len(), args.out.display());
    for r in &chart.report {
        eprintln!("note: {r}");
    }
    Ok(())
}

fn find_src(file: &Path) -> Option<PathBuf> {
    let mut dir = std::path::absolute(file).ok()?.parent()?.to_path_buf();
    loop {
        let toml = dir.join("book.toml");
        if toml.is_file() {
            let cfg = mdbook_preprocessor::config::Config::from_disk(&toml).ok()?;
            return Some(dir.join(cfg.book.src));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// The files the import writes: a root group file and one file per subgraph.
pub(crate) fn render_files(chart: &Chart, root: &str, title: &str, source: &str, line: usize) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let q = |s: &str| serde_json::to_string(s).unwrap();
    let list = |v: &[String]| format!("[{}]", v.iter().map(|s| q(s)).collect::<Vec<_>>().join(", "));
    let write_nodes = |out: &mut String, group: Option<&str>| {
        let members: Vec<&MNode> = chart.nodes.iter().filter(|n| n.group.as_deref() == group).collect();
        if members.is_empty() {
            return;
        }
        out.push_str("nodes:\n");
        for n in members {
            let _ = writeln!(out, "  - id: {}", q(&n.id));
            let _ = writeln!(out, "    title: {}", q(&n.title));
            if n.done {
                out.push_str("    status: done\n");
            }
            if n.optional {
                out.push_str("    optional: true\n");
            }
            if !n.needs.is_empty() {
                let _ = writeln!(out, "    needs: {}", list(&n.needs));
            }
            if !n.helps.is_empty() {
                let _ = writeln!(out, "    helps: {}", list(&n.helps));
            }
            if !n.docs.is_empty() {
                let _ = writeln!(out, "    docs: {}", list(&n.docs));
            }
        }
    };
    let note = format!("Imported from the mermaid diagram at line {line} of `{source}` by `mdbook-reef import`.");

    let mut r = String::from("---\n");
    let _ = writeln!(r, "id: {}", q(root));
    let _ = writeln!(r, "title: {}", q(title));
    r.push_str("collapsed: false\n");
    write_nodes(&mut r, None);
    let _ = write!(r, "---\n# {title}\n\n{note}\n\n```reef\nroot = {}\n```\n", q(root));
    files.insert(format!("{root}.md"), r);

    for g in &chart.groups {
        let mut s = String::from("---\n");
        let _ = writeln!(s, "id: {}", q(&g.id));
        let _ = writeln!(s, "title: {}", q(&g.title));
        let _ = writeln!(s, "group: {}", q(g.parent.as_deref().unwrap_or(root)));
        if !g.needs.is_empty() {
            let _ = writeln!(s, "needs: {}", list(&g.needs));
        }
        if !g.helps.is_empty() {
            let _ = writeln!(s, "helps: {}", list(&g.helps));
        }
        write_nodes(&mut s, Some(&g.id));
        let _ = write!(s, "---\n# {}\n\n{note}\n", g.title);
        files.insert(format!("{}.md", g.id), s);
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"flowchart TB
    classDef done fill:#2e7d32
    Contract["✅ Agree the contract,<br>P20 to P26"]:::done
    subgraph w1["W1's rows"]
        direction LR
        Pace["✅ Table reads paced"]:::done
        Admit["An admission class (optional)"]:::optional
    end
    subgraph m24["M24's steps"]
        Pace["✅ F147 A scan's answers<br>paced by its link"]:::done
        S1["F140 The scan frames"]
    end
    M24["M24 Scans"]
    Contract --> w1
    Contract & Admit --> S1 -.-> Pace --> M24
    m24 --> M24
    X1 ---> X2
"#;

    #[test]
    fn parses_a_chart() {
        let c = parse(SAMPLE);
        let n = |id: &str| c.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("{id}: {:?}", c.nodes));
        assert_eq!(n("Contract").title, "Agree the contract, P20 to P26");
        assert!(n("Contract").done);
        assert!(n("Admit").optional);
        assert_eq!(n("Admit").title, "An admission class");
        assert_eq!(n("Pace").group.as_deref(), Some("w1"));
        assert_eq!(n("Pace_2").group.as_deref(), Some("m24"));
        assert_eq!(n("S1").needs, vec!["Contract", "Admit"]);
        assert_eq!(n("Pace_2").helps, vec!["S1"]);
        assert_eq!(n("M24").needs, vec!["Pace_2", "m24"]);
        assert_eq!(n("X2").needs, vec!["X1"]);
        let w1 = c.groups.iter().find(|g| g.id == "w1").unwrap();
        assert_eq!(w1.needs, vec!["Contract"]);
        assert!(c.report.iter().any(|r| r.contains("`Pace` is defined twice")), "{:?}", c.report);
    }

    #[test]
    fn keys() {
        assert_eq!(label_key("F147 A scan").as_deref(), Some("F147"));
        assert_eq!(label_key("Item 346: paced").as_deref(), Some("346"));
        assert_eq!(label_key("O146 Sixteen").as_deref(), Some("O146"));
        assert_eq!(label_key("The exit's rounds"), None);
    }

    #[test]
    fn files_round_trip_through_the_model() {
        let c = parse(SAMPLE);
        let files = render_files(&c, "plan", "Plan", "x.md", 1);
        let mut nodes = Vec::new();
        for (name, text) in &files {
            let (spec, fm) = reef_model::spec::parse_node_file(text).unwrap().unwrap_or_else(|| panic!("{name}"));
            reef_model::Node::from_spec(spec, name, 1, fm.body.to_string(), &mut nodes);
        }
        let (g, d) = reef_model::Graph::build(nodes);
        assert!(d.is_empty(), "{d:?}");
        assert!(g.index_of("Pace_2").is_some());
        assert_eq!(g.node(g.index_of("w1").unwrap()).parent.as_deref(), Some("plan"));
    }
}
