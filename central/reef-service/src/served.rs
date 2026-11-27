//! The `reef` central reef hands its users: its release's own, found once as the service opens
//!
//! reefadm ships a static, stamped `reef` in every release's `bin/`, beside the server it runs:
//! `releases/<name>/bin/reef-serve`, or reef-app's `releases/<name>/app/server`. Two directories
//! up from either is the release, so the service finds the client there unless `[client] reef`
//! names one. It asks the binary `version --json` once, with a timeout, and serves it only if it
//! is stamped with its commit and that commit's time, since a client can only order a stamped
//! build against its own. A release never changes once it is made, so the build said at start
//! stays true for the life of the process.
//!
//! Anything else serves no client, and says why ([`NoClient`]), which the server prints as it
//! starts: a client only stops hearing of new builds.

use anyhow::Context;
use reef_api::Build;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tracing::instrument;

use crate::conf::Conf;

/// How long the client has to say its build before it is taken for one that can't
const ASK_WAIT: Duration = Duration::from_secs(10);

/// The `reef` the service hands its users: its build, where it is, and how big
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The build it says it is, which is stamped
    pub build: Build,
    /// Where it is on the service's host
    pub path: PathBuf,
    /// How many bytes it is, as it was found
    pub size: u64,
}

/// Why the service hands its users no `reef`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoClient(pub String);

impl std::fmt::Display for NoClient {
    /// Says why
    ///
    /// # Arguments
    ///
    /// * `f` - Where to say it
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Served {
    /// The client a config names, else the release's own, if it is a stamped `reef`
    ///
    /// # Arguments
    ///
    /// * `conf` - reef's config, whose `[client] reef` names the client, if it does
    #[instrument(name = "Served::find", skip_all)]
    pub async fn find(conf: &Conf) -> Result<Served, NoClient> {
        // the binary named, else the one beside the release this process runs from
        let (path, named) = match &conf.client.reef {
            Some(path) => (path.clone(), true),
            None => match beside_release() {
                Some(path) => (path, false),
                None => {
                    return Err(NoClient(
                        "this server's own path names no release, and `[client] reef` names no \
                         client"
                            .to_string(),
                    ));
                }
            },
        };
        // a release with no client of its own is only said; one named that is wrong is a warning
        match Served::read(&path).await {
            Ok(served) => {
                tracing::info!(
                    path = %path.display(),
                    commit = ?served.build.commit,
                    "serving reef's client"
                );
                Ok(served)
            }
            Err(_) if !named && !path.exists() => {
                tracing::info!(path = %path.display(), "no client of reef's to serve");
                Err(NoClient(format!(
                    "no `reef` is at {}, beside this server, and `[client] reef` names none",
                    path.display()
                )))
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %format!("{error:#}"),
                    "serving no client of reef's"
                );
                Err(NoClient(format!("{error:#}")))
            }
        }
    }

    /// Reads a binary as the client to serve: asks it its build, which must be stamped
    ///
    /// # Arguments
    ///
    /// * `path` - The binary
    #[instrument(name = "Served::read", skip_all, err(Debug))]
    pub async fn read(path: &Path) -> anyhow::Result<Served> {
        // a file, whose size is what the route sends
        let meta = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("reading the client at {}", path.display()))?;
        anyhow::ensure!(
            meta.is_file(),
            "the client at {} is not a file",
            path.display()
        );
        // its build, as it says it, within a few seconds, and never waited on past them
        let asked = tokio::process::Command::new(path)
            .args(["version", "--json"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(ASK_WAIT, asked)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "the client at {} didn't say its build within {} s",
                    path.display(),
                    ASK_WAIT.as_secs()
                )
            })?
            .with_context(|| format!("running the client at {}", path.display()))?;
        anyhow::ensure!(
            output.status.success(),
            "the client at {} answered `version --json` with {}, so it is older than reef's \
             updates or not reef at all",
            path.display(),
            output.status
        );
        let build: Build = serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "the client at {} said a build that doesn't read as one",
                path.display()
            )
        })?;
        // and only a stamped build, which a client can order against its own
        anyhow::ensure!(
            build.is_stamped(),
            "the client at {} is not stamped with its commit, so no client could tell whether \
             it is newer: serve a release's own, which reefadm stamps",
            path.display()
        );
        Ok(Served {
            build,
            path: path.to_path_buf(),
            size: meta.len(),
        })
    }
}

/// The `reef` in the release this process runs from: two directories up from this binary,
/// `bin/reef-serve` or `app/server`, then `bin/reef`
fn beside_release() -> Option<PathBuf> {
    // the binary itself, its links resolved, as `/proc/self/exe` names it
    let exe = std::env::current_exe().ok()?;
    let release = exe.parent()?.parent()?;
    Some(release.join("bin/reef"))
}

#[cfg(test)]
mod tests {
    use super::Served;
    use crate::conf::Conf;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// Writes a script that answers `version --json` with some JSON, as a client would
    ///
    /// # Arguments
    ///
    /// * `dir` - Where to write it
    /// * `name` - Its name
    /// * `json` - What it prints
    fn script(dir: &Path, name: &str, json: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho '{json}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A stamped client is served, and an unstamped one, a missing one or one that can't say its
    /// build is refused, saying why
    #[tokio::test]
    async fn only_a_stamped_client_is_served() {
        // a directory of this test's own
        let dir = std::env::temp_dir().join(format!("reef-served-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut conf = Conf::default();
        // stamped: its build, path and size
        let stamped = script(
            &dir,
            "stamped",
            r#"{"version":"0.1.0","commit":"dc5e68c","committed":1795800000,"target":"x86_64-linux-musl"}"#,
        );
        conf.client.reef = Some(stamped.clone());
        let served = Served::find(&conf).await.unwrap();
        assert_eq!(served.build.commit.as_deref(), Some("dc5e68c"));
        assert_eq!(served.path, stamped);
        assert_eq!(served.size, std::fs::metadata(&stamped).unwrap().len());
        // unstamped, as `cargo install` builds it
        let unstamped = script(
            &dir,
            "unstamped",
            r#"{"version":"0.1.0","commit":null,"committed":null,"target":"x86_64-linux-gnu"}"#,
        );
        conf.client.reef = Some(unstamped);
        let refused = Served::find(&conf).await.unwrap_err();
        assert!(refused.0.contains("not stamped"), "{refused}");
        // a reef too old to know `version`, or not a reef at all
        let old = dir.join("old");
        std::fs::write(&old, "#!/bin/sh\nexit 2\n").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        conf.client.reef = Some(old);
        let refused = Served::find(&conf).await.unwrap_err();
        assert!(refused.0.contains("version --json"), "{refused}");
        // one named that isn't there
        conf.client.reef = Some(dir.join("missing"));
        let refused = Served::find(&conf).await.unwrap_err();
        assert!(refused.0.contains("missing"), "{refused}");
        // and none named, beside a test binary that is in no release
        conf.client.reef = None;
        let refused = Served::find(&conf).await.unwrap_err();
        assert!(
            refused.0.contains("`[client] reef` names none"),
            "{refused}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
