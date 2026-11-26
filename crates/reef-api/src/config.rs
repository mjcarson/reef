//! Where reef's config is: one file the service and its clients both read
//!
//! The config is a TOML file, read from a path the caller names, else `$REEF_CONFIG`, else
//! `$XDG_CONFIG_HOME/reef/config.toml`, else `~/.config/reef/config.toml`. A path named by the
//! caller or by `$REEF_CONFIG` must exist; the default path may hold nothing, which means every
//! default. The service reads all of it, and the CLI only `[api]`'s client keys, so the two find
//! each other at the address one file names, and the CLI the CA the service keeps in
//! [`default_certs`], unless `[api] certs` names another directory.

use std::path::{Path, PathBuf};

/// The environment variable naming reef's config file
pub const CONFIG_VAR: &str = "REEF_CONFIG";

/// A config file that was named but isn't there
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing(pub PathBuf);

impl std::fmt::Display for Missing {
    /// Names the file that isn't there
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "reef's config {} is not a file", self.0.display())
    }
}

impl std::error::Error for Missing {}

/// Finds the config file to read, or `None` when every key takes its default
///
/// # Arguments
///
/// * `named` - The file a caller named, such as with `--conf`, which wins over `$REEF_CONFIG`
///
/// # Examples
///
/// ```
/// use reef_api::config;
///
/// // a file that was named must be there
/// assert!(config::locate(Some("/no/such/reef.toml".as_ref())).is_err());
/// ```
pub fn locate(named: Option<&Path>) -> Result<Option<PathBuf>, Missing> {
    // a path the caller or the environment named must exist
    let named = named
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(CONFIG_VAR).map(PathBuf::from));
    if let Some(path) = named {
        return match path.is_file() {
            true => Ok(Some(path)),
            false => Err(Missing(path)),
        };
    }
    // the default path may hold nothing, which means every default
    Ok(default_path().filter(|path| path.is_file()))
}

/// Where reef's config is read from when nothing names it
///
/// `$XDG_CONFIG_HOME/reef/config.toml`, else `~/.config/reef/config.toml`, or `None` with
/// neither variable set.
pub fn default_path() -> Option<PathBuf> {
    // XDG's config home, else its default under the home directory
    let base =
        env_dir("XDG_CONFIG_HOME").or_else(|| env_dir("HOME").map(|home| home.join(".config")))?;
    Some(base.join("reef").join("config.toml"))
}

/// reef's own directory in XDG's data home, which the service keeps its data and certificates in
///
/// `$XDG_DATA_HOME/reef`, else `~/.local/share/reef`, or `None` with neither variable set.
pub fn data_home() -> Option<PathBuf> {
    // XDG's data home, else its default under the home directory
    let base = env_dir("XDG_DATA_HOME")
        .or_else(|| env_dir("HOME").map(|home| home.join(".local").join("share")))?;
    Some(base.join("reef"))
}

/// Where the service keeps the certificates it makes when nothing names a directory
///
/// `$XDG_DATA_HOME/reef/certs`, else `~/.local/share/reef/certs`, so a client on the service's
/// host, as the same user, finds the CA they are signed by.
pub fn default_certs() -> Option<PathBuf> {
    // beside the service's other data
    data_home().map(|home| home.join("certs"))
}

/// An environment variable holding an absolute directory, as XDG asks for
///
/// # Arguments
///
/// * `var` - The variable's name
pub fn env_dir(var: &str) -> Option<PathBuf> {
    // XDG says to ignore a variable that is empty or relative
    let dir = PathBuf::from(std::env::var_os(var)?);
    dir.is_absolute().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::{Missing, locate};
    use std::path::PathBuf;

    /// A named file is read if it is there, and refused, by name, if it isn't
    #[test]
    fn named_file_must_exist() {
        // this crate's manifest is a file
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert_eq!(locate(Some(&manifest)), Ok(Some(manifest.clone())));
        // a file beside it that isn't there is named in the refusal
        let missing = manifest.with_file_name("no-such-config.toml");
        let refused = locate(Some(&missing)).expect_err("a missing file was located");
        assert_eq!(refused, Missing(missing.clone()));
        assert!(refused.to_string().contains("no-such-config.toml"));
        // and a directory is not a file
        let dir = manifest.parent().unwrap().to_path_buf();
        assert_eq!(locate(Some(&dir)), Err(Missing(dir)));
    }
}
