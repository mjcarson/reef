//! Measurements of how a piece of work changed performance: what an author writes under a
//! node's `performance:` key, and the arithmetic the sidebar draws from it.

use serde::{Deserialize, Serialize};

/// A node's measurements: a set of metrics, each measured before and after the work.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Performance {
    /// Names for the two sides, such as `[Buffered, Shared]`. Default: Before and After.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
    /// Markdown shown under the table: how the figures were taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Performance {
    pub fn before_label(&self) -> &str {
        self.labels.first().map_or("Before", String::as_str)
    }
    pub fn after_label(&self) -> &str {
        self.labels.get(1).map_or("After", String::as_str)
    }

    /// What is wrong with these measurements, if anything, one message a problem.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.labels.len() > 2 {
            out.push(format!("performance: `labels` names two sides, not {}", self.labels.len()));
        }
        for m in &self.metrics {
            let name = &m.name;
            if name.trim().is_empty() {
                out.push("performance: a metric with no `name`".into());
            }
            if m.before.is_none() && m.after.is_none() && m.points.is_empty() {
                out.push(format!("performance: metric `{name}` has neither `before`/`after` nor `points`"));
            }
            for (side, v) in [("before", &m.before), ("after", &m.after)] {
                if let Some(v) = v
                    && !v.is_ordered()
                {
                    out.push(format!("performance: metric `{name}`'s `{side}` is not [median, lo, hi] with lo <= median <= hi"));
                }
            }
            if m.before.is_some_and(|b| b.mid() == 0.0) && m.after.is_some() {
                out.push(format!("performance: metric `{name}`'s `before` is 0, so its change cannot be a ratio"));
            }
            if !m.points.is_empty() && m.points.iter().all(|p| p.1.is_none() && p.2.is_none()) {
                out.push(format!("performance: metric `{name}`'s `points` hold no values"));
            }
        }
        out
    }
}

/// Which way a metric improves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Better {
    /// Times, latencies, bytes: less is better.
    #[default]
    #[serde(alias = "less", alias = "smaller", alias = "down")]
    Lower,
    /// Throughput, rates: more is better.
    #[serde(alias = "more", alias = "larger", alias = "bigger", alias = "up")]
    Higher,
}

/// One measured figure.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    /// Matches the same metric on other nodes, so a group can draw how it moved across its
    /// members. Default: the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub better: Better,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// What a sweep's points are measured across, such as "row size (KiB)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    /// A sweep: `[x, before, after]`, either value possibly missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<(f64, Option<f64>, Option<f64>)>,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

impl Metric {
    pub fn key(&self) -> &str {
        self.key.as_deref().unwrap_or(&self.name)
    }

    /// after / before, when both are there.
    pub fn ratio(&self) -> Option<f64> {
        let (b, a) = (self.before?.mid(), self.after?.mid());
        (b != 0.0 && (a / b).is_finite()).then(|| a / b)
    }

    /// Whether the change went the way this metric improves. `None` when there is no change to
    /// judge, or the figures are equal.
    pub fn improved(&self) -> Option<bool> {
        let r = self.ratio()?;
        improved(r, self.better)
    }

    /// Whether both sides have ranges and the ranges do not overlap, the rule for a change that
    /// is not noise. `None` when a side has no range.
    pub fn disjoint(&self) -> Option<bool> {
        match (self.before?, self.after?) {
            (Value::Range([_, bl, bh]), Value::Range([_, al, ah])) => Some(bh < al || ah < bl),
            _ => None,
        }
    }
}

/// Whether a ratio after/before went the way `better` says improves.
pub fn improved(ratio: f64, better: Better) -> Option<bool> {
    if ratio == 1.0 {
        return None;
    }
    Some(match better {
        Better::Lower => ratio < 1.0,
        Better::Higher => ratio > 1.0,
    })
}

/// A figure: one number, or a median with its range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    One(f64),
    /// `[median, lo, hi]`.
    Range([f64; 3]),
}

impl Value {
    pub fn mid(self) -> f64 {
        match self {
            Value::One(v) => v,
            Value::Range([m, _, _]) => m,
        }
    }
    pub fn range(self) -> Option<(f64, f64)> {
        match self {
            Value::One(_) => None,
            Value::Range([_, lo, hi]) => Some((lo, hi)),
        }
    }
    pub fn lo(self) -> f64 {
        self.range().map_or(self.mid(), |r| r.0)
    }
    pub fn hi(self) -> f64 {
        self.range().map_or(self.mid(), |r| r.1)
    }
    fn is_ordered(self) -> bool {
        match self {
            Value::One(v) => v.is_finite(),
            Value::Range([m, lo, hi]) => lo <= m && m <= hi && lo.is_finite() && hi.is_finite(),
        }
    }
}

/// A ratio after/before as a change: `−41%` and `+22%`, or `1.94×` and `0.40×` once the change
/// reaches double or half.
pub fn change(ratio: f64) -> String {
    if !(0.5..2.0).contains(&ratio) {
        let digits = if ratio >= 10.0 { 1 } else { 2 };
        return format!("{ratio:.digits$}×");
    }
    let pct = (ratio - 1.0) * 100.0;
    let digits = if pct.abs() < 10.0 { 1 } else { 0 };
    let s = format!("{:.digits$}", pct.abs());
    if s.trim_start_matches(['0', '.']).is_empty() {
        return "±0%".into();
    }
    format!("{}{s}%", if pct < 0.0 { "−" } else { "+" })
}

/// A number as people write it: thousands grouped, about three significant figures.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let a = v.abs();
    let digits = if a == 0.0 || a >= 100.0 {
        0
    } else if a >= 10.0 {
        1
    } else if a >= 1.0 {
        2
    } else {
        // Three significant figures below one.
        (2 - a.log10().floor() as i32).clamp(0, 9) as usize
    };
    let s = format!("{a:.digits$}");
    let (int, frac) = s.split_once('.').map_or((s.as_str(), None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (k, c) in int.chars().enumerate() {
        if k > 0 && (int.len() - k) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let frac = frac.map(|f| f.trim_end_matches('0')).filter(|f| !f.is_empty());
    let sign = if v < 0.0 && s.chars().any(|c| ('1'..='9').contains(&c)) { "-" } else { "" };
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// A figure with its range: `6,837 [6,783, 6,915]`.
pub fn value(v: Value) -> String {
    match v {
        Value::One(x) => number(x),
        Value::Range([m, lo, hi]) => format!("{} [{}, {}]", number(m), number(lo), number(hi)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(better: Better, before: Value, after: Value) -> Metric {
        Metric { name: "m".into(), better, before: Some(before), after: Some(after), ..Metric::default() }
    }

    #[test]
    fn changes() {
        assert_eq!(change(0.59), "−41%");
        assert_eq!(change(1.22), "+22%");
        assert_eq!(change(1.94), "+94%");
        assert_eq!(change(2.32), "2.32×");
        assert_eq!(change(0.4), "0.40×");
        assert_eq!(change(0.018), "0.02×");
        assert_eq!(change(432.8), "432.8×");
        assert_eq!(change(1.0), "±0%");
        assert_eq!(change(1.031), "+3.1%");
    }

    #[test]
    fn numbers() {
        assert_eq!(number(6837.0), "6,837");
        assert_eq!(number(1188548.0), "1,188,548");
        assert_eq!(number(17.26), "17.3");
        assert_eq!(number(0.956), "0.956");
        assert_eq!(number(0.0109), "0.0109");
        assert_eq!(number(3.0), "3");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(number(0.0), "0");
        assert_eq!(value(Value::Range([6837.0, 6783.0, 6915.0])), "6,837 [6,783, 6,915]");
    }

    #[test]
    fn improvement_both_ways() {
        let up = metric(Better::Higher, Value::One(3523.0), Value::One(6837.0));
        assert_eq!(up.improved(), Some(true));
        let down = metric(Better::Lower, Value::One(17.2), Value::One(10.2));
        assert_eq!(down.improved(), Some(true));
        let worse = metric(Better::Lower, Value::One(0.956), Value::One(1.58));
        assert_eq!(worse.improved(), Some(false));
        let same = metric(Better::Lower, Value::One(2.0), Value::One(2.0));
        assert_eq!(same.improved(), None);
    }

    #[test]
    fn disjointness() {
        let m = metric(Better::Higher, Value::Range([3523.0, 3476.0, 3547.0]), Value::Range([6837.0, 6783.0, 6915.0]));
        assert_eq!(m.disjoint(), Some(true));
        let m = metric(Better::Higher, Value::Range([850.0, 788.0, 905.0]), Value::Range([819.0, 805.0, 851.0]));
        assert_eq!(m.disjoint(), Some(false));
        let m = metric(Better::Higher, Value::One(1.0), Value::Range([2.0, 1.0, 3.0]));
        assert_eq!(m.disjoint(), None);
    }

    #[test]
    fn problems() {
        let p = Performance {
            metrics: vec![
                Metric { name: "none".into(), ..Metric::default() },
                metric(Better::Lower, Value::Range([5.0, 6.0, 7.0]), Value::One(1.0)),
                metric(Better::Lower, Value::One(0.0), Value::One(1.0)),
            ],
            ..Performance::default()
        };
        let msgs = p.problems();
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        assert!(msgs[0].contains("neither"));
        assert!(msgs[1].contains("lo <= median"));
        assert!(msgs[2].contains("is 0"));
    }

    #[test]
    fn reads_json_forms() {
        let m: Metric = serde_json::from_str(r#"{"name":"a","better":"more","before":3,"after":[2,1,3],"points":[[1,2,null]]}"#).unwrap();
        assert_eq!(m.better, Better::Higher);
        assert_eq!(m.before, Some(Value::One(3.0)));
        assert_eq!(m.after, Some(Value::Range([2.0, 1.0, 3.0])));
        assert_eq!(m.points, vec![(1.0, Some(2.0), None)]);
        assert_eq!(m.key(), "a");
    }
}
