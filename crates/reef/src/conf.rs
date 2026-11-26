//! The part of reef's config the CLI reads: where the JSON API is served
//!
//! The CLI reads the service's own config file, found by `reef_api::config`, so the two meet at
//! the address one file names. It reads `[api]`'s `address`, `url`, `ca` and `certs` alone and
//! ignores every other key, since the service is what checks the rest of the file. On the
//! service's own host it calls `address` over HTTPS, trusting the CA the service made and keeps
//! in `certs`, else in `$XDG_DATA_HOME/reef/certs`; on another host, `url` names the service, and
//! `ca` a root its certificate is trusted by beside the system's. The token it signs in with
//! is kept apart, in a credentials file of the user's own ([`credentials`](crate::credentials)),
//! which `--credentials` can name.
//!
//! ```toml
//! [api]
//! # where the JSON API listens on this host, over HTTPS, and so where the CLI calls it
//! address = "127.0.0.1:12180"
//! # where the service keeps the CA it made, which the CLI on its host trusts
//! certs = "/home/me/.local/share/reef/certs"
//! # or, on another host, the service's root, which wins over `address`
//! url = "https://reef.example:12443"
//! # a PEM root to trust the service's certificate by, beside the system's own
//! ca = "/home/me/.config/reef/ca.pem"
//! ```

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The part of reef's config the CLI reads
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Conf {
    /// Where the JSON API is served
    pub api: ApiSettings,
    /// The file this was read from, if one was
    #[serde(skip)]
    pub source: Option<PathBuf>,
    /// The credentials file `--credentials` named, if it named one, over the environment's and
    /// the default
    #[serde(skip)]
    pub credentials: Option<PathBuf>,
}

/// Where the JSON API is served
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    /// The address the service listens on over HTTPS, on this host
    pub address: SocketAddr,
    /// Where the service keeps the CA it made, else the default under `$XDG_DATA_HOME`
    pub certs: Option<PathBuf>,
    /// The service's root, such as `https://reef.example:12443`, which wins over `address`
    pub url: Option<String>,
    /// A PEM root the service's certificate is trusted by, beside the system's own
    pub ca: Option<PathBuf>,
}

impl Default for ApiSettings {
    /// Localhost, at the port the service listens on unless its config names another, trusting
    /// the CA it keeps in the default directory
    fn default() -> Self {
        ApiSettings {
            address: reef_api::DEFAULT_ADDRESS
                .parse()
                .expect("reef-api's default address is an address"),
            certs: None,
            url: None,
            ca: None,
        }
    }
}

impl Conf {
    /// Loads the CLI's part of reef's config
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
        let mut conf = Conf::parse(&text)
            .map_err(|error| anyhow::anyhow!("parsing {}: {error}", path.display()))?;
        conf.source = Some(path);
        Ok(conf)
    }

    /// Parses a config's text, ignoring every key but the CLI's
    ///
    /// # Arguments
    ///
    /// * `text` - The config file's TOML
    pub fn parse(text: &str) -> Result<Conf, toml::de::Error> {
        toml::from_str(text)
    }

    /// Where the service on this host keeps the CA it made: the directory `[api] certs` names,
    /// else the default, `$XDG_DATA_HOME/reef/certs`
    pub fn certs_dir(&self) -> Option<PathBuf> {
        // a configured directory wins
        self.api
            .certs
            .clone()
            .or_else(reef_api::config::default_certs)
    }

    /// The CA the service on this host made, if it is there to trust
    pub fn own_ca(&self) -> Option<PathBuf> {
        // its certificate, if the service has made one, which a CLI on another host never finds
        self.certs_dir()
            .map(|dir| dir.join("ca.pem"))
            .filter(|ca| ca.is_file())
    }

    /// Names a key and the file it is set in, for a message that says what to change
    ///
    /// # Arguments
    ///
    /// * `key` - The key's dotted name, such as `api.address`
    pub fn key(&self, key: &str) -> String {
        // the file this was read from, else the one a new config would be read from
        match self.source.clone().or_else(reef_api::config::default_path) {
            Some(file) => format!("`{key}` in {}", file.display()),
            None => format!(
                "`{key}` in reef's config (`${}`)",
                reef_api::config::CONFIG_VAR
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Conf;

    /// The API's address is read from `[api]`, and the service's own keys are left to it
    #[test]
    fn reads_the_api_address_and_ignores_the_rest() {
        // a whole config, as the service reads it
        let text = "[shoal]\naddress = \"127.0.0.1:12100\"\ncores = 4\n\n[api]\naddress = \"[::1]:9000\"\n";
        let conf = Conf::parse(text).expect("the config parses");
        // the CLI keeps the API's address alone
        assert_eq!(conf.api.address.to_string(), "[::1]:9000");
        assert_eq!(conf.source, None);
    }

    /// A service on another host is named by its root, and a root to trust its certificate by
    #[test]
    fn reads_the_url_and_ca() {
        // the service's keys beside the CLI's, as one file holds them on the service's host
        let text = "[api]\naddress = \"127.0.0.1:12180\"\nurl = \"https://reef.example:12443\"\n\
                    ca = \"/etc/reef/ca.pem\"\n\n[api.tls]\naddress = \"0.0.0.0:12443\"\n";
        let conf = Conf::parse(text).expect("the config parses");
        assert_eq!(conf.api.url.as_deref(), Some("https://reef.example:12443"));
        assert_eq!(conf.api.ca, Some("/etc/reef/ca.pem".into()));
        // and neither said is the address alone
        let conf = Conf::parse("").expect("an empty config parses");
        assert_eq!((conf.api.url, conf.api.ca), (None, None));
    }

    /// A config that says nothing of the API leaves it at the service's default
    #[test]
    fn api_address_defaults_to_the_services() {
        // no `[api]` at all, or an empty file
        for text in ["[shoal]\ncores = 2\n", ""] {
            let conf = Conf::parse(text).expect("the config parses");
            assert_eq!(conf.api.address.to_string(), reef_api::DEFAULT_ADDRESS);
        }
    }

    /// An address that isn't one is refused, as the service refuses it
    #[test]
    fn host_name_is_not_an_address() {
        // the service binds an address, never a name
        assert!(Conf::parse("[api]\naddress = \"localhost:9000\"\n").is_err());
    }

    /// A file that was named is read, and names itself in messages
    #[test]
    fn named_file_is_read_and_named() {
        // a config in a directory of this test's own
        let dir = std::env::temp_dir().join(format!("reef-conf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("reef.toml");
        std::fs::write(&file, "[api]\naddress = \"127.0.0.1:9001\"\n").unwrap();
        // its address is read, and a key names the file
        let conf = Conf::load(Some(&file)).expect("the named config loads");
        assert_eq!(conf.api.address.port(), 9001);
        assert!(conf.key("api.address").contains("reef.toml"));
        // and a named file that isn't there is refused
        assert!(Conf::load(Some(&dir.join("missing.toml"))).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
