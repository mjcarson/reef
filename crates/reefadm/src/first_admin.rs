//! The first admin: added and signed in from the server's own host, wherever reefadm leaves a
//! server with no user
//!
//! Every central reef asks its callers to sign in, so a server with no user yet answers nobody
//! but the sign-in routes. `deploy`, for a team server and a user's alike, and `upgrade` and
//! `deploy --adopt` of a server with no user, each add the first admin, from this host as the
//! service lets the first user be added, and sign them in, so the admin's CLI works as reefadm
//! leaves.
//!
//! - **Asked before the build.** The password is read with the rest of what a command is told,
//!   so the build runs unattended; whether it is needed is asked of the running server first
//!   ([`users`]), and a server with users already asks for none. It is held to the service's own
//!   rule as it is read ([`password`]), so one the service would refuse stops reefadm then, not
//!   once the build is done and the new release is up.
//! - **Asked again before it is added.** A user may have been added meanwhile, so [`add`] asks
//!   once more and adds nobody to a server that has one, saying so.
//! - **The release's own CLI.** The user is added and signed in by the `reef` the release holds,
//!   naming the config where there is one, as the admin, never root.

use crate::cli::{DeployArgs, UpgradeArgs};
use crate::exec::{As, Cmd, Runner};
use crate::layout::{Layout, Mode};
use crate::secret::Secret;
use crate::served::Served;
use crate::{Cx, exec, health};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The first admin a command adds: their name, and the password they sign in with
pub struct Admin {
    /// Their name
    pub name: String,
    /// Their password
    pub password: String,
}

impl std::fmt::Debug for Admin {
    /// Shows the admin with their password left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admin").field("name", &self.name).finish()
    }
}

/// What a command was told of the first admin
#[derive(Debug, Clone, Copy, Default)]
pub struct Told<'a> {
    /// `--first-admin`: their name, else the admin running reefadm
    pub name: Option<&'a str>,
    /// `--no-admin`: add none, leaving it for `reef user add` from the host
    pub none: bool,
    /// `--password-file`: where their password is, else it is asked on the terminal
    pub password_file: Option<&'a Path>,
}

impl<'a> From<&'a DeployArgs> for Told<'a> {
    /// What deploy was told of the first admin
    ///
    /// # Arguments
    ///
    /// * `args` - What deploy was told
    fn from(args: &'a DeployArgs) -> Self {
        Told {
            name: args.first_admin.as_deref(),
            none: args.no_admin,
            password_file: args.password_file.as_deref(),
        }
    }
}

impl<'a> From<&'a UpgradeArgs> for Told<'a> {
    /// What upgrade was told of the first admin
    ///
    /// # Arguments
    ///
    /// * `args` - What upgrade was told
    fn from(args: &'a UpgradeArgs) -> Self {
        Told {
            name: args.first_admin.as_deref(),
            none: args.no_admin,
            password_file: args.password_file.as_deref(),
        }
    }
}

/// The `reef` a deployment's current release holds, as the admin runs it on this host
pub struct LocalCli {
    /// The release's `bin/reef`
    program: PathBuf,
    /// The config it is named, if the deployment has one: a team server always does, and a
    /// user's only if a flag or the user wrote one
    conf: Option<PathBuf>,
}

impl LocalCli {
    /// The CLI of a deployment's current release
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner, to ask whether a user's config is there
    /// * `layout` - The deployment's paths
    pub fn of(runner: &mut dyn Runner, layout: &Layout) -> Result<LocalCli> {
        // a config the CLI is named must exist, and a user's often doesn't
        let conf = match layout.mode {
            Mode::System => true,
            Mode::User => exec::exists(runner, &layout.config, As::Me)?,
        };
        Ok(LocalCli {
            program: layout.current.join("bin/reef"),
            conf: conf.then(|| layout.config.clone()),
        })
    }

    /// One command of it, naming the config if there is one
    ///
    /// # Arguments
    ///
    /// * `args` - The command line after `reef [--conf <config>]`
    pub fn cmd(&self, args: &[&str]) -> Cmd {
        let cmd = Cmd::new(self.program.display().to_string());
        let cmd = match &self.conf {
            Some(conf) => cmd.arg("--conf").arg(conf),
            None => cmd,
        };
        cmd.args(args)
    }
}

/// Asks a running server whether it has a user yet, on its loopback port, or `None`, said why,
/// if it couldn't be asked
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config, which says where it listens
pub fn users(runner: &mut dyn Runner, layout: &Layout, served: &Served) -> Option<bool> {
    // reef's CA as this host's CLI trusts it, then the route that answers anyone
    let asked = health::trusted(runner, layout, served)
        .and_then(|pem| runner.has_users(&health::users_ask(served, pem)));
    match asked {
        Ok(has) => Some(has),
        Err(error) => {
            runner.say(&format!(
                "couldn't ask reef whether it has users: {error:#}"
            ));
            None
        }
    }
}

/// The first admin a command is to add, asking their password now, or none: `--no-admin`, or a
/// server that has a user already
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `told` - What the command was told of the first admin
/// * `users` - Whether the server has a user, if that is known; unknown is asked for anyway
/// * `fallback` - Whom to add if `--first-admin` names nobody, such as the admin running reefadm
pub fn wanted(
    cx: &mut Cx,
    told: &Told,
    users: Option<bool>,
    fallback: Option<String>,
) -> Result<Option<Admin>> {
    // none asked for, or none needed
    if told.none {
        return Ok(None);
    }
    if users == Some(true) {
        return Ok(None);
    }
    // the admin named, else the one running it
    let Some(name) = told.name.map(String::from).or(fallback) else {
        bail!("reefadm can't tell who you are: name the first admin with --first-admin");
    };
    let prompt = format!("{name}'s password, as reef's first admin");
    let password = password(cx.runner, &prompt, told.password_file)?;
    Ok(Some(Admin { name, password }))
}

/// Reads the first admin's password, refusing one the service wouldn't keep before anything else
/// is done
///
/// # Arguments
///
/// * `runner` - The runner, which asks on the terminal or reads the file
/// * `prompt` - What to ask on the terminal
/// * `file` - The file to read it from instead
pub fn password(runner: &mut dyn Runner, prompt: &str, file: Option<&Path>) -> Result<String> {
    let password = runner.password(prompt, file)?;
    // the service's own rule, held to now; a dry run asks nothing, so it has nothing to hold
    if !runner.dry()
        && let Err(why) = reef_api::user::check_password(&password)
    {
        bail!("reef wouldn't keep that password for its first admin: {why}");
    }
    Ok(password)
}

/// Adds the first admin and signs them in from this host, unless the server has a user by now
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `served` - reef's config, which says where it listens
/// * `admin` - Whom to add, and their password
pub fn add(cx: &mut Cx, layout: &Layout, served: &Served, admin: &Admin) -> Result<()> {
    // asked again, since a user may have been added since the password was
    match users(cx.runner, layout, served) {
        Some(true) => {
            cx.runner.say(&format!(
                "reef has a user already, so {} isn't added: `reef login` as one of its users",
                admin.name
            ));
            return Ok(());
        }
        Some(false) => {}
        None => bail!(
            "reefadm couldn't tell whether reef has users, so it added no first admin: `reef \
             user add {}` from this host",
            admin.name
        ),
    }
    // the password, handed over as a file only the admin can read
    let cli = LocalCli::of(cx.runner, layout)?;
    let secret = Secret::write(&cx.env, &admin.password, cx.runner.dry())?;
    let file = secret.path().display().to_string();
    // added from this host, then signed in, as the CLI keeps its token
    cx.runner
        .run(&cli.cmd(&["user", "add", &admin.name, "--password-file", &file]))?;
    cx.runner
        .run(&cli.cmd(&["login", "--user", &admin.name, "--password-file", &file]))?;
    cx.runner.say(&format!(
        "added {} as reef's first admin, signed in from this host",
        admin.name
    ));
    Ok(())
}

/// Adds the first admin to a server with no user, asking whom and their password now, unless
/// told to add none
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `layout` - The deployment's paths
/// * `served` - reef's config, which says where it listens
/// * `told` - What the command was told of the first admin
pub fn ensure(cx: &mut Cx, layout: &Layout, served: &Served, told: &Told) -> Result<()> {
    // none asked for, so the server isn't asked either
    if told.none {
        return Ok(());
    }
    // whether one is needed, then whom, then added
    let has = users(cx.runner, layout, served);
    let fallback = cx.user();
    let Some(admin) = wanted(cx, told, has, fallback)? else {
        return Ok(());
    };
    add(cx, layout, served, &admin)
}
