//! `reefadm deploy --adopt`: reef installed by hand, as the Deploying chapter does it, taken over
//!
//! 1. **Find it** as systemd loads it, drop-ins and all (`systemctl show`): the server its unit
//!    runs, the account it runs as, and the config it reads, from `--conf`, else `REEF_CONFIG=`,
//!    else the default under that account's home. Each path is the config's, else reef's own
//!    default under that home.
//! 2. **Preflight,** as deploy's, the install's ports being its own.
//! 3. **reef fetched,** and the new commit's schema checked against the one the install's data
//!    was written with: the pin at `--from-rev`, else the old reef-serve's own word, if it is new
//!    enough to say it.
//! 4. **The build,** as deploy's, staged as a release.
//! 5. **The install kept:** stopped and backed up, its unit kept beside reefadm's as
//!    `reef.service.before-reefadm`, its commands and app copied to `releases/adopted`, and its
//!    config kept as it is. A config reefadm wouldn't read the same is written where reefadm
//!    reads it, naming each path it left to reef's defaults, the one there kept beside it, and
//!    so is one turning sign-in off, `[auth] required = false` taken out, since every central
//!    reef asks its callers to sign in.
//! 6. **reefadm's unit** on the new release, started, and both ports asked. If it doesn't come
//!    up, the install goes back as it was, its data restored from the backup, and started again
//!    if it was running.
//! 7. **reefadm's record,** the install made by hand kept as the release before.
//! 8. **The first admin,** if the install had no user, its password asked before the build,
//!    added and signed in from this host.

use super::deploy;
use crate::cli::DeployArgs;
use crate::compile::Built;
use crate::exec::{self, ACCOUNT, As, Ask, Cmd, File, Runner};
use crate::first_admin::{self, Told};
use crate::layout::{Kind, Layout, Mode, UNIT};
use crate::release::{self, COMMANDS, Record};
use crate::schema::{Schema, Verdict};
use crate::served::{Served, port_of};
use crate::ship::{self, Facts};
use crate::state::{Release, State};
use crate::systemd::Systemd;
use crate::{Cx, backup, cpu, health, sources, unit};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The release the install made by hand is kept as, which pruning never removes
pub const ADOPTED: &str = "adopted";

/// What is added to the name of each file adopt keeps beside the one it writes in its place
pub const BEFORE: &str = "before-reefadm";

/// The unit's properties adopt reads, as systemd loads it
const PROPERTIES: &str =
    "LoadState,FragmentPath,DropInPaths,ExecStart,User,Environment,ActiveState,UnitFileState";

/// A path reef's config names
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// Its table, such as `shoal`
    pub table: &'static str,
    /// Its key, such as `data`
    pub key: &'static str,
    /// The path
    pub path: PathBuf,
}

/// reef as it was installed by hand
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Where systemd loads its unit from
    pub fragment: PathBuf,
    /// The unit file's text
    pub unit: String,
    /// The drop-ins systemd applies to it
    pub drop_ins: Vec<String>,
    /// The server it runs
    pub kind: Kind,
    /// The server's program
    pub program: PathBuf,
    /// The config it reads
    pub config: PathBuf,
    /// The config's text, if there is one
    pub text: Option<String>,
    /// Its config, with each path it leaves to reef's defaults filled in as reef fills it
    pub served: Served,
    /// The paths it leaves to reef's defaults where reefadm's would differ, which reefadm's
    /// config names
    pub named: Vec<Named>,
    /// Whether it is running
    pub active: bool,
    /// Whether it starts at boot
    pub enabled: bool,
}

impl Found {
    /// Whether reefadm writes a config of its own, since it wouldn't read the install's the same:
    /// one elsewhere, one leaving a path to a default reefadm's unit moves, or one turning
    /// sign-in off, which the new release refuses to start on
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths
    pub fn rewrites(&self, layout: &Layout) -> bool {
        !self.named.is_empty()
            || (self.text.is_some() && self.config != layout.config)
            || self.served.auth.required == Some(false)
    }

    /// The config reefadm writes where it reads it, if the install's isn't read the same there
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths
    pub fn rewritten(&self, layout: &Layout) -> Result<Option<String>> {
        // the install's own config, where reefadm reads it, naming every path it needs
        if !self.rewrites(layout) {
            return Ok(None);
        }
        // its text as it is, comments and all, sign-in off taken out, and each path added as a
        // line of its table
        let mut text = self.text.clone().unwrap_or_default();
        let sign_in_off = crate::served::without_sign_in_off(&text, &self.config)?;
        let took_out = sign_in_off.is_some();
        if let Some(on) = sign_in_off {
            text = on;
        }
        for named in &self.named {
            let value = toml::Value::String(named.path.display().to_string());
            text = with_key(&text, named.table, &format!("{} = {value}", named.key));
        }
        // a line saying where it came from, above it
        let from = match &self.text {
            Some(_) => format!("from {}", self.config.display()),
            None => "for an install that had no config".to_string(),
        };
        let names: Vec<String> = self
            .named
            .iter()
            .map(|named| format!("`[{}] {}`", named.table, named.key))
            .collect();
        let naming = match names.is_empty() {
            true => String::new(),
            false => format!(
                ",\n# naming {}, which it left to reef's defaults",
                names.join(", ")
            ),
        };
        let taking = match took_out {
            true => ",\n# taking out `[auth] required = false`, since every central reef asks its \
                     callers to sign in"
                .to_string(),
            false => String::new(),
        };
        let written = format!(
            "# Written by `reefadm deploy --adopt` {from}{naming}{taking}.\n# reefadm keeps it as \
             it is from here on: edit it as you like, then restart reef.\n\n{text}"
        );
        // a table spelt so that a line can't be added to it, such as an inline one, is the
        // admin's to name
        if let Err(error) = written.parse::<toml::Table>() {
            bail!(
                "reefadm can't add {} to {} as reef reads it: name {} there yourself, then adopt \
                 again ({error})",
                names.join(", "),
                self.config.display(),
                match names.len() {
                    1 => "it",
                    _ => "them",
                }
            );
        }
        Ok(Some(written))
    }
}

/// A config's text with a line added to one of its tables, right under the table's header, or
/// under a header of its own at the end when it has none
///
/// # Arguments
///
/// * `text` - The config's text
/// * `table` - The table, such as `api`
/// * `line` - The line, such as `certs = "/var/lib/reef/certs"`
///
/// # Examples
///
/// ```
/// use reefadm::commands::adopt::with_key;
///
/// let text = "# reef on europa\n[api] # loopback\naddress = \"127.0.0.1:11180\"\n\n[api.tls]\n";
/// assert_eq!(
///     with_key(text, "api", "certs = \"/c\""),
///     "# reef on europa\n[api] # loopback\ncerts = \"/c\"\naddress = \"127.0.0.1:11180\"\n\n[api.tls]\n"
/// );
/// assert_eq!(with_key("[shoal]\ncores = 1", "api", "certs = \"/c\""), "[shoal]\ncores = 1\n\n[api]\ncerts = \"/c\"\n");
/// ```
pub fn with_key(text: &str, table: &str, line: &str) -> String {
    let header = format!("[{table}]");
    let mut out = String::new();
    let mut added = false;
    for each in text.split_inclusive('\n') {
        out.push_str(each);
        // the table's own header, a comment after it or not, and never a table under it
        let bare = each.split('#').next().unwrap_or_default().trim();
        if !added && bare == header {
            if !each.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(line);
            out.push('\n');
            added = true;
        }
    }
    if !added {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{header}\n{line}\n"));
    }
    out
}

/// What adopt changed so far, which putting the install back undoes
#[derive(Debug, Default)]
struct Kept {
    /// The commands the install had, copied into the adopted release
    commands: Vec<&'static str>,
    /// Whether its unit was kept beside the one reefadm writes
    unit: bool,
    /// Whether reefadm's unit was written
    wrote_unit: bool,
    /// Whether the drop-in keeping the unit's own settings was written
    drop_in: bool,
    /// Whether reefadm's config was written, and if so, whether one was there before it
    config: Option<bool>,
    /// Whether the new release was made current
    activated: bool,
    /// Whether the new release was started
    started: bool,
}

/// Takes over reef installed here by hand
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What deploy was told
pub fn adopt(cx: &mut Cx, args: &DeployArgs) -> Result<()> {
    let mode = match args.user {
        true => Mode::User,
        false => Mode::System,
    };
    let layout = Layout::new(mode, &cx.env)?;
    let systemd = Systemd::of(&layout);
    let src = args.src.clone().unwrap_or_else(|| layout.src.clone());
    let overrides = sources::overrides(&args.sources)?;
    // what a builder elsewhere needs to know of this install, said, with nothing changed
    if args.facts {
        return facts(cx, args, &layout, &systemd, &src);
    }
    // the install, as systemd says it runs
    if !systemd.answers(cx.runner)? {
        bail!("systemd doesn't answer here, so there is no unit of reef's to take over");
    }
    let found = find(cx.runner, &layout, &systemd)?;
    cx.runner.say(&describe(&layout, &found));
    deploy::supported(cx.runner)?;
    // everything that would stop it partway, before anything changes; a release shipped here
    // was built elsewhere, which had the room for it
    let building = args.shipped.is_none();
    let problems = preflight(cx.runner, &layout, &found, &src, building)?;
    deploy::stop_for(cx, &problems)?;
    let hostname = super::hostname(cx.runner)?;
    // the first admin's password now, if the install has no user, so the build runs unattended;
    // one stopped can't be asked, so its password is asked for anyway, and asked of it once up
    let told = Told::from(args);
    let has = match (told.none, found.active) {
        (false, true) => first_admin::users(cx.runner, &layout, &found.served),
        _ => None,
    };
    let admin = first_admin::wanted(cx, &told, has, cx.user())?;
    // the release, and the install made by hand as a release of its own
    let (mut record, built, adopted) = match &args.shipped {
        Some(dir) => {
            let (record, built) = ship::shipped(cx.runner, dir, found.kind)?;
            let adopted = ship::adopted(cx.runner, dir)?
                .unwrap_or_else(|| adopted_release(args.from_rev.as_deref(), None));
            (record, built, adopted)
        }
        None => build_here(cx, args, &layout, &found, &src, &overrides)?,
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
    // whether it served plain HTTP, as a reef before C87 did, while it still runs
    let plain = found.active && !cx.runner.dry() && plain_http(cx.runner, &found.served);
    // stopped and backed up, started again if the backup fails
    super::stop(cx.runner, &systemd)?;
    let saved = match &found.served.shoal.address {
        None => match backup::take(cx.runner, &layout, &found.served, &adopted, cx.now, None) {
            Ok(saved) => Some(saved),
            Err(error) => {
                if found.active {
                    let _ = super::start(cx.runner, &systemd);
                }
                return Err(error.context("backing up the install made by hand"));
            }
        },
        Some(address) => {
            cx.runner.say(&format!(
                "reef's store is the shoal node at {address}, which keeps its own data: back that \
                 up yourself"
            ));
            None
        }
    };
    // the install kept, then reefadm's in its place, or the install back as it was
    let mut kept = Kept::default();
    let served = match take_over(cx, &layout, &systemd, &found, &dir, &hostname, &mut kept) {
        Ok(served) => served,
        Err(error) => {
            let stamp = release::stamp(cx.now);
            put_back(
                cx,
                &layout,
                &systemd,
                &found,
                &kept,
                saved.as_deref(),
                &stamp,
            )
            .context("putting the install made by hand back failed too")?;
            bail!(
                "release {name} didn't come up, so the install made by hand is back as it was: \
                 {error:#}"
            );
        }
    };
    // the record, last, the install made by hand kept as the release before
    let state = State {
        mode,
        kind: found.kind,
        repo: record.repo,
        reference: record.reference,
        release: record.release,
        previous: Some(adopted),
        first_admin: admin.as_ref().map(|admin| admin.name.clone()),
        src: args.src.clone(),
    };
    state.write(cx.runner, &layout)?;
    // the first admin, once the record names the release up, so a failure leaves upgrade to add
    // them, since adopt won't run again over its own record
    if let Some(admin) = &admin {
        first_admin::add(cx, &layout, &served, admin).with_context(|| {
            format!(
                "reef is adopted and up on release {name}, but its first admin wasn't added: \
                 `reefadm upgrade` adds them, or `reef user add {}` from this host",
                admin.name
            )
        })?;
    }
    deploy::summary(cx, &layout, &served, &hostname, &name, false);
    notes(cx, &layout, &found, plain, saved.as_deref());
    Ok(())
}

/// Builds the release here, for this machine's CPU, checking the schema the install wrote its
/// data with against the new commit's, before the build and after it
///
/// # Arguments
///
/// * `cx` - The runner, the environment and the time
/// * `args` - What deploy was told
/// * `layout` - The deployment's paths
/// * `found` - The install
/// * `src` - The sources directory
/// * `overrides` - The siblings fetched from elsewhere
fn build_here(
    cx: &mut Cx,
    args: &DeployArgs,
    layout: &Layout,
    found: &Found,
    src: &Path,
    overrides: &BTreeMap<String, String>,
) -> Result<(Record, Built, Release)> {
    // reef at its ref, and the schema the install's data was written with against its commit's
    cx.runner.say(&format!(
        "fetching reef at {} from {}",
        args.reference, args.repo
    ));
    let (reef, manifest) = sources::fetch_reef(cx.runner, src, &args.repo, &args.reference)?;
    let old = match &args.from_rev {
        Some(rev) => pin_at(cx.runner, src, rev)?,
        None => own_word(cx.runner, layout, found)?,
    };
    let adopted = adopted_release(args.from_rev.as_deref(), old.as_ref());
    check_pin(cx, src, &reef, &adopted)?;
    // the toolchains, the siblings and the build, as deploy does
    let class = ship::prepare(
        cx,
        &manifest,
        found.kind,
        src,
        &cpu::detected(),
        args.target_cpu.as_deref(),
    )?;
    let (revs, built, schema) =
        ship::make(cx, src, &reef, &manifest, overrides, found.kind, &class)?;
    if let Some(schema) = &schema {
        check(cx, &adopted, schema, false)?;
    }
    let record = ship::record(
        cx,
        &args.repo,
        &args.reference,
        found.kind,
        revs,
        &manifest,
        &class,
        schema.as_ref(),
    );
    Ok((record, built, adopted))
}

/// Refuses a commit whose schema pin can't read what the install wrote, before it is built
///
/// # Arguments
///
/// * `cx` - The runner
/// * `src` - The sources directory, reef's checkout at the commit among it
/// * `reef` - The commit
/// * `adopted` - The install, as a release, its tables recorded if they are known
pub(crate) fn check_pin(cx: &mut Cx, src: &Path, reef: &str, adopted: &Release) -> Result<()> {
    let pin = src.join("reef/central/reef-service/src/schema.rs");
    match exec::read(cx.runner, &pin, As::Me)? {
        Some(source) => check(cx, adopted, &Schema::from_pin(&source)?, true),
        None if cx.runner.dry() => Ok(()),
        None => bail!("reef's commit {reef} has no {}", pin.display()),
    }
}

/// The install made by hand, as a release of its own: the commit it was built from if the admin
/// named it, and the schema its data was written with if it is known
///
/// # Arguments
///
/// * `from_rev` - The commit it was built from
/// * `schema` - The schema its data was written with
pub(crate) fn adopted_release(from_rev: Option<&str>, schema: Option<&Schema>) -> Release {
    Release {
        name: ADOPTED.to_string(),
        reef: from_rev.unwrap_or("unknown").to_string(),
        shoal: "unknown".to_string(),
        glommio: "unknown".to_string(),
        toolchain: "unknown".to_string(),
        deployed: 0,
        schema: None,
        tables: Default::default(),
        build: None,
    }
    .with_schema(schema)
}

/// Says what a builder elsewhere needs to know of an install to take it over: the server it runs,
/// whatever would stop adopt, and the schema its own reef-serve says
///
/// The reads root alone may make are left out, since this runs with no terminal to ask sudo's
/// password on: adopt finds the install again as root once the release is shipped.
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What deploy was told
/// * `layout` - The deployment's paths
/// * `systemd` - The unit's manager
/// * `src` - The sources directory
fn facts(
    cx: &mut Cx,
    args: &DeployArgs,
    layout: &Layout,
    systemd: &Systemd,
    src: &Path,
) -> Result<()> {
    let mut facts = Facts::here(cx)?;
    if !systemd.answers(cx.runner)? {
        facts.problems.push(
            "systemd doesn't answer there, so there is no unit of reef's to take over".into(),
        );
        return facts.say(cx.runner);
    }
    match find(cx.runner, layout, systemd) {
        Ok(found) => {
            facts.kind = Some(found.kind);
            facts.problems = preflight(cx.runner, layout, &found, src, false)?;
            // whether the install has a user yet, if it runs, for the builder to ask a first
            // admin's password
            if found.active {
                facts.users = first_admin::users(cx.runner, layout, &found.served);
            }
            // the schema as the old reef-serve says it, unless the admin named the commit
            if args.from_rev.is_none()
                && let Some(schema) = own_word(cx.runner, layout, &found)?
            {
                facts.tables = adopted_release(None, Some(&schema)).tables;
            }
        }
        Err(error) => facts.problems.push(format!("{error:#}")),
    }
    facts.say(cx.runner)
}

/// reef as systemd runs it, refusing a unit reefadm wrote or one it can't take over
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `systemd` - The unit's manager
pub fn find(runner: &mut dyn Runner, layout: &Layout, systemd: &Systemd) -> Result<Found> {
    // the unit as systemd loads it, with its drop-ins
    let mut show = Cmd::new("systemctl");
    if systemd.user {
        show = show.arg("--user");
    }
    let property = format!("--property={PROPERTIES}");
    let shown = runner.probe(&show.args(["show", UNIT, &property]))?;
    let properties: BTreeMap<&str, &str> = shown
        .stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let get = |key: &str| properties.get(key).copied().unwrap_or_default();
    let fragment = PathBuf::from(get("FragmentPath"));
    let (which, flag) = match layout.mode {
        Mode::System => ("system unit", ""),
        Mode::User => ("user unit", " --user"),
    };
    if get("LoadState") != "loaded" || fragment.as_os_str().is_empty() {
        bail!(
            "there is no {which} {UNIT} to take over: `reefadm deploy{flag}` deploys reef afresh"
        );
    }
    let unit = exec::read(runner, &fragment, As::Me)?.unwrap_or_default();
    if unit::ours(&unit) {
        bail!(
            "reefadm wrote {} already: `reefadm upgrade{flag}` moves it to a later commit",
            fragment.display()
        );
    }
    // the server it runs: reef-app's beside its public/, or reef-serve
    let argv = argv(get("ExecStart"));
    let Some(program) = argv.first().map(PathBuf::from) else {
        bail!(
            "{} runs nothing reefadm can read: its ExecStart is `{}`",
            fragment.display(),
            get("ExecStart")
        );
    };
    let kind = match program.file_name().and_then(|name| name.to_str()) {
        Some("server") => Kind::App,
        Some("reef-serve") => Kind::Serve,
        _ => bail!(
            "{} runs {}, which is neither reef-app's server nor reef-serve",
            fragment.display(),
            program.display()
        ),
    };
    if kind == Kind::App {
        let public = program.with_file_name("public");
        if !exec::exists(runner, &public, As::Me)? {
            bail!(
                "{} runs {}, and reef-app's server has its public/ beside it, which isn't there",
                fragment.display(),
                program.display()
            );
        }
    }
    // the account a team server runs as, whose home its defaults are under
    let env = environment(get("Environment"));
    let home = match layout.mode {
        Mode::System => account_home(runner, layout, &fragment, get("User"))?,
        Mode::User => layout.home.clone(),
    };
    // the config it reads: --conf, else REEF_CONFIG, else the default under that home
    let config = conf_flag(&argv)
        .or_else(|| env.get("REEF_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| {
            env.get("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("reef/config.toml")
        });
    let text = exec::read(runner, &config, layout.reader())?;
    let mut served = match &text {
        Some(text) => Served::read(text, &config)?,
        None => Served::default(),
    };
    // each path left to reef's defaults, which are under the account's data home
    let data_home = env
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("reef");
    let mut named = Vec::new();
    let defaults = [
        ("shoal", "data", served.shoal.data.is_none(), layout.data()),
        (
            "remotes",
            "clones",
            served.remotes.clones.is_none(),
            layout.clones(),
        ),
        ("api", "certs", served.api.certs.is_none(), layout.certs()),
    ];
    for (table, key, _, ours) in defaults.into_iter().filter(|(_, _, left, _)| *left) {
        // filled in as reef fills it, and named where reefadm's default would differ
        let path = data_home.join(match key {
            "data" => "shoal",
            other => other,
        });
        match key {
            "data" => served.shoal.data = Some(path.clone()),
            "clones" => served.remotes.clones = Some(path.clone()),
            _ => served.api.certs = Some(path.clone()),
        }
        if path != ours {
            named.push(Named { table, key, path });
        }
    }
    Ok(Found {
        fragment,
        unit,
        drop_ins: get("DropInPaths")
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        kind,
        program,
        config,
        text,
        served,
        named,
        active: get("ActiveState") == "active",
        enabled: get("UnitFileState") == "enabled",
    })
}

/// The account a team server's unit runs as, which must be reef's, and its home
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The team server's paths
/// * `fragment` - The unit's file, for the error
/// * `user` - The unit's `User=`
fn account_home(
    runner: &mut dyn Runner,
    layout: &Layout,
    fragment: &Path,
    user: &str,
) -> Result<PathBuf> {
    if user != ACCOUNT {
        bail!(
            "{} runs reef as {}, and reefadm runs a team server as the `{ACCOUNT}` account, whose \
             home is {}: move the install to it as the book's Building and installing page does, \
             then adopt it",
            fragment.display(),
            match user.is_empty() {
                true => "root",
                false => user,
            },
            layout.data_home.display()
        );
    }
    // passwd's sixth field
    let entry = runner.probe(&Cmd::new("getent").args(["passwd", ACCOUNT]))?;
    let home = entry
        .stdout
        .trim()
        .split(':')
        .nth(5)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    match home {
        Some(home) if home == layout.data_home => Ok(home),
        Some(home) => bail!(
            "the `{ACCOUNT}` account's home is {}, and reefadm keeps a team server's under {}",
            home.display(),
            layout.data_home.display()
        ),
        None => bail!(
            "getent names no home for the `{ACCOUNT}` account {} runs as",
            fragment.display()
        ),
    }
}

/// The program and arguments of the unit's ExecStart, as `systemctl show` gives it
///
/// # Arguments
///
/// * `exec_start` - The property, such as `{ path=/opt/reef/app/server ; argv[]=/opt/reef/app/server ; … }`
///
/// # Examples
///
/// ```
/// use reefadm::commands::adopt::argv;
///
/// let shown = "{ path=/usr/local/bin/reef-serve ; argv[]=/usr/local/bin/reef-serve --conf /etc/reef/config.toml ; ignore_errors=no }";
/// assert_eq!(argv(shown), ["/usr/local/bin/reef-serve", "--conf", "/etc/reef/config.toml"]);
/// assert!(argv("").is_empty());
/// ```
pub fn argv(exec_start: &str) -> Vec<String> {
    // the last command, if a drop-in added one, up to the field after it
    exec_start
        .rsplit("argv[]=")
        .next()
        .filter(|_| exec_start.contains("argv[]="))
        .and_then(|rest| rest.split(" ;").next())
        .map(|words| words.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

/// The variables the unit sets, as `systemctl show` gives its Environment
///
/// # Arguments
///
/// * `environment` - The property, such as `REEF_CONFIG=/etc/reef/config.toml`
fn environment(environment: &str) -> BTreeMap<String, String> {
    environment
        .split_whitespace()
        .map(|pair| pair.trim_matches('"'))
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The config a server's arguments name with `--conf`, if they do
///
/// # Arguments
///
/// * `argv` - The program and its arguments
fn conf_flag(argv: &[String]) -> Option<PathBuf> {
    argv.iter()
        .enumerate()
        .find_map(|(at, arg)| match arg.strip_prefix("--conf=") {
            Some(path) => Some(PathBuf::from(path)),
            None if arg == "--conf" => argv.get(at + 1).map(PathBuf::from),
            None => None,
        })
}

/// What adopt found, in a line
///
/// # Arguments
///
/// * `layout` - The deployment's paths
/// * `found` - The install
fn describe(layout: &Layout, found: &Found) -> String {
    let unit = match layout.mode {
        Mode::System => format!("the system unit {UNIT}, as {ACCOUNT}"),
        Mode::User => format!("your user unit {UNIT}"),
    };
    let config = match &found.text {
        Some(_) => format!("its config {}", found.config.display()),
        None => format!("no config at {}", found.config.display()),
    };
    format!(
        "found reef installed by hand: {unit} running {}, with {config}, {}",
        found.program.display(),
        match found.active {
            true => "serving",
            false => "stopped",
        }
    )
}

/// Everything that would stop adopt partway, a line each
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `found` - The install
/// * `src` - The sources directory
fn preflight(
    runner: &mut dyn Runner,
    layout: &Layout,
    found: &Found,
    src: &Path,
    building: bool,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    // a deployment reefadm made is upgraded
    if exec::exists(runner, &layout.state, As::Me)? {
        problems.push(format!(
            "reefadm deployed reef here already, as {} says: `reefadm upgrade` moves it",
            layout.state.display()
        ));
    }
    // the ports are the install's own while it runs, and else must be free
    if !found.active {
        let listening = deploy::listening(runner)?;
        let ports = port_of(&found.served.loopback())
            .into_iter()
            .chain(found.served.tls_port());
        for port in ports {
            if listening.contains(&port) {
                problems.push(format!(
                    "port {port} is taken, and the install made by hand is stopped: stop what \
                     listens there"
                ));
            }
        }
    }
    // room for the build, where it is built
    if building {
        problems.extend(deploy::room(runner, layout, src)?);
    }
    Ok(problems)
}

/// The schema reef's commit pinned, as the checkout here holds its history
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory, reef's checkout among it
/// * `rev` - The commit
pub(crate) fn pin_at(runner: &mut dyn Runner, src: &Path, rev: &str) -> Result<Option<Schema>> {
    let shown = runner.probe(
        &Cmd::new("git")
            .arg("-C")
            .arg(src.join("reef"))
            .arg("show")
            .arg(format!("{rev}:central/reef-service/src/schema.rs")),
    )?;
    match (shown.ok(), runner.dry()) {
        (true, _) => Schema::from_pin(&shown.stdout).map(Some),
        (false, true) => Ok(None),
        (false, false) => bail!(
            "reef's history has no schema at {rev}: {}",
            shown.stderr.trim()
        ),
    }
}

/// The schema the install's own reef-serve says, which it does since C109 and refuses the word
/// before
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `found` - The install
fn own_word(runner: &mut dyn Runner, layout: &Layout, found: &Found) -> Result<Option<Schema>> {
    let serve = match found.kind {
        Kind::Serve => found.program.clone(),
        Kind::App => layout.bin.join("reef-serve"),
    };
    if !exec::exists(runner, &serve, As::Me)? {
        return Ok(None);
    }
    Ok(Schema::of_binary(runner, &serve).ok().flatten())
}

/// Refuses a schema that can't read the data the install wrote, saying when it can't tell
///
/// # Arguments
///
/// * `cx` - The runner
/// * `adopted` - The install, as a release, its tables recorded if they are known
/// * `new` - The new commit's schema
/// * `say_unknown` - Whether to say that the old schema isn't known, which is said once
pub(crate) fn check(cx: &mut Cx, adopted: &Release, new: &Schema, say_unknown: bool) -> Result<()> {
    if adopted.tables.is_empty() {
        if say_unknown {
            cx.runner.say(
                "reefadm can't tell which schema the install's data was written with, so it can't \
                 check that this commit reads it back: name the reef commit it was built from with \
                 --from-rev to check, or rely on the backup it takes first",
            );
        }
        return Ok(());
    }
    let verdict = Verdict::between(&adopted.tables, new);
    if !verdict.reads_back() {
        bail!(
            "this commit's schema {}, and shoal can't migrate a table, so it can't read the data \
             the install made by hand wrote. Export every plan first, then deploy afresh and \
             import them again, as the book's \"When the store's schema changes\" page says",
            verdict.said()
        );
    }
    Ok(())
}

/// Whether the install answers plain HTTP on loopback, as reef did before it served HTTPS
///
/// # Arguments
///
/// * `runner` - The runner
/// * `served` - Its config
fn plain_http(runner: &mut dyn Runner, served: &Served) -> bool {
    // one that serves HTTPS redirects there, which isn't trusted without its CA
    let ask = Ask {
        url: format!("http://{}/api/v1/me", served.loopback()),
        ca: None,
        resolve: None,
    };
    runner.me(&ask).is_ok()
}

/// Keeps the install, puts reefadm's in its place and starts it, returning the config it runs
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `systemd` - The unit's manager
/// * `found` - The install
/// * `release` - The new release's directory
/// * `hostname` - This host's name
/// * `kept` - What was changed so far, for putting it back
fn take_over(
    cx: &mut Cx,
    layout: &Layout,
    systemd: &Systemd,
    found: &Found,
    release: &Path,
    hostname: &str,
    kept: &mut Kept,
) -> Result<Served> {
    let owner = layout.owner();
    let runner = &mut *cx.runner;
    let adopted = layout.releases.join(ADOPTED);
    // the commands it had, and its app, copied into a release of its own
    runner.run(&Cmd::new("rm").arg("-rf").arg(&adopted).by(owner))?;
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-m", "0755"])
            .arg(adopted.join("bin"))
            .by(owner),
    )?;
    for command in COMMANDS {
        let path = layout.bin.join(command);
        if exec::exists(runner, &path, owner)? {
            runner.run(
                &Cmd::new("cp")
                    .arg("-a")
                    .arg(&path)
                    .arg(adopted.join("bin").join(command))
                    .by(owner),
            )?;
            kept.commands.push(command);
        }
    }
    if let (Kind::App, Some(app)) = (found.kind, found.program.parent()) {
        runner.run(
            &Cmd::new("cp")
                .arg("-a")
                .arg(app)
                .arg(adopted.join("app"))
                .by(owner),
        )?;
    }
    // its unit, kept beside the one reefadm writes over it
    if found.fragment == layout.unit {
        runner.run(
            &Cmd::new("cp")
                .arg("-a")
                .arg(&layout.unit)
                .arg(aside(&layout.unit))
                .by(owner),
        )?;
        kept.unit = true;
    }
    // its config, where reefadm reads it, if it isn't read the same there
    if let Some(text) = found.rewritten(layout)? {
        let had = exec::exists(runner, &layout.config, owner)?;
        if had {
            runner.run(
                &Cmd::new("cp")
                    .arg("-a")
                    .arg(&layout.config)
                    .arg(aside(&layout.config))
                    .by(owner),
            )?;
        }
        if let Some(dir) = layout.config.parent() {
            runner.run(
                &Cmd::new("install")
                    .args(["-d", "-m", "0755"])
                    .arg(dir)
                    .by(owner),
            )?;
        }
        kept.config = Some(had);
        runner.write(&File {
            path: layout.config.clone(),
            text,
            mode: 0o644,
            runs_as: owner,
            owner: None,
        })?;
    }
    // the new release, made current
    kept.activated = true;
    release::activate(runner, layout, release)?;
    // the settings its unit had and reefadm's lacks, in a drop-in of their own
    let low = super::low_port(&found.served);
    if let Some((text, _)) = carried(&found.unit, &unit::text(layout, found.kind, low)) {
        let path = drop_in(layout);
        if let Some(dir) = path.parent() {
            runner.run(
                &Cmd::new("install")
                    .args(["-d", "-m", "0755"])
                    .arg(dir)
                    .by(owner),
            )?;
        }
        kept.drop_in = true;
        runner.write(&File {
            path,
            text,
            mode: 0o644,
            runs_as: owner,
            owner: None,
        })?;
    }
    // reefadm's unit, written as deploy writes it, then started
    kept.wrote_unit = true;
    kept.started = true;
    deploy::start(cx, layout, systemd, found.kind, low)?;
    health::wait(cx.runner, layout, &found.served, hostname)?;
    Ok(found.served.clone())
}

/// The drop-in adopt keeps the install's own unit settings in
///
/// # Arguments
///
/// * `layout` - The deployment's paths
pub fn drop_in(layout: &Layout) -> PathBuf {
    let name = layout
        .unit
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    layout
        .unit
        .with_file_name(format!("{name}.d"))
        .join("adopted.conf")
}

/// The settings a unit made by hand has and reefadm's lacks, as a drop-in that keeps them with
/// the comments above each, and their names, or none if it has none
///
/// ExecStart is reefadm's, on its current release, and so is `REEF_CONFIG`, where it reads the
/// config; `[Install]` is left out, since systemd reads it from the unit alone.
///
/// # Arguments
///
/// * `old` - The unit made by hand
/// * `ours` - The unit reefadm writes in its place
///
/// # Examples
///
/// ```
/// use reefadm::commands::adopt::carried;
///
/// let ours = "[Service]\nUser=reef\nExecStart=/opt/reef/current/app/server\n";
/// let old = "# europa's\n[Service]\nUser=reef\nExecStart=/opt/reef/app/server\n# off the benchmarks\nAllowedCPUs=15\n";
/// let (text, keys) = carried(old, ours).unwrap();
/// assert!(text.ends_with("# europa's\n\n[Service]\n# off the benchmarks\nAllowedCPUs=15\n"));
/// assert_eq!(keys, ["AllowedCPUs"]);
/// assert_eq!(carried(ours, ours), None);
/// ```
pub fn carried(old: &str, ours: &str) -> Option<(String, Vec<String>)> {
    // reefadm's own settings, by section, a line each
    let mut theirs = std::collections::BTreeSet::new();
    let mut section = String::new();
    for line in ours.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            section = name.to_string();
        } else if line.contains('=') && !line.starts_with('#') {
            theirs.insert((section.clone(), line.to_string()));
        }
    }
    // the old unit's, a directive at a time, with the comments just above it
    let mut preamble = Vec::new();
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    let mut keys = Vec::new();
    let mut section = String::new();
    let mut lines = old.lines();
    while let Some(line) = lines.next() {
        let mut line = line.trim().to_string();
        // a directive continued on the next line, joined
        while line.ends_with('\\') {
            line.pop();
            match lines.next() {
                Some(next) => line.push_str(next.trim()),
                None => break,
            }
        }
        if line.is_empty() {
            comments.clear();
        } else if line.starts_with('#') || line.starts_with(';') {
            match section.is_empty() {
                true => preamble.push(line),
                false => comments.push(line),
            }
        } else if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            section = name.to_string();
            comments.clear();
        } else if let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim(), value.trim());
            let reefadms = key == "ExecStart"
                || (key == "Environment" && value.trim_matches('"').starts_with("REEF_CONFIG="))
                || section == "Install"
                || theirs.contains(&(section.clone(), line.clone()));
            if !reefadms {
                let at = match sections.iter().position(|(name, _)| *name == section) {
                    Some(at) => at,
                    None => {
                        sections.push((section.clone(), Vec::new()));
                        sections.len() - 1
                    }
                };
                sections[at].1.append(&mut comments);
                sections[at].1.push(line.clone());
                keys.push(key.to_string());
            }
            comments.clear();
        }
    }
    if keys.is_empty() {
        return None;
    }
    // a line saying what it is, the unit's own opening comments, then each section's settings
    let mut text = format!(
        "# Written by `reefadm deploy --adopt`: what {UNIT} had, made by hand, that reefadm's unit\n\
         # doesn't. reefadm never writes it again, so it is yours to change.\n"
    );
    if !preamble.is_empty() {
        text.push('\n');
        text.push_str(&preamble.join("\n"));
        text.push('\n');
    }
    for (name, lines) in sections {
        text.push_str(&format!("\n[{name}]\n{}\n", lines.join("\n")));
    }
    keys.dedup();
    Some((text, keys))
}

/// A file's name with adopt's mark added, for the one it keeps beside what it writes
///
/// # Arguments
///
/// * `path` - The file
fn aside(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.{BEFORE}"))
}

/// Puts the install back as it was, with its data as the backup holds it, and starts it again if
/// it was running
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `systemd` - The unit's manager
/// * `found` - The install
/// * `kept` - What adopt changed
/// * `saved` - The backup taken first, if one was
/// * `stamp` - When adopt ran, which names the data the new release wrote
fn put_back(
    cx: &mut Cx,
    layout: &Layout,
    systemd: &Systemd,
    found: &Found,
    kept: &Kept,
    saved: Option<&Path>,
    stamp: &str,
) -> Result<()> {
    let owner = layout.owner();
    let runner = &mut *cx.runner;
    runner.say("putting the install made by hand back as it was");
    // the new release stopped, whatever state it is in
    let _ = super::stop(runner, systemd);
    // its unit, or none of reefadm's over one systemd loads from elsewhere
    if kept.unit {
        runner.run(
            &Cmd::new("mv")
                .args(["-f", "-T"])
                .arg(aside(&layout.unit))
                .arg(&layout.unit)
                .by(owner),
        )?;
    } else if kept.wrote_unit {
        runner.run(&Cmd::new("rm").arg("-f").arg(&layout.unit).by(owner))?;
    }
    if kept.drop_in {
        runner.run(&Cmd::new("rm").arg("-f").arg(drop_in(layout)).by(owner))?;
    }
    runner.run(&systemd.daemon_reload())?;
    // its commands as they were, and none it didn't have
    if kept.activated {
        for command in COMMANDS {
            let path = layout.bin.join(command);
            let cmd = match kept.commands.contains(&command) {
                // over the link, not through it into the new release
                true => Cmd::new("cp")
                    .args(["-a", "--remove-destination"])
                    .arg(layout.releases.join(ADOPTED).join("bin").join(command))
                    .arg(&path),
                false => Cmd::new("rm").arg("-f").arg(&path),
            };
            runner.run(&cmd.by(owner))?;
        }
    }
    // its config
    match kept.config {
        Some(true) => runner.run(
            &Cmd::new("mv")
                .args(["-f", "-T"])
                .arg(aside(&layout.config))
                .arg(&layout.config)
                .by(owner),
        )?,
        Some(false) => runner.run(&Cmd::new("rm").arg("-f").arg(&layout.config).by(owner))?,
        None => {}
    }
    // its data as it was, if the new release ran on it, what that wrote kept aside
    if let (true, Some(saved)) = (kept.started, saved) {
        backup::restore(
            runner,
            layout,
            &found.served,
            saved,
            &format!("failed-{stamp}"),
        )?;
    }
    // started again as it was: enabled or not, running or not
    runner.run(&systemd.ctl(&["reset-failed", UNIT]))?;
    if kept.started && !found.enabled {
        runner.run(&systemd.ctl(&["disable", UNIT]))?;
    }
    if found.active {
        super::start(runner, systemd)?;
        runner.say("the install made by hand is serving again");
    }
    Ok(())
}

/// Says what became of the install made by hand
///
/// # Arguments
///
/// * `cx` - The runner
/// * `layout` - The deployment's paths
/// * `found` - The install
/// * `plain` - Whether it served plain HTTP
/// * `saved` - The backup taken first, if one was
fn notes(cx: &mut Cx, layout: &Layout, found: &Found, plain: bool, saved: Option<&Path>) {
    let adopted = layout.releases.join(ADOPTED);
    let mut lines = vec![format!(
        "  the install made by hand is kept: its commands{} in {}",
        match found.kind {
            Kind::App => " and app",
            Kind::Serve => "",
        },
        adopted.display()
    )];
    if found.fragment == layout.unit {
        lines.push(format!(
            "  its unit is kept as {}",
            aside(&layout.unit).display()
        ));
    }
    if let Some(saved) = saved {
        lines.push(format!("  its data as it was is in {}", saved.display()));
    }
    // what it ran from that reefadm's release doesn't
    let ran_from = match found.kind {
        Kind::App => found.program.parent().map(Path::to_path_buf),
        Kind::Serve => Some(found.program.clone()),
    };
    if let Some(ran_from) = ran_from.filter(|path| !path.starts_with(&layout.bin)) {
        lines.push(format!(
            "  {} isn't used any more, and is yours to remove",
            ran_from.display()
        ));
    }
    let low = super::low_port(&found.served);
    if let Some((_, keys)) = carried(&found.unit, &unit::text(layout, found.kind, low)) {
        lines.push(format!(
            "  its unit's own settings are kept in {}: {}",
            drop_in(layout).display(),
            keys.join(", ")
        ));
    }
    if !found.drop_ins.is_empty() {
        lines.push(format!(
            "  its drop-ins still apply to reefadm's unit: {}",
            found.drop_ins.join(", ")
        ));
    }
    // its config, as it is, or where reefadm reads it, the one there before kept beside it
    match (found.rewrites(layout), &found.text) {
        (false, Some(_)) => lines.push(format!(
            "  its config {} is kept as it is",
            found.config.display()
        )),
        (false, None) => {}
        (true, _) => lines.push(format!(
            "  reef's config is written to {}{}, naming each path the install left to reef's \
             defaults{}",
            layout.config.display(),
            match &found.text {
                Some(_) => format!(" from {}", found.config.display()),
                None => String::new(),
            },
            match found.config == layout.config {
                true => format!(
                    ", the one before kept as {}",
                    aside(&layout.config).display()
                ),
                false => String::new(),
            }
        )),
    }
    // a server that served plain HTTP has clients that call it so
    if plain {
        let ca = layout
            .ca_copy
            .clone()
            .unwrap_or_else(|| found.served.certs(layout).join("ca.pem"));
        lines.push(format!(
            "  it served plain HTTP, and this release serves HTTPS on every port: each client's \
             `url` needs `https://` if it names one, and a client on another host needs reef's \
             CA, {}, as `ca`",
            ca.display()
        ));
    }
    for line in lines {
        cx.runner.say(&line);
    }
}
