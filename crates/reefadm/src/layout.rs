//! Where a deployment's files go, for a team server and for one person's user unit
//!
//! | | team | user |
//! |---|---|---|
//! | releases | `/opt/reef/releases/<name>` | `~/.local/share/reef/releases/<name>` |
//! | current | `/opt/reef/current` | `~/.local/share/reef/current` |
//! | commands | `/usr/local/bin` | `~/.cargo/bin` |
//! | unit | `/etc/systemd/system/reef.service` | `~/.config/systemd/user/reef.service` |
//! | config | `/etc/reef/config.toml` | `~/.config/reef/config.toml`, only if a flag needs one |
//! | data home | `/var/lib/reef` | `~/.local/share/reef` |
//! | backups | `/var/backups/reef` | `~/.local/state/reef/backups` |
//! | state | `/etc/reef/reefadm.toml` | `~/.local/share/reef/reefadm.toml` |
//! | sources | `~/.local/share/reefadm/src` | the same |
//!
//! The `XDG_*` variables move the user's paths as they move reef's own defaults
//! ([`reef_api::config`]), and the user unit is given the same ones, since systemd's user
//! manager may not have them.

use crate::Env;
use crate::exec::As;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The unit reef runs as, for a team and a user alike
pub const UNIT: &str = "reef.service";

/// Which kind of deployment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A team server: a system unit under the `reef` account, with sign-in and a TLS port
    System,
    /// One person's server: a user unit on loopback, with no sign-in
    User,
}

/// Which server it runs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// reef-app: the web app and the JSON API on one port
    App,
    /// reef-serve: the JSON API alone
    Serve,
}

impl Kind {
    /// The server's program, under a release
    pub fn program(self) -> &'static str {
        match self {
            Kind::App => "app/server",
            Kind::Serve => "bin/reef-serve",
        }
    }
}

/// Every path a deployment uses
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Which kind of deployment these are the paths of
    pub mode: Mode,
    /// The admin's own home, which the sources live under
    pub home: PathBuf,
    /// Where each release is kept, by name
    pub releases: PathBuf,
    /// The link to the release in use
    pub current: PathBuf,
    /// Where `reef`, `reef-serve` and `reefadm` are linked to the current release's
    pub bin: PathBuf,
    /// The unit file
    pub unit: PathBuf,
    /// reef's config file
    pub config: PathBuf,
    /// A copy of reef's own CA that every account on the host can read, for a team server
    pub ca_copy: Option<PathBuf>,
    /// reef's data home: its store, its clones and its certificates, by default
    pub data_home: PathBuf,
    /// Where backups go
    pub backups: PathBuf,
    /// reefadm's record of what it deployed
    pub state: PathBuf,
    /// The checkouts reef is built from, side by side
    pub src: PathBuf,
    /// The variables the user unit is given, so reef finds what the admin's shell does
    pub unit_env: Vec<(String, String)>,
}

impl Layout {
    /// The paths of one kind of deployment, for the admin whose environment this is
    ///
    /// # Arguments
    ///
    /// * `mode` - Which kind of deployment
    /// * `env` - The admin's environment
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::Env;
    /// use reefadm::layout::{Layout, Mode};
    ///
    /// let env = Env::from_pairs([("HOME", "/home/ada")]);
    /// let layout = Layout::new(Mode::System, &env).unwrap();
    /// assert_eq!(layout.unit, std::path::Path::new("/etc/systemd/system/reef.service"));
    /// assert_eq!(layout.src, std::path::Path::new("/home/ada/.local/share/reefadm/src"));
    /// ```
    pub fn new(mode: Mode, env: &Env) -> Result<Self> {
        // the admin's home holds the sources whichever kind it is
        let Some(home) = env.dir("HOME") else {
            bail!("reefadm needs $HOME, where it keeps the checkouts it builds from");
        };
        let data_root = env
            .dir("XDG_DATA_HOME")
            .unwrap_or_else(|| home.join(".local/share"));
        let src = data_root.join("reefadm/src");
        let layout = match mode {
            // a team server's files are the system's
            Mode::System => Layout {
                mode,
                home,
                releases: "/opt/reef/releases".into(),
                current: "/opt/reef/current".into(),
                bin: "/usr/local/bin".into(),
                unit: Path::new("/etc/systemd/system").join(UNIT),
                config: "/etc/reef/config.toml".into(),
                ca_copy: Some("/etc/reef/reef-ca.pem".into()),
                data_home: "/var/lib/reef".into(),
                backups: "/var/backups/reef".into(),
                state: "/etc/reef/reefadm.toml".into(),
                src,
                unit_env: Vec::new(),
            },
            // one person's are under their home, where reef looks by default
            Mode::User => {
                let config_root = env
                    .dir("XDG_CONFIG_HOME")
                    .unwrap_or_else(|| home.join(".config"));
                let state_root = env
                    .dir("XDG_STATE_HOME")
                    .unwrap_or_else(|| home.join(".local/state"));
                let cargo = env.dir("CARGO_HOME").unwrap_or_else(|| home.join(".cargo"));
                let data_home = data_root.join("reef");
                // the variables that moved reef's defaults, for the unit to see as well
                let unit_env = ["XDG_DATA_HOME", "XDG_CONFIG_HOME"]
                    .into_iter()
                    .filter_map(|key| env.dir(key).map(|dir| (key.to_string(), dir)))
                    .map(|(key, dir)| (key, dir.display().to_string()))
                    .collect();
                Layout {
                    mode,
                    releases: data_home.join("releases"),
                    current: data_home.join("current"),
                    bin: cargo.join("bin"),
                    unit: config_root.join("systemd/user").join(UNIT),
                    config: config_root.join("reef/config.toml"),
                    ca_copy: None,
                    backups: state_root.join("reef/backups"),
                    state: data_home.join("reefadm.toml"),
                    data_home,
                    home,
                    src,
                    unit_env,
                }
            }
        };
        Ok(layout)
    }

    /// Who writes and changes this deployment's files: root for a team server, else the admin
    pub fn owner(&self) -> As {
        match self.mode {
            Mode::System => As::Root,
            Mode::User => As::Me,
        }
    }

    /// Who may read the service's data: its account for a team server, else the admin
    pub fn reader(&self) -> As {
        match self.mode {
            Mode::System => As::Root,
            Mode::User => As::Me,
        }
    }

    /// The store's data directory, when the config names none
    pub fn data(&self) -> PathBuf {
        self.data_home.join("shoal")
    }

    /// The clones' directory, when the config names none
    pub fn clones(&self) -> PathBuf {
        self.data_home.join("clones")
    }

    /// The certificates' directory, when the config names none
    pub fn certs(&self) -> PathBuf {
        self.data_home.join("certs")
    }
}

/// The PATH every command reefadm runs gets: cargo's bin first, where rustup puts cargo and dx
///
/// A fresh rustup, and a shell over ssh, leave it off PATH.
///
/// # Arguments
///
/// * `env` - reefadm's environment
///
/// # Examples
///
/// ```
/// use reefadm::Env;
/// use reefadm::layout::path_with_cargo;
///
/// let env = Env::from_pairs([("HOME", "/home/ada"), ("PATH", "/usr/bin")]);
/// assert_eq!(path_with_cargo(&env), "/home/ada/.cargo/bin:/usr/bin");
/// ```
pub fn path_with_cargo(env: &Env) -> String {
    // cargo's home, else the default under the admin's home
    let cargo = env
        .dir("CARGO_HOME")
        .or_else(|| env.dir("HOME").map(|home| home.join(".cargo")))
        .map(|dir| dir.join("bin").display().to_string());
    let path = env.get("PATH").unwrap_or("/usr/local/bin:/usr/bin:/bin");
    match cargo {
        // once, at the front
        Some(bin) if !path.split(':').any(|entry| entry == bin) => format!("{bin}:{path}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, Mode};
    use crate::Env;
    use std::path::Path;

    /// A user's paths are under their home, moved as the XDG variables move them
    #[test]
    fn a_users_paths_follow_the_xdg_variables() {
        // the defaults, under the home
        let env = Env::from_pairs([("HOME", "/home/ada")]);
        let layout = Layout::new(Mode::User, &env).unwrap();
        assert_eq!(
            layout.data(),
            Path::new("/home/ada/.local/share/reef/shoal")
        );
        assert_eq!(
            layout.unit,
            Path::new("/home/ada/.config/systemd/user/reef.service")
        );
        assert_eq!(layout.bin, Path::new("/home/ada/.cargo/bin"));
        assert_eq!(
            layout.backups,
            Path::new("/home/ada/.local/state/reef/backups")
        );
        assert!(layout.unit_env.is_empty());
        // moved, and handed on to the unit
        let env = Env::from_pairs([
            ("HOME", "/home/ada"),
            ("XDG_DATA_HOME", "/data"),
            ("XDG_CONFIG_HOME", "relative"),
        ]);
        let layout = Layout::new(Mode::User, &env).unwrap();
        assert_eq!(layout.certs(), Path::new("/data/reef/certs"));
        assert_eq!(layout.src, Path::new("/data/reefadm/src"));
        // a relative variable is ignored, as reef ignores it
        assert_eq!(
            layout.config,
            Path::new("/home/ada/.config/reef/config.toml")
        );
        assert_eq!(
            layout.unit_env,
            [("XDG_DATA_HOME".to_string(), "/data".to_string())]
        );
    }

    /// A team server's paths are the system's, but for the sources, which are the admin's
    #[test]
    fn a_team_servers_paths_are_the_systems() {
        let env = Env::from_pairs([("HOME", "/home/ada"), ("XDG_DATA_HOME", "/data")]);
        let layout = Layout::new(Mode::System, &env).unwrap();
        assert_eq!(layout.data(), Path::new("/var/lib/reef/shoal"));
        assert_eq!(layout.current, Path::new("/opt/reef/current"));
        assert_eq!(
            layout.ca_copy.as_deref(),
            Some(Path::new("/etc/reef/reef-ca.pem"))
        );
        assert_eq!(layout.src, Path::new("/data/reefadm/src"));
        // and with no home, there is nowhere to build
        assert!(Layout::new(Mode::System, &Env::default()).is_err());
    }
}
