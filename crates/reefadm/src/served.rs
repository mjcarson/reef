//! The keys of reef's config that reefadm reads and writes, with the service's defaults
//!
//! The service's own `Conf` (`central/reef-service/src/conf.rs`) refuses keys it doesn't know,
//! and lives in the central workspace, which reefadm never builds against. This reads the same
//! file leniently, keeping only what reefadm needs: where the data, the clones and the
//! certificates are, where the server listens, and whether shoal runs in process. It writes a
//! team server's config from deploy's flags, which a later deploy keeps as it is.

use crate::layout::Layout;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The loopback address the service listens on when the config names none
pub const LOOPBACK: &str = "127.0.0.1:12180";

/// The port a team server serves other hosts on, unless deploy is told another
pub const TLS_PORT: u16 = 12443;

/// The shards shoal runs, unless the config says otherwise
pub const CORES: u16 = 2;

/// The first line of a config reefadm writes
const HEADER: &str = "# Written by `reefadm deploy`, which keeps it as it is from here on: edit it as you\n\
                      # like, then `sudo systemctl restart reef`. The keys are in the book's Configuring page.\n";

/// reef's config, as far as reefadm reads it
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Served {
    /// `[shoal]`: where the store keeps its data, or the node it talks to
    #[serde(default)]
    pub shoal: ShoalKeys,
    /// `[api]`: where the service listens and keeps its certificates
    #[serde(default)]
    pub api: ApiKeys,
    /// `[auth]`: whether callers sign in
    #[serde(default)]
    pub auth: AuthKeys,
    /// `[remotes]`: where remote projects are cloned
    #[serde(default)]
    pub remotes: RemoteKeys,
}

/// `[shoal]`'s keys
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShoalKeys {
    /// An external node's address; none means shoal runs in the service's process
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The data directory
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<PathBuf>,
    /// How many shards
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<u16>,
    /// Each shard's memory budget, such as `256MiB`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// `[api]`'s keys
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeys {
    /// The loopback address
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Where reef keeps the CA and certificate it makes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certs: Option<PathBuf>,
    /// A readable copy of reef's CA, for the CLI on this host
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca: Option<PathBuf>,
    /// Further names reef's own certificate covers
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// `[api.tls]`: the port other hosts reach
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsKeys>,
}

/// `[api.tls]`'s keys
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsKeys {
    /// The address other hosts reach
    pub address: String,
    /// A certificate chain of the admin's own, as PEM
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<PathBuf>,
    /// Its private key, as PEM
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<PathBuf>,
}

/// `[auth]`'s keys
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthKeys {
    /// Whether every caller signs in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// `[remotes]`'s keys
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteKeys {
    /// Where remote projects are cloned
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clones: Option<PathBuf>,
    /// How often every remote project is fetched
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_every: Option<String>,
}

/// What a team server's config is made from: deploy's flags, with their defaults filled in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamFlags {
    /// The names other hosts call it by, which reef's own certificate covers
    pub names: Vec<String>,
    /// The port other hosts reach
    pub port: u16,
    /// Whether the admin's own certificate is copied in as `fullchain.pem` and `privkey.pem`
    pub own_cert: bool,
    /// How many shards
    pub cores: u16,
    /// Each shard's memory budget
    pub memory: String,
}

impl Served {
    /// Reads a config's text, leniently, keeping only what reefadm needs
    ///
    /// # Arguments
    ///
    /// * `text` - The config file's text
    /// * `path` - Where it was read from, for the error
    pub fn read(text: &str, path: &std::path::Path) -> Result<Self> {
        toml::from_str(text).with_context(|| format!("reading reef's config {}", path.display()))
    }

    /// A team server's config, as the book's Configuring page gives it
    ///
    /// # Arguments
    ///
    /// * `layout` - The team server's paths
    /// * `flags` - What deploy was told
    pub fn team(layout: &Layout, flags: &TeamFlags) -> Self {
        // the admin's certificate goes beside the config, where the account can read its key
        let (cert, key) = match flags.own_cert {
            true => (
                Some(PathBuf::from("/etc/reef/fullchain.pem")),
                Some(PathBuf::from("/etc/reef/privkey.pem")),
            ),
            false => (None, None),
        };
        Served {
            shoal: ShoalKeys {
                address: None,
                data: Some(layout.data()),
                cores: Some(flags.cores),
                memory: Some(flags.memory.clone()),
            },
            api: ApiKeys {
                address: Some(LOOPBACK.to_string()),
                certs: Some(layout.certs()),
                ca: layout.ca_copy.clone(),
                names: flags.names.clone(),
                tls: Some(TlsKeys {
                    address: format!("0.0.0.0:{}", flags.port),
                    cert,
                    key,
                }),
            },
            auth: AuthKeys {
                required: Some(true),
            },
            remotes: RemoteKeys {
                clones: Some(layout.clones()),
                fetch_every: Some("5m".to_string()),
            },
        }
    }

    /// The config's text, as reefadm writes it: a line saying so, then its keys
    pub fn text(&self) -> Result<String> {
        let keys = toml::to_string(self).context("writing reef's config")?;
        Ok(format!("{HEADER}\n{keys}"))
    }

    /// The store's data directory
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths, whose defaults apply
    pub fn data(&self, layout: &Layout) -> PathBuf {
        self.shoal.data.clone().unwrap_or_else(|| layout.data())
    }

    /// The clones' directory
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths, whose defaults apply
    pub fn clones(&self, layout: &Layout) -> PathBuf {
        self.remotes
            .clones
            .clone()
            .unwrap_or_else(|| layout.clones())
    }

    /// The certificates' directory
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths, whose defaults apply
    pub fn certs(&self, layout: &Layout) -> PathBuf {
        self.api.certs.clone().unwrap_or_else(|| layout.certs())
    }

    /// The loopback address, such as `127.0.0.1:12180`
    pub fn loopback(&self) -> String {
        self.api
            .address
            .clone()
            .unwrap_or_else(|| LOOPBACK.to_string())
    }

    /// The TLS port's address, if it serves one
    pub fn tls_address(&self) -> Option<String> {
        self.api.tls.as_ref().map(|tls| tls.address.clone())
    }

    /// The TLS port's number, if it serves one
    pub fn tls_port(&self) -> Option<u16> {
        self.tls_address().and_then(|address| port_of(&address))
    }

    /// How many shards shoal runs
    pub fn cores(&self) -> u16 {
        self.shoal.cores.unwrap_or(CORES)
    }

    /// What in deploy's flags disagrees with a config kept from before, a line each
    ///
    /// # Arguments
    ///
    /// * `given` - The flags deploy was given, as the config they would write
    /// * `explicit` - Which of them the admin typed, rather than took by default
    pub fn conflicts(&self, given: &Served, explicit: &Explicit) -> Vec<String> {
        let mut conflicts = Vec::new();
        // the port, each name, the shards and their memory, and a certificate of the admin's own
        if explicit.port && self.tls_address() != given.tls_address() {
            conflicts.push(format!(
                "--port says {}, but `[api.tls] address` is {}",
                given.tls_address().unwrap_or_default(),
                self.tls_address().unwrap_or_else(|| "not set".to_string())
            ));
        }
        for name in given.api.names.iter().filter(|_| explicit.names) {
            if !self.api.names.contains(name) {
                conflicts.push(format!("--name {name} isn't in `[api] names`"));
            }
        }
        if explicit.cores && self.cores() != given.cores() {
            conflicts.push(format!(
                "--cores says {}, but `[shoal] cores` is {}",
                given.cores(),
                self.cores()
            ));
        }
        if explicit.memory && self.shoal.memory != given.shoal.memory {
            conflicts.push(format!(
                "--memory says {}, but `[shoal] memory` is {}",
                given.shoal.memory.as_deref().unwrap_or_default(),
                self.shoal.memory.as_deref().unwrap_or("its default")
            ));
        }
        let cert = |served: &Served| served.api.tls.as_ref().and_then(|tls| tls.cert.clone());
        if explicit.cert && cert(self) != cert(given) {
            conflicts.push("--cert is given, but `[api.tls] cert` names another file".to_string());
        }
        conflicts
    }
}

/// Which of deploy's config flags the admin typed
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Explicit {
    /// `--port`
    pub port: bool,
    /// `--name`
    pub names: bool,
    /// `--cores`
    pub cores: bool,
    /// `--memory`
    pub memory: bool,
    /// `--cert` and `--key`
    pub cert: bool,
}

/// The port at the end of an address, such as `12180` from `127.0.0.1:12180` or `[::1]:12180`
///
/// # Arguments
///
/// * `address` - The address
///
/// # Examples
///
/// ```
/// use reefadm::served::port_of;
///
/// assert_eq!(port_of("0.0.0.0:12443"), Some(12443));
/// assert_eq!(port_of("[::1]:12180"), Some(12180));
/// assert_eq!(port_of("localhost"), None);
/// ```
pub fn port_of(address: &str) -> Option<u16> {
    address.rsplit_once(':')?.1.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{Explicit, Served, TeamFlags};
    use crate::Env;
    use crate::layout::{Layout, Mode};
    use std::collections::BTreeSet;
    use std::path::Path;

    /// A team server's layout
    fn team() -> Layout {
        Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap()
    }

    /// The flags of a plain deploy
    fn flags() -> TeamFlags {
        TeamFlags {
            names: vec!["reef.example.com".into()],
            port: 12443,
            own_cert: false,
            cores: 2,
            memory: "256MiB".into(),
        }
    }

    /// Each `[section] key` a config's text sets, as `section.key`
    ///
    /// # Arguments
    ///
    /// * `text` - The config's text
    fn keys(text: &str) -> BTreeSet<String> {
        let mut section = String::new();
        let mut keys = BTreeSet::new();
        for line in text.lines().map(str::trim) {
            // a section, then its keys
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                section = name.to_string();
            } else if let Some((key, _)) = line.split_once('=')
                && !line.starts_with('#')
            {
                keys.insert(format!("{section}.{}", key.trim()));
            }
        }
        keys
    }

    /// The config's block from the book's Configuring page
    fn documented() -> String {
        let page = include_str!("../../../docs/src/central/deploy/config.md");
        let start = page.find("```toml\n").unwrap() + "```toml\n".len();
        page[start..][..page[start..].find("```").unwrap()].to_string()
    }

    /// A team server's config sets every key the book's Configuring page does
    #[test]
    fn a_team_config_has_every_documented_key() {
        // with the admin's own certificate, since the page names both of its files
        let flags = TeamFlags {
            own_cert: true,
            ..flags()
        };
        let text = Served::team(&team(), &flags).text().unwrap();
        let written = keys(&text);
        // `names` and `hosts` are the page's to explain; reefadm sets names, and leaves hosts open
        let missing: Vec<_> = keys(&documented())
            .into_iter()
            .filter(|key| !written.contains(key) && key != "remotes.hosts")
            .collect();
        assert!(missing.is_empty(), "missing {missing:?} from:\n{text}");
        // and the values that make it a team server
        let served = Served::read(&text, Path::new("config.toml")).unwrap();
        assert_eq!(served.auth.required, Some(true));
        assert_eq!(served.tls_port(), Some(12443));
        assert_eq!(served.loopback(), "127.0.0.1:12180");
        assert_eq!(served.data(&team()), Path::new("/var/lib/reef/shoal"));
        assert_eq!(
            served.api.ca.as_deref(),
            Some(Path::new("/etc/reef/reef-ca.pem"))
        );
        assert!(text.starts_with("# Written by `reefadm deploy`"));
    }

    /// A config written by hand reads leniently, its paths falling back to the defaults
    #[test]
    fn a_config_written_by_hand_reads_leniently() {
        // keys reefadm doesn't read are left alone
        let text = "[api]\nurl = \"https://x\"\n[api.tls]\naddress = \"0.0.0.0:443\"\n\
                    [remotes]\nhosts = [\"github.com\"]\n";
        let served = Served::read(text, Path::new("c.toml")).unwrap();
        assert_eq!(served.tls_port(), Some(443));
        assert_eq!(served.clones(&team()), Path::new("/var/lib/reef/clones"));
        assert_eq!(served.cores(), 2);
        // and a file that isn't TOML says which it is
        let error = Served::read("[api", Path::new("/etc/reef/config.toml")).unwrap_err();
        assert!(format!("{error:#}").contains("/etc/reef/config.toml"));
    }

    /// A flag the admin typed that a kept config disagrees with is named; defaults never are
    #[test]
    fn flags_that_disagree_with_a_kept_config_are_named() {
        let kept = Served::team(&team(), &flags());
        let given = Served::team(
            &team(),
            &TeamFlags {
                port: 443,
                cores: 4,
                names: vec!["other.example.com".into()],
                ..flags()
            },
        );
        // only what was typed counts
        assert!(kept.conflicts(&given, &Explicit::default()).is_empty());
        let typed = Explicit {
            port: true,
            names: true,
            cores: true,
            ..Explicit::default()
        };
        let conflicts = kept.conflicts(&given, &typed);
        assert_eq!(conflicts.len(), 3, "{conflicts:?}");
        assert!(conflicts[0].contains("0.0.0.0:443"));
        assert!(conflicts[1].contains("other.example.com"));
        assert!(conflicts[2].contains("--cores says 4"));
    }
}
