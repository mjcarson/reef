//! The sidebar's Performance section: how a piece of work changed what was measured, as a
//! table and charts drawn as inline SVG. Pure, so it is tested natively.
//!
//! The charts are interactive, the way a dashboard's are. Each `<svg>` names its chart and its
//! view (hidden series, a zoomed x range) in `data-` attributes, and `chartui` asks this module
//! for what to draw as the pointer moves:
//!
//! - **Hover:** a crosshair snapped to the nearest x on line charts, the row under the pointer on
//!   bar charts, and a tooltip with every series' figure there.
//! - **Legend:** a click shows that series alone, and a second click shows them all again.
//!   Ctrl- or cmd-click hides or shows just that one.
//! - **Zoom:** a drag across a line chart zooms to that x range, and a double-click or *Reset
//!   zoom* goes back.
//! - **Linked rows:** a metric's table row, its change bar and its before/after bars light up
//!   together.
//!
//! Colours are classes only; `reef.css` gives them from the book's theme, so a theme switch
//! needs no redraw. Before is orange and after blue, a pair checked for colour-blind
//! separation in light and dark. A change that improved is blue and one that regressed is red,
//! and every one also says so in words, so colour never carries the meaning alone.

use crate::markup::esc;
use reef_model::perf::{self, Better, Metric, Performance, Value};
use reef_model::{AccChart, Graph, Node};
use std::fmt::Write;

/// The charts' width in SVG units. They scale to the sidebar's width.
pub const W: f64 = 360.0;
/// About how wide a character of the charts' 11px text is, for fitting labels.
const CHAR_W: f64 = 6.0;

/// Which chart of a node's section.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartRef {
    /// Every metric's change on one scale.
    Change,
    /// Every metric's two figures, each on its own scale.
    Pair,
    /// A sweep: the metric at this index of the node's metrics.
    Sweep(usize),
    /// A group's members, followed for the metric with this key.
    Trend(String),
    /// The chart the node's acceptance criterion at this index draws: a metric of its own or
    /// another node's, or a whole chart, with the criterion's target drawn on it.
    Accept(usize),
}

impl ChartRef {
    fn attrs(&self, i: usize) -> String {
        let (kind, arg) = match self {
            ChartRef::Change => ("change", String::new()),
            ChartRef::Pair => ("pair", String::new()),
            ChartRef::Sweep(k) => ("sweep", k.to_string()),
            ChartRef::Trend(key) => ("trend", key.clone()),
            ChartRef::Accept(c) => ("accept", c.to_string()),
        };
        format!(r#" data-node="{i}" data-chart="{kind}" data-arg="{}""#, esc(&arg))
    }

    /// The chart a `data-chart` and `data-arg` name.
    pub fn parse(kind: &str, arg: &str) -> Option<ChartRef> {
        Some(match kind {
            "change" => ChartRef::Change,
            "pair" => ChartRef::Pair,
            "sweep" => ChartRef::Sweep(arg.parse().ok()?),
            "trend" => ChartRef::Trend(arg.to_string()),
            "accept" => ChartRef::Accept(arg.parse().ok()?),
            _ => return None,
        })
    }

    /// Whether a drag zooms it: the charts with an x axis. A criterion's chart is one when it
    /// draws a sweep, which takes the graph to tell; see [`zoomable`].
    pub fn zoomable(&self) -> bool {
        matches!(self, ChartRef::Sweep(_) | ChartRef::Trend(_))
    }
}

/// What a reader has done to a chart: series hidden from its legend, and an x range zoomed to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartView {
    pub hidden: Vec<usize>,
    pub x: Option<(f64, f64)>,
}

impl ChartView {
    fn attrs(&self) -> String {
        let mut s = String::new();
        if !self.hidden.is_empty() {
            let h: Vec<String> = self.hidden.iter().map(|k| k.to_string()).collect();
            let _ = write!(s, r#" data-hidden="{}""#, h.join(","));
        }
        if let Some((a, b)) = self.x {
            let _ = write!(s, r#" data-x0="{a}" data-x1="{b}""#);
        }
        s
    }

    /// The view the `data-hidden`, `data-x0` and `data-x1` attributes hold.
    pub fn parse(hidden: Option<&str>, x0: Option<&str>, x1: Option<&str>) -> ChartView {
        let hidden = hidden.map_or(Vec::new(), |h| h.split(',').filter_map(|k| k.trim().parse().ok()).collect());
        let x = match (x0.and_then(|v| v.parse().ok()), x1.and_then(|v| v.parse().ok())) {
            (Some(a), Some(b)) if a < b => Some((a, b)),
            _ => None,
        };
        ChartView { hidden, x }
    }

    fn shows(&self, series: usize) -> bool {
        !self.hidden.contains(&series)
    }

    /// The view after a click on series `s` of `n` in the legend. A plain click shows `s`
    /// alone, or, when it is shown alone already, every series again. A `toggle` click (ctrl or
    /// cmd) hides or shows `s` only, never hiding the last one shown.
    pub fn legend_click(&self, s: usize, n: usize, toggle: bool) -> ChartView {
        let mut v = self.clone();
        if toggle {
            if v.hidden.contains(&s) {
                v.hidden.retain(|&h| h != s);
            } else if v.hidden.len() + 1 < n {
                v.hidden.push(s);
                v.hidden.sort_unstable();
            }
        } else {
            let alone: Vec<usize> = (0..n).filter(|&k| k != s).collect();
            v.hidden = if v.hidden == alone { Vec::new() } else { alone };
        }
        v
    }
}

/// What to show for the pointer over a chart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hover {
    /// The tooltip's HTML.
    pub tip: String,
    /// SVG drawn over the chart: a crosshair and the points it meets.
    pub overlay: String,
    /// The metric under the pointer, whose table row and bars light up together.
    pub metric: Option<usize>,
    /// The node a click opens.
    pub goto: Option<String>,
}

/// The section for node `i`, or nothing when it has no measurements.
pub fn section(g: &Graph, i: usize) -> String {
    let n = g.node(i);
    let own = n.performance.as_ref().filter(|p| !p.metrics.is_empty());
    let trends = if g.is_group(i) { trends(g, i) } else { Vec::new() };
    if own.is_none() && trends.is_empty() && n.perf_doc.is_none() {
        return String::new();
    }
    let mut body = String::new();
    let headline = match own {
        Some(p) => {
            own_section(&mut body, g, i, p);
            headline(p)
        }
        None if !trends.is_empty() => {
            let members: std::collections::BTreeSet<usize> = trends.iter().flat_map(|t| t.steps.iter().map(|s| s.node)).collect();
            format!("{} across {} members", plural(trends.len(), "metric", "metrics"), members.len())
        }
        None => String::new(),
    };
    for t in &trends {
        trend_section(&mut body, g, i, t);
    }
    if let Some(d) = &n.perf_doc {
        let what = match (&d.heading_text, d.tables) {
            (Some(h), _) => format!("<em>{}</em>", esc(h)),
            (None, k) => plural(k, "table", "tables"),
        };
        let _ = write!(
            body,
            r#"<details class="reef-perf-doc" data-href="{href}"{heading} open><summary>From {label} <span class="reef-note">{what}</span></summary><div class="reef-doc-text"><p class="reef-note">Loading…</p></div></details>"#,
            href = esc(&d.href),
            heading = d.heading.as_ref().map_or(String::new(), |h| format!(r#" data-heading="{}""#, esc(h))),
            label = esc(&d.label),
        );
    }
    let headline = if headline.is_empty() { String::new() } else { format!(r#" <span class="reef-perf-headline">{headline}</span>"#) };
    let head = format!("Performance{headline}");
    crate::markup::sec(&head, "reef-perf", &format!(r#"<div class="reef-perf-body">{body}</div>"#))
}

fn plural(k: usize, one: &str, many: &str) -> String {
    format!("{k} {}", if k == 1 { one } else { many })
}

/// The first two changes, as words and a sign of whether each improved.
fn headline(p: &Performance) -> String {
    p.metrics
        .iter()
        .filter_map(|m| Some((m, m.ratio()?)))
        .take(2)
        .map(|(m, r)| format!("{} {}", esc(m.key()), change_span(r, m.better)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A change, coloured and marked by whether it improved.
fn change_span(ratio: f64, better: Better) -> String {
    let (cls, mark, word) = match perf::improved(ratio, better) {
        Some(true) => ("reef-perf-good", "✓", "better"),
        Some(false) => ("reef-perf-bad", "✗", "worse"),
        None => ("reef-perf-same", "", "unchanged"),
    };
    let mark = if mark.is_empty() { String::new() } else { format!(" {mark}") };
    format!(r#"<span class="{cls}" title="{word}">{}{mark}</span>"#, perf::change(ratio))
}

fn unit_note(unit: Option<&str>) -> String {
    unit.map_or(String::new(), |u| format!(r#" <span class="reef-note">{}</span>"#, esc(u)))
}

fn with_unit(v: String, unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!("{v} {}", esc(u)),
        None => v,
    }
}

/// The metrics with a figure on either side, with their index among the node's metrics.
fn pairs(p: &Performance) -> Vec<(usize, &Metric)> {
    p.metrics.iter().enumerate().filter(|(_, m)| m.before.is_some() || m.after.is_some()).collect()
}
fn changed(p: &Performance) -> Vec<(usize, &Metric)> {
    p.metrics.iter().enumerate().filter(|(_, m)| m.ratio().is_some()).collect()
}
fn both(p: &Performance) -> Vec<(usize, &Metric)> {
    p.metrics.iter().enumerate().filter(|(_, m)| m.before.is_some() && m.after.is_some()).collect()
}

fn own_section(s: &mut String, g: &Graph, i: usize, p: &Performance) {
    let (bl, al) = (p.before_label(), p.after_label());
    let rows = pairs(p);
    if !rows.is_empty() {
        table(s, &rows, bl, al);
    }
    let v = ChartView::default();
    if !changed(p).is_empty() {
        let _ = write!(s, r#"<h4>Change <span class="reef-note">{} against {}</span></h4>"#, esc(al), esc(bl));
        s.push_str(&chart(g, i, &ChartRef::Change, &v));
    }
    if !both(p).is_empty() {
        let _ = write!(s, "<h4>{} and {}</h4>", esc(bl), esc(al));
        s.push_str(&chart(g, i, &ChartRef::Pair, &v));
    }
    for (k, m) in p.metrics.iter().enumerate().filter(|(_, m)| !m.points.is_empty()) {
        let _ = write!(s, "<h4>{}{}</h4>", esc(&m.name), unit_note(m.unit.as_deref()));
        s.push_str(&chart(g, i, &ChartRef::Sweep(k), &v));
    }
    if let Some(note) = p.note.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = write!(s, r#"<div class="reef-perf-note">{note}</div>"#);
    }
}

fn table(s: &mut String, ms: &[(usize, &Metric)], bl: &str, al: &str) {
    let disjoint = ms.iter().any(|(_, m)| m.disjoint().is_some());
    let _ = write!(s, r#"<div class="reef-perf-table"><table><thead><tr><th>Metric</th><th>{}</th><th>{}</th><th>Change</th>"#, esc(bl), esc(al));
    if disjoint {
        s.push_str(r#"<th title="Whether the two ranges do not overlap">Disjoint</th>"#);
    }
    s.push_str("</tr></thead><tbody>");
    for (k, m) in ms {
        let _ = write!(
            s,
            r#"<tr data-metric="{k}"><td>{}{}<span class="reef-perf-dir">{}</span></td><td>{}</td><td>{}</td><td>{}</td>"#,
            esc(&m.name),
            unit_note(m.unit.as_deref()),
            if m.better == Better::Higher { "higher is better" } else { "lower is better" },
            m.before.map_or("—".into(), perf::value),
            m.after.map_or("—".into(), perf::value),
            m.ratio().map_or("—".into(), |r| change_span(r, m.better)),
        );
        if disjoint {
            let _ = write!(s, "<td>{}</td>", disjoint_word(m.disjoint()));
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table></div>");
}

fn disjoint_word(d: Option<bool>) -> &'static str {
    match d {
        Some(true) => "yes",
        Some(false) => "no",
        None => "—",
    }
}

/// A label cut to fit `width`, with an ellipsis.
fn fit(text: &str, width: f64) -> String {
    let max = (width / CHAR_W).floor().max(1.0) as usize;
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A horizontal bar from `x0` to `x1`, its far end rounded and its baseline end square.
fn hbar(x0: f64, x1: f64, y: f64, h: f64) -> String {
    let len = (x1 - x0).abs();
    let r = 4.0f64.min(len / 2.0).min(h / 2.0);
    if len < 0.5 {
        return format!("M{x0:.1},{y:.1}h0.5v{h:.1}h-0.5z");
    }
    let dir = if x1 >= x0 { 1.0 } else { -1.0 };
    let xe = x1 - dir * r;
    let sweep = if dir > 0.0 { 1 } else { 0 };
    format!(
        "M{x0:.1},{y:.1}H{xe:.1}A{r:.1},{r:.1} 0 0 {sweep} {x1:.1},{y1:.1}V{y2:.1}A{r:.1},{r:.1} 0 0 {sweep} {xe:.1},{yb:.1}H{x0:.1}Z",
        y1 = y + r,
        y2 = y + h - r,
        yb = y + h,
    )
}

fn svg_open(h: f64, label: &str, i: usize, r: &ChartRef, v: &ChartView) -> String {
    svg_open_z(h, label, i, r, v, r.zoomable())
}

fn svg_open_z(h: f64, label: &str, i: usize, r: &ChartRef, v: &ChartView, zoomable: bool) -> String {
    format!(
        r#"<svg class="reef-perf-chart{zoom}" viewBox="0 0 {W} {h:.0}" role="img" aria-label="{}"{}{}>"#,
        esc(label),
        r.attrs(i),
        v.attrs(),
        zoom = if zoomable { " reef-perf-zoomable" } else { "" },
    )
}

/// The overlay the pointer draws in, last so it is on top.
const OVERLAY: &str = r#"<g class="reef-perf-overlay"></g>"#;

/// The chart `r` of node `i`'s section, drawn as `v` says. Empty when the node has no such
/// chart.
pub fn chart(g: &Graph, i: usize, r: &ChartRef, v: &ChartView) -> String {
    let p = g.node(i).performance.as_ref();
    match r {
        ChartRef::Change => p.map_or(String::new(), |p| change_bars(i, p, v, r)),
        ChartRef::Pair => p.map_or(String::new(), |p| pair_bars(i, p, v, r, &both(p), &[])),
        ChartRef::Sweep(k) => p.and_then(|p| Some((p, p.metrics.get(*k)?))).map_or(String::new(), |(p, m)| {
            line_svg(&sweep_chart(m, p, &[]), v, i, r)
        }),
        ChartRef::Trend(key) => trends(g, i).into_iter().find(|t| &t.key == key).map_or(String::new(), |t| {
            line_svg(&trend_chart(g, &t).0, v, i, r)
        }),
        ChartRef::Accept(c) => {
            let Some((p, ch, targets)) = accept_chart(g, i, *c) else { return String::new() };
            match ch {
                AccDraw::Change => change_bars(i, p, v, r),
                AccDraw::Pair(ms) => pair_bars(i, p, v, r, &ms, &targets),
                AccDraw::Sweep(m) => line_svg(&sweep_chart(m, p, &targets), v, i, r),
            }
        }
    }
}

/// What a criterion's chart draws, resolved.
enum AccDraw<'a> {
    Change,
    Pair(Vec<(usize, &'a Metric)>),
    Sweep(&'a Metric),
}

/// The chart node `i`'s criterion `c` draws: the performance it is drawn from, which chart,
/// and the target lines to draw on it.
fn accept_chart(g: &Graph, i: usize, c: usize) -> Option<(&Performance, AccDraw<'_>, Vec<f64>)> {
    let (j, ch) = g.acceptance(i).get(c)?.chart?;
    let crit = g.node(i).acceptance.get(c)?;
    let p = g.node(j).performance.as_ref()?;
    let targets: Vec<f64> = [crit.at_least, crit.at_most].into_iter().flatten().collect();
    let draw = match ch {
        AccChart::Change => AccDraw::Change,
        AccChart::Pair => AccDraw::Pair(both(p)),
        AccChart::Metric(k) => {
            let m = p.metrics.get(k)?;
            if !m.points.is_empty() {
                AccDraw::Sweep(m)
            } else if m.before.is_some() || m.after.is_some() {
                AccDraw::Pair(vec![(k, m)])
            } else {
                return None;
            }
        }
    };
    // A whole chart has no one metric for a target to belong to.
    let targets = if matches!(ch, AccChart::Metric(_)) { targets } else { Vec::new() };
    Some((p, draw, targets))
}

/// Whether a drag zooms chart `r` of node `i`: the charts with an x axis.
pub fn zoomable(g: &Graph, i: usize, r: &ChartRef) -> bool {
    match r {
        ChartRef::Accept(c) => matches!(accept_chart(g, i, *c), Some((_, AccDraw::Sweep(_), _))),
        r => r.zoomable(),
    }
}

/// What the pointer at `(x, y)`, in the chart's SVG units, is over.
pub fn hover(g: &Graph, i: usize, r: &ChartRef, v: &ChartView, x: f64, y: f64) -> Option<Hover> {
    let p = g.node(i).performance.as_ref();
    match r {
        ChartRef::Change => change_hover(p?, y),
        ChartRef::Pair => pair_hover(p?, &both(p?), y),
        ChartRef::Sweep(k) => {
            let p = p?;
            let m = p.metrics.get(*k)?;
            sweep_hover(m, p, &[], v, x)
        }
        ChartRef::Accept(c) => {
            let (p, ch, targets) = accept_chart(g, i, *c)?;
            match ch {
                AccDraw::Change => change_hover(p, y),
                AccDraw::Pair(ms) => pair_hover(p, &ms, y),
                AccDraw::Sweep(m) => sweep_hover(m, p, &targets, v, x),
            }
        }
        ChartRef::Trend(key) => {
            let t = trends(g, i).into_iter().find(|t| &t.key == key)?;
            trend_hover(g, &t, v, x)
        }
    }
}

/// The plot area of a zoomable chart, as `(left, top, right, bottom)` in SVG units.
pub fn plot_area(g: &Graph, i: usize, r: &ChartRef, v: &ChartView) -> Option<(f64, f64, f64, f64)> {
    let lc = line_chart(g, i, r)?;
    let lay = layout(&lc, v);
    Some((lay.plot.l, lay.plot.t, lay.plot.r, lay.plot.b))
}

/// The x value at `px` across a zoomable chart.
pub fn x_at(g: &Graph, i: usize, r: &ChartRef, v: &ChartView, px: f64) -> Option<f64> {
    let lc = line_chart(g, i, r)?;
    Some(layout(&lc, v).plot.x_inv(px))
}

fn line_chart(g: &Graph, i: usize, r: &ChartRef) -> Option<LineChart> {
    match r {
        ChartRef::Sweep(k) => {
            let p = g.node(i).performance.as_ref()?;
            Some(sweep_chart(p.metrics.get(*k)?, p, &[]))
        }
        ChartRef::Accept(c) => match accept_chart(g, i, *c)? {
            (p, AccDraw::Sweep(m), targets) => Some(sweep_chart(m, p, &targets)),
            _ => None,
        },
        ChartRef::Trend(key) => {
            let t = trends(g, i).into_iter().find(|t| &t.key == key)?;
            Some(trend_chart(g, &t).0)
        }
        _ => None,
    }
}

// Change bars ---------------------------------------------------------------------------------

const CH_ROW: f64 = 34.0;
const CH_BAR: f64 = 12.0;
const CH_TOP: f64 = 16.0;

/// A metric's change on a log scale, signed so that right is better.
fn signed(m: &Metric) -> f64 {
    let l = m.ratio().unwrap_or(1.0).log2();
    if m.better == Better::Higher { l } else { -l }
}

/// One bar a metric, from a centre line, on a log scale of the ratio: right is better, left
/// worse, whichever way the metric improves.
fn change_bars(i: usize, p: &Performance, v: &ChartView, r: &ChartRef) -> String {
    let ms = changed(p);
    let max = ms.iter().map(|(_, m)| signed(m).abs()).fold(0.25f64.log2().abs() / 2.0, f64::max);
    let mid = W / 2.0;
    let half = mid - 52.0;
    let h = CH_TOP + ms.len() as f64 * CH_ROW + 2.0;
    let mut s = svg_open(h, "Each metric's change: right is better, left is worse", i, r, v);
    let _ = write!(
        s,
        r#"<text class="reef-perf-axis-text" x="{:.1}" y="10" text-anchor="end">← worse</text><text class="reef-perf-axis-text" x="{:.1}" y="10">better →</text>"#,
        mid - 6.0,
        mid + 6.0,
    );
    for (row, (k, m)) in ms.iter().enumerate() {
        let y = CH_TOP + row as f64 * CH_ROW;
        let sv = signed(m);
        let x1 = mid + sv / max * half;
        let r = m.ratio().unwrap_or(1.0);
        let cls = match perf::improved(r, m.better) {
            Some(true) => "reef-perf-better",
            Some(false) => "reef-perf-worse",
            None => "reef-perf-axis",
        };
        let faint = if m.disjoint() == Some(false) { " reef-perf-faint" } else { "" };
        let _ = write!(
            s,
            r#"<g class="reef-perf-row" data-metric="{k}"><rect class="reef-perf-band" x="0" y="{y:.1}" width="{W}" height="{CH_ROW}" rx="3"/><text class="reef-perf-label" x="4" y="{ty:.1}">{short}</text><path class="{cls}{faint}" d="{d}"/><text class="reef-perf-value" x="{vx:.1}" y="{vy:.1}" text-anchor="{anchor}">{change}</text></g>"#,
            change = perf::change(r),
            short = esc(&fit(&m.name, W - 8.0)),
            ty = y + 10.0,
            d = hbar(mid, x1, y + 16.0, CH_BAR),
            vx = if sv >= 0.0 { x1 + 4.0 } else { x1 - 4.0 },
            vy = y + 16.0 + CH_BAR - 2.5,
            anchor = if sv >= 0.0 { "start" } else { "end" },
        );
    }
    let _ = write!(s, r#"<line class="reef-perf-baseline" x1="{mid}" y1="{CH_TOP:.1}" x2="{mid}" y2="{:.1}"/>{OVERLAY}</svg>"#, h - 2.0);
    s
}

/// A tooltip's lines for one metric: both figures, the change, and whether the ranges overlap.
fn metric_tip(m: &Metric, bl: &str, al: &str) -> String {
    let unit = m.unit.as_deref();
    let mut t = format!(r#"<div class="reef-tip-head">{}</div>"#, esc(&m.name));
    for (cls, label, val) in [("reef-perf-before", bl, m.before), ("reef-perf-after", al, m.after)] {
        if let Some(val) = val {
            let _ = write!(
                t,
                r#"<div class="reef-tip-row"><span class="reef-swatch {cls}"></span><span class="reef-tip-name">{}</span><span class="reef-tip-val">{}</span></div>"#,
                esc(label),
                with_unit(perf::value(val), unit),
            );
        }
    }
    if let Some(r) = m.ratio() {
        let _ = write!(t, r#"<div class="reef-tip-row"><span class="reef-tip-name">Change</span><span class="reef-tip-val">{}</span></div>"#, change_span(r, m.better));
    }
    if let Some(d) = m.disjoint() {
        let _ = write!(
            t,
            r#"<div class="reef-tip-note">{}</div>"#,
            if d { "The ranges do not overlap." } else { "The ranges overlap: this may be noise." }
        );
    }
    let _ = write!(t, r#"<div class="reef-tip-note">{}</div>"#, if m.better == Better::Higher { "Higher is better." } else { "Lower is better." });
    t
}

fn change_hover(p: &Performance, y: f64) -> Option<Hover> {
    let ms = changed(p);
    let row = ((y - CH_TOP) / CH_ROW).floor();
    if row < 0.0 {
        return None;
    }
    let &(k, m) = ms.get(row as usize)?;
    Some(Hover { tip: metric_tip(m, p.before_label(), p.after_label()), metric: Some(k), ..Hover::default() })
}

// Before and after bars -----------------------------------------------------------------------

const PB_BAR: f64 = 12.0;
const PB_GAP: f64 = 4.0;
const PB_TITLE: f64 = 16.0;
const PB_BLOCK: f64 = PB_TITLE + 2.0 * PB_BAR + PB_GAP + 12.0;
const LEGEND: f64 = 18.0;

/// Small multiples: each metric's two figures as bars on its own scale from zero, a range drawn
/// as a whisker. A side hidden from the legend is left out and the scale fits the other.
/// `ms` are the metrics to draw, with their index; `targets` are drawn on each as dashed lines.
fn pair_bars(i: usize, p: &Performance, v: &ChartView, r: &ChartRef, ms: &[(usize, &Metric)], targets: &[f64]) -> String {
    let (bl, al) = (p.before_label(), p.after_label());
    let side_w = (bl.chars().count().max(al.chars().count()) as f64 * CHAR_W + 8.0).clamp(36.0, 96.0);
    let x0 = side_w;
    let span = W - x0 - 96.0;
    let h = LEGEND + ms.len() as f64 * PB_BLOCK;
    let mut s = svg_open(h, &format!("{bl} and {al}, each metric on its own scale"), i, r, v);
    legend(&mut s, 4.0, 4.0, &[("reef-perf-before", bl), ("reef-perf-after", al)], v);
    for (row, (k, m)) in ms.iter().enumerate() {
        let y = LEGEND + row as f64 * PB_BLOCK;
        let sides = [(m.before.unwrap_or(Value::One(0.0)), "reef-perf-before", bl), (m.after.unwrap_or(Value::One(0.0)), "reef-perf-after", al)];
        let top = sides
            .iter()
            .enumerate()
            .filter(|(j, _)| v.shows(*j))
            .map(|(_, (val, _, _))| val.hi())
            .chain(targets.iter().copied())
            .fold(f64::MIN_POSITIVE, f64::max);
        let x = |val: f64| x0 + (val.max(0.0) / top).min(1.04) * span;
        let unit = m.unit.as_deref().map_or(String::new(), |u| format!(" ({u})"));
        let _ = write!(
            s,
            r#"<g class="reef-perf-row" data-metric="{k}"><rect class="reef-perf-band" x="0" y="{y:.1}" width="{W}" height="{bh:.1}" rx="3"/><text class="reef-perf-label" x="4" y="{ty:.1}">{}</text>"#,
            esc(&fit(&format!("{}{unit}", m.name), W - 8.0)),
            bh = PB_BLOCK - 6.0,
            ty = y + 11.0,
        );
        for (j, (val, cls, label)) in sides.into_iter().enumerate() {
            let by = y + PB_TITLE + j as f64 * (PB_BAR + PB_GAP);
            let _ = write!(
                s,
                r#"<text class="reef-perf-side" x="{sx:.1}" y="{ty:.1}" text-anchor="end">{}</text>"#,
                esc(&fit(label, side_w - 8.0)),
                sx = x0 - 6.0,
                ty = by + PB_BAR - 2.5,
            );
            if !v.shows(j) {
                continue;
            }
            let _ = write!(s, r#"<path class="{cls}" d="{}"/>"#, hbar(x0, x(val.mid()), by, PB_BAR));
            if let Some((lo, hi)) = val.range() {
                let cy = by + PB_BAR / 2.0;
                let _ = write!(
                    s,
                    r#"<path class="reef-perf-whisker" d="M{l:.1},{cy:.1}H{h:.1}M{l:.1},{a:.1}V{b:.1}M{h:.1},{a:.1}V{b:.1}"/>"#,
                    l = x(lo),
                    h = x(hi),
                    a = cy - 4.0,
                    b = cy + 4.0,
                );
            }
            let _ = write!(s, r#"<text class="reef-perf-value" x="{:.1}" y="{:.1}">{}</text>"#, x(val.hi()) + 5.0, by + PB_BAR - 2.5, perf::number(val.mid()));
        }
        for &t in targets {
            let _ = write!(
                s,
                r#"<line class="reef-acc-target" x1="{tx:.1}" y1="{a:.1}" x2="{tx:.1}" y2="{b:.1}"><title>target {}</title></line>"#,
                perf::number(t),
                tx = x(t),
                a = y + PB_TITLE - 4.0,
                b = y + PB_TITLE + 2.0 * PB_BAR + PB_GAP + 4.0,
            );
        }
        let _ = write!(
            s,
            r#"<line class="reef-perf-baseline" x1="{x0:.1}" y1="{:.1}" x2="{x0:.1}" y2="{:.1}"/></g>"#,
            y + PB_TITLE - 2.0,
            y + PB_TITLE + 2.0 * PB_BAR + PB_GAP + 2.0
        );
    }
    s.push_str(OVERLAY);
    s.push_str("</svg>");
    s
}

fn pair_hover(p: &Performance, ms: &[(usize, &Metric)], y: f64) -> Option<Hover> {
    let row = ((y - LEGEND) / PB_BLOCK).floor();
    if row < 0.0 {
        return None;
    }
    let &(k, m) = ms.get(row as usize)?;
    Some(Hover { tip: metric_tip(m, p.before_label(), p.after_label()), metric: Some(k), ..Hover::default() })
}

/// A legend whose items hide and show their series.
fn legend(s: &mut String, x: f64, y: f64, items: &[(&str, &str)], v: &ChartView) {
    let mut cx = x;
    for (k, (cls, label)) in items.iter().enumerate() {
        let label = fit(label, 120.0);
        let w = 14.0 + label.chars().count() as f64 * CHAR_W;
        let _ = write!(
            s,
            r#"<g class="reef-perf-legend-item{off}" data-series="{k}"><title>Click to show only this; ctrl-click to hide or show it</title><rect class="reef-perf-hit-area" x="{hx:.1}" y="{hy:.1}" width="{hw:.1}" height="16"/><rect class="{cls}" x="{cx:.1}" y="{y:.1}" width="10" height="10" rx="2"/><text class="reef-perf-legend" x="{tx:.1}" y="{ty:.1}">{}</text></g>"#,
            esc(&label),
            off = if v.shows(k) { "" } else { " off" },
            hx = cx - 3.0,
            hy = y - 3.0,
            hw = w + 6.0,
            tx = cx + 14.0,
            ty = y + 9.0,
        );
        cx += w + 14.0;
    }
}

// Line charts ---------------------------------------------------------------------------------

/// Round tick values over `[lo, hi]`, about `count` of them.
pub fn nice_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
        return vec![lo];
    }
    let raw = (hi - lo) / count.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().map(|f| f * mag).find(|&s| s >= raw).unwrap_or(10.0 * mag);
    let start = (lo / step).floor() * step;
    let mut out = Vec::new();
    let mut v = start;
    while v <= hi + step * 1e-9 {
        out.push((v / step).round() * step);
        v += step;
    }
    if out.last().is_some_and(|&l| l < hi) {
        out.push(out.last().unwrap() + step);
    }
    out
}

/// A tick's label: in full under ten thousand, compact above.
pub fn tick(v: f64) -> String {
    let a = v.abs();
    let (d, suffix) = if a >= 1e9 {
        (1e9, "G")
    } else if a >= 1e6 {
        (1e6, "M")
    } else if a >= 1e4 {
        (1e3, "K")
    } else {
        return perf::number(v);
    };
    let x = v / d;
    let s = if (x - x.round()).abs() < 1e-9 { format!("{}", x.round()) } else { format!("{x:.1}") };
    format!("{s}{suffix}")
}

/// A plot area and its scales.
#[derive(Debug, Clone)]
struct Plot {
    l: f64,
    r: f64,
    t: f64,
    b: f64,
    x_log: bool,
    x_lo: f64,
    x_hi: f64,
    y_log: bool,
    y_lo: f64,
    y_hi: f64,
}

impl Plot {
    /// An x value in the scale's own terms: its log on a log axis.
    fn tx(&self, v: f64) -> f64 {
        if self.x_log { v.ln() } else { v }
    }
    fn x(&self, v: f64) -> f64 {
        let (a, b) = (self.tx(self.x_lo), self.tx(self.x_hi));
        self.l + (self.tx(v) - a) / (b - a) * (self.r - self.l)
    }
    fn x_inv(&self, px: f64) -> f64 {
        let (a, b) = (self.tx(self.x_lo), self.tx(self.x_hi));
        let t = a + (px.clamp(self.l, self.r) - self.l) / (self.r - self.l) * (b - a);
        if self.x_log { t.exp() } else { t }
    }
    fn y(&self, v: f64) -> f64 {
        let f = if self.y_log {
            (v.max(self.y_lo).ln() - self.y_lo.ln()) / (self.y_hi.ln() - self.y_lo.ln())
        } else {
            (v - self.y_lo) / (self.y_hi - self.y_lo)
        };
        self.b - f * (self.b - self.t)
    }
}

/// A log y scale for values `vals` that span more than fifty times, all above zero: powers of
/// ten, with the 2s and 5s between them when there are few.
fn y_log_scale(vals: &[f64]) -> Option<(f64, f64, Vec<f64>)> {
    let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(lo > 0.0 && hi / lo > 50.0) {
        return None;
    }
    let (a, b) = (lo.log10().floor() as i32, hi.log10().ceil() as i32);
    let mut ticks: Vec<f64> = (a..=b).map(|e| 10f64.powi(e)).collect();
    if ticks.len() <= 3 {
        ticks = (a..=b).flat_map(|e| [1.0, 2.0, 5.0].map(|m| m * 10f64.powi(e))).filter(|&t| t <= 10f64.powi(b)).collect();
    }
    Some((ticks[0], *ticks.last().unwrap(), ticks))
}

/// The y scale and its ticks for values `vals`, from zero when none is below it.
fn y_scale(vals: &[f64]) -> (f64, f64, Vec<f64>) {
    let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let lo = if !lo.is_finite() || lo >= 0.0 { 0.0 } else { lo };
    let hi = if !hi.is_finite() || hi <= lo { lo + 1.0 } else { hi };
    let ticks = nice_ticks(lo, hi, 4);
    (ticks[0], *ticks.last().unwrap(), ticks)
}

/// One series of a line chart, its points sorted by x.
#[derive(Debug, Clone)]
struct Line {
    cls: &'static str,
    label: String,
    pts: Vec<(f64, Value)>,
}

/// A line chart's data, before any view is applied.
#[derive(Debug, Clone)]
struct LineChart {
    aria: String,
    x_label: Option<String>,
    unit: Option<String>,
    series: Vec<Line>,
    x_log: bool,
    /// The x range of all the data.
    full: (f64, f64),
    /// May the y axis go log when its values span widely.
    y_log_ok: bool,
    /// Names for integer x positions, in place of numbers.
    x_names: Option<Vec<String>>,
    legend: bool,
    h: f64,
    /// Targets drawn as dashed lines across the plot.
    targets: Vec<f64>,
}

/// A line chart laid out for a view: its plot, and each shown series cut to the x range.
struct Layout {
    plot: Plot,
    ticks: Vec<f64>,
    /// Per series: its points cut to the range, with points added where it crosses an edge.
    /// `None` when the series is hidden.
    cut: Vec<Option<Vec<(f64, f64)>>>,
}

/// The polyline `pts` cut to `[lo, hi]` in x, interpolating where it crosses an edge in the
/// terms `tf` puts x in (its log on a log axis).
fn cut(pts: &[(f64, f64)], lo: f64, hi: f64, tf: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
    let inside = |x: f64| x >= lo && x <= hi;
    let mut out = Vec::new();
    for (k, &(x, y)) in pts.iter().enumerate() {
        if inside(x) {
            out.push((x, y));
        }
        if let Some(&(x2, y2)) = pts.get(k + 1) {
            let (ta, tb) = (tf(x), tf(x2));
            for edge in [lo, hi] {
                let te = tf(edge);
                let crosses = (ta < te && te < tb) || (tb < te && te < ta);
                if crosses {
                    let f = (te - ta) / (tb - ta);
                    out.push((edge, y + f * (y2 - y)));
                }
            }
            // Two edges crossed by one segment come out in its direction.
            let n = out.len();
            if n >= 2 && out[n - 1].0 < out[n - 2].0 && x2 > x {
                out.swap(n - 1, n - 2);
            }
        }
    }
    out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    out
}

fn layout(lc: &LineChart, v: &ChartView) -> Layout {
    let (x_lo, x_hi) = v.x.unwrap_or(lc.full);
    let (x_lo, x_hi) = if x_hi > x_lo { (x_lo, x_hi) } else { (x_lo - 1.0, x_hi + 1.0) };
    let tf = |x: f64| if lc.x_log { x.ln() } else { x };
    let cuts: Vec<Option<Vec<(f64, f64)>>> = lc
        .series
        .iter()
        .enumerate()
        .map(|(k, s)| {
            v.shows(k).then(|| {
                let mids: Vec<(f64, f64)> = s.pts.iter().map(|&(x, val)| (x, val.mid())).collect();
                cut(&mids, x_lo, x_hi, tf)
            })
        })
        .collect();
    // The y scale fits what is shown: the cut lines and the ranges of the points inside.
    let mut vals: Vec<f64> = cuts.iter().flatten().flatten().map(|p| p.1).collect();
    vals.extend(lc.targets.iter().copied());
    for (k, s) in lc.series.iter().enumerate() {
        if v.shows(k) {
            vals.extend(s.pts.iter().filter(|p| p.0 >= x_lo && p.0 <= x_hi).flat_map(|p| [p.1.lo(), p.1.hi()]));
        }
    }
    let y_log = if lc.y_log_ok { y_log_scale(&vals) } else { None };
    let (y_lo, y_hi, ticks) = y_log.clone().unwrap_or_else(|| y_scale(&vals));
    // A log axis says so on a line of its own, under the legend.
    let t = if lc.legend { 26.0 } else { 22.0 } + if y_log.is_some() { 10.0 } else { 0.0 };
    let plot = Plot { l: 46.0, r: W - 14.0, t, b: lc.h - 34.0, x_log: lc.x_log, x_lo, x_hi, y_log: y_log.is_some(), y_lo, y_hi };
    Layout { plot, ticks, cut: cuts }
}

fn line_svg(lc: &LineChart, v: &ChartView, i: usize, r: &ChartRef) -> String {
    let lay = layout(lc, v);
    let p = &lay.plot;
    // Every line chart has an x axis to zoom along.
    let mut s = svg_open_z(lc.h, &lc.aria, i, r, v, true);
    if lc.legend {
        let items: Vec<(&str, &str)> = lc.series.iter().map(|l| (l.cls, l.label.as_str())).collect();
        legend(&mut s, 4.0, 4.0, &items, v);
    }
    // Y axis.
    for &t in &lay.ticks {
        let y = p.y(t);
        let cls = if t == p.y_lo { "reef-perf-baseline" } else { "reef-perf-grid" };
        let _ = write!(
            s,
            r#"<line class="{cls}" x1="{l:.1}" y1="{y:.1}" x2="{r:.1}" y2="{y:.1}"/><text class="reef-perf-axis-text" x="{tx:.1}" y="{ty:.1}" text-anchor="end">{}</text>"#,
            tick(t),
            l = p.l,
            r = p.r,
            tx = p.l - 5.0,
            ty = y + 3.5,
        );
    }
    if p.y_log {
        let _ = write!(s, r#"<text class="reef-perf-axis-text" x="4" y="{:.1}">log scale</text>"#, p.t - 7.0);
    }
    // X axis: named positions, the measured points when they are few, else round values.
    let mut xs: Vec<f64> = lc.series.iter().flat_map(|l| l.pts.iter().map(|q| q.0)).filter(|&x| x >= p.x_lo && x <= p.x_hi).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let xt: Vec<(f64, String)> = if let Some(names) = &lc.x_names {
        xs.iter().filter_map(|&x| Some((x, names.get(x as usize)?.clone()))).collect()
    } else if xs.len() <= 7 {
        xs.iter().map(|&x| (x, tick(x))).collect()
    } else if p.x_log {
        let (a, b) = (p.x_lo.log10().ceil() as i32, p.x_hi.log10().floor() as i32);
        let t: Vec<f64> = (a..=b).map(|e| 10f64.powi(e)).collect();
        let t = if t.len() >= 2 { t } else { (p.x_lo.log2().ceil() as i32..=p.x_hi.log2().floor() as i32).step_by(2).map(|e| 2f64.powi(e)).collect() };
        t.into_iter().map(|x| (x, tick(x))).collect()
    } else {
        nice_ticks(p.x_lo, p.x_hi, 5).into_iter().filter(|&x| x >= p.x_lo && x <= p.x_hi).map(|x| (x, tick(x))).collect()
    };
    let mut last_end = f64::NEG_INFINITY;
    let n_xt = xt.len();
    for (k, (t, label)) in xt.into_iter().enumerate() {
        let x = p.x(t);
        let label = fit(&label, 8.0 * CHAR_W);
        let w = label.chars().count() as f64 * CHAR_W;
        let (anchor, left) = if k == 0 && x - w / 2.0 < p.l - 20.0 {
            ("start", x)
        } else if k + 1 == n_xt && x + w / 2.0 > W {
            ("end", x - w)
        } else {
            ("middle", x - w / 2.0)
        };
        let _ = write!(s, r#"<line class="reef-perf-tick" x1="{x:.1}" y1="{b:.1}" x2="{x:.1}" y2="{b2:.1}"/>"#, b = p.b, b2 = p.b + 4.0);
        if left < last_end + 4.0 {
            continue;
        }
        last_end = left + w;
        let _ = write!(s, r#"<text class="reef-perf-axis-text" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}">{}</text>"#, esc(&label), ty = p.b + 15.0);
    }
    if let Some(xl) = &lc.x_label {
        let _ = write!(s, r#"<text class="reef-perf-axis-text" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#, (p.l + p.r) / 2.0, lc.h - 4.0, esc(&fit(xl, W - 20.0)));
    }
    // The plot area takes the pointer: hover, drag to zoom.
    let _ = write!(s, r#"<rect class="reef-perf-plot" x="{l:.1}" y="{t:.1}" width="{w:.1}" height="{h:.1}"/>"#, l = p.l, t = p.t, w = p.r - p.l, h = p.b - p.t);
    for &t in &lc.targets {
        let y = p.y(t);
        let _ = write!(
            s,
            r#"<line class="reef-acc-target" x1="{l:.1}" y1="{y:.1}" x2="{r:.1}" y2="{y:.1}"/><text class="reef-acc-target-text" x="{r:.1}" y="{ty:.1}" text-anchor="end">target {}</text>"#,
            tick(t),
            l = p.l,
            r = p.r,
            ty = y - 3.0,
        );
    }
    // Series: lines, whiskers, dots.
    for (k, line) in lc.series.iter().enumerate() {
        let Some(c) = &lay.cut[k] else { continue };
        if c.len() >= 2 {
            let d: String = c.iter().enumerate().map(|(j, &(x, y))| format!("{}{:.1},{:.1}", if j == 0 { "M" } else { "L" }, p.x(x), p.y(y))).collect();
            let _ = write!(s, r#"<path class="{}-line" d="{d}"/>"#, line.cls);
        }
        for &(x, val) in line.pts.iter().filter(|q| q.0 >= p.x_lo && q.0 <= p.x_hi) {
            let (cx, cy) = (p.x(x), p.y(val.mid()));
            if let Some((lo, hi)) = val.range() {
                let _ = write!(s, r#"<path class="reef-perf-whisker" d="M{cx:.1},{a:.1}V{b:.1}"/>"#, a = p.y(hi), b = p.y(lo));
            }
            let _ = write!(s, r#"<circle class="{} reef-perf-dot" cx="{cx:.1}" cy="{cy:.1}" r="4"/>"#, line.cls);
        }
    }
    if v.x.is_some() {
        let (bw, bx) = (78.0, W - 82.0);
        let _ = write!(
            s,
            r#"<g class="reef-perf-reset" data-chart-act="reset"><rect x="{bx:.1}" y="2" width="{bw}" height="16" rx="3"/><text x="{tx:.1}" y="13.5" text-anchor="middle">Reset zoom</text></g>"#,
            tx = bx + bw / 2.0,
        );
    }
    s.push_str(OVERLAY);
    s.push_str("</svg>");
    s
}

/// The data x nearest the pointer at `px`, among the shown series' points in the range, and
/// each shown series' point there.
fn snap(lc: &LineChart, lay: &Layout, px: f64) -> Option<(f64, Vec<(usize, Value)>)> {
    let p = &lay.plot;
    if px < p.l - 8.0 || px > p.r + 8.0 {
        return None;
    }
    let x = lc
        .series
        .iter()
        .enumerate()
        .filter(|(k, _)| lay.cut[*k].is_some())
        .flat_map(|(_, l)| l.pts.iter().map(|q| q.0))
        .filter(|&x| x >= p.x_lo && x <= p.x_hi)
        .min_by(|a, b| (p.x(*a) - px).abs().total_cmp(&(p.x(*b) - px).abs()))?;
    let at = lc
        .series
        .iter()
        .enumerate()
        .filter(|(k, _)| lay.cut[*k].is_some())
        .filter_map(|(k, l)| l.pts.iter().find(|q| q.0 == x).map(|q| (k, q.1)))
        .collect();
    Some((x, at))
}

/// The crosshair at `x`, and a ring on each series' point there.
fn crosshair(lc: &LineChart, p: &Plot, x: f64, at: &[(usize, Value)]) -> String {
    let px = p.x(x);
    let mut o = format!(r#"<line class="reef-perf-cross" x1="{px:.1}" y1="{t:.1}" x2="{px:.1}" y2="{b:.1}"/>"#, t = p.t, b = p.b);
    for (k, val) in at {
        let _ = write!(o, r#"<circle class="{} reef-perf-dot reef-perf-ring" cx="{px:.1}" cy="{:.1}" r="6"/>"#, lc.series[*k].cls, p.y(val.mid()));
    }
    o
}

// Sweeps --------------------------------------------------------------------------------------

fn sweep_chart(m: &Metric, p: &Performance, targets: &[f64]) -> LineChart {
    let mut pts = m.points.clone();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let side = |pick: fn(&(f64, Option<f64>, Option<f64>)) -> Option<f64>| -> Vec<(f64, Value)> {
        pts.iter().filter_map(|q| Some((q.0, Value::One(pick(q)?)))).collect()
    };
    let xs: Vec<f64> = pts.iter().map(|q| q.0).collect();
    let (lo, hi) = (xs.iter().copied().fold(f64::INFINITY, f64::min), xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    let (lo, hi) = if lo.is_finite() { (lo, hi) } else { (0.0, 1.0) };
    LineChart {
        aria: format!("{} across {}: {} and {}", m.name, m.x.as_deref().unwrap_or("x"), p.before_label(), p.after_label()),
        x_label: m.x.clone(),
        unit: m.unit.clone(),
        series: vec![
            Line { cls: "reef-perf-before", label: p.before_label().into(), pts: side(|q| q.1) },
            Line { cls: "reef-perf-after", label: p.after_label().into(), pts: side(|q| q.2) },
        ],
        x_log: lo > 0.0 && hi / lo > 20.0,
        full: (lo, hi),
        y_log_ok: true,
        x_names: None,
        legend: true,
        h: 196.0,
        targets: targets.to_vec(),
    }
}

fn sweep_hover(m: &Metric, p: &Performance, targets: &[f64], v: &ChartView, px: f64) -> Option<Hover> {
    let lc = sweep_chart(m, p, targets);
    let lay = layout(&lc, v);
    let (x, at) = snap(&lc, &lay, px)?;
    let unit = m.unit.as_deref();
    let mut tip = format!(r#"<div class="reef-tip-head">{} {}</div>"#, esc(m.x.as_deref().unwrap_or("at")), perf::number(x));
    for (k, val) in &at {
        let l = &lc.series[*k];
        let _ = write!(
            tip,
            r#"<div class="reef-tip-row"><span class="reef-swatch {}"></span><span class="reef-tip-name">{}</span><span class="reef-tip-val">{}</span></div>"#,
            l.cls,
            esc(&l.label),
            with_unit(perf::value(*val), unit),
        );
    }
    let b = at.iter().find(|a| a.0 == 0).map(|a| a.1.mid());
    let a = at.iter().find(|a| a.0 == 1).map(|a| a.1.mid());
    if let (Some(b), Some(a)) = (b, a)
        && b != 0.0
    {
        let _ = write!(tip, r#"<div class="reef-tip-row"><span class="reef-tip-name">Change</span><span class="reef-tip-val">{}</span></div>"#, change_span(a / b, m.better));
    }
    Some(Hover { tip, overlay: crosshair(&lc, &lay.plot, x, &at), metric: None, goto: None })
}

// Group trends --------------------------------------------------------------------------------

/// One step of a group's trend: a member and its figures.
#[derive(Debug, Clone)]
struct Step {
    node: usize,
    before: Option<Value>,
    after: Option<Value>,
}

/// A metric followed across a group's members, in plan order.
#[derive(Debug, Clone)]
struct Trend {
    key: String,
    unit: Option<String>,
    better: Better,
    steps: Vec<Step>,
}

/// The metrics that two or more of a group's members measure, each followed across them.
fn trends(g: &Graph, i: usize) -> Vec<Trend> {
    let mut out: Vec<Trend> = Vec::new();
    for j in g.plan_order(i) {
        let Some(p) = &g.node(j).performance else { continue };
        for m in &p.metrics {
            if m.before.is_none() && m.after.is_none() {
                continue;
            }
            let step = Step { node: j, before: m.before, after: m.after };
            match out.iter_mut().find(|t| t.key == m.key()) {
                Some(t) => {
                    t.unit = t.unit.take().or_else(|| m.unit.clone());
                    t.steps.push(step);
                }
                None => out.push(Trend { key: m.key().to_string(), unit: m.unit.clone(), better: m.better, steps: vec![step] }),
            }
        }
    }
    out.retain(|t| t.steps.len() >= 2);
    out
}

fn trend_section(s: &mut String, g: &Graph, i: usize, t: &Trend) {
    let _ = write!(
        s,
        r#"<h4>{} across the members{} <span class="reef-note">{}</span></h4>"#,
        esc(&t.key),
        unit_note(t.unit.as_deref()),
        if t.better == Better::Higher { "higher is better" } else { "lower is better" },
    );
    s.push_str(&chart(g, i, &ChartRef::Trend(t.key.clone()), &ChartView::default()));
    s.push_str(r#"<div class="reef-perf-table"><table><thead><tr><th>Member</th><th>Before</th><th>After</th><th>Change</th></tr></thead><tbody>"#);
    for st in &t.steps {
        let n = g.node(st.node);
        let ratio = match (st.before, st.after) {
            (Some(b), Some(a)) if b.mid() != 0.0 => Some(a.mid() / b.mid()),
            _ => None,
        };
        let _ = write!(
            s,
            r#"<tr><td><a href="javascript:void 0" data-goto="{id}" title="{title}">{id}</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            st.before.map_or("—".into(), perf::value),
            st.after.map_or("—".into(), perf::value),
            ratio.map_or("—".into(), |r| change_span(r, t.better)),
            id = esc(&n.id),
            title = esc(&n.title),
        );
    }
    s.push_str("</tbody></table></div>");
}

/// Where the metric stood as the work went: the first member's before, then each member's
/// after, at x 0, 1, 2 and so on. With it, the member at each x (none for the start).
fn trend_chart(g: &Graph, t: &Trend) -> (LineChart, Vec<Option<usize>>) {
    let mut pts: Vec<(f64, Value)> = Vec::new();
    let mut names = Vec::new();
    let mut members = Vec::new();
    if let Some(b) = t.steps.first().and_then(|st| st.before) {
        pts.push((0.0, b));
        names.push("start".to_string());
        members.push(None);
    }
    for st in &t.steps {
        if let Some(a) = st.after.or(st.before) {
            pts.push((pts.len() as f64, a));
            names.push(g.node(st.node).id.clone());
            members.push(Some(st.node));
        }
    }
    let hi = (pts.len().max(2) - 1) as f64;
    let lc = LineChart {
        aria: format!("{} across the group's members, in plan order", t.key),
        x_label: Some("in plan order".into()),
        unit: t.unit.clone(),
        series: vec![Line { cls: "reef-perf-after", label: t.key.clone(), pts }],
        x_log: false,
        full: (0.0, hi),
        y_log_ok: false,
        x_names: Some(names),
        legend: false,
        h: 186.0,
        targets: Vec::new(),
    };
    (lc, members)
}

fn trend_hover(g: &Graph, t: &Trend, v: &ChartView, px: f64) -> Option<Hover> {
    let (lc, members) = trend_chart(g, t);
    let lay = layout(&lc, v);
    let (x, at) = snap(&lc, &lay, px)?;
    let k = x as usize;
    let val = at.first()?.1;
    let unit = lc.unit.as_deref();
    let mut tip = String::new();
    let goto = match members.get(k).copied().flatten() {
        Some(j) => {
            let n: &Node = g.node(j);
            let _ = write!(tip, r#"<div class="reef-tip-head">{} <code>{}</code></div>"#, esc(&n.title), esc(&n.id));
            let _ = write!(tip, r#"<div class="reef-tip-row"><span class="reef-tip-name">After it</span><span class="reef-tip-val">{}</span></div>"#, with_unit(perf::value(val), unit));
            Some(n.id.clone())
        }
        None => {
            let first = members.iter().flatten().next().map_or(String::new(), |&j| g.node(j).id.clone());
            let _ = write!(tip, r#"<div class="reef-tip-head">Before {}</div>"#, esc(&first));
            let _ = write!(tip, r#"<div class="reef-tip-row"><span class="reef-tip-name">At the start</span><span class="reef-tip-val">{}</span></div>"#, with_unit(perf::value(val), unit));
            None
        }
    };
    if k > 0
        && let Some(prev) = lc.series[0].pts.get(k - 1).map(|q| q.1.mid())
        && prev != 0.0
    {
        let _ = write!(
            tip,
            r#"<div class="reef-tip-row"><span class="reef-tip-name">From the last</span><span class="reef-tip-val">{}</span></div>"#,
            change_span(val.mid() / prev, t.better)
        );
    }
    if goto.is_some() {
        tip.push_str(r#"<div class="reef-tip-note">Click to open it.</div>"#);
    }
    Some(Hover { tip, overlay: crosshair(&lc, &lay.plot, x, &at), metric: None, goto })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_model::{Metric, PerfDoc};

    fn metric(name: &str, better: Better, before: Value, after: Value) -> Metric {
        Metric { name: name.into(), better, before: Some(before), after: Some(after), ..Metric::default() }
    }

    fn node(id: &str, parent: Option<&str>, needs: &[&str], perf: Option<Performance>) -> Node {
        Node {
            id: id.into(),
            title: format!("<{id}>"),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            performance: perf,
            ..Node::default()
        }
    }

    fn o151() -> Performance {
        Performance {
            labels: vec!["Buffered".into(), "Shared".into()],
            metrics: vec![
                metric("inserts <a second>", Better::Higher, Value::Range([3523.0, 3476.0, 3547.0]), Value::Range([6837.0, 6783.0, 6915.0])),
                Metric { unit: Some("ms".into()), ..metric("insert p99", Better::Lower, Value::Range([17.2, 17.1, 17.3]), Value::Range([10.2, 10.1, 10.6])) },
                metric("read p99", Better::Lower, Value::One(0.956), Value::One(1.58)),
                Metric {
                    name: "p99 by width".into(),
                    unit: Some("µs".into()),
                    x: Some("row size (KiB)".into()),
                    points: vec![(1.0, Some(2.0), Some(1.0)), (64.0, Some(5.0), None), (1024.0, Some(9.0), Some(3.0))],
                    ..Metric::default()
                },
            ],
            note: Some("<p>Eight rounds.</p>".into()),
        }
    }

    fn one() -> Graph {
        Graph::build(vec![node("o151", None, &[], Some(o151()))]).0
    }

    #[test]
    fn own_measurements() {
        let g = one();
        let s = section(&g, 0);
        assert!(s.starts_with(r#"<details class="reef-sec reef-perf" open>"#), "open by default: {s}");
        assert!(s.contains("inserts &lt;a second&gt;"), "escaped");
        assert!(s.contains("+94% ✓"), "{s}");
        assert!(s.contains("−41% ✓"), "{s}");
        assert!(s.contains("+65% ✗"), "a regression is marked in words too: {s}");
        assert!(s.contains("<th>Buffered</th><th>Shared</th>"));
        assert!(s.contains(r#"<tr data-metric="0">"#), "rows link to their bars");
        assert!(s.contains(r#"<th title="Whether the two ranges do not overlap">Disjoint</th>"#));
        assert!(s.contains("6,837 [6,783, 6,915]"));
        assert_eq!(s.matches("<svg").count(), 3, "change bars, pair bars, one sweep: {s}");
        assert!(s.contains(r#"data-node="0" data-chart="change""#));
        assert!(s.contains(r#"data-chart="sweep" data-arg="3""#));
        assert!(s.contains("reef-perf-whisker"));
        assert!(s.contains("reef-perf-before-line") && s.contains("reef-perf-after-line"));
        assert!(s.contains(r#"class="reef-perf-legend-item" data-series="1""#));
        assert!(s.contains("Eight rounds."));
        assert!(!s.contains("across the members"), "a leaf has no trend");
        assert!(!s.contains("Reset zoom"));
    }

    #[test]
    fn hovering_bars_names_the_metric() {
        let g = one();
        let h = hover(&g, 0, &ChartRef::Change, &ChartView::default(), 100.0, CH_TOP + CH_ROW * 2.0 + 5.0).unwrap();
        assert_eq!(h.metric, Some(2));
        assert!(h.tip.contains("read p99") && h.tip.contains("+65% ✗"), "{}", h.tip);
        assert!(hover(&g, 0, &ChartRef::Change, &ChartView::default(), 100.0, 2.0).is_none(), "above the rows");
        let h = hover(&g, 0, &ChartRef::Pair, &ChartView::default(), 100.0, LEGEND + 3.0).unwrap();
        assert_eq!(h.metric, Some(0));
        assert!(h.tip.contains("The ranges do not overlap."), "{}", h.tip);
        assert!(h.tip.contains("Higher is better."));
    }

    #[test]
    fn hovering_a_sweep_snaps_to_a_point() {
        let g = one();
        let r = ChartRef::Sweep(3);
        let v = ChartView::default();
        let (l, _, rgt, _) = plot_area(&g, 0, &r, &v).unwrap();
        // Near the right end: x 1024, both sides.
        let h = hover(&g, 0, &r, &v, rgt - 3.0, 50.0).unwrap();
        assert!(h.tip.contains("row size (KiB) 1,024"), "{}", h.tip);
        assert!(h.tip.contains("Buffered") && h.tip.contains("9 µs") && h.tip.contains("3 µs"), "{}", h.tip);
        assert!(h.tip.contains("0.33×"), "{}", h.tip);
        assert_eq!(h.overlay.matches("<circle").count(), 2);
        assert!(h.overlay.contains("reef-perf-cross"));
        // The middle point has a before only.
        let mid = l + (rgt - l) / 2.0;
        let h = hover(&g, 0, &r, &v, mid, 50.0).unwrap();
        assert!(h.tip.contains("64") && !h.tip.contains("Change"), "{}", h.tip);
        // Hiding Before leaves After's points only.
        let v = ChartView { hidden: vec![0], x: None };
        let h = hover(&g, 0, &r, &v, mid + 10.0, 50.0).unwrap();
        assert!(!h.tip.contains("Buffered") && h.tip.contains("1,024"), "64 has no After, so it snaps past it: {}", h.tip);
        assert!(hover(&g, 0, &r, &v, 2.0, 50.0).is_none(), "left of the plot");
    }

    #[test]
    fn zoom_and_legend_change_the_drawing() {
        let g = one();
        let r = ChartRef::Sweep(3);
        let full = chart(&g, 0, &r, &ChartView::default());
        assert_eq!(full.matches("reef-perf-dot").count(), 5);
        let zoomed = ChartView { hidden: vec![], x: Some((1.0, 64.0)) };
        let s = chart(&g, 0, &r, &zoomed);
        assert!(s.contains(r#"data-x0="1" data-x1="64""#), "{s}");
        assert!(s.contains("Reset zoom"));
        assert_eq!(s.matches("reef-perf-dot").count(), 3, "only the points inside: {s}");
        let hidden = chart(&g, 0, &r, &ChartView { hidden: vec![1], x: None });
        assert!(hidden.contains(r#"data-hidden="1""#));
        assert!(hidden.contains(r#"reef-perf-legend-item off" data-series="1""#));
        assert!(!hidden.contains("reef-perf-after-line"));
        // Zooming maps back from pixels.
        let (l, _, rgt, _) = plot_area(&g, 0, &r, &ChartView::default()).unwrap();
        assert!((x_at(&g, 0, &r, &ChartView::default(), l).unwrap() - 1.0).abs() < 1e-9);
        assert!((x_at(&g, 0, &r, &ChartView::default(), rgt).unwrap() - 1024.0).abs() < 1e-6);
        let pair = chart(&g, 0, &ChartRef::Pair, &ChartView { hidden: vec![0], x: None });
        assert_eq!(pair.matches(r#"<path class="reef-perf-before""#).count(), 0);
        assert_eq!(pair.matches(r#"<path class="reef-perf-after""#).count(), 3);
    }

    #[test]
    fn legend_clicks() {
        let v = ChartView::default();
        let alone = v.legend_click(1, 2, false);
        assert_eq!(alone.hidden, vec![0], "a click shows that series alone");
        assert_eq!(alone.legend_click(1, 2, false).hidden, Vec::<usize>::new(), "again: all");
        assert_eq!(alone.legend_click(0, 2, false).hidden, vec![1]);
        let t = v.legend_click(0, 2, true);
        assert_eq!(t.hidden, vec![0]);
        assert_eq!(t.legend_click(1, 2, true).hidden, vec![0], "never hides the last one shown");
        assert_eq!(t.legend_click(0, 2, true).hidden, Vec::<usize>::new());
        let parsed = ChartView::parse(Some("1"), Some("2.5"), Some("8"));
        assert_eq!(parsed, ChartView { hidden: vec![1], x: Some((2.5, 8.0)) });
        assert_eq!(ChartView::parse(None, Some("8"), Some("2")), ChartView::default());
        assert_eq!(ChartRef::parse("trend", "lat"), Some(ChartRef::Trend("lat".into())));
        assert_eq!(ChartRef::parse("sweep", "x"), None);
    }

    #[test]
    fn cutting_a_line_to_a_range() {
        let id = |x: f64| x;
        assert_eq!(cut(&[(0.0, 0.0), (10.0, 10.0)], 2.0, 4.0, id), vec![(2.0, 2.0), (4.0, 4.0)]);
        assert_eq!(cut(&[(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)], 0.0, 7.5, id), vec![(0.0, 0.0), (5.0, 5.0), (7.5, 2.5)]);
        assert_eq!(cut(&[(10.0, 0.0), (0.0, 10.0)], 2.0, 4.0, id).len(), 2);
    }

    fn group() -> Graph {
        let p = |b: f64, a: f64| Some(Performance { metrics: vec![Metric { key: Some("lat".into()), ..metric("latency", Better::Lower, Value::One(b), Value::One(a)) }], ..Performance::default() });
        Graph::build(vec![
            node("grp", None, &[], None),
            node("b", Some("grp"), &["a"], p(8.0, 6.0)),
            node("a", Some("grp"), &[], p(10.0, 8.0)),
            node("c", Some("grp"), &[], None),
        ])
        .0
    }

    #[test]
    fn a_group_trend_needs_two_members() {
        let g = group();
        let s = section(&g, g.index_of("grp").unwrap());
        assert!(s.contains("lat across the members"), "{s}");
        assert!(s.contains("1 metric across 2 members"), "{s}");
        assert!(s.contains(r#"data-chart="trend" data-arg="lat""#));
        assert!(s.find(r#"data-goto="a""#) < s.find(r#"data-goto="b""#), "plan order, not written order");
        let (g, _) = Graph::build(vec![node("grp", None, &[], None), node("a", Some("grp"), &[], None)]);
        assert_eq!(section(&g, 0), "", "no member measures anything");
    }

    #[test]
    fn hovering_a_trend_opens_its_member() {
        let g = group();
        let i = g.index_of("grp").unwrap();
        let r = ChartRef::Trend("lat".into());
        let v = ChartView::default();
        let (l, _, rgt, _) = plot_area(&g, i, &r, &v).unwrap();
        let h = hover(&g, i, &r, &v, rgt, 60.0).unwrap();
        assert_eq!(h.goto.as_deref(), Some("b"));
        assert!(h.tip.contains("&lt;b&gt;") && h.tip.contains("−25%"), "from 8 to 6: {}", h.tip);
        let h = hover(&g, i, &r, &v, l, 60.0).unwrap();
        assert_eq!(h.goto, None, "the start is no member");
        assert!(h.tip.contains("Before a"), "{}", h.tip);
    }

    #[test]
    fn docs_only() {
        let mut n = node("o2", None, &[], None);
        n.perf_doc = Some(PerfDoc { href: "x.html#o2".into(), label: "O2. Two".into(), heading: None, heading_text: None, tables: 3 });
        let (g, _) = Graph::build(vec![n]);
        let s = section(&g, 0);
        assert!(s.contains(r#"class="reef-perf-doc" data-href="x.html#o2" open>"#), "{s}");
        assert!(s.contains("From O2. Two <span class=\"reef-note\">3 tables</span>"), "{s}");
        assert!(!s.contains("<svg"));
        let (g, _) = Graph::build(vec![node("plain", None, &[], None)]);
        assert_eq!(section(&g, 0), "");
    }

    #[test]
    fn ticks() {
        assert_eq!(nice_ticks(0.0, 17.3, 4), vec![0.0, 5.0, 10.0, 15.0, 20.0]);
        assert_eq!(nice_ticks(0.0, 6915.0, 4), vec![0.0, 2000.0, 4000.0, 6000.0, 8000.0]);
        assert_eq!(tick(20000.0), "20K");
        assert_eq!(tick(2500.0), "2,500");
        assert_eq!(tick(1_500_000.0), "1.5M");
    }

    #[test]
    fn a_wide_sweep_is_log() {
        assert_eq!(y_log_scale(&[0.3, 480.0]).unwrap().2, vec![0.1, 1.0, 10.0, 100.0, 1000.0]);
        assert_eq!(y_log_scale(&[2.0, 150.0]).unwrap().2, vec![1.0, 10.0, 100.0, 1000.0]);
        assert_eq!(y_log_scale(&[1.5, 90.0]).unwrap().2, vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]);
        assert!(y_log_scale(&[1.0, 40.0]).is_none());
        assert!(y_log_scale(&[0.0, 400.0]).is_none());
    }

    #[test]
    fn bars_round_their_far_end() {
        let right = hbar(10.0, 100.0, 0.0, 12.0);
        assert!(right.starts_with("M10.0,0.0H96.0A4.0"), "{right}");
        let left = hbar(100.0, 10.0, 0.0, 12.0);
        assert!(left.contains("H14.0A4.0,4.0 0 0 0"), "{left}");
        assert_eq!(fit("abcdefghij", 30.0), "abcd…");
    }
}
