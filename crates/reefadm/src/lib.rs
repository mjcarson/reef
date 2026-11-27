//! reefadm: central reef deployed, upgraded and run from one command
//!
//! ```sh
//! reefadm deploy                    # a team server on this host: a system unit, sign-in, a TLS port
//! reefadm deploy --user             # one person's server: a user unit on loopback
//! reefadm --dry-run deploy          # every step it would take, taking none
//! reefadm upgrade                   # to the ref's newest commit, put back if it doesn't come up
//! reefadm uninstall                 # what deploy put here, keeping the data unless --purge
//! reefadm status                    # its release, unit, ports, data and certificate
//! reefadm backup                    # its data, copied while it is stopped; `restore FILE` back
//! reefadm recover ada               # a lost admin, from the store itself
//! reefadm --host europa deploy      # any of these, on another host over ssh
//! ```
//!
//! It fetches reef at a ref, and shoal and glommio beside it at the revs reef's
//! `central/sources.toml` pins, installs the toolchains they build with, builds the CLI,
//! reef-serve and reef-app as the book's Deploying chapter does, and installs them as a release
//! under a `current` link, with the unit, the config, a readable copy of reef's own CA and the
//! first admin. It keeps a record of what it deployed, for the next upgrade.
//!
//! - **Every side effect goes through one [`exec::Runner`],** so `--dry-run` shows the steps,
//!   and the tests compare them.
//! - **It runs as the admin,** and puts `sudo` in front of only the steps that need root.
//! - **Each step looks before it acts,** so a deploy that stopped partway, run again, carries on
//!   from there.

pub mod backup;
pub mod cli;
pub mod commands;
pub mod compile;
pub mod cpu;
pub mod exec;
pub mod health;
pub mod layout;
pub mod release;
pub mod remote;
pub mod schema;
pub mod secret;
pub mod served;
pub mod ship;
pub mod sources;
pub mod state;
pub mod systemd;
pub mod toolchain;
pub mod unit;

pub use cli::Cli;

use anyhow::Result;
use exec::Runner;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// An exit code a command ends with having said why already, such as another host's reefadm's
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub u8);

impl std::fmt::Display for Exit {
    /// Names the exit code
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exited {}", self.0)
    }
}

impl std::error::Error for Exit {}

/// The environment reefadm runs in, read once, so a test can give it one of its own
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env(BTreeMap<String, String>);

impl Env {
    /// This process's environment
    pub fn from_process() -> Self {
        Env(std::env::vars().collect())
    }

    /// An environment of the given variables alone
    ///
    /// # Arguments
    ///
    /// * `pairs` - Each variable's name and value
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::Env;
    ///
    /// let env = Env::from_pairs([("HOME", "/home/ada")]);
    /// assert_eq!(env.get("HOME"), Some("/home/ada"));
    /// assert_eq!(env.get("USER"), None);
    /// ```
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Env(pairs
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect())
    }

    /// A variable's value, if it is set and not empty
    ///
    /// # Arguments
    ///
    /// * `key` - Its name
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// A variable naming a directory, if it names an absolute one, as the XDG spec reads them
    ///
    /// # Arguments
    ///
    /// * `key` - Its name
    pub fn dir(&self, key: &str) -> Option<PathBuf> {
        self.get(key)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
    }
}

/// What every command runs with: the runner, the environment, the time, and whether to say yes
pub struct Cx<'a> {
    /// Everything reefadm does to the host goes through it
    pub runner: &'a mut dyn Runner,
    /// The environment reefadm runs in
    pub env: Env,
    /// Whether to answer yes to every question
    pub yes: bool,
    /// When the command runs, which names its release
    pub now: SystemTime,
}

impl Cx<'_> {
    /// Asks a question, unless `--yes` answered it already
    ///
    /// # Arguments
    ///
    /// * `question` - What to ask
    pub fn confirm(&mut self, question: &str) -> Result<bool> {
        match self.yes {
            true => Ok(true),
            false => self.runner.confirm(question),
        }
    }

    /// The admin's own name, from the environment
    pub fn user(&self) -> Option<String> {
        self.env
            .get("USER")
            .or_else(|| self.env.get("LOGNAME"))
            .map(String::from)
    }
}

/// Runs one command line on another host over ssh, ending as it ended there
///
/// `deploy`, `deploy --adopt` and `upgrade` are built here for the host's CPU and installed there;
/// any other command runs there as it would here. A dry run copies reefadm there and asks it what
/// it would do, which changes nothing else.
///
/// # Arguments
///
/// * `host` - The host, as ssh reaches it
/// * `args` - The command line's arguments, after the program's name, `--host` among them
/// * `yes` - Whether `--yes` was given
/// * `runner` - Everything it does here goes through this
/// * `env` - The environment it runs in
/// * `now` - When it runs
pub fn run_on(
    host: &str,
    args: &[String],
    yes: bool,
    runner: &mut dyn Runner,
    env: Env,
    now: SystemTime,
) -> Result<()> {
    run_on_with(host, args, yes, runner, env, now, &cpu::compiled())
}

/// Runs one command line on another host as [`run_on`] does, for a reefadm compiled with the
/// features given beyond plain x86-64's, which the copy it sends there must have none of
///
/// # Arguments
///
/// * `host` - The host, as ssh reaches it
/// * `args` - The command line's arguments, after the program's name, `--host` among them
/// * `yes` - Whether `--yes` was given
/// * `runner` - Everything it does here goes through this
/// * `env` - The environment it runs in
/// * `now` - When it runs
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
pub fn run_on_with(
    host: &str,
    args: &[String],
    yes: bool,
    runner: &mut dyn Runner,
    env: Env,
    now: SystemTime,
    compiled: &[&str],
) -> Result<()> {
    let mut cx = Cx {
        runner,
        env,
        yes,
        now,
    };
    // the commands that build are built here, and the rest run there
    let cli = Cli::from_args(std::iter::once("reefadm".to_string()).chain(args.iter().cloned()))?;
    match &cli.command {
        cli::Command::Deploy(deploy) if deploy.shipped.is_none() && !deploy.facts => {
            commands::hosted::deploy(&mut cx, host, deploy, compiled)
        }
        cli::Command::Upgrade(upgrade)
            if upgrade.finish.is_none() && upgrade.shipped.is_none() && !upgrade.facts =>
        {
            commands::hosted::upgrade(&mut cx, host, upgrade, compiled)
        }
        _ => remote::run(&mut cx, host, args, compiled),
    }
}

/// Runs one command line
///
/// # Arguments
///
/// * `cli` - The command line, parsed
/// * `runner` - Everything it does to the host goes through this
/// * `env` - The environment it runs in
/// * `now` - When it runs
pub fn run(cli: Cli, runner: &mut dyn Runner, env: Env, now: SystemTime) -> Result<()> {
    let mut cx = Cx {
        runner,
        env,
        yes: cli.yes,
        now,
    };
    // another host's command line is its own, which only `run_on` has whole
    if let Some(host) = &cli.host {
        anyhow::bail!("--host {host} runs through `reefadm::run_on`, with the whole command line");
    }
    match cli.command {
        cli::Command::Deploy(args) => commands::deploy::deploy(&mut cx, &args),
        cli::Command::Upgrade(args) => commands::upgrade::upgrade(&mut cx, &args),
        cli::Command::Uninstall(args) => commands::uninstall::uninstall(&mut cx, &args),
        cli::Command::Status(args) => commands::status::status(&mut cx, &args),
        cli::Command::Logs(args) => commands::status::logs(&mut cx, &args),
        cli::Command::Backup(args) => commands::backups::take(&mut cx, &args),
        cli::Command::Restore(args) => commands::backups::restore(&mut cx, &args),
        cli::Command::Recover(args) => commands::recover::recover(&mut cx, &args),
    }
}
