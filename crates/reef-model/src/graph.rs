//! The whole plan as one graph: nodes, the group tree, dependencies, and what follows from them
//! (a group's status and progress, and which work is ready to start).

use crate::accept::{AccChart, Criterion, Kind, Met, Resolved};
use crate::perf::Performance;
use crate::spec::{NodeSpec, Status};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A link to a doc, resolved for the page that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub href: String,
}

/// A doc section with a node's measurements in it, found at build time and read when the
/// sidebar opens it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerfDoc {
    /// The doc section's link, as in [`Link::href`].
    pub href: String,
    /// The doc section's own label.
    pub label: String,
    /// The heading id of the subsection to show. `None`: show the section's tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// The subsection's title, or the count of tables to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_text: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tables: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// One node of the plan, as the viewer receives it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    /// The status the author wrote, if any. See [`Graph::status`] for the effective one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub helps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// The file this node was written in, relative to the book's `src`.
    #[serde(default)]
    pub source: String,
    /// The line of that file the node starts on.
    #[serde(default)]
    pub line: usize,
    /// Whether this node is an entry in another node's `nodes:` list rather than a file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inline: bool,
    /// The doc links as written, relative to `source`.
    #[serde(skip)]
    pub docs_raw: Vec<String>,
    /// The body as markdown.
    #[serde(skip)]
    pub body: String,
    /// The description as written, if any.
    #[serde(skip)]
    pub description_raw: Option<String>,
    /// The design choices as written.
    #[serde(skip)]
    pub decisions_raw: Vec<String>,
    /// The description rendered: written, or taken from a doc.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The design choices rendered: written, or taken from a doc.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub design: String,
    /// Where the design choices were taken from: the doc heading's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_from: Option<String>,
    /// The body rendered for the page that shows it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub html: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<Link>,
    /// How the work changed performance, as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<Performance>,
    /// A doc section that holds the work's measurements, read in the sidebar when it is opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perf_doc: Option<PerfDoc>,
    /// How the work will be known to be done. Each criterion's text is markdown as written, and
    /// HTML in the viewer's payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<Criterion>,
    /// The node's own page, when its file is a chapter of the book.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

impl Node {
    /// Flatten a spec and its inline members into nodes. `source` is the file it came from.
    pub fn from_spec(spec: NodeSpec, source: &str, line: usize, body: String, out: &mut Vec<Node>) {
        let NodeSpec { id, title, status, optional, group, needs, helps, docs, collapsed, description, decisions, performance, acceptance, body: inline_body, nodes } = spec;
        let title = title
            .or_else(|| crate::spec::first_heading(&body))
            .unwrap_or_else(|| id.clone());
        let body = inline_body.unwrap_or(body);
        let parent_id = id.clone();
        out.push(Node {
            id,
            title,
            status,
            optional,
            parent: group,
            needs,
            helps,
            collapsed,
            source: source.to_string(),
            line,
            inline: false,
            docs_raw: docs,
            description_raw: description,
            decisions_raw: decisions,
            performance,
            acceptance,
            body,
            ..Node::default()
        });
        for mut child in nodes {
            if child.group.is_none() {
                child.group = Some(parent_id.clone());
            }
            let start = out.len();
            Node::from_spec(child, source, line, String::new(), out);
            for n in &mut out[start..] {
                n.inline = true;
            }
        }
    }
}

/// A problem found in the plan, with where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: String,
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "{}:{}: {}", self.source, l, self.message),
            None => write!(f, "{}: {}", self.source, self.message),
        }
    }
}

/// The kind of an arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// Required: drawn solid.
    Needs,
    /// Helps but does not gate: drawn dashed.
    Helps,
}

/// The plan, indexed.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    index: HashMap<String, usize>,
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    needs: Vec<Vec<usize>>,
    helps: Vec<Vec<usize>>,
    status: Vec<Status>,
    ready: Vec<bool>,
    progress: Vec<(usize, usize)>,
    accept: Vec<Vec<Resolved>>,
}

impl Graph {
    /// Index the nodes and check them. A problem is reported and the offending part dropped, so
    /// the graph that comes back is always consistent: no duplicate ids, no dangling references,
    /// no cycles.
    pub fn build(nodes: Vec<Node>) -> (Graph, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let mut g = Graph::default();
        for node in nodes {
            if node.id.trim().is_empty() {
                diags.push(diag(&node, "a node with an empty id".into()));
                continue;
            }
            if let Some(&first) = g.index.get(&node.id) {
                let first = &g.nodes[first];
                diags.push(diag(
                    &node,
                    format!("duplicate id `{}`, first defined at {}:{}", node.id, first.source, first.line),
                ));
                continue;
            }
            if let Some(p) = &node.performance {
                for m in p.problems() {
                    diags.push(diag(&node, m));
                }
            }
            g.index.insert(node.id.clone(), g.nodes.len());
            g.nodes.push(node);
        }
        let n = g.nodes.len();
        g.parent = vec![None; n];
        g.children = vec![Vec::new(); n];
        for i in 0..n {
            if let Some(p) = g.nodes[i].parent.clone() {
                match g.index.get(&p) {
                    Some(&pi) if pi != i => g.parent[i] = Some(pi),
                    Some(_) => diags.push(diag(&g.nodes[i], format!("`{p}` is its own group"))),
                    None => diags.push(diag(&g.nodes[i], format!("group `{p}` is not a node"))),
                }
            }
        }
        // Break cycles in the group tree.
        for i in 0..n {
            let mut seen = vec![i];
            let mut cur = g.parent[i];
            while let Some(c) = cur {
                if seen.contains(&c) {
                    diags.push(diag(&g.nodes[i], format!("group cycle through `{}`", g.nodes[i].id)));
                    g.parent[i] = None;
                    break;
                }
                seen.push(c);
                cur = g.parent[c];
            }
        }
        for i in 0..n {
            match g.parent[i] {
                Some(p) => g.children[p].push(i),
                None => g.roots.push(i),
            }
        }
        g.needs = vec![Vec::new(); n];
        g.helps = vec![Vec::new(); n];
        for i in 0..n {
            for (kind, list) in [(EdgeKind::Needs, g.nodes[i].needs.clone()), (EdgeKind::Helps, g.nodes[i].helps.clone())] {
                let key = if kind == EdgeKind::Needs { "needs" } else { "helps" };
                for d in list {
                    let Some(&di) = g.index.get(&d) else {
                        diags.push(diag(&g.nodes[i], format!("{key} `{d}`, which is not a node")));
                        continue;
                    };
                    if di == i {
                        diags.push(diag(&g.nodes[i], format!("`{d}` {key} itself")));
                    } else if g.is_ancestor(di, i) || g.is_ancestor(i, di) {
                        diags.push(diag(
                            &g.nodes[i],
                            format!("{key} `{d}`, but one contains the other; draw the arrow to a member instead"),
                        ));
                    } else {
                        let v = if kind == EdgeKind::Needs { &mut g.needs[i] } else { &mut g.helps[i] };
                        if !v.contains(&di) {
                            v.push(di);
                        }
                    }
                }
            }
        }
        g.break_cycles(&mut diags);
        g.resolve_acceptance(&mut diags);
        g.derive();
        (g, diags)
    }

    /// Detect cycles in the order work can happen in. Each node has a start and a finish: a
    /// node starts after what it needs (or is helped by) finishes, a member starts after its
    /// group starts, and a group finishes after its members finish.
    fn break_cycles(&mut self, diags: &mut Vec<Diagnostic>) {
        let n = self.nodes.len();
        loop {
            // Events: 2i = start(i), 2i+1 = finish(i).
            let mut succ: Vec<Vec<(usize, Option<(usize, usize, EdgeKind)>)>> = vec![Vec::new(); 2 * n];
            for i in 0..n {
                succ[2 * i].push((2 * i + 1, None));
                for &d in &self.needs[i] {
                    succ[2 * d + 1].push((2 * i, Some((i, d, EdgeKind::Needs))));
                }
                for &d in &self.helps[i] {
                    succ[2 * d + 1].push((2 * i, Some((i, d, EdgeKind::Helps))));
                }
                if let Some(p) = self.parent[i] {
                    succ[2 * p].push((2 * i, None));
                    succ[2 * i + 1].push((2 * p + 1, None));
                }
            }
            let Some(cycle) = find_cycle(&succ) else { return };
            // Drop the last dependency edge on the cycle and report the cycle.
            let edges: Vec<(usize, usize, EdgeKind)> = cycle.iter().filter_map(|e| *e).collect();
            let Some(&(i, d, kind)) = edges.last() else { return };
            let path: Vec<String> = edges.iter().map(|(i, d, _)| format!("{} -> {}", self.nodes[*d].id, self.nodes[*i].id)).collect();
            diags.push(diag(&self.nodes[i], format!("dependency cycle: {}", path.join(", "))));
            match kind {
                EdgeKind::Needs => self.needs[i].retain(|&x| x != d),
                EdgeKind::Helps => self.helps[i].retain(|&x| x != d),
            }
        }
    }

    /// Check each acceptance criterion's references, and work out whether it is met: the
    /// author's word, or else its metric's `after` figure against its target.
    fn resolve_acceptance(&mut self, diags: &mut Vec<Diagnostic>) {
        let mut all = Vec::with_capacity(self.nodes.len());
        for i in 0..self.nodes.len() {
            let mut out = Vec::new();
            for (k, c) in self.nodes[i].acceptance.iter().enumerate() {
                let mut problem = |m: String| diags.push(diag(&self.nodes[i], format!("acceptance {}: {m}", k + 1)));
                if c.text.trim().is_empty() {
                    problem("has no text".into());
                }
                let refers = c.metric.is_some() || c.chart.is_some() || c.has_target() || c.node.is_some();
                let mut r = Resolved { status: c.status.unwrap_or_default(), chart: None };
                if c.kind != Kind::Performance {
                    if refers {
                        problem(format!("a {} criterion takes no `metric`, `chart`, `node` or target; those are for `performance`", c.kind.label()));
                    }
                    out.push(r);
                    continue;
                }
                if c.metric.is_some() && c.chart.is_some() {
                    problem("names both `metric` and `chart`; a criterion draws one".into());
                }
                if c.has_target() && c.metric.is_none() {
                    problem("has a target but no `metric` to check it against".into());
                }
                let j = match &c.node {
                    None => Some(i),
                    Some(id) => match self.index.get(id) {
                        Some(&j) => Some(j),
                        None => {
                            problem(format!("node `{id}` is not a node"));
                            None
                        }
                    },
                };
                if let Some(j) = j {
                    let perf = self.nodes[j].performance.as_ref();
                    let id = &self.nodes[j].id;
                    if let Some(name) = &c.metric {
                        let metrics = perf.map_or(&[][..], |p| p.metrics.as_slice());
                        let found = metrics.iter().position(|m| m.key() == name).or_else(|| metrics.iter().position(|m| &m.name == name));
                        match found {
                            Some(m) => {
                                r.chart = Some((j, AccChart::Metric(m)));
                                if c.status.is_none()
                                    && let Some(ok) = c.check(&metrics[m])
                                {
                                    r.status = if ok { Met::Met } else { Met::Unmet };
                                }
                            }
                            None => problem(format!("metric `{name}` is not in `{id}`'s performance")),
                        }
                    } else if let Some(chart) = &c.chart {
                        match chart.as_str() {
                            "change" => r.chart = Some((j, AccChart::Change)),
                            "pair" | "before-after" => r.chart = Some((j, AccChart::Pair)),
                            other => problem(format!("chart `{other}` is not `change` or `pair`")),
                        }
                        if r.chart.is_some() && perf.is_none_or(|p| p.metrics.is_empty()) {
                            problem(format!("chart `{chart}`, but `{id}` has no performance metrics"));
                            r.chart = None;
                        }
                    }
                }
                out.push(r);
            }
            all.push(out);
        }
        self.accept = all;
    }

    fn derive(&mut self) {
        let n = self.nodes.len();
        self.status = vec![Status::Todo; n];
        self.progress = vec![(0, 0); n];
        // Children before parents.
        let order = self.post_order();
        for &i in &order {
            if self.children[i].is_empty() {
                let s = self.nodes[i].status.unwrap_or_default();
                self.status[i] = s;
                self.progress[i] = (usize::from(s == Status::Done), 1);
                continue;
            }
            let (mut done, mut total) = (0, 0);
            for &c in &self.children[i] {
                done += self.progress[c].0;
                total += self.progress[c].1;
            }
            self.progress[i] = (done, total);
            self.status[i] = match self.nodes[i].status {
                Some(s) => s,
                None => {
                    let kids = &self.children[i];
                    let required: Vec<usize> = kids.iter().copied().filter(|&c| !self.nodes[c].optional).collect();
                    let counted = if required.is_empty() { kids.clone() } else { required };
                    if counted.iter().all(|&c| self.status[c] == Status::Done) {
                        Status::Done
                    } else if kids.iter().any(|&c| matches!(self.status[c], Status::Done | Status::InProgress)) {
                        Status::InProgress
                    } else if kids.iter().any(|&c| self.status[c] == Status::Blocked) {
                        Status::Blocked
                    } else {
                        Status::Todo
                    }
                }
            };
        }
        self.ready = (0..n)
            .map(|i| {
                if self.status[i] != Status::Todo || !self.children[i].is_empty() {
                    return false;
                }
                let mut cur = Some(i);
                while let Some(c) = cur {
                    if self.needs[c].iter().any(|&d| self.status[d] != Status::Done) {
                        return false;
                    }
                    cur = self.parent[c];
                }
                true
            })
            .collect();
    }

    fn post_order(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<(usize, bool)> = self.roots.iter().rev().map(|&r| (r, false)).collect();
        while let Some((i, expanded)) = stack.pop() {
            if expanded {
                out.push(i);
            } else {
                stack.push((i, true));
                for &c in self.children[i].iter().rev() {
                    stack.push((c, false));
                }
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }
    pub fn node(&self, i: usize) -> &Node {
        &self.nodes[i]
    }
    pub fn parent(&self, i: usize) -> Option<usize> {
        self.parent[i]
    }
    pub fn children(&self, i: usize) -> &[usize] {
        &self.children[i]
    }
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }
    pub fn is_group(&self, i: usize) -> bool {
        !self.children[i].is_empty()
    }
    pub fn needs(&self, i: usize) -> &[usize] {
        &self.needs[i]
    }
    pub fn helps(&self, i: usize) -> &[usize] {
        &self.helps[i]
    }
    /// The effective status: the author's, or for a group, what its members add up to.
    pub fn status(&self, i: usize) -> Status {
        self.status[i]
    }
    /// Not started, and everything it and its groups need is done.
    pub fn is_ready(&self, i: usize) -> bool {
        self.ready[i]
    }
    /// Done and total work items under a node (a leaf counts itself).
    pub fn progress(&self, i: usize) -> (usize, usize) {
        self.progress[i]
    }
    /// Node `i`'s acceptance criteria resolved, one for each of `node(i).acceptance`.
    pub fn acceptance(&self, i: usize) -> &[Resolved] {
        &self.accept[i]
    }
    /// For each kind in [`Kind::ALL`] that `shown` lets through: how many of node `i`'s criteria
    /// are met, and how many there are. `deep` counts its members' too.
    pub fn acceptance_counts(&self, i: usize, shown: &[bool; 4], deep: bool) -> [(usize, usize); 4] {
        let mut out = [(0, 0); 4];
        let mut add = |n: usize| {
            for (c, r) in self.nodes[n].acceptance.iter().zip(&self.accept[n]) {
                let k = c.kind.index();
                if shown[k] {
                    out[k].0 += usize::from(r.status == Met::Met);
                    out[k].1 += 1;
                }
            }
        };
        add(i);
        if deep {
            for d in self.descendants(i) {
                add(d);
            }
        }
        out
    }
    /// Whether any node has acceptance criteria.
    pub fn has_acceptance(&self) -> bool {
        self.nodes.iter().any(|n| !n.acceptance.is_empty())
    }
    /// Whether `a` is a strict ancestor (enclosing group) of `b`.
    pub fn is_ancestor(&self, a: usize, b: usize) -> bool {
        let mut cur = self.parent[b];
        while let Some(c) = cur {
            if c == a {
                return true;
            }
            cur = self.parent[c];
        }
        false
    }
    /// Groups enclosing `i`, innermost first.
    pub fn ancestors(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.parent[i];
        while let Some(c) = cur {
            out.push(c);
            cur = self.parent[c];
        }
        out
    }
    /// Every node that needs or is helped by `i`.
    pub fn dependents(&self, i: usize) -> Vec<(usize, EdgeKind)> {
        let mut out = Vec::new();
        for j in 0..self.nodes.len() {
            if self.needs[j].contains(&i) {
                out.push((j, EdgeKind::Needs));
            } else if self.helps[j].contains(&i) {
                out.push((j, EdgeKind::Helps));
            }
        }
        out
    }
    /// Every node in the subtree under `i`, `i` excluded.
    pub fn descendants(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = self.children[i].clone();
        while let Some(c) = stack.pop() {
            out.push(c);
            stack.extend_from_slice(&self.children[c]);
        }
        out
    }

    /// The nodes under `i` in the order the work happens: a topological order over what each
    /// needs or is helped by, its enclosing groups' arrows included, ties broken by the order
    /// the nodes were written in.
    pub fn plan_order(&self, i: usize) -> Vec<usize> {
        let mut set = self.descendants(i);
        set.sort_unstable();
        let n = set.len();
        // What each member waits for: its own arrows and its groups' up to `i`.
        let waits: Vec<Vec<usize>> = set
            .iter()
            .map(|&v| {
                let mut out = Vec::new();
                let mut cur = Some(v);
                while let Some(c) = cur.filter(|&c| c != i) {
                    out.extend(self.needs[c].iter().chain(&self.helps[c]).copied());
                    cur = self.parent[c];
                }
                out
            })
            .collect();
        let mut succ = vec![Vec::new(); n];
        let mut indeg = vec![0usize; n];
        for (kv, w) in waits.iter().enumerate() {
            for (ku, &u) in set.iter().enumerate() {
                if ku != kv && w.iter().any(|&d| d == u || self.is_ancestor(d, u)) && !self.is_ancestor(u, set[kv]) {
                    succ[ku].push(kv);
                    indeg[kv] += 1;
                }
            }
        }
        let mut ready: std::collections::BTreeSet<usize> = (0..n).filter(|&k| indeg[k] == 0).collect();
        let mut out = Vec::with_capacity(n);
        while let Some(k) = ready.pop_first() {
            out.push(set[k]);
            for &s in &succ[k] {
                indeg[s] -= 1;
                if indeg[s] == 0 {
                    ready.insert(s);
                }
            }
        }
        // Anything left is on a cycle the build already reported: keep it, in written order.
        if out.len() < n {
            let seen: std::collections::HashSet<usize> = out.iter().copied().collect();
            out.extend(set.iter().copied().filter(|v| !seen.contains(v)));
        }
        out
    }
}

fn diag(node: &Node, message: String) -> Diagnostic {
    Diagnostic { source: node.source.clone(), line: Some(node.line), message }
}

/// Find any cycle in a directed graph, returned as the edge labels along it.
fn find_cycle<L: Copy>(succ: &[Vec<(usize, L)>]) -> Option<Vec<L>> {
    let n = succ.len();
    // 0 = unvisited, 1 = on stack, 2 = done
    let mut state = vec![0u8; n];
    let mut via: Vec<Option<(usize, L)>> = vec![None; n];
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        state[s] = 1;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if *next < succ[v].len() {
                let (w, label) = succ[v][*next];
                *next += 1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        via[w] = Some((v, label));
                        stack.push((w, 0));
                    }
                    1 => {
                        // Walk back from v to w.
                        let mut labels = vec![label];
                        let mut cur = v;
                        while cur != w {
                            let (p, l) = via[cur].expect("on the stack");
                            labels.push(l);
                            cur = p;
                        }
                        labels.reverse();
                        return Some(labels);
                    }
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn node(id: &str, parent: Option<&str>, needs: &[&str], status: Option<Status>) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            status,
            source: "t.md".into(),
            ..Node::default()
        }
    }

    #[test]
    fn plan_order() {
        let (g, d) = Graph::build(vec![
            node("top", None, &[], None),
            node("c", Some("top"), &["b"], None),
            node("a", Some("top"), &[], None),
            node("sub", Some("top"), &["a"], None),
            node("b", Some("sub"), &[], None),
            node("z", Some("top"), &[], None),
        ]);
        assert!(d.is_empty(), "{d:?}");
        let ids: Vec<&str> = g.plan_order(g.index_of("top").unwrap()).into_iter().map(|k| g.node(k).id.as_str()).collect();
        // b waits for a through its group; c waits for b; z waits for nothing.
        assert_eq!(ids, ["a", "sub", "b", "c", "z"]);
    }

    #[test]
    fn performance_is_checked() {
        use crate::perf::{Metric, Performance};
        let mut n = node("a", None, &[], None);
        n.line = 4;
        n.performance = Some(Performance { metrics: vec![Metric { name: "x".into(), ..Metric::default() }], ..Performance::default() });
        let (_, d) = Graph::build(vec![n]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].to_string(), "t.md:4: performance: metric `x` has neither `before`/`after` nor `points`");
    }

    #[test]
    fn acceptance_is_resolved() {
        use crate::perf::{Metric, Performance, Value};
        let crit = |kind, metric: Option<&str>, node: Option<&str>, at_least: Option<f64>| Criterion {
            kind,
            text: "t".into(),
            metric: metric.map(Into::into),
            node: node.map(Into::into),
            at_least,
            ..Criterion::default()
        };
        let mut a = node("a", None, &[], None);
        a.performance = Some(Performance {
            metrics: vec![Metric { name: "rw50, inserts".into(), key: Some("inserts".into()), after: Some(Value::One(6837.0)), before: Some(Value::One(3523.0)), ..Metric::default() }],
            ..Performance::default()
        });
        a.acceptance = vec![
            crit(Kind::Performance, Some("inserts"), None, Some(6000.0)),
            crit(Kind::Performance, Some("rw50, inserts"), None, Some(9000.0)),
            Criterion { status: Some(Met::Met), ..crit(Kind::Performance, Some("inserts"), None, Some(9000.0)) },
            crit(Kind::Unit, None, None, None),
            Criterion { chart: Some("change".into()), ..crit(Kind::Performance, None, None, None) },
        ];
        let mut b = node("b", None, &[], None);
        b.acceptance = vec![
            crit(Kind::Performance, Some("inserts"), Some("a"), Some(6000.0)),
            crit(Kind::Performance, Some("nope"), Some("a"), None),
            crit(Kind::Performance, None, Some("missing"), None),
            crit(Kind::Feature, Some("inserts"), None, None),
            crit(Kind::Performance, None, None, Some(1.0)),
            Criterion { text: " ".into(), ..crit(Kind::Unit, None, None, None) },
        ];
        let (g, d) = Graph::build(vec![a, b]);
        let r = g.acceptance(0);
        assert_eq!(r[0], Resolved { status: Met::Met, chart: Some((0, AccChart::Metric(0))) });
        assert_eq!(r[1].status, Met::Unmet);
        assert_eq!(r[2].status, Met::Met, "the author's word wins");
        assert_eq!(r[3], Resolved::default());
        assert_eq!(r[4].chart, Some((0, AccChart::Change)));
        assert_eq!(g.acceptance(1)[0], Resolved { status: Met::Met, chart: Some((0, AccChart::Metric(0))) });
        let msgs: Vec<String> = d.iter().map(|d| d.message.clone()).collect();
        assert_eq!(msgs.len(), 5, "{msgs:?}");
        assert!(msgs[0].contains("acceptance 2: metric `nope` is not in `a`'s performance"), "{msgs:?}");
        assert!(msgs[1].contains("node `missing`"), "{msgs:?}");
        assert!(msgs[2].contains("a feature criterion takes no"), "{msgs:?}");
        assert!(msgs[3].contains("no `metric`"), "{msgs:?}");
        assert!(msgs[4].contains("no text"), "{msgs:?}");
        let all = [true; 4];
        assert_eq!(g.acceptance_counts(0, &all, false), [(0, 1), (0, 0), (2, 4), (0, 0)]);
        assert_eq!(g.acceptance_counts(0, &[true, true, false, true], false), [(0, 1), (0, 0), (0, 0), (0, 0)]);
    }

    #[test]
    fn status_and_ready() {
        let (g, d) = Graph::build(vec![
            node("a", None, &[], Some(Status::Done)),
            node("grp", None, &["a"], None),
            node("x", Some("grp"), &[], Some(Status::Done)),
            node("y", Some("grp"), &["x"], None),
            node("z", Some("grp"), &["y"], None),
            node("gate", None, &["grp"], None),
        ]);
        assert!(d.is_empty(), "{d:?}");
        let i = |s| g.index_of(s).unwrap();
        assert_eq!(g.status(i("grp")), Status::InProgress);
        assert_eq!(g.progress(i("grp")), (1, 3));
        assert!(g.is_ready(i("y")));
        assert!(!g.is_ready(i("z")));
        assert!(!g.is_ready(i("gate")));
    }

    #[test]
    fn group_needs_gate_members() {
        let (g, _) = Graph::build(vec![
            node("a", None, &[], None),
            node("grp", None, &["a"], None),
            node("x", Some("grp"), &[], None),
        ]);
        assert!(!g.is_ready(g.index_of("x").unwrap()));
        assert!(g.is_ready(g.index_of("a").unwrap()));
    }

    #[test]
    fn reports_problems() {
        let (g, d) = Graph::build(vec![
            node("a", None, &["b", "missing"], None),
            node("b", None, &["a"], None),
            node("a", None, &[], None),
            node("g", None, &[], None),
            node("m", Some("g"), &["g"], None),
            node("o", Some("nope"), &[], None),
        ]);
        let msgs: Vec<String> = d.iter().map(|d| d.message.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains("duplicate id `a`")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("`missing`")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("dependency cycle")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("contains the other")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("group `nope`")), "{msgs:?}");
        assert_eq!(g.len(), 5);
    }

    #[test]
    fn cycle_through_groups() {
        // y (in grp) needs gate, gate needs grp: grp can't finish before y starts.
        let (_, d) = Graph::build(vec![
            node("grp", None, &[], None),
            node("y", Some("grp"), &["gate"], None),
            node("gate", None, &["grp"], None),
        ]);
        assert!(d.iter().any(|d| d.message.contains("dependency cycle")), "{d:?}");
    }
}
