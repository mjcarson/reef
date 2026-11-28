//! `reefadm upgrade`: the reef reefadm deployed moved to a later commit, or put back if it doesn't
//! come up
//!
//! The first half runs in the reefadm the admin typed:
//! 1. **reef at the new ref,** and the `central/sources.toml` it holds. The same commits as
//!    the running release need no build, unless `--rebuild`.
//! 2. **The schema, before the build:** the new commit's pin in
//!    `central/reef-service/src/schema.rs` against the tables the running release recorded. A
//!    table added reads back; a table changed or gone can't, since shoal can't migrate a table,
//!    so it is refused unless `--fresh` starts a new data directory, the old one moved aside.
//! 3. **The first admin's password,** if the running server has no user yet, asked before the
//!    build so it runs unattended.
//! 4. **The toolchains, the siblings and the build,** as deploy does, then the new release
//!    staged, the built `reef-serve` saying its schema again.
//!
//! The second half, `upgrade --finish <release>`, runs in the reefadm the new release holds, so
//! its unit and its steps are the new code's:
//! 5. **Stop, back up, swap `current`,** take an older config's `[auth] required = false` out,
//!    rewrite the unit if its text changed, start, and wait for both ports. If it doesn't come
//!    up, the old release goes back, with the backup restored over whatever the new one wrote,
//!    and the upgrade fails saying why.
//! 6. **The record,** the old release kept as the one before, then every other release pruned.
//! 7. **The first admin,** added and signed in from this host if the server has no user yet,
//!    since every central reef asks its callers to sign in. The same commits as the running
//!    release do this alone, so an upgrade run again adds one an earlier one couldn't.

use crate::cli::UpgradeArgs;
use crate::compile::Built;
use crate::cpu::{self, Class};
use crate::exec::{self, As, Cmd, Runner};
use crate::first_admin::{self, Admin, Told};
use crate::layout::Layout;
use crate::release::{self, Record};
use crate::schema::{Schema, Verdict};
use crate::secret::Secret;
use crate::served::Served;
use crate::ship::{self, Facts};
use crate::sources::{self, Manifest};
use crate::state::Release;
use crate::systemd::Systemd;
use crate::{Cx, backup, health};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Upgrades the reef reefadm deployed here, or with `--finish`, finishes one
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What upgrade was told
pub fn upgrade(cx: &mut Cx, args: &UpgradeArgs) -> Result<()> {
    if let Some(name) = &args.finish {
        return finish(cx, args, name);
    }
    // what a builder elsewhere needs to know of this host, said, with nothing changed
    if args.facts {
        return facts(cx, args);
    }
    let (layout, state) = super::deployed(cx, args.user)?;
    let told = Told::from(args);
    // a release built elsewhere for this host's CPU, shipped here, with the first admin's
    // password sent beside it if the builder found the server had no user
    if let Some(dir) = &args.shipped {
        let (record, built) = ship::shipped(cx.runner, dir, state.kind)?;
        let admin = match told.password_file {
            Some(_) => first_admin::wanted(cx, &told, None, cx.user())?,
            None => None,
        };
        return hand_over(cx, args, &layout, &built, &record, admin.as_ref());
    }
    // the checkouts deploy used, unless told others
    let src = args
        .src
        .clone()
        .or_else(|| state.src.clone())
        .unwrap_or_else(|| layout.src.clone());
    let repo = args.repo.clone().unwrap_or_else(|| state.repo.clone());
    let reference = args
        .reference
        .clone()
        .unwrap_or_else(|| state.reference.clone());
    let overrides = sources::overrides(&args.sources)?;
    // reef at the new ref, what its commit pins, and the class this machine's CPU is built for
    cx.runner
        .say(&format!("fetching reef at {reference} from {repo}"));
    let (reef, manifest) = sources::fetch_reef(cx.runner, &src, &repo, &reference)?;
    let class = ship::prepare(
        cx,
        &manifest,
        state.kind,
        &src,
        &cpu::detected(),
        args.target_cpu.as_deref(),
    )?;
    let running = &state.release;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    if same(&reef, &manifest, &class, running) && !args.rebuild {
        cx.runner.say(&format!(
            "reef is at {} already, as release {}: nothing to upgrade",
            short(&reef),
            running.name
        ));
        // but a first admin an upgrade before couldn't add, if the server has no user still
        return first_admin::ensure(cx, &layout, &served, &told);
    }
    // the first admin's password now, if the server has no user, so the build runs unattended
    let admin = match told.none {
        true => None,
        false => {
            let has = first_admin::users(cx.runner, &layout, &served);
            first_admin::wanted(cx, &told, has, cx.user())?
        }
    };
    // the new commit's schema, before a build is spent on one that can't read the data
    let pin = src.join("reef/central/reef-service/src/schema.rs");
    match exec::read(cx.runner, &pin, As::Me)? {
        Some(source) => check_schema(cx, running, &Schema::from_pin(&source)?, args.fresh)?,
        None if cx.runner.dry() => {}
        None => bail!("reef's commit {reef} has no {}", pin.display()),
    }
    // the siblings and the build, as deploy does, then the built server's own word on its schema
    let (revs, built, schema) =
        ship::make(cx, &src, &reef, &manifest, &overrides, state.kind, &class)?;
    if let Some(schema) = &schema {
        check_schema(cx, running, schema, args.fresh)?;
    }
    let record = ship::record(
        cx,
        &repo,
        &reference,
        state.kind,
        revs,
        &manifest,
        &class,
        schema.as_ref(),
    );
    hand_over(cx, args, &layout, &built, &record, admin.as_ref())?;
    after(cx, &src, running, &record.release)
}

/// Whether a commit, its pins and a class are the running release's, which needs no upgrade
///
/// # Arguments
///
/// * `reef` - reef's commit
/// * `manifest` - What it pins
/// * `class` - The CPU class it would be built for
/// * `running` - The running release
pub(crate) fn same(reef: &str, manifest: &Manifest, class: &Class, running: &Release) -> bool {
    reef == running.reef
        && manifest.shoal.rev == running.shoal
        && manifest.glommio.rev == running.glommio
        && manifest.toolchain == running.toolchain
        && running.build.as_deref() == Some(class.flags.as_str())
}

/// Says what a builder elsewhere needs to know of this host for an upgrade: its record
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What upgrade was told
fn facts(cx: &mut Cx, args: &UpgradeArgs) -> Result<()> {
    let mut facts = Facts::here(cx)?;
    match super::deployed(cx, args.user) {
        Ok((layout, state)) => {
            facts.kind = Some(state.kind);
            facts.state = Some(state);
            // whether the server has a user yet, for the builder to ask a first admin's password
            let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
            facts.users = first_admin::users(cx.runner, &layout, &served);
        }
        Err(error) => facts.problems.push(format!("{error:#}")),
    }
    facts.say(cx.runner)
}

/// Stages a new release and hands over to its own reefadm, which makes it the one in use
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What upgrade was told
/// * `layout` - The deployment's paths
/// * `built` - The release's files
/// * `record` - What it was built from
/// * `admin` - The first admin to add once it is up, if the server has no user
fn hand_over(
    cx: &mut Cx,
    args: &UpgradeArgs,
    layout: &Layout,
    built: &Built,
    record: &Record,
    admin: Option<&Admin>,
) -> Result<()> {
    if layout.mode == crate::layout::Mode::System {
        cx.runner.elevate()?;
    }
    let name = record.release.name.clone();
    let dir = release::stage(cx.runner, layout, built, record)?;
    // the first admin's password, handed over as a file only the admin can read, which is
    // removed once the new release's reefadm is done with it
    let secret = match admin {
        Some(admin) => Some(Secret::write(&cx.env, &admin.password, cx.runner.dry())?),
        None => None,
    };
    let mut told = args.clone();
    if let (Some(admin), Some(secret)) = (admin, &secret) {
        told.first_admin = Some(admin.name.clone());
        told.password_file = Some(secret.path().to_path_buf());
    }
    // the new release's reefadm finishes it, so its unit and its steps are the ones taken; a
    // dry run shows them as this one would take them
    match cx.runner.dry() {
        true => finish(cx, &told, &name),
        false => cx.runner.run(&finish_command(&dir, &told, &name, cx.yes)),
    }
}

/// Says what users and their repos need once the server moved
///
/// # Arguments
///
/// * `cx` - The runner
/// * `src` - The sources directory, reef's checkout among it
/// * `old` - The release upgraded from
/// * `new` - The release upgraded to
pub(crate) fn after(cx: &mut Cx, src: &Path, old: &Release, new: &Release) -> Result<()> {
    let skill = cx.runner.probe(
        &Cmd::new("git")
            .arg("-C")
            .arg(src.join("reef"))
            .args(["diff", "--quiet", &old.reef, &new.reef])
            .args(["--", "crates/reef/assets/skill"]),
    )?;
    cx.runner.say(
        "  users' stamped `reef` follows by itself, the next time it calls the server; a `reef` \
         built with `cargo install` needs `reef update` once",
    );
    if skill.code == Some(1) {
        cx.runner.say(
            "  reef's skill changed: run `reef claude install` in each repo set up for Claude \
             Code, and commit what it writes",
        );
    }
    Ok(())
}

/// A commit's first 12 characters
///
/// # Arguments
///
/// * `rev` - The commit
fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

/// Refuses a schema that can't read the data the running release wrote, unless `--fresh`
///
/// # Arguments
///
/// * `cx` - The runner
/// * `running` - The running release, whose tables are recorded
/// * `new` - The new schema
/// * `fresh` - Whether the upgrade starts a new data directory
pub(crate) fn check_schema(
    cx: &mut Cx,
    running: &Release,
    new: &Schema,
    fresh: bool,
) -> Result<()> {
    // a release whose tables weren't recorded can't be compared, so it is said and passed
    if running.tables.is_empty() {
        cx.runner.say(&format!(
            "reefadm has no record of release {}'s tables, so it can't tell whether this schema \
             reads its data back; the backup it takes first is the way back",
            running.name
        ));
        return Ok(());
    }
    let verdict = Verdict::between(&running.tables, new);
    match (verdict.reads_back(), fresh) {
        (true, _) => {
            if !verdict.added.is_empty() {
                cx.runner.say(&format!(
                    "its schema adds {}, which start empty",
                    verdict.added.join(", ")
                ));
            }
            Ok(())
        }
        (false, true) => {
            cx.runner.say(&format!(
                "its schema {}, so it starts on a fresh data directory, the old one moved aside",
                verdict.said()
            ));
            Ok(())
        }
        (false, false) => bail!(
            "this commit's schema {}, and shoal can't migrate a table, so it can't read the data \
             release {} wrote. Export every plan first, then upgrade with --fresh and import them \
             again, as the book's \"When the store's schema changes\" page says",
            verdict.said(),
            running.name
        ),
    }
}

/// The command that runs the new release's reefadm to finish the upgrade
///
/// # Arguments
///
/// * `dir` - The new release's directory
/// * `args` - What upgrade was told
/// * `name` - The new release's name
/// * `yes` - Whether `--yes` was given
fn finish_command(dir: &Path, args: &UpgradeArgs, name: &str, yes: bool) -> Cmd {
    let flags = [
        (args.user, "--user"),
        (args.no_backup, "--no-backup"),
        (args.fresh, "--fresh"),
        (args.no_admin, "--no-admin"),
        (yes, "--yes"),
    ];
    let cmd = Cmd::new(dir.join("bin/reefadm").display().to_string())
        .args(["upgrade", "--finish", name])
        .args(flags.iter().filter(|(on, _)| *on).map(|(_, flag)| *flag));
    // the first admin to add, and their password's file, only for a server with no user, so
    // a release's reefadm from before them still finishes an upgrade of one that has users
    let cmd = match &args.first_admin {
        Some(admin) => cmd.args(["--first-admin", admin]),
        None => cmd,
    };
    match &args.password_file {
        Some(file) => cmd.arg("--password-file").arg(file),
        None => cmd,
    }
}

/// Makes a staged release the one in use, putting the old one back if it doesn't come up
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What upgrade was told
/// * `name` - The staged release's name
fn finish(cx: &mut Cx, args: &UpgradeArgs, name: &str) -> Result<()> {
    let (layout, mut state) = super::deployed(cx, args.user)?;
    let record = match release::record(cx.runner, &layout, name) {
        Ok(record) => record,
        // a dry run has staged nothing to read back
        Err(_) if cx.runner.dry() => Record {
            repo: state.repo.clone(),
            reference: state.reference.clone(),
            kind: state.kind,
            release: Release {
                name: name.to_string(),
                ..state.release.clone()
            },
        },
        Err(error) => return Err(error),
    };
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    let hostname = super::hostname(cx.runner)?;
    let old = state.release.clone();
    let old_unit = exec::read(cx.runner, &layout.unit, As::Me)?;
    let stamp = release::stamp(cx.now);
    if layout.mode == crate::layout::Mode::System {
        cx.runner.elevate()?;
    }
    // the old release's data, backed up while nothing writes it
    super::stop(cx.runner, &systemd)?;
    let saved = match args.no_backup {
        true => None,
        false => Some(backup::take(
            cx.runner, &layout, &served, &old, cx.now, None,
        )?),
    };
    // a schema that can't read it starts on a fresh data directory, the old one kept aside
    if args.fresh {
        fresh(cx.runner, &layout, &served, &old.name)?;
    }
    // a config turning sign-in off, which the new release refuses to start on
    super::sign_in_on(cx.runner, &layout)?;
    // the new release, its unit, and its start
    release::activate(cx.runner, &layout, &layout.releases.join(name))?;
    super::write_unit(
        cx.runner,
        &layout,
        &systemd,
        record.kind,
        super::low_port(&served),
    )?;
    let up = super::start(cx.runner, &systemd)
        .and_then(|()| health::wait(cx.runner, &layout, &served, &hostname));
    if let Err(error) = up {
        roll_back(
            cx,
            &layout,
            &served,
            &old,
            old_unit.as_deref(),
            saved.as_deref(),
            &stamp,
        )
        .with_context(|| format!("putting release {} back failed too", old.name))?;
        bail!(
            "release {name} didn't come up, so reef is back on release {}: {error:#}",
            old.name
        );
    }
    // the record, the old release kept as the way back, and every other release pruned
    let deployed = cx
        .now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    state.previous = Some(old.clone());
    state.release = Release {
        deployed,
        ..record.release
    };
    state.repo = record.repo;
    state.reference = record.reference;
    state.kind = record.kind;
    if args.src.is_some() {
        state.src = args.src.clone();
    }
    state.write(cx.runner, &layout)?;
    release::prune(cx.runner, &layout, &[name, &old.name])?;
    cx.runner.say(&format!(
        "reef is up on release {name}, upgraded from {}",
        old.name
    ));
    // the first admin, if the server has no user yet, once the record names the release up
    first_admin::ensure(cx, &layout, &served, &Told::from(args)).with_context(|| {
        format!(
            "reef is up on release {name}, but its first admin wasn't added: `reefadm upgrade` \
             again adds them, or `reef user add NAME` from this host"
        )
    })
}

/// Moves the data and the clones aside, and makes them again empty, for a schema that can't
/// read them
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `old` - The name of the release that wrote them
fn fresh(runner: &mut dyn Runner, layout: &Layout, served: &Served, old: &str) -> Result<()> {
    let owner = layout.owner();
    for dir in [served.data(layout), served.clones(layout)] {
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let aside = dir.with_file_name(format!("{name}.{old}"));
        runner.run(&Cmd::new("mv").arg("-T").arg(&dir).arg(&aside).by(owner))?;
        // made again, as deploy made it
        let made = Cmd::new("install").args(["-d", "-m", "0750"]);
        let made = match layout.mode {
            crate::layout::Mode::System => made.args(["-o", exec::ACCOUNT, "-g", exec::ACCOUNT]),
            crate::layout::Mode::User => made,
        };
        runner.run(&made.arg(&dir).by(owner))?;
        runner.say(&format!(
            "moved {} aside as {}",
            dir.display(),
            aside.display()
        ));
    }
    runner.say(
        "the new data directory is empty: register each project again, then `reef import` each \
         plan from its exported book, as the book's \"When the store's schema changes\" page says",
    );
    Ok(())
}

/// Puts the old release back, with its unit, and the backup over whatever the new one wrote
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `old` - The release to put back
/// * `old_unit` - The unit's text before the upgrade
/// * `saved` - The backup taken before the upgrade, if one was
/// * `stamp` - When the upgrade ran, which names what the failed release wrote
fn roll_back(
    cx: &mut Cx,
    layout: &Layout,
    served: &Served,
    old: &Release,
    old_unit: Option<&str>,
    saved: Option<&Path>,
    stamp: &str,
) -> Result<()> {
    let systemd = Systemd::of(layout);
    let hostname = super::hostname(cx.runner)?;
    cx.runner.say(&format!("putting release {} back", old.name));
    // the new release stopped, whatever state it is in
    let _ = super::stop(cx.runner, &systemd);
    release::activate(cx.runner, layout, &layout.releases.join(&old.name))?;
    // the unit as it was, if the new release's differs
    let now_unit = exec::read(cx.runner, &layout.unit, As::Me)?;
    if let Some(text) = old_unit.filter(|text| now_unit.as_deref() != Some(*text)) {
        cx.runner.write(&crate::exec::File {
            path: layout.unit.clone(),
            text: text.to_string(),
            mode: 0o644,
            runs_as: layout.owner(),
            owner: None,
        })?;
        cx.runner.run(&systemd.daemon_reload())?;
    }
    // the data as it was, what the new release wrote kept aside
    match saved {
        Some(saved) => {
            backup::restore(cx.runner, layout, served, saved, &format!("failed-{stamp}"))?;
        }
        None => cx
            .runner
            .say("there was no backup, so the old release starts over whatever the new one wrote"),
    }
    // a release that kept failing has used up systemd's starts, so they are counted afresh
    cx.runner
        .run(&systemd.ctl(&["reset-failed", crate::layout::UNIT]))?;
    super::start(cx.runner, &systemd)?;
    health::wait(cx.runner, layout, served, &hostname).map(|_| ())
}
