//! A release built on one machine, for another's CPU, and installed there
//!
//! The machine reefadm is typed on builds, and the target installs. They meet here:
//! - **[`Facts`]**, what reefadm's copy on the target says before anything is built: its CPU's
//!   features, its OS, its record, and anything that would stop the command there.
//! - **The builder's half:** [`prepare`] installs what building needs here and picks the target's
//!   [`Class`], [`make`] fetches the siblings and builds for it, and [`stage_out`] stages the
//!   release, unprivileged, in an out directory emptied first, which [`ship`] streams to the
//!   target as one tar.
//! - **The target's half:** [`shipped`] reads the release unpacked in [`SHIPPED`], refusing one
//!   whose reef-serve was compiled with a feature this CPU lacks, and the command installs it as
//!   it would one built here.

use crate::compile::{self, Built, Stamp};
use crate::cpu::{self, Class};
use crate::exec::{self, As, Cmd, File, Runner, quote};
use crate::layout::Kind;
use crate::release::{self, RECORD, Record};
use crate::schema::Schema;
use crate::sources::{self, Manifest, Revs};
use crate::state::{Release, State};
use crate::{Cx, Env, toolchain};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where a release is unpacked on the target, under the admin's home there
pub const SHIPPED: &str = ".cache/reefadm/shipped";

/// The record of an install made by hand that adopt takes over, beside the release's own
pub const ADOPTED: &str = "adopted.toml";

/// What reefadm's copy on a target says before anything is built for it
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    /// Its architecture, as Rust names it
    pub arch: String,
    /// Its OS, as os-release names it
    #[serde(default)]
    pub os: Option<String>,
    /// Whether it is the Ubuntu reefadm is made for
    #[serde(default)]
    pub ubuntu: bool,
    /// The admin there
    #[serde(default)]
    pub user: Option<String>,
    /// The admin's home there
    pub home: PathBuf,
    /// Its CPU's features, as std detects them there
    pub features: BTreeSet<String>,
    /// Whatever would stop the command there, a line each
    #[serde(default)]
    pub problems: Vec<String>,
    /// The server it runs or would run
    #[serde(default)]
    pub kind: Option<Kind>,
    /// reefadm's record there, for an upgrade
    #[serde(default)]
    pub state: Option<State>,
    /// The tables an install made by hand wrote its data with, as its own reef-serve says them
    #[serde(default)]
    pub tables: BTreeMap<String, String>,
}

impl Facts {
    /// This host's facts, before a command adds its own
    ///
    /// # Arguments
    ///
    /// * `cx` - The runner and the environment
    pub fn here(cx: &mut Cx) -> Result<Facts> {
        let (os, ubuntu) = match os_release(cx.runner)? {
            Some((name, ubuntu)) => (Some(name), ubuntu),
            None => (None, false),
        };
        Ok(Facts {
            arch: std::env::consts::ARCH.to_string(),
            os,
            ubuntu,
            user: cx.user(),
            home: cx.env.dir("HOME").unwrap_or_default(),
            features: cpu::detected(),
            ..Facts::default()
        })
    }

    /// Says the facts, as the line the builder reads
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    pub fn say(&self, runner: &mut dyn Runner) -> Result<()> {
        runner.say(&serde_json::to_string(self).context("writing the target's facts")?);
        Ok(())
    }

    /// Reads the facts a target's reefadm said: the last line it printed
    ///
    /// # Arguments
    ///
    /// * `printed` - What it printed
    pub fn read(printed: &str) -> Result<Facts> {
        let line = printed
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        serde_json::from_str(line).with_context(|| format!("reading the target's facts: {line}"))
    }
}

/// The OS's name, and whether it is the Ubuntu reefadm is made for, as `/etc/os-release` says
///
/// # Arguments
///
/// * `runner` - The runner
pub fn os_release(runner: &mut dyn Runner) -> Result<Option<(String, bool)>> {
    // os-release names the system, each value quoted or not
    let Some(text) = exec::read(runner, Path::new("/etc/os-release"), As::Me)? else {
        return Ok(None);
    };
    let field = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.trim().trim_matches('"').to_string())
    };
    let ubuntu = field("ID").as_deref() == Some("ubuntu")
        && field("VERSION_ID").as_deref() == Some(crate::commands::deploy::UBUNTU);
    Ok(Some((
        field("PRETTY_NAME").unwrap_or_else(|| "another system".to_string()),
        ubuntu,
    )))
}

/// Installs what building needs here, and picks the class a target's CPU is built for
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `manifest` - What reef's commit pins
/// * `kind` - The server being built
/// * `src` - The sources directory
/// * `target` - The target's features
/// * `named` - A CPU the admin named instead
pub fn prepare(
    cx: &mut Cx,
    manifest: &Manifest,
    kind: Kind,
    src: &Path,
    target: &BTreeSet<String>,
    named: Option<&str>,
) -> Result<Class> {
    toolchain::ensure(cx.runner, manifest, kind, src, cx.yes)?;
    // the class: named, or what the target and this machine share and the nightly knows
    let class = match named {
        Some(named) => Class::named(named),
        None => {
            let known = cpu::known_by(cx.runner, &manifest.toolchain)?;
            Class::of(target, &cpu::detected(), known.as_ref())?
        }
    };
    cx.runner
        .say(&format!("building the server for `{}`", class.flags));
    if !class.dropped.is_empty() {
        cx.runner.say(&format!(
            "  the target also has {}, which this machine's CPU hasn't, so the build leaves them out",
            class.dropped.join(", ")
        ));
    }
    Ok(class)
}

/// Fetches the siblings reef's commit pins and builds for a class, returning what was built and
/// the schema the built reef-serve says
///
/// # Arguments
///
/// * `cx` - The runner
/// * `src` - The sources directory
/// * `reef` - reef's commit
/// * `manifest` - What it pins
/// * `overrides` - The siblings fetched from elsewhere
/// * `kind` - The server being built
/// * `class` - The CPU class
pub fn make(
    cx: &mut Cx,
    src: &Path,
    reef: &str,
    manifest: &Manifest,
    overrides: &BTreeMap<String, String>,
    kind: Kind,
    class: &Class,
) -> Result<(Revs, Built, Option<Schema>)> {
    let revs = sources::fetch_siblings(cx.runner, src, reef, manifest, overrides)?;
    cx.runner.say(&format!(
        "building reef {} with shoal {} and glommio {}",
        short(&revs.reef),
        short(&revs.shoal),
        short(&revs.glommio)
    ));
    // the CLI is stamped with reef's commit and when it was made, which its updater orders by
    let stamp = Stamp {
        commit: revs.reef.clone(),
        committed: sources::committed(cx.runner, src, &revs.reef)?,
    };
    let built = compile::build(cx.runner, src, &manifest.toolchain, kind, class, &stamp)?;
    // its own word on its schema, which the builder may run, since it has every feature it uses
    let schema = Schema::of_binary(cx.runner, &built.serve)?;
    Ok((revs, built, schema))
}

/// What a release built here was built from: the ref, the revs, the toolchain, the class's flags
/// and the schema its reef-serve says
///
/// # Arguments
///
/// * `cx` - The time
/// * `repo` - Where reef was fetched from
/// * `reference` - The ref it was fetched at
/// * `kind` - The server it runs
/// * `revs` - The commits it was built from
/// * `manifest` - What reef's commit pins
/// * `class` - The CPU class it was built for
/// * `schema` - The schema its reef-serve says
#[allow(clippy::too_many_arguments)]
pub fn record(
    cx: &Cx,
    repo: &str,
    reference: &str,
    kind: Kind,
    revs: Revs,
    manifest: &Manifest,
    class: &Class,
    schema: Option<&Schema>,
) -> Record {
    Record {
        repo: repo.to_string(),
        reference: reference.to_string(),
        kind,
        release: Release {
            name: release::name(&revs.reef, cx.now),
            reef: revs.reef,
            shoal: revs.shoal,
            glommio: revs.glommio,
            toolchain: manifest.toolchain.clone(),
            deployed: 0,
            schema: None,
            tables: Default::default(),
            build: Some(class.flags.clone()),
        }
        .with_schema(schema),
    }
}

/// A commit's first 12 characters
///
/// # Arguments
///
/// * `rev` - The commit
pub fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

/// The directory a release is staged in here to be shipped
///
/// # Arguments
///
/// * `env` - The admin's environment
pub fn out_dir(env: &Env) -> Result<PathBuf> {
    let Some(home) = env.dir("HOME") else {
        bail!("reefadm needs $HOME, where it stages the release it ships");
    };
    Ok(env
        .dir("XDG_DATA_HOME")
        .unwrap_or_else(|| home.join(".local/share"))
        .join("reefadm/out"))
}

/// Stages a release here, unprivileged, in the out directory emptied first, and returns its
/// directory: `bin/`, `app/`, `release.toml`, and an install it takes over's record
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `built` - What the build made
/// * `record` - What it was built from
/// * `adopted` - The install made by hand it takes over, for adopt
pub fn stage_out(
    cx: &mut Cx,
    built: &Built,
    record: &Record,
    adopted: Option<&Release>,
) -> Result<PathBuf> {
    let out = out_dir(&cx.env)?;
    cx.runner.run(&Cmd::new("rm").arg("-rf").arg(&out))?;
    cx.runner
        .run(&Cmd::new("install").args(["-d", "-m", "0755"]).arg(&out))?;
    let dir = release::stage_in(cx.runner, &out, As::Me, built, record)?;
    if let Some(adopted) = adopted {
        cx.runner.write(&File {
            path: dir.join(ADOPTED),
            text: toml::to_string(adopted).context("writing the adopted install's record")?,
            mode: 0o644,
            runs_as: As::Me,
            owner: None,
        })?;
    }
    Ok(dir)
}

/// The one step that streams a staged release to a target, as a tar, into a directory there
/// emptied first
///
/// # Arguments
///
/// * `dir` - The release, staged here
/// * `host` - The target, as ssh reaches it
/// * `there` - Where it is unpacked there
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// let cmd = reefadm::ship::ship(Path::new("/out/r1"), "europa", "/home/ada/.cache/reefadm/shipped");
/// assert_eq!(
///     cmd.shown(),
///     "sh -c 'tar -C /out/r1 -cf - . | ssh -o BatchMode=yes europa -- '\\''rm -rf \
///      /home/ada/.cache/reefadm/shipped && mkdir -p /home/ada/.cache/reefadm/shipped && tar -C \
///      /home/ada/.cache/reefadm/shipped -xf -'\\'''"
/// );
/// ```
pub fn ship(dir: &Path, host: &str, there: &str) -> Cmd {
    let unpack = format!(
        "rm -rf {there} && mkdir -p {there} && tar -C {there} -xf -",
        there = quote(there)
    );
    let script = format!(
        "tar -C {} -cf - . | ssh -o BatchMode=yes {} -- {}",
        quote(&dir.display().to_string()),
        quote(host),
        quote(&unpack)
    );
    Cmd::new("sh").args(["-c", &script])
}

/// A release shipped here: its record and its files, refused if its reef-serve was compiled with
/// a feature this CPU lacks
///
/// # Arguments
///
/// * `runner` - The runner
/// * `dir` - Where it was unpacked
/// * `kind` - The server it runs, for a dry run's, which has nothing shipped
pub fn shipped(runner: &mut dyn Runner, dir: &Path, kind: Kind) -> Result<(Record, Built)> {
    // what it was built from, or in a dry run, which ships nothing, a stand-in
    let path = dir.join(RECORD);
    let record: Record = match exec::read(runner, &path, As::Me)? {
        Some(text) => {
            toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?
        }
        None if runner.dry() => Record {
            repo: "shipped".to_string(),
            reference: "shipped".to_string(),
            kind,
            release: Release {
                name: "shipped".to_string(),
                reef: "shipped".to_string(),
                shoal: "shipped".to_string(),
                glommio: "shipped".to_string(),
                toolchain: "shipped".to_string(),
                deployed: 0,
                schema: None,
                tables: Default::default(),
                build: None,
            },
        },
        None => bail!(
            "{} isn't there, so {} holds no release reefadm shipped",
            path.display(),
            dir.display()
        ),
    };
    let built = Built {
        reef: dir.join("bin/reef"),
        reefadm: dir.join("bin/reefadm"),
        serve: dir.join("bin/reef-serve"),
        web: (record.kind == Kind::App).then(|| dir.join("app")),
    };
    // built for this CPU: nothing its reef-serve was compiled with that the CPU lacks
    if !runner.dry() {
        let printed = runner
            .probe(&Cmd::new(built.serve.display().to_string()).args(["version", "--json"]))?;
        let compiled = serde_json::from_str::<serde_json::Value>(printed.stdout.trim())
            .ok()
            .and_then(|value| {
                value.get("features")?.as_array().map(|features| {
                    features
                        .iter()
                        .filter_map(|feature| feature.as_str().map(str::to_string))
                        .collect::<BTreeSet<_>>()
                })
            });
        let Some(compiled) = compiled else {
            bail!(
                "{}'s `version --json` didn't say what it was compiled for: {}",
                built.serve.display(),
                printed.stderr.trim()
            );
        };
        let here = cpu::detected();
        let missing: Vec<&String> = compiled.difference(&here).collect();
        if !missing.is_empty() {
            bail!(
                "release {} was built for a CPU with {}, which this one hasn't, so it would crash \
                 here",
                record.release.name,
                missing
                    .iter()
                    .map(|feature| feature.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok((record, built))
}

/// The install made by hand a shipped release takes over, as the builder recorded it
///
/// # Arguments
///
/// * `runner` - The runner
/// * `dir` - Where the release was unpacked
pub fn adopted(runner: &mut dyn Runner, dir: &Path) -> Result<Option<Release>> {
    let path = dir.join(ADOPTED);
    match exec::read(runner, &path, As::Me)? {
        Some(text) => Ok(Some(
            toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?,
        )),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{Facts, shipped};
    use crate::exec::Script;
    use crate::layout::Kind;
    use std::path::Path;

    /// The builder reads the last line the target's reefadm printed, as it wrote it
    #[test]
    fn facts_read_back_from_the_last_line() {
        let facts = Facts {
            arch: "x86_64".into(),
            home: "/home/ada".into(),
            features: ["aes".to_string()].into(),
            problems: vec!["port 12443 is taken".into()],
            kind: Some(Kind::App),
            ..Facts::default()
        };
        let mut script = Script::default();
        facts.say(&mut script).unwrap();
        let printed = format!("a warning first\n{}\n\n", script.said[0]);
        assert_eq!(Facts::read(&printed).unwrap(), facts);
        assert!(Facts::read("not json").is_err());
    }

    /// A shipped release is read from its record, and refused when its reef-serve was compiled
    /// with a feature this CPU hasn't
    #[test]
    fn a_release_built_for_another_cpu_is_refused() {
        let record = "repo = \"r\"\nref = \"main\"\nkind = \"serve\"\n\n[release]\nname = \"r1\"\n\
                      reef = \"a\"\nshoal = \"b\"\nglommio = \"c\"\ntoolchain = \"n\"\ndeployed = 0\n";
        let mut script = Script::default();
        script.prints("cat /d/release.toml", record);
        script.prints(
            "/d/bin/reef-serve version --json",
            r#"{"schema":1,"tables":[],"features":["sse2","no-cpu-has-this"]}"#,
        );
        let error = shipped(&mut script, Path::new("/d"), Kind::Serve)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("with no-cpu-has-this, which this one hasn't"),
            "{error}"
        );
        // and one built for what this CPU has is read as it was built
        script.prints(
            "/d/bin/reef-serve version --json",
            r#"{"schema":1,"tables":[],"features":["sse2"]}"#,
        );
        let (record, built) = shipped(&mut script, Path::new("/d"), Kind::App).unwrap();
        assert_eq!(record.release.name, "r1");
        assert_eq!(built.serve, Path::new("/d/bin/reef-serve"));
        assert_eq!(built.web, None);
    }
}
