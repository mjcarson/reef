//! `reefadm backup` and `reefadm restore`: reef's data copied while it is stopped, and put back
//!
//! Both stop the unit, and start it again whatever happened, then wait for it to answer.
//! A restore moves what is there aside rather than over it, and says where, and warns first
//! when the backup's data was written by a schema the running release can't read, or with
//! another shard count.

use crate::cli::{BackupArgs, RestoreArgs};
use crate::exec::Runner;
use crate::layout::{Layout, Mode};
use crate::schema::{Schema, Verdict};
use crate::served::Served;
use crate::systemd::Systemd;
use crate::{Cx, backup, health, release};
use anyhow::{Context, Result, bail};

/// Backs up the data of the reef reefadm deployed, stopping it while it does
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What backup was told
pub fn take(cx: &mut Cx, args: &BackupArgs) -> Result<()> {
    let (layout, state) = super::deployed(cx, args.user)?;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    if layout.mode == Mode::System {
        cx.runner.elevate()?;
    }
    // stopped while it is copied, then started whatever the copy did
    super::stop(cx.runner, &systemd)?;
    let taken = backup::take(
        cx.runner,
        &layout,
        &served,
        &state.release,
        cx.now,
        args.to.as_deref(),
    );
    started(cx.runner, &layout, &served)?;
    taken.map(|_| ())
}

/// Puts a backup back over the data of the reef reefadm deployed
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What restore was told
pub fn restore(cx: &mut Cx, args: &RestoreArgs) -> Result<()> {
    let (layout, state) = super::deployed(cx, args.user)?;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    if layout.mode == Mode::System {
        cx.runner.elevate()?;
    }
    // what wrote it, against what would read it
    let manifest = backup::manifest(cx.runner, &layout, &args.file);
    let mut warnings = Vec::new();
    match &manifest {
        Ok(manifest) => {
            let running = Schema {
                id: 0,
                tables: state
                    .release
                    .tables
                    .iter()
                    .filter_map(|(name, fingerprint)| {
                        fingerprint
                            .parse()
                            .ok()
                            .map(|fingerprint| (name.clone(), fingerprint))
                    })
                    .collect(),
            };
            let verdict = Verdict::between(&manifest.release.tables, &running);
            if !manifest.release.tables.is_empty()
                && !running.tables.is_empty()
                && !verdict.reads_back()
            {
                warnings.push(format!(
                    "release {} can't read it, since its schema {} since release {} wrote it",
                    state.release.name,
                    verdict.said(),
                    manifest.release.name
                ));
            }
            if manifest.cores != served.cores() {
                warnings.push(format!(
                    "it was written with {} shards, and the config starts {}, which moves every row",
                    manifest.cores,
                    served.cores()
                ));
            }
        }
        Err(error) if !cx.runner.dry() => return Err(anyhow::anyhow!("{error:#}")),
        Err(_) => {}
    }
    let from = manifest
        .as_ref()
        .map(|manifest| format!(", written by release {}", manifest.release.name))
        .unwrap_or_default();
    let question = format!(
        "put {}{from} back over reef's data, moving what is there aside?{}",
        args.file.display(),
        warnings
            .iter()
            .map(|warning| format!("\n  but {warning}"))
            .collect::<String>()
    );
    if !cx.confirm(&question)? {
        bail!("nothing was restored");
    }
    // stopped while it is put back, then started whatever happened
    super::stop(cx.runner, &systemd)?;
    let aside = format!("before-restore-{}", release::stamp(cx.now));
    let restored = backup::restore(cx.runner, &layout, &served, &args.file, &aside);
    cx.runner
        .run(&systemd.ctl(&["reset-failed", crate::layout::UNIT]))
        .context("resetting the unit's failures")?;
    started(cx.runner, &layout, &served)?;
    for (path, away) in restored? {
        cx.runner.say(&format!(
            "restored {}; what was there is in {}",
            path.display(),
            away.display()
        ));
    }
    Ok(())
}

/// Starts the unit and waits for it to answer
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
fn started(runner: &mut dyn Runner, layout: &Layout, served: &Served) -> Result<()> {
    let systemd = Systemd::of(layout);
    super::start(runner, &systemd)?;
    let hostname = super::hostname(runner)?;
    health::wait(runner, layout, served, &hostname).map(|_| ())
}
