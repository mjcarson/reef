//! The schema a release reads and writes: its structural id and each table's fingerprint
//!
//! A built `reef-serve version --json` says it. A commit's source says it too, in the pin test of
//! `central/reef-service/src/schema.rs`, which the test keeps equal to the binary's, so an
//! upgrade can compare schemas before it spends a build on one it would refuse.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
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

#[cfg(test)]
mod tests {
    use super::Schema;

    /// This commit's pin reads, with every table it lists
    #[test]
    fn this_commits_pin_reads() {
        let source = include_str!("../../../central/reef-service/src/schema.rs");
        let pin = Schema::from_pin(source).unwrap();
        assert_eq!(pin.id, 13509127152932721421);
        assert_eq!(
            pin.tables[0],
            ("ProjectRow".to_string(), 12832186597557680855)
        );
        assert_eq!(
            pin.tables.last().map(|(name, _)| name.as_str()),
            Some("PlanBranchRow")
        );
        assert_eq!(pin.tables.len(), 16);
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
