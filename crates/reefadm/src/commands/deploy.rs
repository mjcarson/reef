//! `reefadm deploy`: a host with systemd taken to a server that answers, in one command
//!
//! 1. **Preflight:** systemd answers, reefadm hasn't deployed here already, no unit made by
//!    hand is in the way, the ports are free and the disk has room for a build. A dry run says
//!    each problem and goes on.
//! 2. **The first admin's password,** asked now, so the build runs unattended.
//! 3. **reef,** fetched at its ref, and the `central/sources.toml` it holds.
//! 4. **What building needs:** packages, rustup, the toolchains, and for the app, wasm and dx.
//! 5. **shoal and glommio,** at the revs reef pins, beside it.
//! 6. **The build,** then a release made of it, made current.
//! 7. **For a team server,** the `reef` account, its directories and the config, which a later
//!    deploy keeps as it is, but for an older config's `[auth] required = false`, which it
//!    takes out, since every central reef asks its callers to sign in.
//! 8. **The unit,** written if it changed, then enabled and started, or restarted.
//! 9. **Health:** reef's CA copied out, and both ports asked until they answer.
//! 10. **The first admin,** added and signed in from this host, on a team server and a user's
//!     alike.
//! 11. **The firewall,** opened for the TLS port if ufw or firewalld is on, after asking.
//! 12. **reefadm's record** of what it deployed, written last, so a deploy that stopped
//!     partway is run again rather than upgraded.

use crate::cli::DeployArgs;
use crate::compile::Built;
use crate::exec::{ACCOUNT, As, Cmd, File, Runner};
use crate::first_admin::{self, Admin, LocalCli, Told};
use crate::layout::{Kind, Layout, Mode};
use crate::release::Record;
use crate::secret::Secret;
use crate::served::{CORES, Explicit, LOOPBACK, Served, TLS_PORT, TeamFlags, port_of};
use crate::ship::{self, Facts};
use crate::state::State;
use crate::systemd::Systemd;
use crate::{Cx, cpu, exec, health, release, sources, unit};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The free disk a first build needs, in GiB: the checkouts and their release and wasm builds,
/// dx's own, and the toolchains took 11.6 GiB on a bare Ubuntu 26.04, so this leaves some over
const FIRST_BUILD_GIB: u64 = 15;

/// The free disk a later build over the same checkouts needs, in GiB
const LATER_BUILD_GIB: u64 = 5;

/// Each shard's memory budget, unless deploy is told another
const MEMORY: &str = "256MiB";

/// The Ubuntu release reefadm is made and tested for
pub const UBUNTU: &str = "26.04";

/// Deploys reef on this host
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What deploy was told
pub fn deploy(cx: &mut Cx, args: &DeployArgs) -> Result<()> {
    // an install made by hand is taken over rather than deployed beside
    if args.adopt {
        return super::adopt::adopt(cx, args);
    }
    let mode = match args.user {
        true => Mode::User,
        false => Mode::System,
    };
    let kind = match args.api_only {
        true => Kind::Serve,
        false => Kind::App,
    };
    let layout = Layout::new(mode, &cx.env)?;
    let systemd = Systemd::of(&layout);
    let src = args.src.clone().unwrap_or_else(|| layout.src.clone());
    let overrides = sources::overrides(&args.sources)?;
    let kept = super::kept_config(cx.runner, &layout)?;
    // what a builder elsewhere needs to know of this host, said, with nothing changed
    if args.facts {
        let mut facts = Facts::here(cx)?;
        facts.problems = preflight(cx, &layout, &systemd, kept.as_ref(), args, &src, false)?;
        facts.kind = Some(kind);
        return facts.say(cx.runner);
    }
    // everything that would stop it partway, before anything changes; a release shipped here
    // was built elsewhere, which had the room for it
    supported(cx.runner)?;
    let building = args.shipped.is_none();
    let problems = preflight(cx, &layout, &systemd, kept.as_ref(), args, &src, building)?;
    stop_for(cx, &problems)?;
    let hostname = super::hostname(cx.runner)?;
    // the first admin's password now, so the build runs unattended
    let admin = wanted_admin(cx, args)?;
    // the release: shipped here built, or built here for this machine's CPU
    let (mut record, built) = match &args.shipped {
        Some(dir) => ship::shipped(cx.runner, dir, kind)?,
        None => build_here(cx, args, kind, &src, &overrides)?,
    };
    // root's steps from here on, which the build may have outlasted sudo's timestamp for
    if mode == Mode::System {
        cx.runner.elevate()?;
    }
    let name = record.release.name.clone();
    record.release.deployed = cx
        .now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let dir = release::stage(cx.runner, &layout, &built, &record)?;
    release::activate(cx.runner, &layout, &dir)?;
    // the account, its directories and the config, or a user's config if a flag needs one
    let served = match mode {
        Mode::System => team(cx, &layout, args, kept, &hostname)?,
        Mode::User => user_config(cx, &layout, args, kept)?,
    };
    // a config kept from before that turns sign-in off, which the release refuses to start on
    super::sign_in_on(cx.runner, &layout)?;
    check_shards(cx.runner, &layout, &served)?;
    // the unit, then the server answering on both ports
    let low = super::low_port(&served);
    start(cx, &layout, &systemd, kind, low)?;
    health::wait(cx.runner, &layout, &served, &hostname)?;
    // the first admin, signed in from here
    if let Some(admin) = &admin {
        add_admin(cx, &layout, admin)?;
    }
    if let (Mode::System, Some(port)) = (mode, served.tls_port()) {
        firewall(cx, port)?;
    }
    // the record, last, so a deploy that stopped partway is run again
    let state = State {
        mode,
        kind,
        repo: record.repo,
        reference: record.reference,
        release: record.release,
        previous: None,
        first_admin: admin.map(|admin| admin.name),
        src: args.src.clone(),
    };
    state.write(cx.runner, &layout)?;
    summary(cx, &layout, &served, &hostname, &name, true);
    Ok(())
}

/// Builds a release here, for this machine's CPU: reef at its ref, the toolchains, the siblings
/// and the build
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What deploy was told
/// * `kind` - The server it builds
/// * `src` - The sources directory
/// * `overrides` - The siblings fetched from elsewhere
fn build_here(
    cx: &mut Cx,
    args: &DeployArgs,
    kind: Kind,
    src: &Path,
    overrides: &BTreeMap<String, String>,
) -> Result<(Record, Built)> {
    cx.runner.say(&format!(
        "fetching reef at {} from {}",
        args.reference, args.repo
    ));
    let (reef, manifest) = sources::fetch_reef(cx.runner, src, &args.repo, &args.reference)?;
    let class = ship::prepare(
        cx,
        &manifest,
        kind,
        src,
        &cpu::detected(),
        args.target_cpu.as_deref(),
    )?;
    let (revs, built, schema) = ship::make(cx, src, &reef, &manifest, overrides, kind, &class)?;
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
    Ok((record, built))
}

/// Everything that would stop a deploy partway, a line each
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `systemd` - Its unit
/// * `kept` - The config a deploy before this one left
/// * `args` - What deploy was told
/// * `src` - The sources directory
/// * `building` - Whether the build is here, which needs room for it
fn preflight(
    cx: &mut Cx,
    layout: &Layout,
    systemd: &Systemd,
    kept: Option<&Served>,
    args: &DeployArgs,
    src: &Path,
    building: bool,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let runner = &mut *cx.runner;
    // reef runs as a systemd unit, on Linux
    if !cfg!(target_os = "linux") {
        problems.push("reef runs on Linux only".to_string());
    }
    if !systemd.answers(runner)? {
        problems.push(match layout.mode {
            Mode::System => {
                "systemd doesn't answer here, and reef runs as a systemd unit".to_string()
            }
            Mode::User => {
                "your systemd user manager doesn't answer: log in afresh, so it starts".to_string()
            }
        });
    }
    // a deployment reefadm made is upgraded, not deployed again
    if exec::exists(runner, &layout.state, As::Me)? {
        problems.push(format!(
            "reefadm deployed reef here already, as {} says: `reefadm upgrade` moves it to a \
             later commit",
            layout.state.display()
        ));
    }
    // a unit made by hand is in the way
    let unit = exec::read(runner, &layout.unit, As::Me)?;
    let ours = unit.as_deref().is_none_or(unit::ours);
    if !ours {
        problems.push(format!(
            "{} wasn't written by reefadm: reef was installed here by hand, and `reefadm deploy \
             --adopt` takes it over",
            layout.unit.display()
        ));
    }
    // the ports, unless it is reefadm's own unit holding them
    let serving = unit.is_some() && ours && systemd.is_active(runner)?;
    if !serving {
        let loopback = kept
            .map(Served::loopback)
            .unwrap_or_else(|| LOOPBACK.to_string());
        let tls = match layout.mode {
            Mode::System => kept
                .map(Served::tls_port)
                .unwrap_or(Some(args.port.unwrap_or(TLS_PORT))),
            Mode::User => None,
        };
        let listening = listening(runner)?;
        for port in port_of(&loopback).into_iter().chain(tls) {
            if listening.contains(&port) {
                problems.push(format!(
                    "port {port} is taken: stop what listens there, or move reef's"
                ));
            }
        }
    }
    // room for the build, where it is built
    if building {
        problems.extend(room(runner, layout, src)?);
    }
    Ok(problems)
}

/// Whether the disk has room for a build, or the problem if it hasn't
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `src` - The sources directory
pub(super) fn room(runner: &mut dyn Runner, layout: &Layout, src: &Path) -> Result<Option<String>> {
    // a first build needs far more than one over the same checkouts
    let built_before = exec::exists(runner, &src.join("reef/central/target/release"), As::Me)?;
    let needed = if built_before {
        LATER_BUILD_GIB
    } else {
        FIRST_BUILD_GIB
    };
    let room = existing(runner, src, &layout.home)?;
    Ok(match free_gib(runner, &room)? {
        Some(free) if free < needed => Some(format!(
            "{} has {free} GiB free, and building reef there needs about {needed} GiB: free some, \
             or build elsewhere with --src",
            room.display()
        )),
        _ => None,
    })
}

/// Says when this host isn't the Ubuntu reefadm is made for, and goes on
///
/// # Arguments
///
/// * `runner` - The runner
pub(super) fn supported(runner: &mut dyn Runner) -> Result<()> {
    // os-release names the system, each value quoted or not
    let Some(text) = exec::read(runner, Path::new("/etc/os-release"), As::Me)? else {
        return Ok(());
    };
    let field = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.trim().trim_matches('"').to_string())
    };
    let ubuntu = field("ID").as_deref() == Some("ubuntu");
    if !(ubuntu && field("VERSION_ID").as_deref() == Some(UBUNTU)) {
        runner.say(&format!(
            "reefadm is made for Ubuntu {UBUNTU}, and this host runs {}: it goes on, untested here",
            field("PRETTY_NAME").unwrap_or_else(|| "another system".to_string())
        ));
    }
    Ok(())
}

/// Stops on a preflight's problems, or in a dry run, says them and goes on
///
/// # Arguments
///
/// * `cx` - The runner
/// * `problems` - What would stop it
pub(super) fn stop_for(cx: &mut Cx, problems: &[String]) -> Result<()> {
    match (problems.is_empty(), cx.runner.dry()) {
        (true, _) => Ok(()),
        (false, true) => {
            for problem in problems {
                cx.runner.say(&format!("would stop: {problem}"));
            }
            Ok(())
        }
        (false, false) => bail!(
            "reef can't be deployed here yet:\n  {}",
            problems.join("\n  ")
        ),
    }
}

/// Every TCP port something listens on here
///
/// # Arguments
///
/// * `runner` - The runner
pub(super) fn listening(runner: &mut dyn Runner) -> Result<Vec<u16>> {
    // ss's local address is its fourth column, its port after the last colon
    let output = runner.probe(&Cmd::new("ss").args(["-Hltn"]))?;
    Ok(output
        .stdout
        .lines()
        .filter_map(|line| line.split_whitespace().nth(3))
        .filter_map(port_of)
        .collect())
}

/// The directory a path is in that exists already: the path, else the home it falls back to
///
/// # Arguments
///
/// * `runner` - The runner
/// * `path` - The path
/// * `home` - Where to look instead
fn existing(runner: &mut dyn Runner, path: &Path, home: &Path) -> Result<PathBuf> {
    // the path itself, or its nearest parent there is
    for dir in path.ancestors() {
        if exec::exists(runner, dir, As::Me)? {
            return Ok(dir.to_path_buf());
        }
    }
    Ok(home.to_path_buf())
}

/// The free space on a directory's filesystem, in GiB, if df says
///
/// # Arguments
///
/// * `runner` - The runner
/// * `dir` - The directory
fn free_gib(runner: &mut dyn Runner, dir: &Path) -> Result<Option<u64>> {
    // POSIX df: a header, then the filesystem's line, its fourth column the KiB available
    let output = runner.probe(&Cmd::new("df").arg("-Pk").arg(dir))?;
    Ok(output
        .stdout
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|kib| kib.parse::<u64>().ok())
        .map(|kib| kib / (1024 * 1024)))
}

/// The first admin's name and password, which every deployment adds unless told not to
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What deploy was told
fn wanted_admin(cx: &mut Cx, args: &DeployArgs) -> Result<Option<Admin>> {
    // the admin running it, unless told another; a fresh host has no user to ask about
    let fallback = cx.user();
    first_admin::wanted(cx, &Told::from(args), None, fallback)
}

/// A team server's account, directories and config, returning the config it runs with
///
/// # Arguments
///
/// * `cx` - The runner
/// * `layout` - The team server's paths
/// * `args` - What deploy was told
/// * `kept` - The config a deploy before this one left
/// * `hostname` - This host's name, which reef's certificate covers by default
fn team(
    cx: &mut Cx,
    layout: &Layout,
    args: &DeployArgs,
    kept: Option<Served>,
    hostname: &str,
) -> Result<Served> {
    let runner = &mut *cx.runner;
    // the account the service runs as, whose home is the data
    if !runner.probe(&Cmd::new("id").args(["-u", ACCOUNT]))?.ok() {
        runner.run(
            &Cmd::new("useradd")
                .args(["--system", "--home-dir"])
                .arg(&layout.data_home)
                .args(["--create-home", "--shell", "/usr/sbin/nologin", ACCOUNT])
                .root(),
        )?;
    }
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-m", "0755", "/etc/reef"])
            .root(),
    )?;
    // the config deploy's flags make, unless one is kept, which they must agree with
    let flags = TeamFlags {
        names: match args.names.is_empty() {
            true => vec![hostname.to_string()],
            false => args.names.clone(),
        },
        port: args.port.unwrap_or(TLS_PORT),
        own_cert: args.cert.is_some(),
        cores: args.cores.unwrap_or(CORES),
        memory: args.memory.clone().unwrap_or_else(|| MEMORY.to_string()),
    };
    let given = Served::team(layout, &flags);
    let explicit = Explicit {
        port: args.port.is_some(),
        names: !args.names.is_empty(),
        cores: args.cores.is_some(),
        memory: args.memory.is_some(),
        cert: args.cert.is_some(),
    };
    let served = match kept {
        Some(kept) => {
            let conflicts = kept.conflicts(&given, &explicit);
            if !conflicts.is_empty() {
                bail!(
                    "{} is kept as it is, and disagrees with what deploy was told:\n  {}\nedit it, \
                     or leave those flags out",
                    layout.config.display(),
                    conflicts.join("\n  ")
                );
            }
            runner.say(&format!("keeping {} as it is", layout.config.display()));
            kept
        }
        None => {
            runner.write(&File {
                path: layout.config.clone(),
                text: given.text()?,
                mode: 0o644,
                runs_as: As::Root,
                owner: None,
            })?;
            given
        }
    };
    // a certificate of the admin's own, where the account can read its key
    if let (Some(cert), Some(key)) = (&args.cert, &args.key) {
        runner.run(
            &Cmd::new("install")
                .args(["-m", "0644"])
                .arg(cert)
                .arg("/etc/reef/fullchain.pem")
                .root(),
        )?;
        runner.run(
            &Cmd::new("install")
                .args(["-m", "0640", "-g", ACCOUNT])
                .arg(key)
                .arg("/etc/reef/privkey.pem")
                .root(),
        )?;
    }
    // the data, the clones and the certificates, the account's alone, wherever the config puts them
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-o", ACCOUNT, "-g", ACCOUNT, "-m", "0750"])
            .arg(&layout.data_home)
            .arg(served.data(layout))
            .arg(served.clones(layout))
            .arg(served.certs(layout))
            .root(),
    )?;
    Ok(served)
}

/// A user's config: kept, written if a flag needs one, else reef's defaults
///
/// # Arguments
///
/// * `cx` - The runner
/// * `layout` - The user's paths
/// * `args` - What deploy was told
/// * `kept` - The config the user has already
fn user_config(
    cx: &mut Cx,
    layout: &Layout,
    args: &DeployArgs,
    kept: Option<Served>,
) -> Result<Served> {
    let mut given = Served::default();
    given.shoal.cores = args.cores;
    given.shoal.memory = args.memory.clone();
    let explicit = Explicit {
        cores: args.cores.is_some(),
        memory: args.memory.is_some(),
        ..Explicit::default()
    };
    match kept {
        // a config of the user's own is theirs, and must agree with the flags
        Some(kept) => {
            let conflicts = kept.conflicts(&given, &explicit);
            if !conflicts.is_empty() {
                bail!(
                    "{} disagrees with what deploy was told:\n  {}",
                    layout.config.display(),
                    conflicts.join("\n  ")
                );
            }
            Ok(kept)
        }
        // only shards and their memory need one; reef's defaults are the rest
        None if explicit.cores || explicit.memory => {
            cx.runner.write(&File {
                path: layout.config.clone(),
                text: given.text()?,
                mode: 0o644,
                runs_as: As::Me,
                owner: None,
            })?;
            Ok(given)
        }
        None => Ok(given),
    }
}

/// Refuses a shard count that differs from the one the data was written with
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
fn check_shards(runner: &mut dyn Runner, layout: &Layout, served: &Served) -> Result<()> {
    // shoal records the shards it wrote with beside the data
    let meta = served.data(layout).join("shoal-meta.json");
    let Some(text) = exec::read(runner, &meta, layout.owner())? else {
        return Ok(());
    };
    let written = serde_json::from_str::<serde_json::Value>(&text)
        .with_context(|| format!("reading {}", meta.display()))?
        .get("shards")
        .and_then(serde_json::Value::as_u64);
    match written {
        Some(shards) if shards != u64::from(served.cores()) => bail!(
            "the data in {} was written with {shards} shards, and reef would start with {}, which \
             moves every row: set `[shoal] cores = {shards}`, or pass --cores {shards}",
            served.data(layout).display(),
            served.cores()
        ),
        _ => Ok(()),
    }
}

/// Writes the unit if it changed, then starts it, or restarts it on the new release
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `systemd` - Its unit
/// * `kind` - The server it runs
/// * `low` - Whether a port it serves is under 1024
pub(super) fn start(
    cx: &mut Cx,
    layout: &Layout,
    systemd: &Systemd,
    kind: Kind,
    low: bool,
) -> Result<()> {
    let user = cx.user();
    let runner = &mut *cx.runner;
    // the unit, written only when it changed
    super::write_unit(runner, layout, systemd, kind, low)?;
    // started, or restarted onto the release just made current
    match systemd.is_active(runner)? {
        true => {
            runner.run(&systemd.ctl(&["enable", crate::layout::UNIT]))?;
            runner.run(&systemd.restart())?;
        }
        false => runner.run(&systemd.enable_now())?,
    }
    // a user unit keeps running once its user logs out only if they linger
    if let (Mode::User, Some(user)) = (layout.mode, user) {
        let linger =
            runner.probe(&Cmd::new("loginctl").args(["show-user", &user, "--property=Linger"]))?;
        if linger.stdout.trim() != "Linger=yes" {
            let enabled = runner.attempt(&Cmd::new("loginctl").args(["enable-linger", &user]))?;
            if !enabled.ok() {
                runner.say(&format!(
                    "reef stops when you log out, until you run `sudo loginctl enable-linger {user}`"
                ));
            }
        }
    }
    Ok(())
}

/// Adds the first admin and signs them in from this host, unless they are signed in already
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `admin` - Their name and password
fn add_admin(cx: &mut Cx, layout: &Layout, admin: &Admin) -> Result<()> {
    let (name, password) = (admin.name.as_str(), admin.password.as_str());
    // the release's CLI, reading the deployment's config if it has one, which names reef's CA
    let reef = LocalCli::of(cx.runner, layout)?;
    let cli = |args: &[&str]| reef.cmd(args);
    // signed in already, from a deploy before this one
    let whoami = cx.runner.probe(&cli(&["whoami"]))?;
    if whoami.ok() && whoami.stdout.starts_with(&format!("`{name}`")) {
        cx.runner.say(&format!("{name} is signed in already"));
        return Ok(());
    }
    // the password, handed over as a file only the admin can read
    let secret = Secret::write(&cx.env, password, cx.runner.dry())?;
    let file = secret.path().display().to_string();
    let added = cx
        .runner
        .attempt(&cli(&["user", "add", name, "--password-file", &file]))?;
    if !added.ok() {
        // a user already there, from a deploy before this one, is signed in as they are
        let said = format!("{}{}", added.stdout, added.stderr);
        let exists = ["409", "401", "exists", "already", "taken", "sign in"]
            .iter()
            .any(|word| said.contains(word));
        if !exists {
            bail!(
                "adding {name} as reef's first admin failed: {}",
                said.trim()
            );
        }
        cx.runner.say(&format!(
            "reef has users already, so signing {name} in as one of them"
        ));
    }
    cx.runner
        .run(&cli(&["login", "--user", name, "--password-file", &file]))
}

/// Opens the TLS port in ufw or firewalld, if either is on, after asking
///
/// # Arguments
///
/// * `cx` - The runner
/// * `port` - The port
fn firewall(cx: &mut Cx, port: u16) -> Result<()> {
    let rule = format!("{port}/tcp");
    // ufw, if it is active and doesn't let the port through already
    let ufw = cx.runner.probe(&Cmd::new("ufw").arg("status").root())?;
    if ufw.ok() && ufw.stdout.contains("Status: active") {
        if !ufw.stdout.contains(&rule)
            && cx.confirm(&format!(
                "ufw is on: let {rule} through, for other hosts to reach reef?"
            ))?
        {
            cx.runner
                .run(&Cmd::new("ufw").args(["allow", &rule]).root())?;
        }
        return Ok(());
    }
    // else firewalld, if it runs
    let firewalld = cx
        .runner
        .probe(&Cmd::new("firewall-cmd").arg("--state").root())?;
    if firewalld.ok()
        && firewalld.stdout.trim() == "running"
        && cx.confirm(&format!(
            "firewalld is on: let {rule} through, for other hosts to reach reef?"
        ))?
    {
        cx.runner.run(
            &Cmd::new("firewall-cmd")
                .args(["--permanent", &format!("--add-port={rule}")])
                .root(),
        )?;
        cx.runner
            .run(&Cmd::new("firewall-cmd").arg("--reload").root())?;
    }
    Ok(())
}

/// Says where reef is, what to hand users, and what comes next
///
/// # Arguments
///
/// * `cx` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `hostname` - This host's name
/// * `name` - The release's name
/// * `fresh` - Whether reef is new here, with no project yet
pub(super) fn summary(
    cx: &mut Cx,
    layout: &Layout,
    served: &Served,
    hostname: &str,
    name: &str,
    fresh: bool,
) {
    let loopback = format!("https://{}", served.loopback());
    let mut lines = match (
        layout.mode,
        health::tls_ask(served, hostname),
        &layout.ca_copy,
    ) {
        (Mode::System, Some((outside, address)), Some(ca)) => vec![
            format!(
                "reef is up at https://{outside}:{} for other hosts, and {loopback} on this one, \
                 from release {name}",
                address.port()
            ),
            format!(
                "  hand each user its CA, {}, to name as `ca` in their config",
                ca.display()
            ),
            format!(
                "  this host's CLI: `export REEF_CONFIG={}`",
                layout.config.display()
            ),
        ],
        _ => vec![format!(
            "reef is up at {loopback}, from release {name}, where `reef` finds it as it is"
        )],
    };
    // a reef new here has no project yet
    if fresh {
        lines.push("  next, from a project's repo: `reef project add NAME`".to_string());
    }
    for line in lines {
        cx.runner.say(&line);
    }
}
