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
//!    reads it, naming each path it left to reef's defaults, the one there kept beside it.
//! 6. **reefadm's unit** on the new release, started, and both ports asked. If it doesn't come
//!    up, the install goes back as it was, its data restored from the backup, and started again
//!    if it was running.
//! 7. **reefadm's record,** the install made by hand kept as the release before.

use super::deploy;
use crate::cli::DeployArgs;
use crate::exec::{self, ACCOUNT, As, Ask, Cmd, File, Runner};
use crate::layout::{Kind, Layout, Mode, UNIT};
use crate::release::{self, COMMANDS, Record};
use crate::schema::{Schema, Verdict};
use crate::served::{Served, port_of};
use crate::state::{Release, State};
use crate::systemd::Systemd;
use crate::{Cx, backup, compile, health, sources, toolchain, unit};
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
    /// one elsewhere, or one leaving a path to a default reefadm's unit moves
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths
    pub fn rewrites(&self, layout: &Layout) -> bool {
        !self.named.is_empty() || (self.text.is_some() && self.config != layout.config)
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
        let mut table: toml::Table = match &self.text {
            Some(text) => text
                .parse()
                .with_context(|| format!("reading reef's config {}", self.config.display()))?,
            None => toml::Table::new(),
        };
        for named in &self.named {
            let section = table
                .entry(named.table)
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let Some(section) = section.as_table_mut() else {
                bail!(
                    "{} has `{}` as a value, where reef reads a table",
                    self.config.display(),
                    named.table
                );
            };
            section.insert(
                named.key.to_string(),
                toml::Value::String(named.path.display().to_string()),
            );
        }
        // a line saying where it came from, then its keys
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
        let keys = toml::to_string(&table).context("writing reef's config")?;
        Ok(Some(format!(
            "# Written by `reefadm deploy --adopt` {from}{naming}.\n# reefadm keeps it as it is \
             from here on: edit it as you like, then restart reef.\n\n{keys}"
        )))
    }
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
    // the install, as systemd says it runs
    if !systemd.answers(cx.runner)? {
        bail!("systemd doesn't answer here, so there is no unit of reef's to take over");
    }
    let found = find(cx.runner, &layout, &systemd)?;
    cx.runner.say(&describe(&layout, &found));
    deploy::supported(cx.runner)?;
    // everything that would stop it partway, before anything changes
    let problems = preflight(cx.runner, &layout, &found, &src)?;
    deploy::stop_for(cx, &problems)?;
    let hostname = super::hostname(cx.runner)?;
    // reef at its ref, and the schema the install's data was written with against its commit's
    cx.runner.say(&format!(
        "fetching reef at {} from {}",
        args.reference, args.repo
    ));
    let (reef, manifest) = sources::fetch_reef(cx.runner, &src, &args.repo, &args.reference)?;
    let old = old_schema(cx.runner, &layout, &src, &found, args.from_rev.as_deref())?;
    let adopted = Release {
        name: ADOPTED.to_string(),
        reef: args
            .from_rev
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        shoal: "unknown".to_string(),
        glommio: "unknown".to_string(),
        toolchain: "unknown".to_string(),
        deployed: 0,
        schema: None,
        tables: Default::default(),
    }
    .with_schema(old.as_ref());
    let pin = src.join("reef/central/reef-service/src/schema.rs");
    match exec::read(cx.runner, &pin, As::Me)? {
        Some(source) => check(cx, &adopted, &Schema::from_pin(&source)?, true)?,
        None if cx.runner.dry() => {}
        None => bail!("reef's commit {reef} has no {}", pin.display()),
    }
    // the toolchains, the siblings and the build, as deploy does
    toolchain::ensure(cx.runner, &manifest, found.kind, &src, cx.yes)?;
    let revs = sources::fetch_siblings(cx.runner, &src, &reef, &manifest, &overrides)?;
    cx.runner.say(&format!(
        "building reef {} with shoal {} and glommio {}",
        deploy::short(&revs.reef),
        deploy::short(&revs.shoal),
        deploy::short(&revs.glommio)
    ));
    let built = compile::build(cx.runner, &src, &manifest.toolchain, found.kind)?;
    let schema = Schema::of_binary(cx.runner, &built.serve)?;
    if let Some(schema) = &schema {
        check(cx, &adopted, schema, false)?;
    }
    // root's steps from here on, which the build may have outlasted sudo's timestamp for
    if mode == Mode::System {
        cx.runner.elevate()?;
    }
    let name = release::name(&revs.reef, cx.now);
    let deployed = cx
        .now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let record = Record {
        repo: args.repo.clone(),
        reference: args.reference.clone(),
        kind: found.kind,
        release: Release {
            name: name.clone(),
            reef: revs.reef,
            shoal: revs.shoal,
            glommio: revs.glommio,
            toolchain: manifest.toolchain,
            deployed,
            schema: None,
            tables: Default::default(),
        }
        .with_schema(schema.as_ref()),
    };
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
        first_admin: None,
        src: args.src.clone(),
    };
    state.write(cx.runner, &layout)?;
    deploy::summary(cx, &layout, &served, &hostname, &name, false);
    notes(cx, &layout, &found, plain, saved.as_deref());
    Ok(())
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
    let text = exec::read(runner, &fragment, As::Me)?.unwrap_or_default();
    if unit::ours(&text) {
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
    // room for the build
    problems.extend(deploy::room(runner, layout, src)?);
    Ok(problems)
}

/// The schema the install's data was written with: the pin at a commit of reef's, else the old
/// reef-serve's own word, if it is new enough to say it
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `src` - The sources directory, reef's checkout among it
/// * `found` - The install
/// * `from_rev` - The commit it was built from, if the admin named it
fn old_schema(
    runner: &mut dyn Runner,
    layout: &Layout,
    src: &Path,
    found: &Found,
    from_rev: Option<&str>,
) -> Result<Option<Schema>> {
    // the pin at the commit named, which reef's history holds
    if let Some(rev) = from_rev {
        let shown = runner.probe(
            &Cmd::new("git")
                .arg("-C")
                .arg(src.join("reef"))
                .arg("show")
                .arg(format!("{rev}:central/reef-service/src/schema.rs")),
        )?;
        return match (shown.ok(), runner.dry()) {
            (true, _) => Schema::from_pin(&shown.stdout).map(Some),
            (false, true) => Ok(None),
            (false, false) => bail!(
                "reef's history has no schema at {rev}: {}",
                shown.stderr.trim()
            ),
        };
    }
    // else the old reef-serve, which says it since C109, and refuses the word before
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
fn check(cx: &mut Cx, adopted: &Release, new: &Schema, say_unknown: bool) -> Result<()> {
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
    // the new release, reefadm's unit on it, and its start
    kept.activated = true;
    release::activate(runner, layout, release)?;
    kept.wrote_unit = true;
    let low = super::low_port(&found.served);
    super::write_unit(runner, layout, systemd, found.kind, low)?;
    kept.started = true;
    deploy::start(cx, layout, systemd, found.kind, low)?;
    health::wait(cx.runner, layout, &found.served, hostname)?;
    Ok(found.served.clone())
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
