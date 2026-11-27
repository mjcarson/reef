//! reefadm's record of what it deployed, which upgrade, uninstall and the admin commands read
//!
//! It is `/etc/reef/reefadm.toml` for a team server, and `reefadm.toml` in the user's data home
//! for a user unit: which kind of deployment, from which repo and ref, the release in use, the
//! one before it, and the schema each reads, by table. Fingerprints are kept as strings, since
//! they pass TOML's largest integer.

use crate::exec::{As, File, Runner};
use crate::layout::{Kind, Layout, Mode};
use crate::schema::Schema;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What reefadm deployed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// A team server or a user unit
    pub mode: Mode,
    /// reef-app or reef-serve
    pub kind: Kind,
    /// Where reef is fetched from
    pub repo: String,
    /// The ref an upgrade fetches, unless told another
    #[serde(rename = "ref")]
    pub reference: String,
    /// The release in use
    pub release: Release,
    /// The release before it, which an upgrade that fails goes back to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Release>,
    /// The first admin deploy added, if it added one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_admin: Option<String>,
}

/// One release
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Its name, its directory's under the releases
    pub name: String,
    /// reef's commit
    pub reef: String,
    /// shoal's commit
    pub shoal: String,
    /// glommio's commit
    pub glommio: String,
    /// The nightly it was built with
    pub toolchain: String,
    /// When it was made current, in seconds since the epoch
    pub deployed: u64,
    /// The schema's id, if it is known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Each table's fingerprint, by row type, if they are known
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tables: BTreeMap<String, String>,
}

impl Release {
    /// Records a schema's id and tables
    ///
    /// # Arguments
    ///
    /// * `schema` - The schema, if it is known
    pub fn with_schema(mut self, schema: Option<&Schema>) -> Self {
        if let Some(schema) = schema {
            self.schema = Some(schema.id.to_string());
            self.tables = schema
                .tables
                .iter()
                .map(|(name, fingerprint)| (name.clone(), fingerprint.to_string()))
                .collect();
        }
        self
    }
}

impl State {
    /// Reads the record of a layout's deployment, if there is one
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    /// * `layout` - The deployment's paths
    pub fn read(runner: &mut dyn Runner, layout: &Layout) -> Result<Option<Self>> {
        // every account may read it, as it holds no secret
        let Some(text) = crate::exec::read(runner, &layout.state, As::Me)? else {
            return Ok(None);
        };
        let state = toml::from_str(&text)
            .with_context(|| format!("reading reefadm's record {}", layout.state.display()))?;
        Ok(Some(state))
    }

    /// Writes the record of a layout's deployment
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    /// * `layout` - The deployment's paths
    pub fn write(&self, runner: &mut dyn Runner, layout: &Layout) -> Result<()> {
        let keys = toml::to_string(self).context("writing reefadm's record")?;
        runner.write(&File {
            path: layout.state.clone(),
            text: format!("# What reefadm deployed here, which its next upgrade reads.\n{keys}"),
            mode: 0o644,
            runs_as: layout.owner(),
            owner: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Release, State};
    use crate::layout::{Kind, Mode};
    use crate::schema::Schema;

    /// A record round trips through TOML, its fingerprints past TOML's integers intact
    #[test]
    fn a_record_round_trips() {
        let schema = Schema {
            id: 13509127152932721421,
            tables: vec![("ProjectRow".into(), 12832186597557680855)],
        };
        let state = State {
            mode: Mode::System,
            kind: Kind::App,
            repo: "https://github.com/mjcarson/reef.git".into(),
            reference: "CentralizedPlanner".into(),
            release: Release {
                name: "992226c4a1b2-20261127072258".into(),
                reef: "992226c".into(),
                shoal: "dbfb82e".into(),
                glommio: "25d2c35".into(),
                toolchain: "nightly-2026-08-21".into(),
                deployed: 1_795_764_178,
                schema: None,
                tables: Default::default(),
            }
            .with_schema(Some(&schema)),
            previous: None,
            first_admin: Some("ada".into()),
        };
        let text = toml::to_string(&state).unwrap();
        assert!(text.contains("ref = \"CentralizedPlanner\""), "{text}");
        assert!(
            text.contains("ProjectRow = \"12832186597557680855\""),
            "{text}"
        );
        assert_eq!(toml::from_str::<State>(&text).unwrap(), state);
    }
}
