//! `reefadm status` and `reefadm logs`: how the reef reefadm deployed is doing, and what it said
//!
//! Status shows what was deployed, the unit's state, whether each port answers, the data's size
//! and the disk left, when reef's own certificate ends, and the unit's locked-memory limit. It
//! fails, naming each, when the unit isn't running or a port doesn't answer, so a script or a
//! monitor can run it.

use crate::cli::{LogsArgs, StatusArgs};
use crate::exec::{Cmd, Runner};
use crate::layout::{Kind, Layout, Mode, UNIT};
use crate::systemd::Systemd;
use crate::{Cx, health, release};
use anyhow::{Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// What reef's own certificate's record says, as far as status reads it
#[derive(Deserialize)]
struct Leaf {
    /// When it ends, in seconds since the epoch
    until: i64,
}

/// Says how the reef reefadm deployed is doing, failing if it isn't serving
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What status was told
pub fn status(cx: &mut Cx, args: &StatusArgs) -> Result<()> {
    let (layout, state) = super::deployed(cx, args.user)?;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    let runner = &mut *cx.runner;
    let mut problems = Vec::new();
    // what was deployed
    let kind = match state.kind {
        Kind::App => "reef-app",
        Kind::Serve => "reef-serve",
    };
    let mode = match state.mode {
        Mode::System => "a team server",
        Mode::User => "your user unit",
    };
    runner.say(&format!(
        "reef here: {mode} running {kind}, release {} (reef {} at {}), deployed {}",
        state.release.name,
        short(&state.release.reef),
        state.reference,
        release::day(state.release.deployed)
    ));
    // the unit's state, since when, and whether it starts at boot
    let shown = show(
        runner,
        &systemd,
        &[
            "ActiveState",
            "SubState",
            "ActiveEnterTimestamp",
            "UnitFileState",
            "LimitMEMLOCK",
        ],
    )?;
    let active = shown.get("ActiveState").map(String::as_str) == Some("active");
    runner.say(&format!(
        "unit {UNIT}: {} ({}){}, {}",
        shown.get("ActiveState").map_or("unknown", String::as_str),
        shown.get("SubState").map_or("unknown", String::as_str),
        match shown.get("ActiveEnterTimestamp") {
            Some(since) if active && !since.is_empty() => format!(" since {since}"),
            _ => String::new(),
        },
        shown.get("UnitFileState").map_or("unknown", String::as_str),
    ));
    if !active {
        problems.push(format!("the unit {UNIT} isn't running"));
    }
    // each port, asked once
    let hostname = super::hostname(runner)?;
    let pem = health::trusted(runner, &layout, &served)?;
    for ask in health::asks(&served, &hostname, pem) {
        match runner.me(&ask) {
            Ok(me) => runner.say(&format!(
                "answering {}: {}",
                ask.url.trim_end_matches("/api/v1/me"),
                match me.required {
                    true => "callers sign in",
                    false => "no sign-in",
                }
            )),
            Err(error) => {
                // the innermost cause says it, such as a connection refused
                runner.say(&format!(
                    "not answering {}: {}",
                    ask.url.trim_end_matches("/api/v1/me"),
                    error.root_cause()
                ));
                problems.push(format!("{} doesn't answer", ask.url));
            }
        }
    }
    // the data's size, and the disk left under it
    let data = served.data(&layout);
    let clones = served.clones(&layout);
    let (data_size, clones_size) = (
        size(runner, &layout, &data)?,
        size(runner, &layout, &clones)?,
    );
    let left = free(runner, &data)?;
    runner.say(&format!(
        "data {}: {data_size}; clones {}: {clones_size}; {left} free",
        data.display(),
        clones.display(),
    ));
    // when reef's own certificate ends, which it renews itself a month before
    let leaf = served.certs(&layout).join("leaf.toml");
    if let Some(text) = crate::exec::read(runner, &leaf, layout.owner())?
        && let Ok(leaf) = toml::from_str::<Leaf>(&text)
    {
        runner.say(&format!(
            "reef's own certificate is good until {}, and is made again a month before",
            release::day(leaf.until.max(0) as u64)
        ));
    }
    // the locked memory each shard's io buffers want
    if let Some(limit) = shown.get("LimitMEMLOCK") {
        runner.say(&format!("locked memory: {limit}"));
    }
    match problems.is_empty() {
        true => Ok(()),
        false => bail!("reef isn't serving: {}", problems.join("; ")),
    }
}

/// Shows the unit's journal since boot, or follows it
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What logs was told
pub fn logs(cx: &mut Cx, args: &LogsArgs) -> Result<()> {
    let (layout, _) = super::deployed(cx, args.user)?;
    let mut journal = Systemd::of(&layout).journal(args.lines);
    if args.follow {
        journal = journal.arg("--follow");
    }
    cx.runner.run(&journal)
}

/// A commit's first 12 characters
///
/// # Arguments
///
/// * `rev` - The commit
fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

/// Some of the unit's properties, by name
///
/// # Arguments
///
/// * `runner` - The runner
/// * `systemd` - The unit
/// * `properties` - The properties to read
fn show(
    runner: &mut dyn Runner,
    systemd: &Systemd,
    properties: &[&str],
) -> Result<BTreeMap<String, String>> {
    let property = format!("--property={}", properties.join(","));
    let mut cmd = Cmd::new("systemctl");
    if systemd.user {
        cmd = cmd.arg("--user");
    }
    let output = runner.probe(&cmd.args(["show", UNIT, &property]))?;
    Ok(output
        .stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect())
}

/// A directory's size, as du says it, read as whoever may
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `dir` - The directory
fn size(runner: &mut dyn Runner, layout: &Layout, dir: &Path) -> Result<String> {
    let output = runner.probe(&Cmd::new("du").arg("-sh").arg(dir).by(layout.owner()))?;
    Ok(output
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_string())
}

/// The space left on a directory's filesystem, as df says it
///
/// # Arguments
///
/// * `runner` - The runner
/// * `dir` - The directory
fn free(runner: &mut dyn Runner, dir: &Path) -> Result<String> {
    // the directory, else its parent, since the account's own may not be readable
    let dir = dir.parent().unwrap_or(dir);
    let output = runner.probe(&Cmd::new("df").arg("-Ph").arg(dir))?;
    Ok(output
        .stdout
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .unwrap_or("unknown")
        .to_string())
}
