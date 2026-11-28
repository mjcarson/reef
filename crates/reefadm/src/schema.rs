//! The schema a release reads and writes: its structural id and each table's fingerprint
//!
//! A built `reef-serve version --json` says it. A commit's source says it too, in the pin test of
//! `central/reef-service/src/schema.rs`, which the test keeps equal to the binary's, so an
//! upgrade can compare schemas before it spends a build on one it would refuse.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// A schema: its id and its tables, in the order it serves them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// The id, which moves with any change to a row, a table or a projection
    pub id: u64,
    /// Each table's row type and its fingerprint
    pub tables: Vec<(String, u64)>,
}

/// What `reef-serve version --json` prints
#[derive(Deserialize)]
struct Printed {
    /// The schema's id
    schema: u64,
    /// Each table as `[name, fingerprint]`
    tables: Vec<(String, u64)>,
}

impl Schema {
    /// Reads what `reef-serve version --json` printed
    ///
    /// # Arguments
    ///
    /// * `text` - Its output
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::schema::Schema;
    ///
    /// let schema = Schema::from_json(r#"{"schema":1,"tables":[["ProjectRow",2]]}"#).unwrap();
    /// assert_eq!(schema.tables, [("ProjectRow".to_string(), 2)]);
    /// ```
    pub fn from_json(text: &str) -> Result<Self> {
        let printed: Printed =
            serde_json::from_str(text.trim()).context("reading `reef-serve version --json`")?;
        Ok(Schema {
            id: printed.schema,
            tables: printed.tables,
        })
    }

    /// Reads the pin in a commit's `central/reef-service/src/schema.rs`
    ///
    /// # Arguments
    ///
    /// * `source` - The file's text
    pub fn from_pin(source: &str) -> Result<Self> {
        // the id's constant, then the tables' list, as the pin test writes them
        let id = source
            .lines()
            .find_map(|line| line.trim().strip_prefix("const SCHEMA_ID: u64 = "))
            .and_then(|rest| rest.trim_end_matches(';').trim().parse().ok())
            .context("schema.rs has no `const SCHEMA_ID: u64 = …;` pin")?;
        let start = source
            .find("const TABLES")
            .context("schema.rs has no `const TABLES` pin")?;
        let mut tables = Vec::new();
        for line in source[start..].lines().skip(1) {
            let line = line.trim();
            if line.starts_with("];") {
                break;
            }
            // each `("NameRow", 1234),`
            let entry = line
                .strip_prefix("(\"")
                .and_then(|line| line.strip_suffix("),"))
                .and_then(|line| line.split_once("\", "));
            let Some((name, fingerprint)) = entry else {
                bail!("schema.rs's `TABLES` pin has a line reefadm can't read: {line}");
            };
            tables.push((name.to_string(), fingerprint.trim().parse()?));
        }
        Ok(Schema { id, tables })
    }

    /// The schema of a built `reef-serve`, or none in a dry run, which has built nothing
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    /// * `serve` - The `reef-serve` to ask
    pub fn of_binary(runner: &mut dyn crate::exec::Runner, serve: &Path) -> Result<Option<Self>> {
        if runner.dry() {
            return Ok(None);
        }
        let output = runner.probe(
            &crate::exec::Cmd::new(serve.display().to_string()).args(["version", "--json"]),
        )?;
        if !output.ok() {
            bail!(
                "{} can't say its schema: {}",
                serve.display(),
                output.stderr.trim()
            );
        }
        Schema::from_json(&output.stdout).map(Some)
    }
}

/// How a new schema's tables compare with the ones a data directory was written with
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Tables the new schema adds, which it starts empty
    pub added: Vec<String>,
    /// Tables whose rows changed shape, which it can't read back
    pub changed: Vec<String>,
    /// Tables it no longer has
    pub removed: Vec<String>,
}

impl Verdict {
    /// Compares the tables a release recorded with a new schema's
    ///
    /// # Arguments
    ///
    /// * `old` - The running release's tables, each fingerprint as recorded
    /// * `new` - The new schema
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::schema::{Schema, Verdict};
    /// use std::collections::BTreeMap;
    ///
    /// let old = BTreeMap::from([("NodeRow".to_string(), "1".to_string())]);
    /// let new = Schema { id: 9, tables: vec![("NodeRow".into(), 1), ("UserRow".into(), 2)] };
    /// let verdict = Verdict::between(&old, &new);
    /// assert_eq!(verdict.added, ["UserRow"]);
    /// assert!(verdict.reads_back());
    /// ```
    pub fn between(old: &BTreeMap<String, String>, new: &Schema) -> Self {
        let mut verdict = Verdict::default();
        // each new table is either kept as it was, changed, or new
        for (name, fingerprint) in &new.tables {
            match old.get(name) {
                Some(kept) if *kept == fingerprint.to_string() => {}
                Some(_) => verdict.changed.push(name.clone()),
                None => verdict.added.push(name.clone()),
            }
        }
        // and each old table the new schema doesn't have is gone
        verdict.removed = old
            .keys()
            .filter(|name| !new.tables.iter().any(|(table, _)| table == *name))
            .cloned()
            .collect();
        verdict
    }

    /// Whether the new schema reads back what the old one wrote: tables only added
    pub fn reads_back(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }

    /// What moved, said plainly, for a refusal
    pub fn said(&self) -> String {
        let mut parts = Vec::new();
        if !self.changed.is_empty() {
            parts.push(format!("changed {}", self.changed.join(", ")));
        }
        if !self.removed.is_empty() {
            parts.push(format!("removed {}", self.removed.join(", ")));
        }
        if !self.added.is_empty() {
            parts.push(format!("added {}", self.added.join(", ")));
        }
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::{Schema, Verdict};
    use std::collections::BTreeMap;

    /// A table added passes, and a table changed or gone is refused by name
    #[test]
    fn a_table_added_passes_and_one_changed_or_gone_is_refused() {
        let old = BTreeMap::from([
            ("NodeRow".to_string(), "1".to_string()),
            ("PageRow".to_string(), "2".to_string()),
            ("LoopRow".to_string(), "3".to_string()),
        ]);
        // the same tables, and one more, read back
        let added = Schema {
            id: 2,
            tables: vec![
                ("NodeRow".into(), 1),
                ("PageRow".into(), 2),
                ("LoopRow".into(), 3),
                ("EditRow".into(), 4),
            ],
        };
        let verdict = Verdict::between(&old, &added);
        assert!(verdict.reads_back(), "{verdict:?}");
        assert_eq!(verdict.added, ["EditRow"]);
        // a row that changed shape, and a table dropped, don't
        let moved = Schema {
            id: 3,
            tables: vec![("NodeRow".into(), 10), ("PageRow".into(), 2)],
        };
        let verdict = Verdict::between(&old, &moved);
        assert!(!verdict.reads_back());
        assert_eq!(verdict.changed, ["NodeRow"]);
        assert_eq!(verdict.removed, ["LoopRow"]);
        assert_eq!(verdict.said(), "changed NodeRow; removed LoopRow");
    }

    /// This commit's pin reads, with every table it lists
    #[test]
    fn this_commits_pin_reads() {
        let source = include_str!("../../../central/reef-service/src/schema.rs");
        let pin = Schema::from_pin(source).unwrap();
        assert_eq!(pin.id, 14478251474438974043);
        assert_eq!(
            pin.tables[0],
            ("ProjectRow".to_string(), 12832186597557680855)
        );
        assert_eq!(
            pin.tables.last().map(|(name, _)| name.as_str()),
            Some("PlanDocsRow")
        );
        assert_eq!(pin.tables.len(), 18);
        // and a file with no pin says so
        assert!(Schema::from_pin("fn main() {}").is_err());
    }

    /// The binary's JSON reads past 2^53, as reef-serve prints it
    #[test]
    fn the_binarys_json_reads_exactly() {
        let printed =
            r#"{"schema":13509127152932721421,"tables":[["ProjectRow",12832186597557680855]]}"#;
        let schema = Schema::from_json(printed).unwrap();
        assert_eq!(schema.id, 13509127152932721421);
        assert_eq!(
            schema.tables,
            [("ProjectRow".to_string(), 12832186597557680855)]
        );
    }
}
