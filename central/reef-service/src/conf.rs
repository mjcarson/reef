//! reef's config: where shoal is, or how to run it in this process
//!
//! The config is a TOML file, read from `$REEF_CONFIG`, else `$XDG_CONFIG_HOME/reef/config.toml`,
//! else `~/.config/reef/config.toml`. A default path with no file there means every default; a
//! path named by `$REEF_CONFIG` or by a caller must exist.
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
//! ```

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The environment variable naming reef's config file
pub const CONFIG_VAR: &str = "REEF_CONFIG";

/// reef's config
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Conf {
    /// How reef reaches shoal
    pub shoal: ShoalSettings,
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

impl Conf {
    /// Loads reef's config
    ///
    /// # Arguments
    ///
    /// * `path` - The file to read, or `None` for `$REEF_CONFIG` or the default path
    pub fn load(path: Option<&Path>) -> anyhow::Result<Conf> {
        // a path the caller or the environment named must exist
        let named = path
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os(CONFIG_VAR).map(PathBuf::from));
        let path = match named {
            Some(path) => {
                anyhow::ensure!(
                    path.is_file(),
                    "reef's config {} is not a file",
                    path.display()
                );
                path
            }
            // the default path may hold nothing, which means every default
            None => match Self::default_path() {
                Some(path) if path.is_file() => path,
                _ => return Ok(Conf::default()),
            },
        };
        // read and parse it, saying which file was wrong
        let text = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))?;
        let mut conf: Conf = toml::from_str(&text)
            .map_err(|error| anyhow::anyhow!("parsing {}: {error}", path.display()))?;
        conf.source = Some(path);
        Ok(conf)
    }

    /// Where reef's config is read from when nothing names it
    ///
    /// `$XDG_CONFIG_HOME/reef/config.toml`, else `~/.config/reef/config.toml`, or `None` with
    /// neither variable set.
    pub fn default_path() -> Option<PathBuf> {
        // XDG's config home, else its default under the home directory
        let base = env_dir("XDG_CONFIG_HOME")
            .or_else(|| env_dir("HOME").map(|home| home.join(".config")))?;
        Some(base.join("reef").join("config.toml"))
    }

    /// The in-process pool's data directory: the one configured, else the default
    ///
    /// The default is `$XDG_DATA_HOME/reef/shoal`, else `~/.local/share/reef/shoal`.
    pub fn data_dir(&self) -> anyhow::Result<PathBuf> {
        // a configured directory wins
        if let Some(dir) = &self.shoal.data {
            return Ok(dir.clone());
        }
        // else XDG's data home, else its default under the home directory
        let base = env_dir("XDG_DATA_HOME")
            .or_else(|| env_dir("HOME").map(|home| home.join(".local").join("share")))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no data directory for shoal: set {}, or `XDG_DATA_HOME` or `HOME`",
                    self.key("shoal.data")
                )
            })?;
        Ok(base.join("reef").join("shoal"))
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

/// An environment variable holding an absolute directory, as XDG asks for
///
/// # Arguments
///
/// * `var` - The variable's name
fn env_dir(var: &str) -> Option<PathBuf> {
    // XDG says to ignore a variable that is empty or relative
    let dir = PathBuf::from(std::env::var_os(var)?);
    dir.is_absolute().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::Conf;

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
    }

    /// A misspelled key is refused rather than ignored
    #[test]
    fn unknown_keys_refused() {
        // `adress` is not a key
        let parsed = toml::from_str::<Conf>("[shoal]\nadress = \"127.0.0.1:12100\"\n");
        assert!(parsed.is_err(), "a misspelled key was accepted");
    }
}
