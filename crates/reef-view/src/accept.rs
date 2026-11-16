//! Acceptance criteria in the viewer: which kinds the reader shows, the sidebar's Acceptance
//! section, and the pills a box shows for them. Pure, so it is tested natively.
//!
//! Each kind has its own colour (`reef.css`), and every badge also names its kind in words, so
//! colour never carries the meaning alone.

use crate::markup::esc;
use crate::perf::{self, ChartRef, ChartView};
use reef_model::{AccChart, Graph, Kind, Met};
use std::fmt::Write;

/// Which kinds of criteria are shown, in [`Kind::ALL`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filter(pub [bool; 4]);

impl Default for Filter {
    fn default() -> Self {
        Filter([true; 4])
    }
}

impl Filter {
    /// Where the filter is kept: one for every view, on every page of the book.
    pub const STORAGE_KEY: &'static str = "reef:acceptance";

    pub fn shows(&self, k: Kind) -> bool {
        self.0[k.index()]
    }

    pub fn toggled(mut self, k: Kind) -> Filter {
        self.0[k.index()] = !self.0[k.index()];
        self
    }

    /// The shown kinds as stored: `unit,perf`.
    pub fn to_stored(self) -> String {
        Kind::ALL.iter().filter(|k| self.shows(**k)).map(|k| k.as_str()).collect::<Vec<_>>().join(",")
    }

    pub fn from_stored(s: &str) -> Filter {
        let mut f = Filter([false; 4]);
        for k in s.split(',').filter_map(|w| Kind::ALL.into_iter().find(|k| k.as_str() == w.trim())) {
            f.0[k.index()] = true;
        }
        f
    }
}

/// The kind a `data-act="acc:<kind>"` names.
pub fn act_kind(act: &str) -> Option<Kind> {
    let k = act.strip_prefix("acc:")?;
    Kind::ALL.into_iter().find(|kind| kind.as_str() == k)
}

fn plural_label(k: Kind) -> &'static str {
    match k {
        Kind::Unit => "unit tests",
        Kind::Integration => "integration tests",
        Kind::Performance => "performance criteria",
        Kind::Feature => "features",
    }
}

/// One filter chip: shows or hides kind `k` everywhere. `count` is `(met, total)`, if shown.
pub fn chip(k: Kind, f: &Filter, count: Option<(usize, usize)>) -> String {
    let on = f.shows(k);
    let count = count.map_or(String::new(), |(m, t)| format!(r#" <span class="reef-acc-count">{m}/{t}</span>"#));
    format!(
        r#"<button class="reef-acc-chip k-{kind}{cls}" data-act="acc:{kind}" aria-pressed="{on}" title="{verb} {plural}">{label}{count}</button>"#,
        kind = k.as_str(),
        cls = if on { " on" } else { "" },
        verb = if on { "Hide" } else { "Show" },
        plural = plural_label(k),
        label = k.label(),
    )
}

/// The toolbar's filter: a chip for each kind.
pub fn toolbar(f: &Filter) -> String {
    let chips: String = Kind::ALL.iter().map(|&k| chip(k, f, None)).collect();
    format!(r#"<span class="reef-acc-filter" role="group" aria-label="Acceptance criteria shown">{chips}</span>"#)
}

fn state(m: Met) -> (&'static str, &'static str) {
    match m {
        Met::Met => ("✓", "met"),
        Met::Unmet => ("✗", "not met"),
        Met::Todo => ("○", "not checked yet"),
    }
}

/// The sidebar's Acceptance section for node `i`, or nothing when it has no criteria.
pub fn section(g: &Graph, i: usize, f: &Filter) -> String {
    let n = g.node(i);
    if n.acceptance.is_empty() {
        return String::new();
    }
    let all = g.acceptance_counts(i, &[true; 4], false);
    let mut body = String::from(r#"<div class="reef-acc-chips">"#);
    for k in Kind::ALL {
        let c = all[k.index()];
        if c.1 > 0 {
            body.push_str(&chip(k, f, Some(c)));
        }
    }
    body.push_str("</div><ul class=\"reef-acc-list\">");
    let (mut met, mut shown, mut hidden) = (0, 0, 0);
    for (c, (crit, r)) in n.acceptance.iter().zip(g.acceptance(i)).enumerate() {
        if !f.shows(crit.kind) {
            hidden += 1;
            continue;
        }
        shown += 1;
        met += usize::from(r.status == Met::Met);
        let (mark, word) = state(r.status);
        let kind = crit.kind.as_str();
        let _ = write!(
            body,
            r#"<li class="reef-acc-item k-{kind} reef-acc-{st}"><div class="reef-acc-line"><span class="reef-acc-state" title="{word}" aria-label="{word}">{mark}</span><span class="reef-acc-badge k-{kind}">{label}</span><span class="reef-acc-text">{text}</span></div>"#,
            st = r.status.as_str(),
            label = crit.kind.label(),
            text = crit.text,
        );
        let mut meta = Vec::new();
        if let Some(t) = &crit.test {
            meta.push(format!("<code>{}</code>", esc(t)));
        }
        if let Some((j, ch)) = r.chart {
            let p = g.node(j).performance.as_ref();
            let what = match ch {
                AccChart::Metric(k) => p.and_then(|p| p.metrics.get(k)).map_or(String::new(), |m| esc(&m.name)),
                AccChart::Change => "every metric's change".into(),
                AccChart::Pair => "before and after".into(),
            };
            let from = if j == i {
                String::new()
            } else {
                format!(r#" of <a href="javascript:void 0" data-goto="{}">{}</a>"#, esc(&g.node(j).id), esc(&g.node(j).id))
            };
            meta.push(format!("{what}{from}"));
            if let AccChart::Metric(k) = ch
                && let Some(t) = crit.target_text(p.and_then(|p| p.metrics.get(k)).and_then(|m| m.unit.as_deref()))
            {
                meta.push(format!("target {}", esc(&t)));
            }
        }
        if !meta.is_empty() {
            let _ = write!(body, r#"<div class="reef-acc-meta">{}</div>"#, meta.join(" · "));
        }
        if r.chart.is_some() {
            body.push_str(&perf::chart(g, i, &ChartRef::Accept(c), &ChartView::default()));
        }
        body.push_str("</li>");
    }
    body.push_str("</ul>");
    if hidden > 0 {
        let _ = write!(body, r#"<p class="reef-note">{hidden} hidden by the filter.</p>"#);
    }
    let head = format!(r#"Acceptance <span class="reef-note">{met} of {shown} met</span>"#);
    crate::markup::sec(&head, "reef-acc", &body)
}

/// A box's pills: for each shown kind it has criteria of, `(kind, met, total)`. A collapsed
/// group counts its members' too.
pub fn pills(g: &Graph, i: usize, f: &Filter, deep: bool) -> Vec<(Kind, usize, usize)> {
    let c = g.acceptance_counts(i, &f.0, deep);
    Kind::ALL.into_iter().filter(|k| c[k.index()].1 > 0).map(|k| (k, c[k.index()].0, c[k.index()].1)).collect()
}

/// A pill's text: `U 2/3`.
pub fn pill_text(k: Kind, met: usize, total: usize) -> String {
    format!("{} {met}/{total}", k.letter())
}

/// A pill's tooltip: `unit tests: 2 of 3 met`.
pub fn pill_title(k: Kind, met: usize, total: usize) -> String {
    format!("{}: {met} of {total} met", plural_label(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_model::{Criterion, Metric, Node, Performance, Value};

    fn plan() -> Graph {
        let crit = |kind, text: &str| Criterion { kind, text: text.into(), ..Criterion::default() };
        let a = Node {
            id: "a".into(),
            title: "A".into(),
            performance: Some(Performance {
                metrics: vec![Metric { name: "inserts".into(), unit: Some("/s".into()), before: Some(Value::One(3.0)), after: Some(Value::One(7.0)), ..Metric::default() }],
                ..Performance::default()
            }),
            acceptance: vec![
                crit(Kind::Unit, "Answers <em>at once</em>"),
                Criterion { test: Some("tests/a.rs".into()), status: Some(Met::Met), ..crit(Kind::Integration, "Ends") },
                Criterion { metric: Some("inserts".into()), at_least: Some(6.0), ..crit(Kind::Performance, "Doubles") },
            ],
            ..Node::default()
        };
        let g = Node { id: "g".into(), title: "G".into(), ..Node::default() };
        let a = Node { parent: Some("g".into()), ..a };
        Graph::build(vec![g, a]).0
    }

    #[test]
    fn stores_the_filter() {
        let f = Filter::default().toggled(Kind::Integration);
        assert_eq!(f.to_stored(), "unit,perf,feature");
        assert_eq!(Filter::from_stored(&f.to_stored()), f);
        assert_eq!(Filter::from_stored(""), Filter([false; 4]));
        assert_eq!(act_kind("acc:perf"), Some(Kind::Performance));
        assert_eq!(act_kind("fit"), None);
    }

    #[test]
    fn the_section() {
        let g = plan();
        let a = g.index_of("a").unwrap();
        let s = section(&g, a, &Filter::default());
        assert!(s.contains("2 of 3 met"), "{s}");
        assert!(s.contains(r#"class="reef-acc-badge k-unit">unit test<"#), "{s}");
        assert!(s.contains("Answers <em>at once</em>"), "text is HTML already: {s}");
        assert!(s.contains("<code>tests/a.rs</code>"), "{s}");
        assert!(s.contains("target ≥ 6 /s"), "{s}");
        assert!(s.contains(r#"data-chart="accept" data-arg="2""#), "{s}");
        assert!(s.contains("reef-acc-target"), "the target is drawn: {s}");
        let s = section(&g, a, &Filter::default().toggled(Kind::Unit));
        assert!(s.contains("2 of 2 met") && s.contains("1 hidden by the filter"), "{s}");
        assert!(s.contains(r#"aria-pressed="false""#), "{s}");
        assert!(section(&g, g.index_of("g").unwrap(), &Filter::default()).is_empty());
    }

    #[test]
    fn pills_count_members_when_collapsed() {
        let g = plan();
        let gi = g.index_of("g").unwrap();
        assert!(pills(&g, gi, &Filter::default(), false).is_empty());
        let p = pills(&g, gi, &Filter::default(), true);
        assert_eq!(p, vec![(Kind::Unit, 0, 1), (Kind::Integration, 1, 1), (Kind::Performance, 1, 1)]);
        assert_eq!(pill_text(Kind::Performance, 1, 1), "P 1/1");
        let hidden = pills(&g, gi, &Filter::default().toggled(Kind::Unit), true);
        assert_eq!(hidden.len(), 2);
    }

    #[test]
    fn the_criterion_chart_answers_the_pointer() {
        let g = plan();
        let a = g.index_of("a").unwrap();
        let r = ChartRef::Accept(2);
        let svg = perf::chart(&g, a, &r, &ChartView::default());
        assert!(svg.contains(r#"data-metric="0""#), "{svg}");
        let h = perf::hover(&g, a, &r, &ChartView::default(), 100.0, 30.0).expect("a hover over the bars");
        assert_eq!(h.metric, Some(0));
        assert!(!perf::zoomable(&g, a, &r));
        assert!(perf::chart(&g, a, &ChartRef::Accept(0), &ChartView::default()).is_empty());
    }

    #[test]
    fn a_sweep_criterion_zooms_and_draws_its_target() {
        let sweep = Metric { name: "decode".into(), x: Some("KiB".into()), points: vec![(1.0, Some(1.2), Some(0.5)), (64.0, Some(31.0), Some(2.4))], ..Metric::default() };
        let owner = Node { id: "o".into(), title: "O".into(), performance: Some(Performance { metrics: vec![sweep], ..Performance::default() }), ..Node::default() };
        let c = Criterion { kind: Kind::Performance, text: "fast".into(), node: Some("o".into()), metric: Some("decode".into()), at_most: Some(3.0), ..Criterion::default() };
        let n = Node { id: "n".into(), title: "N".into(), acceptance: vec![c], ..Node::default() };
        let (g, d) = Graph::build(vec![owner, n]);
        assert!(d.is_empty(), "{d:?}");
        let i = g.index_of("n").unwrap();
        assert_eq!(g.acceptance(i)[0].status, Met::Met, "every after point is under 3");
        let r = ChartRef::Accept(0);
        let svg = perf::chart(&g, i, &r, &ChartView::default());
        assert!(svg.contains("reef-perf-zoomable") && svg.contains("target 3"), "{svg}");
        assert!(perf::zoomable(&g, i, &r));
        assert!(perf::plot_area(&g, i, &r, &ChartView::default()).is_some());
        let s = section(&g, i, &Filter::default());
        assert!(s.contains(r#"data-goto="o""#), "a borrowed metric links its node: {s}");
    }
}
