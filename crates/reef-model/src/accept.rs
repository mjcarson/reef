//! Acceptance criteria: how a piece of work will be known to be done. Each is a unit test, an
//! integration test, a performance figure or a feature that must work. A performance criterion
//! can name a metric from a `performance:` block, whose chart the sidebar draws beside it, and a
//! target that the metric's `after` figure is checked against.

use crate::perf::Metric;
use serde::{Deserialize, Deserializer, Serialize};

/// What kind of check a criterion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Unit,
    Integration,
    Performance,
    Feature,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Unit, Kind::Integration, Kind::Performance, Kind::Feature];

    /// The kind an author's word names, if any.
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "unit" | "unit-test" | "unit-tests" => Kind::Unit,
            "integration" | "integration-test" | "integration-tests" | "e2e" | "end-to-end" => Kind::Integration,
            "performance" | "perf" | "bench" | "benchmark" => Kind::Performance,
            "feature" | "feature-support" | "support" | "supports" => Kind::Feature,
            _ => return None,
        })
    }

    /// The name used in markup: classes, filter keys, `data-` attributes.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Unit => "unit",
            Kind::Integration => "integration",
            Kind::Performance => "perf",
            Kind::Feature => "feature",
        }
    }

    /// The name shown to a reader.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Unit => "unit test",
            Kind::Integration => "integration test",
            Kind::Performance => "performance",
            Kind::Feature => "feature",
        }
    }

    /// The one letter a box's pill shows.
    pub fn letter(self) -> &'static str {
        match self {
            Kind::Unit => "U",
            Kind::Integration => "I",
            Kind::Performance => "P",
            Kind::Feature => "F",
        }
    }

    /// Its place in [`Kind::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Whether a criterion is met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Met {
    #[default]
    #[serde(alias = "open", alias = "pending")]
    Todo,
    #[serde(alias = "pass", alias = "passed", alias = "passes", alias = "done", alias = "yes")]
    Met,
    #[serde(alias = "fail", alias = "failed", alias = "fails", alias = "no")]
    Unmet,
}

impl Met {
    pub fn as_str(self) -> &'static str {
        match self {
            Met::Todo => "todo",
            Met::Met => "met",
            Met::Unmet => "unmet",
        }
    }
}

/// One criterion as written. Its `text` is markdown, rendered to HTML by the preprocessor.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Criterion {
    pub kind: Kind,
    pub text: String,
    /// The author's word on whether it is met. Default: checked against `metric`'s target, or
    /// to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Met>,
    /// The test that checks it, such as `tests/scan.rs::dropped_client`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    /// A performance metric, by key or name, whose chart is drawn beside the criterion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// The node whose metric or chart this is. Default: the criterion's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// A whole chart of the Performance section: `change` or `pair`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_least: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_most: Option<f64>,
}

impl Criterion {
    pub fn has_target(&self) -> bool {
        self.at_least.is_some() || self.at_most.is_some()
    }

    fn passes(&self, v: f64) -> bool {
        self.at_least.is_none_or(|t| v >= t) && self.at_most.is_none_or(|t| v <= t)
    }

    /// Whether `m`'s `after` figure meets this criterion's target: its median, or for a sweep
    /// every point measured after. `None` when there is no target or no figure to check.
    pub fn check(&self, m: &Metric) -> Option<bool> {
        if !self.has_target() {
            return None;
        }
        if let Some(a) = m.after {
            return Some(self.passes(a.mid()));
        }
        let after: Vec<f64> = m.points.iter().filter_map(|p| p.2).collect();
        if after.is_empty() {
            return None;
        }
        Some(after.iter().all(|&v| self.passes(v)))
    }

    /// The target in words: `≥ 6,000`, `≤ 1.2 ms`, or both.
    pub fn target_text(&self, unit: Option<&str>) -> Option<String> {
        let unit = unit.map_or(String::new(), |u| format!(" {u}"));
        let parts: Vec<String> = [("≥", self.at_least), ("≤", self.at_most)]
            .into_iter()
            .filter_map(|(op, v)| v.map(|v| format!("{op} {}{unit}", crate::perf::number(v))))
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Long {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "criterion", alias = "description")]
    text: Option<String>,
    #[serde(default)]
    status: Option<Met>,
    #[serde(default)]
    test: Option<String>,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    chart: Option<String>,
    #[serde(default, alias = "at_least", alias = "min")]
    at_least: Option<f64>,
    #[serde(default, alias = "at_most", alias = "max")]
    at_most: Option<f64>,
}

/// Either the long form, `{ kind: unit, text: ... }`, or a shorthand whose one kind key holds
/// the text or the rest of the criterion: `{ unit: ... }`, `{ perf: { text: ..., metric: ... } }`.
/// A shorthand may carry other keys beside its kind key too.
impl<'de> Deserialize<'de> for Criterion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        use serde_json::Value as J;
        let mut map = match J::deserialize(d)? {
            J::Object(m) => m,
            J::String(_) => return Err(D::Error::custom("an acceptance criterion needs a kind: write `- unit: <text>`, or `kind:` and `text:`")),
            _ => return Err(D::Error::custom("an acceptance criterion is a map, such as `- unit: <text>`")),
        };
        let kind_keys: Vec<String> = map.keys().filter(|k| *k != "kind" && Kind::parse(k).is_some()).cloned().collect();
        let kind = match (map.get("kind"), kind_keys.as_slice()) {
            (Some(_), []) => None,
            (None, [k]) => {
                let key = k.clone();
                match map.remove(&key).unwrap_or(J::Null) {
                    J::String(s) => {
                        map.insert("text".into(), J::String(s));
                    }
                    J::Object(inner) => map.extend(inner),
                    J::Null => {}
                    _ => return Err(D::Error::custom(format!("acceptance criterion `{key}:` holds its text, or a map"))),
                }
                Kind::parse(&key)
            }
            (None, []) => return Err(D::Error::custom("an acceptance criterion needs a kind: unit, integration, performance or feature")),
            _ => return Err(D::Error::custom(format!("an acceptance criterion names more than one kind: {}", kind_keys.join(", ")))),
        };
        let long: Long = serde_json::from_value(J::Object(map)).map_err(D::Error::custom)?;
        let kind = match (kind, long.kind) {
            (Some(k), _) => k,
            (None, Some(k)) => Kind::parse(&k).ok_or_else(|| {
                D::Error::custom(format!("acceptance criterion kind `{k}` is not unit, integration, performance or feature"))
            })?,
            (None, None) => unreachable!("a kind key or `kind:`"),
        };
        Ok(Criterion {
            kind,
            text: long.text.unwrap_or_default(),
            status: long.status,
            test: long.test,
            metric: long.metric,
            node: long.node,
            chart: long.chart,
            at_least: long.at_least,
            at_most: long.at_most,
        })
    }
}

/// Which chart a criterion draws, once its references are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccChart {
    /// The metric at this index of the node's metrics.
    Metric(usize),
    /// The node's change bars.
    Change,
    /// The node's before and after bars.
    Pair,
}

/// A criterion resolved against the plan: whether it is met, and the chart it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resolved {
    pub status: Met,
    /// The node the chart is drawn from, and which.
    pub chart: Option<(usize, AccChart)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::Value;

    fn json(s: &str) -> Criterion {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn long_and_short_forms() {
        let c = json(r#"{"kind":"integration","text":"x","test":"t"}"#);
        assert_eq!((c.kind, c.text.as_str(), c.test.as_deref()), (Kind::Integration, "x", Some("t")));
        let c = json(r#"{"unit":"y"}"#);
        assert_eq!((c.kind, c.text.as_str()), (Kind::Unit, "y"));
        let c = json(r#"{"perf":{"text":"z","metric":"inserts","at-least":6000}}"#);
        assert_eq!((c.kind, c.metric.as_deref(), c.at_least), (Kind::Performance, Some("inserts"), Some(6000.0)));
        let c = json(r#"{"feature-support":"w","status":"passed"}"#);
        assert_eq!((c.kind, c.status), (Kind::Feature, Some(Met::Met)));
        assert!(serde_json::from_str::<Criterion>(r#"{"unit":"a","perf":"b"}"#).is_err());
        assert!(serde_json::from_str::<Criterion>(r#"{"text":"a"}"#).is_err());
        assert!(serde_json::from_str::<Criterion>(r#"{"kind":"nope","text":"a"}"#).is_err());
        assert!(serde_json::from_str::<Criterion>(r#""just text""#).is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let c = Criterion { kind: Kind::Performance, text: "t".into(), metric: Some("m".into()), at_most: Some(2.0), ..Criterion::default() };
        let back: Criterion = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn checks_a_target() {
        let c = Criterion { kind: Kind::Performance, at_least: Some(6000.0), ..Criterion::default() };
        let mut m = Metric { name: "m".into(), after: Some(Value::Range([6837.0, 6783.0, 6915.0])), ..Metric::default() };
        assert_eq!(c.check(&m), Some(true));
        m.after = Some(Value::One(10.0));
        assert_eq!(c.check(&m), Some(false));
        m.after = None;
        m.points = vec![(1.0, None, Some(7000.0)), (2.0, Some(1.0), Some(5000.0))];
        assert_eq!(c.check(&m), Some(false));
        m.points = vec![];
        assert_eq!(c.check(&m), None);
        assert_eq!(c.target_text(Some("ms")).as_deref(), Some("≥ 6,000 ms"));
    }
}
