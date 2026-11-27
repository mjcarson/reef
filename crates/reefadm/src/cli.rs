//! reefadm's command line

use crate::sources;
use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

/// Deploys, upgrades and runs central reef on a Linux host with systemd
#[derive(Debug, Parser)]
#[command(name = "reefadm", version, about)]
pub struct Cli {
    /// Show every step without taking any: probes run, changes are only shown
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// Answer yes to every question: installing packages and rustup, opening the firewall
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,
    /// What to do
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses a command line, returning clap's error rather than exiting, for tests
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments, the program's name first
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::Cli;
    ///
    /// let cli = Cli::from_args(["reefadm", "deploy", "--user"]).unwrap();
    /// assert!(!cli.dry_run);
    /// assert!(Cli::from_args(["reefadm", "deploy", "--user", "--port", "443"]).is_err());
    /// ```
    pub fn from_args<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Cli as Parser>::try_parse_from(args)
    }
}

/// reefadm's commands
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Deploy central reef on this host, from nothing to a server that answers
    ///
    /// A team server by default: reef-app as the system unit `reef`, under an account of its own,
    /// sign-in required, HTTPS for other hosts on --port, and its first admin added and signed
    /// in. `--user` deploys one person's user unit on loopback instead, as reef's own plan runs.
    /// It fetches and builds reef here, which takes a while the first time, and asks before
    /// installing packages or rustup.
    Deploy(DeployArgs),
    /// Move the reef reefadm deployed here to a later commit, or build it again
    ///
    /// It fetches reef at --ref (by default the ref it was deployed from), refuses a commit whose
    /// schema changed a table the data was written with, builds it, backs the data up, makes the
    /// new release current and restarts. If the new release doesn't come up, the old one goes
    /// back, with the backup restored.
    Upgrade(UpgradeArgs),
    /// Remove what deploy put here, keeping the data and config unless --purge
    Uninstall(UninstallArgs),
    /// Say how reef is doing: its release, its unit, each port, its data and its certificate
    ///
    /// It exits 1, naming each, when the unit isn't running or a port doesn't answer.
    Status(StatusArgs),
    /// Show reef's journal since boot, or follow it
    Logs(LogsArgs),
    /// Back reef's data up, stopping it while it is copied
    ///
    /// The tarball holds the store, the clones, reef's certificates and, on a team server, the
    /// `reef` account's ssh keys and git config, readable by root alone.
    Backup(BackupArgs),
    /// Put a backup back over reef's data, moving what is there aside
    Restore(RestoreArgs),
    /// Make a user an admin again with a new password, from the store itself
    ///
    /// For when no admin can sign in. It stops reef while it runs, since the store is locked.
    Recover(RecoverArgs),
}

/// What `reefadm status` is told
#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
}

/// What `reefadm logs` is told
#[derive(Debug, Clone, Args)]
pub struct LogsArgs {
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
    /// Keep showing what it says
    #[arg(long, short = 'f')]
    pub follow: bool,
    /// How many of its last lines to show
    #[arg(long, short = 'n', default_value_t = 100)]
    pub lines: u32,
}

/// What `reefadm backup` is told
#[derive(Debug, Clone, Args)]
pub struct BackupArgs {
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
    /// The directory to write it in [default: /var/backups/reef, or ~/.local/state/reef/backups]
    #[arg(long, value_name = "DIR")]
    pub to: Option<PathBuf>,
}

/// What `reefadm restore` is told
#[derive(Debug, Clone, Args)]
pub struct RestoreArgs {
    /// The backup to put back
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
}

/// What `reefadm recover` is told
#[derive(Debug, Clone, Args)]
pub struct RecoverArgs {
    /// The user to make an admin again, or to add as one if no user has the name
    #[arg(value_name = "NAME")]
    pub name: String,
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
    /// Read the new password from a file, rather than asking twice
    #[arg(long, value_name = "PATH")]
    pub password_file: Option<PathBuf>,
}

/// What `reefadm upgrade` is told
#[derive(Debug, Clone, Args)]
pub struct UpgradeArgs {
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
    /// The branch, tag or commit of reef to upgrade to [default: the one deployed from]
    #[arg(long = "ref", value_name = "REF")]
    pub reference: Option<String>,
    /// Where reef is fetched from [default: where it was deployed from]
    #[arg(long, value_name = "URL")]
    pub repo: Option<String>,
    /// Fetch shoal or glommio from another URL than central/sources.toml names, as NAME=URL
    #[arg(long = "source", value_name = "NAME=URL")]
    pub sources: Vec<String>,
    /// Where the checkouts are kept and built [default: ~/.local/share/reefadm/src]
    #[arg(long, value_name = "DIR")]
    pub src: Option<PathBuf>,
    /// Build and install a new release even when the commits are the running one's
    #[arg(long)]
    pub rebuild: bool,
    /// Take no backup of the data before the new release starts
    #[arg(long)]
    pub no_backup: bool,
    /// Start on a fresh data directory, the old one moved aside, for a schema that can't read it
    #[arg(long)]
    pub fresh: bool,
    /// Finish an upgrade to a staged release, as the new release's reefadm
    #[arg(long, hide = true, value_name = "RELEASE")]
    pub finish: Option<String>,
}

/// What `reefadm uninstall` is told
#[derive(Debug, Clone, Args)]
pub struct UninstallArgs {
    /// The admin's user unit, rather than the team server
    #[arg(long)]
    pub user: bool,
    /// Remove the data, the clones, the certificates, the config, the account and the checkouts
    /// too, keeping only the backups
    #[arg(long)]
    pub purge: bool,
}

/// What `reefadm deploy` is told
#[derive(Debug, Clone, Args)]
pub struct DeployArgs {
    /// One person's server: a user unit on loopback, with no sign-in
    #[arg(long, conflicts_with_all = ["names", "port", "cert", "key", "first_admin", "no_admin", "password_file"])]
    pub user: bool,
    /// Serve the JSON API alone, with reef-serve, for teams that use only the CLI and Claude Code
    #[arg(long)]
    pub api_only: bool,
    /// A name other hosts call this one by, which reef's own certificate covers; repeat for more
    /// [default: this host's name]
    #[arg(long = "name", value_name = "NAME")]
    pub names: Vec<String>,
    /// The port other hosts reach, over HTTPS [default: 12443]
    #[arg(long)]
    pub port: Option<u16>,
    /// A certificate chain of your own for that port, as PEM, the server's certificate first
    #[arg(long, requires = "key", value_name = "PATH")]
    pub cert: Option<PathBuf>,
    /// The private key of --cert, as PEM
    #[arg(long, requires = "cert", value_name = "PATH")]
    pub key: Option<PathBuf>,
    /// How many shards the store runs, each wanting 30 MiB of locked memory [default: 2]
    #[arg(long)]
    pub cores: Option<u16>,
    /// Each shard's memory budget, such as 256MiB [default: 256MiB]
    #[arg(long, value_name = "SIZE")]
    pub memory: Option<String>,
    /// The first admin's name [default: you]
    #[arg(long, value_name = "NAME")]
    pub first_admin: Option<String>,
    /// Add no first admin, leaving it to `reef user add` from this host
    #[arg(long, conflicts_with_all = ["first_admin", "password_file"])]
    pub no_admin: bool,
    /// Read the first admin's password from a file, rather than asking twice
    #[arg(long, value_name = "PATH")]
    pub password_file: Option<PathBuf>,
    /// Where reef is fetched from
    #[arg(long, value_name = "URL", default_value = sources::REPO)]
    pub repo: String,
    /// The branch, tag or commit of reef to deploy
    #[arg(long = "ref", value_name = "REF", default_value = sources::REF)]
    pub reference: String,
    /// Fetch shoal or glommio from another URL than central/sources.toml names, as NAME=URL
    #[arg(long = "source", value_name = "NAME=URL")]
    pub sources: Vec<String>,
    /// Where the checkouts are kept and built [default: ~/.local/share/reefadm/src]
    #[arg(long, value_name = "DIR")]
    pub src: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};

    /// deploy's flags read as given, and a user unit takes none of a team server's
    #[test]
    fn deploys_flags_read_as_given() {
        let cli = Cli::from_args([
            "reefadm",
            "--dry-run",
            "deploy",
            "--name",
            "reef.example.com",
            "--name",
            "10.0.0.5",
            "--port",
            "443",
            "--first-admin",
            "ada",
            "-y",
        ])
        .unwrap();
        assert!(cli.dry_run && cli.yes);
        let Command::Deploy(args) = cli.command else {
            panic!("deploy reads as deploy");
        };
        assert_eq!(args.names, ["reef.example.com", "10.0.0.5"]);
        assert_eq!(args.port, Some(443));
        assert_eq!(args.reference, "CentralizedPlanner");
        // a user unit has no TLS port, no first admin and no certificate of its own
        for flag in [
            &["--port", "443"][..],
            &["--first-admin", "ada"],
            &["--name", "x"],
        ] {
            let mut line = vec!["reefadm", "deploy", "--user"];
            line.extend(flag);
            assert!(Cli::from_args(line).is_err(), "{flag:?}");
        }
        // a certificate needs its key, and no admin takes no admin's name
        assert!(Cli::from_args(["reefadm", "deploy", "--cert", "c.pem"]).is_err());
        assert!(
            Cli::from_args(["reefadm", "deploy", "--no-admin", "--first-admin", "ada"]).is_err()
        );
    }
}
