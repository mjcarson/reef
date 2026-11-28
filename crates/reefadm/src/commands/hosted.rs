//! `reefadm --host HOST deploy`, `deploy --adopt` and `upgrade`: probed there, built here for its
//! CPU, installed there
//!
//! 1. **reefadm's copy there** ([`remote::reach`]), which says the host's facts
//!    ([`ship::Facts`]): its CPU's features, its OS and admin, its record, and anything that would
//!    stop the command there. A problem stops it before anything is built.
//! 2. **The build here**, as a local deploy's, for the class the target's features and this
//!    machine's share: the toolchains, the disk and the schema checks are this machine's.
//! 3. **The release staged here** and shipped there as one tar ([`ship::ship`]).
//! 4. **The install there**, by reefadm's copy with `--shipped`, over `ssh -t`, so sudo's prompts
//!    and reef's are on this terminal. A server there with no user yet gets its first admin, whose
//!    password is asked here before the build and sent there. Its command line is built here, so the flags that name
//!    this machine's things, its checkouts, its repo and its ref, never reach it.
//! 5. **The shipped release and the password removed** there, however it ended.
//!
//! A dry run still copies reefadm there and asks its facts, shows the build and the ship, and
//! runs the install there as a dry run of its own.

use super::{adopt, deploy, upgrade};
use crate::cli::{DeployArgs, UpgradeArgs};
use crate::exec::quote;
use crate::first_admin::{self, Admin, Told};
use crate::layout::{Kind, Layout, Mode};
use crate::remote::{self, There};
use crate::ship::{self, Facts, SHIPPED};
use crate::{Cx, sources};
use anyhow::{Context, Result, bail};
use std::path::Path;

/// Deploys reef on another host, built here for its CPU
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `host` - The host, as ssh reaches it
/// * `args` - What deploy was told
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
pub fn deploy(cx: &mut Cx, host: &str, args: &DeployArgs, compiled: &[&str]) -> Result<()> {
    if args.adopt {
        return adopt(cx, host, args, compiled);
    }
    let mode = match args.user {
        true => Mode::User,
        false => Mode::System,
    };
    let kind = match args.api_only {
        true => Kind::Serve,
        false => Kind::App,
    };
    // reefadm there, and what it says of the host
    let there = remote::reach(cx, host, compiled)?;
    let mut asked = vec!["deploy".to_string(), "--facts".to_string()];
    asked.extend(team(args));
    let facts = facts(cx, &there, &asked)?;
    deploy::stop_for(cx, &facts.problems)?;
    // the first admin, by the name the host's admin has unless told another, asked for now so
    // the build runs unattended
    let told = Told::from(args);
    let first = admin_there(cx, host, &told, None, facts.user.clone())?;
    // the build here, for the host's CPU
    let layout = Layout::new(mode, &cx.env)?;
    let src = args.src.clone().unwrap_or_else(|| layout.src.clone());
    room(cx, &layout, &src)?;
    let overrides = sources::overrides(&args.sources)?;
    cx.runner.say(&format!(
        "fetching reef at {} from {}",
        args.reference, args.repo
    ));
    let (reef, manifest) = sources::fetch_reef(cx.runner, &src, &args.repo, &args.reference)?;
    let class = ship::prepare(
        cx,
        &manifest,
        kind,
        &src,
        &facts.features,
        args.target_cpu.as_deref(),
    )?;
    let (revs, built, schema) = ship::make(cx, &src, &reef, &manifest, &overrides, kind, &class)?;
    let record = ship::record(
        cx,
        &args.repo,
        &args.reference,
        kind,
        revs,
        &manifest,
        &class,
        schema.as_ref(),
    );
    let out = ship::stage_out(cx, &built, &record, None)?;
    // installed there by reefadm's copy, the first admin added and signed in there
    let mut line = vec!["deploy".to_string()];
    line.extend(team(args));
    if args.api_only {
        line.push("--api-only".to_string());
    }
    line.extend(admin_flags(&told, first.as_ref()));
    let password = first.map(|admin| admin.password);
    install(cx, &there, &out, line, password.as_deref())
}

/// The first admin to add on a host's server, by the name its admin has unless told another,
/// their password asked for now so the build runs unattended, or none: `--no-admin`, or a server
/// that has a user already
///
/// # Arguments
///
/// * `cx` - The runner
/// * `host` - The host, as ssh reaches it
/// * `told` - What the command was told of the first admin
/// * `users` - Whether the server there has a user, if it said
/// * `admin` - The admin there, as the host's reefadm said
fn admin_there(
    cx: &mut Cx,
    host: &str,
    told: &Told,
    users: Option<bool>,
    admin: Option<String>,
) -> Result<Option<Admin>> {
    // none asked for, or none needed
    if told.none || users == Some(true) {
        return Ok(None);
    }
    let Some(name) = told.name.map(String::from).or(admin) else {
        bail!("{host} didn't say who its admin is: name the first admin with --first-admin");
    };
    let prompt = format!("{name}'s password, as reef's first admin on {host}");
    let password = first_admin::password(cx.runner, &prompt, told.password_file)?;
    Ok(Some(Admin { name, password }))
}

/// The flags that tell reefadm's copy there of the first admin: whom to add, or to add none
///
/// # Arguments
///
/// * `told` - What the command was told of the first admin
/// * `admin` - The first admin to add, whose password is sent beside the flags
fn admin_flags(told: &Told, admin: Option<&Admin>) -> Vec<String> {
    match (admin, told.none) {
        (Some(admin), _) => vec!["--first-admin".to_string(), admin.name.clone()],
        (None, true) => vec!["--no-admin".to_string()],
        (None, false) => Vec::new(),
    }
}

/// Takes over reef installed by hand on another host, built here for its CPU
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `host` - The host, as ssh reaches it
/// * `args` - What deploy was told
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
fn adopt(cx: &mut Cx, host: &str, args: &DeployArgs, compiled: &[&str]) -> Result<()> {
    let mode = match args.user {
        true => Mode::User,
        false => Mode::System,
    };
    // reefadm there, and what it finds of the install
    let there = remote::reach(cx, host, compiled)?;
    let mut asked = vec![
        "deploy".to_string(),
        "--adopt".to_string(),
        "--facts".to_string(),
    ];
    asked.extend(adopting(args));
    let facts = facts(cx, &there, &asked)?;
    deploy::stop_for(cx, &facts.problems)?;
    let Some(kind) = facts.kind else {
        bail!("{host} didn't say which server its install runs");
    };
    // the first admin, if the install has no user, asked for now so the build runs unattended
    let told = Told::from(args);
    let first = admin_there(cx, host, &told, facts.users, facts.user.clone())?;
    // the build here, checked against the schema the install wrote its data with
    let layout = Layout::new(mode, &cx.env)?;
    let src = args.src.clone().unwrap_or_else(|| layout.src.clone());
    room(cx, &layout, &src)?;
    let overrides = sources::overrides(&args.sources)?;
    cx.runner.say(&format!(
        "fetching reef at {} from {}",
        args.reference, args.repo
    ));
    let (reef, manifest) = sources::fetch_reef(cx.runner, &src, &args.repo, &args.reference)?;
    // the commit named, as this checkout's history holds it, else the install's own reef-serve
    let adopted = match &args.from_rev {
        Some(rev) => {
            adopt::adopted_release(Some(rev), adopt::pin_at(cx.runner, &src, rev)?.as_ref())
        }
        None => {
            let mut adopted = adopt::adopted_release(None, None);
            adopted.tables = facts.tables.clone();
            adopted
        }
    };
    adopt::check_pin(cx, &src, &reef, &adopted)?;
    let class = ship::prepare(
        cx,
        &manifest,
        kind,
        &src,
        &facts.features,
        args.target_cpu.as_deref(),
    )?;
    let (revs, built, schema) = ship::make(cx, &src, &reef, &manifest, &overrides, kind, &class)?;
    if let Some(schema) = &schema {
        adopt::check(cx, &adopted, schema, false)?;
    }
    let record = ship::record(
        cx,
        &args.repo,
        &args.reference,
        kind,
        revs,
        &manifest,
        &class,
        schema.as_ref(),
    );
    let out = ship::stage_out(cx, &built, &record, Some(&adopted))?;
    // taken over there by reefadm's copy, its first admin added there if it had no user
    let mut line = vec!["deploy".to_string(), "--adopt".to_string()];
    line.extend(adopting(args));
    line.extend(admin_flags(&told, first.as_ref()));
    let password = first.map(|admin| admin.password);
    install(cx, &there, &out, line, password.as_deref())
}

/// Upgrades the reef reefadm deployed on another host, built here for its CPU
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `host` - The host, as ssh reaches it
/// * `args` - What upgrade was told
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
pub fn upgrade(cx: &mut Cx, host: &str, args: &UpgradeArgs, compiled: &[&str]) -> Result<()> {
    // reefadm there, and its record of what it deployed
    let there = remote::reach(cx, host, compiled)?;
    let mut asked = vec!["upgrade".to_string(), "--facts".to_string()];
    if args.user {
        asked.push("--user".to_string());
    }
    let facts = facts(cx, &there, &asked)?;
    let Some(state) = facts.state.clone() else {
        bail!(
            "{}",
            match facts.problems.is_empty() {
                true => format!("reefadm hasn't deployed reef on {host}"),
                false => facts.problems.join("; "),
            }
        );
    };
    // the checkouts here, never the host's own, from the repo and ref it was deployed from
    let layout = Layout::new(state.mode, &cx.env)?;
    let src = args.src.clone().unwrap_or_else(|| layout.src.clone());
    let repo = args.repo.clone().unwrap_or_else(|| state.repo.clone());
    let reference = args
        .reference
        .clone()
        .unwrap_or_else(|| state.reference.clone());
    let overrides = sources::overrides(&args.sources)?;
    room(cx, &layout, &src)?;
    cx.runner
        .say(&format!("fetching reef at {reference} from {repo}"));
    let (reef, manifest) = sources::fetch_reef(cx.runner, &src, &repo, &reference)?;
    let class = ship::prepare(
        cx,
        &manifest,
        state.kind,
        &src,
        &facts.features,
        args.target_cpu.as_deref(),
    )?;
    let running = &state.release;
    let told = Told::from(args);
    if upgrade::same(&reef, &manifest, &class, running) && !args.rebuild {
        cx.runner.say(&format!(
            "reef on {host} is at {} already, as release {}: nothing to upgrade",
            ship::short(&reef),
            running.name
        ));
        // a server there with no user still is told how to get one
        if facts.users == Some(false) && !told.none {
            cx.runner.say(&format!(
                "reef on {host} has no user yet: `reefadm --host {host} upgrade --rebuild` adds \
                 its first admin, or `reef user add NAME` there"
            ));
        }
        return Ok(());
    }
    // the first admin, if the server there has no user, asked for now so the build runs
    // unattended
    let first = admin_there(cx, host, &told, facts.users, facts.user.clone())?;
    // the schema, before the build and after it, against the tables the running release wrote
    let pin = src.join("reef/central/reef-service/src/schema.rs");
    match crate::exec::read(cx.runner, &pin, crate::exec::As::Me)? {
        Some(source) => upgrade::check_schema(
            cx,
            running,
            &crate::schema::Schema::from_pin(&source)?,
            args.fresh,
        )?,
        None if cx.runner.dry() => {}
        None => bail!("reef's commit {reef} has no {}", pin.display()),
    }
    let (revs, built, schema) =
        ship::make(cx, &src, &reef, &manifest, &overrides, state.kind, &class)?;
    if let Some(schema) = &schema {
        upgrade::check_schema(cx, running, schema, args.fresh)?;
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
    let out = ship::stage_out(cx, &built, &record, None)?;
    // made current there by the new release's own reefadm, as a local upgrade is
    let mut line = vec!["upgrade".to_string()];
    for (on, flag) in [
        (args.user, "--user"),
        (args.no_backup, "--no-backup"),
        (args.fresh, "--fresh"),
    ] {
        if on {
            line.push(flag.to_string());
        }
    }
    line.extend(admin_flags(&told, first.as_ref()));
    let password = first.map(|admin| admin.password);
    install(cx, &there, &out, line, password.as_deref())?;
    upgrade::after(cx, &src, running, &record.release)
}

/// What a host's reefadm says of it, through a probe, so a dry run asks it too
///
/// # Arguments
///
/// * `cx` - The runner
/// * `there` - reefadm's copy there
/// * `asked` - The command that says it, `--facts` among its arguments
fn facts(cx: &mut Cx, there: &There, asked: &[String]) -> Result<Facts> {
    let said = cx.runner.probe(&remote::ssh(
        &there.host,
        &remote::line(&there.reefadm, asked),
    ))?;
    if said.code.is_none() {
        bail!(
            "reefadm on {} was ended by a signal asking what it has, as a reefadm built for a \
             newer CPU than {}'s is: rebuild reefadm with RUSTFLAGS unset",
            there.host,
            there.host
        );
    }
    if !said.ok() {
        bail!(
            "reefadm on {} couldn't say what it has: {}",
            there.host,
            said.stderr.trim()
        );
    }
    let facts = Facts::read(&said.stdout).with_context(|| format!("asking {}", there.host))?;
    // a host of another architecture runs nothing built here
    if facts.arch != std::env::consts::ARCH {
        bail!(
            "{} is {}, and this machine builds for {}",
            there.host,
            facts.arch,
            std::env::consts::ARCH
        );
    }
    if !facts.ubuntu {
        cx.runner.say(&format!(
            "reefadm is made for Ubuntu {}, and {} runs {}: it goes on, untested there",
            deploy::UBUNTU,
            there.host,
            facts.os.as_deref().unwrap_or("another system")
        ));
    }
    Ok(facts)
}

/// Stops when this machine has no room for the build
///
/// # Arguments
///
/// * `cx` - The runner
/// * `layout` - This machine's paths, which the sources are under
/// * `src` - The sources directory
fn room(cx: &mut Cx, layout: &Layout, src: &Path) -> Result<()> {
    let problems: Vec<String> = deploy::room(cx.runner, layout, src)?.into_iter().collect();
    deploy::stop_for(cx, &problems)
}

/// The flags a team server's deploy takes that name the server, not this machine
///
/// # Arguments
///
/// * `args` - What deploy was told
fn team(args: &DeployArgs) -> Vec<String> {
    let mut flags = Vec::new();
    if args.user {
        flags.push("--user".to_string());
    }
    for name in &args.names {
        flags.extend(["--name".to_string(), name.clone()]);
    }
    let paths = [("--cert", &args.cert), ("--key", &args.key)];
    for (flag, path) in paths {
        if let Some(path) = path {
            flags.extend([flag.to_string(), path.display().to_string()]);
        }
    }
    let values = [
        ("--port", args.port.map(|port| port.to_string())),
        ("--cores", args.cores.map(|cores| cores.to_string())),
        ("--memory", args.memory.clone()),
    ];
    for (flag, value) in values {
        if let Some(value) = value {
            flags.extend([flag.to_string(), value]);
        }
    }
    flags
}

/// The flags an adopt takes that name the install, not this machine
///
/// # Arguments
///
/// * `args` - What deploy was told
fn adopting(args: &DeployArgs) -> Vec<String> {
    let mut flags = Vec::new();
    if args.user {
        flags.push("--user".to_string());
    }
    if let Some(rev) = &args.from_rev {
        flags.extend(["--from-rev".to_string(), rev.clone()]);
    }
    flags
}

/// Ships a staged release to a host and installs it there with reefadm's copy, removing what was
/// shipped however the install ended, and ending as it ended
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `there` - reefadm's copy there
/// * `out` - The release, staged here
/// * `line` - The install's command, before `--shipped`
/// * `password` - The first admin's password, sent to a file only the admin there reads
fn install(
    cx: &mut Cx,
    there: &There,
    out: &Path,
    mut line: Vec<String>,
    password: Option<&str>,
) -> Result<()> {
    let shipped = format!("{}/{SHIPPED}", there.home);
    // the release, streamed there as one tar, which a dry run only shows
    cx.runner.run(&ship::ship(out, &there.host, &shipped))?;
    let dry = cx.runner.dry();
    let secret = match (password, dry) {
        (Some(password), false) => Some(remote::send(cx, there, password.as_bytes())?),
        _ => None,
    };
    // reefadm's copy installs it, its own dry run when this is one
    let mut full = Vec::new();
    if dry {
        full.push("--dry-run".to_string());
    }
    if cx.yes {
        full.push("--yes".to_string());
    }
    full.append(&mut line);
    full.extend(["--shipped".to_string(), shipped.clone()]);
    if let Some(file) = &secret {
        full.extend(["--password-file".to_string(), file.clone()]);
    }
    let code = cx.runner.real().interact(&remote::terminal(
        &there.host,
        &remote::line(&there.reefadm, &full),
    ));
    // what was shipped, and the password, gone however it ended
    if !dry {
        let mut gone = format!("rm -rf {}", quote(&shipped));
        if let Some(file) = &secret {
            gone.push_str(&format!(" {}", quote(file)));
        }
        let _ = cx.runner.real().run(&remote::ssh(&there.host, &gone));
    }
    remote::ended(&there.host, code?)
}
