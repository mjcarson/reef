//! reef's config: where shoal is, or how to run it in this process
//!
//! The config is a TOML file, read from `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`,
//! else `~/.config/reef/config.toml`. A default path with no file there means every default; a
//! path named by `$REEF_CONFIG` or by a caller must exist. `reef_api::config` finds it, so the
//! CLI, which reads `[api] address` alone, finds the same file.
//!
//! ```toml
//! [shoal]
//! # an external node, served by `reef-shoal`; unset starts shoal in this process
//! address = "127.0.0.1:12100"
//! # the in-process pool's data, absolute and off tmpfs
//! data = "/home/me/.local/share/reef/shoal"
//! # how many shards the in-process pool runs, each on a core of its own
//! cores = 2
//! # each shard's memory budget
//! memory = "256MiB"
//!
//! [api]
//! # where the JSON API listens: a loopback address until reef serves over TLS (C51)
//! address = "127.0.0.1:12180"
//!
//! [auth]
//! # whether every caller must sign in; off, reef answers anyone who reaches its port
//! required = true
//!
//! [remotes]
//! # where the clones of projects registered by their git URLs are kept, absolute
//! clones = "/home/me/.local/share/reef/clones"
//! # how often every one of them is fetched, in seconds, minutes or hours; `0s` never
//! fetch_every = "5m"
//! ```

use reef_api::config::env_dir;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The environment variable naming reef's config file, which `reef-api` shares with clients
pub use reef_api::config::CONFIG_VAR;

/// reef's config
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Conf {
    /// How reef reaches shoal
    pub shoal: ShoalSettings,
    /// Where the JSON API is served
    pub api: ApiSettings,
    /// Whether callers sign in
    pub auth: AuthSettings,
    /// Where projects registered by their git URLs are cloned, and how often they are fetched
    pub remotes: RemoteSettings,
    /// The file this was read from, if one was
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

/// How reef reaches shoal: an external node, or a pool it starts itself
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShoalSettings {
    /// An external node's address; unset starts a pool in this process
    pub address: Option<String>,
    /// The in-process pool's data directory, or the default under `$XDG_DATA_HOME`
    pub data: Option<PathBuf>,
    /// How many shards the in-process pool runs
    pub cores: usize,
    /// Each in-process shard's memory budget, such as `256MiB`
    pub memory: String,
}

impl Default for ShoalSettings {
    /// In-process, with two shards of 256 MiB each, its data in the default directory
    fn default() -> Self {
        ShoalSettings {
            address: None,
            data: None,
            cores: 2,
            memory: "256MiB".to_string(),
        }
    }
}

/// Where the JSON API is served
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiSettings {
    /// The address it listens on, which must be a loopback one until reef serves over TLS (C51)
    pub address: SocketAddr,
}

/// Whether callers sign in
///
/// Off, which is the default, reef answers anyone who reaches its port, as a single user's
/// server on localhost does. On, every route but the ones that sign a user in asks who is
/// calling.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthSettings {
    /// Whether every caller must sign in
    pub required: bool,
}

/// Where projects registered by their git URLs are cloned, and how often they are fetched
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteSettings {
    /// The directory every clone is kept in, or the default under `$XDG_DATA_HOME`
    pub clones: Option<PathBuf>,
    /// How often every remote project is fetched, such as `5m`, or `0s` for never
    pub fetch_every: String,
}

impl Default for RemoteSettings {
    /// Clones in the default directory, each fetched every five minutes
    fn default() -> Self {
        RemoteSettings {
            clones: None,
            fetch_every: "5m".to_string(),
        }
    }
}

impl Default for ApiSettings {
    /// Localhost, at the port reef's clients look for it on
    fn default() -> Self {
        ApiSettings {
            address: reef_api::DEFAULT_ADDRESS
                .parse()
                .expect("reef-api's default address is an address"),
        }
    }
}

impl Conf {
    /// Loads reef's config
    ///
    /// # Arguments
    ///
    /// * `path` - The file to read, or `None` for `$REEF_CONFIG` or the default path
    pub fn load(path: Option<&Path>) -> anyhow::Result<Conf> {
        // the file named, which must exist, else the default path's, if a file is there
        let Some(path) = reef_api::config::locate(path)? else {
            return Ok(Conf::default());
        };
        // read and parse it, saying which file was wrong
        let text = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))?;
        let mut conf: Conf = toml::from_str(&text)
            .map_err(|error| anyhow::anyhow!("parsing {}: {error}", path.display()))?;
        conf.source = Some(path);
        // an interval that can't be read is said now, rather than once the service is up
        conf.fetch_every()?;
        Ok(conf)
    }

    /// Where reef's config is read from when nothing names it
    ///
    /// `$XDG_CONFIG_HOME/reef/config.toml`, else `~/.config/reef/config.toml`, or `None` with
    /// neither variable set.
    pub fn default_path() -> Option<PathBuf> {
        // the one place reef's clients look too
        reef_api::config::default_path()
    }

    /// The in-process pool's data directory: the one configured, else the default
    ///
    /// The default is `$XDG_DATA_HOME/reef/shoal`, else `~/.local/share/reef/shoal`.
    pub fn data_dir(&self) -> anyhow::Result<PathBuf> {
        // a configured directory wins
        if let Some(dir) = &self.shoal.data {
            return Ok(dir.clone());
        }
        // else reef's directory in XDG's data home
        Ok(self.data_home("shoal.data")?.join("shoal"))
    }

    /// The directory remote projects' clones are kept in: the one configured, else the default
    ///
    /// The default sits beside shoal's: `$XDG_DATA_HOME/reef/clones`, else
    /// `~/.local/share/reef/clones`. A clone's path is kept with its project, so one configured
    /// must be absolute.
    pub fn clones_dir(&self) -> anyhow::Result<PathBuf> {
        // a configured directory wins, if it names one place whatever the service's cwd
        if let Some(dir) = &self.remotes.clones {
            if !dir.is_absolute() {
                anyhow::bail!(
                    "{} is {}, which is not absolute: name the directory from the root",
                    self.key("remotes.clones"),
                    dir.display()
                );
            }
            return Ok(dir.clone());
        }
        // else reef's directory in XDG's data home
        Ok(self.data_home("remotes.clones")?.join("clones"))
    }

    /// How often every remote project is fetched, or `None` if never
    pub fn fetch_every(&self) -> anyhow::Result<Option<Duration>> {
        // a number and its unit, refused with the key to change
        let every = parse_every(&self.remotes.fetch_every).ok_or_else(|| {
            anyhow::anyhow!(
                "{} is `{}`, which is not an interval: write a whole number of seconds, minutes \
                 or hours, such as `30s`, `5m` or `1h`, or `0s` for never",
                self.key("remotes.fetch_every"),
                self.remotes.fetch_every
            )
        })?;
        // nothing between fetches is never fetching
        Ok(Some(every).filter(|every| !every.is_zero()))
    }

    /// reef's own directory in XDG's data home, else in its default under the home directory
    ///
    /// # Arguments
    ///
    /// * `key` - The key that would name a directory instead, for the message if there is none
    fn data_home(&self, key: &str) -> anyhow::Result<PathBuf> {
        // XDG's data home, else its default under the home directory
        let base = env_dir("XDG_DATA_HOME")
            .or_else(|| env_dir("HOME").map(|home| home.join(".local").join("share")))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no data directory for reef: set {}, or `XDG_DATA_HOME` or `HOME`",
                    self.key(key)
                )
            })?;
        Ok(base.join("reef"))
    }

    /// Names a key and the file it is set in, for a message that says what to change
    ///
    /// # Arguments
    ///
    /// * `key` - The key's dotted name, such as `shoal.data`
    pub fn key(&self, key: &str) -> String {
        // the file this was read from, else the one a new config would be read from
        match self.source.clone().or_else(Self::default_path) {
            Some(file) => format!("`{key}` in {}", file.display()),
            None => format!("`{key}` in reef's config (`${CONFIG_VAR}`)"),
        }
    }
}

/// Reads an interval written as a whole number and a unit: `s`, `m` or `h`
///
/// A bare `0` is no interval at all. Returns `None` for anything else.
///
/// # Arguments
///
/// * `text` - The interval as written, such as `5m`
fn parse_every(text: &str) -> Option<Duration> {
    // nothing at all needs no unit
    let text = text.trim();
    if text == "0" {
        return Some(Duration::ZERO);
    }
    // the digits, then the unit they count
    let digits = text.find(|c: char| !c.is_ascii_digit())?;
    let (count, unit) = text.split_at(digits);
    let count: u64 = count.parse().ok()?;
    let seconds = match unit {
        "s" => count,
        "m" => count.checked_mul(60)?,
        "h" => count.checked_mul(60 * 60)?,
        _ => return None,
    };
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::{Conf, parse_every};
    use std::time::Duration;

    /// A config sets each key it names and leaves the rest at their defaults
    #[test]
    fn keys_override_defaults() {
        // a config naming two keys
        let conf: Conf = toml::from_str("[shoal]\naddress = \"127.0.0.1:12100\"\ncores = 4\n")
            .expect("the config parses");
        // keeps both, and every default it didn't name
        assert_eq!(conf.shoal.address.as_deref(), Some("127.0.0.1:12100"));
        assert_eq!(conf.shoal.cores, 4);
        assert_eq!(conf.shoal.memory, "256MiB");
        assert_eq!(conf.shoal.data, None);
        // the API listens on localhost unless told otherwise
        assert_eq!(conf.api.address.to_string(), reef_api::DEFAULT_ADDRESS);
    }

    /// Sign-in is off unless the config says it is required, and a misspelt key is refused
    #[test]
    fn auth_is_off_unless_required() {
        // nothing said is off
        let conf: Conf = toml::from_str("").expect("an empty config parses");
        assert!(!conf.auth.required);
        // and said, on
        let conf: Conf = toml::from_str("[auth]\nrequired = true\n").expect("it parses");
        assert!(conf.auth.required);
        // a key the service doesn't know would leave sign-in off unnoticed, so it is refused
        let parsed = toml::from_str::<Conf>("[auth]\nrequried = true\n");
        assert!(parsed.is_err(), "a misspelt key was read");
    }

    /// The API's address is read as an address, and one that isn't is refused
    #[test]
    fn api_address_parses() {
        // an IPv6 loopback address and port
        let conf: Conf = toml::from_str("[api]\naddress = \"[::1]:9000\"\n").expect("it parses");
        assert!(conf.api.address.ip().is_loopback());
        assert_eq!(conf.api.address.port(), 9000);
        // a host name is not an address
        let parsed = toml::from_str::<Conf>("[api]\naddress = \"localhost:9000\"\n");
        assert!(parsed.is_err(), "a host name was read as an address");
    }

    /// A misspelled key is refused rather than ignored
    #[test]
    fn unknown_keys_refused() {
        // `adress` is not a key
        let parsed = toml::from_str::<Conf>("[shoal]\nadress = \"127.0.0.1:12100\"\n");
        assert!(parsed.is_err(), "a misspelled key was accepted");
    }

    /// Remote projects are fetched every five minutes unless the config says otherwise
    #[test]
    fn fetch_every_reads_its_unit() {
        // the default
        let conf: Conf = toml::from_str("").expect("an empty config parses");
        assert_eq!(conf.fetch_every().unwrap(), Some(Duration::from_secs(300)));
        // each unit, and zero for never
        assert_eq!(parse_every("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_every("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_every("1h"), Some(Duration::from_secs(3600)));
        let never: Conf = toml::from_str("[remotes]\nfetch_every = \"0s\"\n").expect("it parses");
        assert_eq!(never.fetch_every().unwrap(), None);
        assert_eq!(parse_every("0"), Some(Duration::ZERO));
        // no unit, an unknown one, a fraction or a sign is refused, naming the key
        for bad in ["5", "5d", "1.5m", "-1s", "m", ""] {
            assert_eq!(parse_every(bad), None, "`{bad}` was read");
        }
        let bad: Conf = toml::from_str("[remotes]\nfetch_every = \"5\"\n").expect("it parses");
        let error = bad
            .fetch_every()
            .expect_err("an interval with no unit was read");
        assert!(error.to_string().contains("remotes.fetch_every"), "{error}");
    }

    /// A configured clones directory is used as it is, and must be absolute
    #[test]
    fn clones_dir_must_be_absolute() {
        // an absolute directory wins over the default
        let conf: Conf =
            toml::from_str("[remotes]\nclones = \"/srv/reef/clones\"\n").expect("it parses");
        assert_eq!(
            conf.clones_dir().unwrap(),
            std::path::Path::new("/srv/reef/clones")
        );
        // a relative one would move with the service's working directory, so it is refused
        let conf: Conf = toml::from_str("[remotes]\nclones = \"clones\"\n").expect("it parses");
        let error = conf
            .clones_dir()
            .expect_err("a relative directory was used");
        assert!(error.to_string().contains("remotes.clones"), "{error}");
    }

    /// With nothing configured, clones sit beside shoal's data, under reef's data directory
    #[test]
    fn clones_default_beside_shoals_data() {
        // both defaults, whichever variable this test runs with
        let conf = Conf::default();
        let (Ok(data), Ok(clones)) = (conf.data_dir(), conf.clones_dir()) else {
            return;
        };
        assert_eq!(data.parent(), clones.parent());
        assert!(clones.ends_with("reef/clones"), "{}", clones.display());
    }
}
