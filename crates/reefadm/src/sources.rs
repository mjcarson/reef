//! The checkouts reef is built from: reef at a ref, then shoal and glommio beside it
//!
//! reef's `central/sources.toml` names the sibling checkouts its central workspace builds
//! against, each at a pinned `rev`, and the toolchains it builds with. They are cloned side by
//! side under the sources directory, so central's `../../shoal` and `../../glommio` resolve.
//! reefadm owns these checkouts: each fetch puts the checkout at its rev with `--force`, over
//! anything a build left changed in it.

use crate::exec::{Cmd, Runner};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The repo reef is fetched from, unless deploy is told another
pub const REPO: &str = "https://github.com/mjcarson/reef.git";

/// The ref reef is fetched at, unless deploy is told another: the branch central lives on until
/// it reaches main
pub const REF: &str = "CentralizedPlanner";

/// The manifest built into this reefadm, which a dry run shows when it has no checkout to read
const BUILT_IN: &str = include_str!("../../../central/sources.toml");

/// `central/sources.toml`: the toolchains and sibling checkouts a reef commit builds with
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The nightly the server builds on, such as `nightly-2026-08-21`
    pub toolchain: String,
    /// The oldest stable the CLI and reefadm build on, such as `1.98`
    pub stable_at_least: String,
    /// shoal's checkout
    pub shoal: Sibling,
    /// glommio's checkout
    pub glommio: Sibling,
    /// Where git fetches a URL a sibling names from instead, by the URL named
    #[serde(default)]
    pub rewrite: BTreeMap<String, String>,
}

/// One sibling checkout
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sibling {
    /// Where it is cloned from
    pub url: String,
    /// The branch its rev is on
    pub branch: String,
    /// The commit reef builds against
    pub rev: String,
    /// Whether its submodules are needed too
    #[serde(default)]
    pub submodules: bool,
}

/// The revs a release was built from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revs {
    /// reef's commit
    pub reef: String,
    /// shoal's commit
    pub shoal: String,
    /// glommio's commit
    pub glommio: String,
}

impl Manifest {
    /// Reads a manifest's text
    ///
    /// # Arguments
    ///
    /// * `text` - The file's text
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::sources::Manifest;
    ///
    /// let text = std::fs::read_to_string("../../central/sources.toml").unwrap();
    /// let manifest = Manifest::read(&text).unwrap();
    /// assert!(manifest.toolchain.starts_with("nightly-"));
    /// assert!(manifest.glommio.submodules);
    /// ```
    pub fn read(text: &str) -> Result<Self> {
        toml::from_str(text).context("reading central/sources.toml")
    }

    /// The manifest built into this reefadm
    pub fn built_in() -> Self {
        // it is read by this crate's own tests, so it always parses
        Manifest::read(BUILT_IN).expect("central/sources.toml parses")
    }
}

/// Where each sibling's URL is overridden, from `--source NAME=URL`
///
/// # Arguments
///
/// * `sources` - Each `NAME=URL` given
///
/// # Examples
///
/// ```
/// use reefadm::sources::overrides;
///
/// let given = vec!["shoal=/mirror/shoal.git".to_string()];
/// assert_eq!(overrides(&given).unwrap()["shoal"], "/mirror/shoal.git");
/// assert!(overrides(&["openraft=x".to_string()]).is_err());
/// ```
pub fn overrides(sources: &[String]) -> Result<BTreeMap<String, String>> {
    let mut overrides = BTreeMap::new();
    for source in sources {
        // only the siblings the manifest names
        let Some((name, url)) = source.split_once('=') else {
            bail!("--source takes NAME=URL, not `{source}`");
        };
        if !matches!(name, "shoal" | "glommio") {
            bail!("--source names `shoal` or `glommio`, not `{name}`");
        }
        overrides.insert(name.to_string(), url.to_string());
    }
    Ok(overrides)
}

/// `git` in a checkout, fetching each URL a rewrite names from where it says instead
///
/// # Arguments
///
/// * `dir` - The checkout
/// * `rewrite` - Where to fetch each URL named from instead
fn git(dir: &Path, rewrite: &BTreeMap<String, String>) -> Cmd {
    with_rewrites(Cmd::new("git"), rewrite).arg("-C").arg(dir)
}

/// git's `-c url.<to>.insteadOf=<from>` for each rewrite, which reach its submodules' fetches too
///
/// # Arguments
///
/// * `cmd` - The git command
/// * `rewrite` - Where to fetch each URL named from instead
fn with_rewrites(cmd: Cmd, rewrite: &BTreeMap<String, String>) -> Cmd {
    rewrite.iter().fold(cmd, |cmd, (from, to)| {
        cmd.arg("-c").arg(format!("url.{to}.insteadOf={from}"))
    })
}

/// Clones or fetches a repo, and puts its checkout at a ref or rev, returning the commit
///
/// # Arguments
///
/// * `runner` - The runner
/// * `dir` - The checkout
/// * `url` - Where it is fetched from
/// * `fetch` - What to fetch: a branch, a tag or a commit
/// * `at` - What to check out once fetched: the commit fetched, or a rev on it
/// * `rewrite` - Where to fetch each URL named from instead
fn checkout(
    runner: &mut dyn Runner,
    dir: &Path,
    url: &str,
    fetch: &str,
    at: &str,
    rewrite: &BTreeMap<String, String>,
) -> Result<String> {
    // a checkout reefadm made already is fetched again, from the URL it is now told
    let made = runner
        .probe(&git(dir, rewrite).args(["rev-parse", "--git-dir"]))?
        .ok();
    if made {
        runner.run(&git(dir, rewrite).args(["remote", "set-url", "origin", url]))?;
    } else {
        runner.run(
            &with_rewrites(Cmd::new("git"), rewrite)
                .args(["clone", "--quiet", "--no-checkout", url])
                .arg(dir),
        )?;
    }
    runner.run(&git(dir, rewrite).args(["fetch", "--quiet", "origin", fetch]))?;
    // a pinned commit the branch no longer holds, as when its history was rewritten, is fetched
    // by itself, which a host like GitHub serves while it keeps the commit
    if at != "FETCH_HEAD" && !runner.dry() {
        let held = runner.probe(
            &git(dir, rewrite)
                .args(["cat-file", "-e"])
                .arg(format!("{at}^{{commit}}")),
        )?;
        if !held.ok() {
            runner.say(&format!(
                "{fetch} at {url} no longer holds {at}, so it is fetched by itself: its history \
                 was rewritten, or the pin is behind it"
            ));
            runner.run(&git(dir, rewrite).args(["fetch", "--quiet", "origin", at]))?;
        }
    }
    runner.run(&git(dir, rewrite).args(["checkout", "--quiet", "--force", "--detach", at]))?;
    // the commit it is at, which a dry run can't know before it is fetched
    let head = runner.probe(&git(dir, rewrite).args(["rev-parse", "HEAD"]))?;
    match (head.ok(), runner.dry()) {
        (true, _) => Ok(head.stdout.trim().to_string()),
        (false, true) if at == "FETCH_HEAD" => Ok(fetch.to_string()),
        (false, true) => Ok(at.to_string()),
        (false, false) => bail!(
            "{} has no commit checked out: {}",
            dir.display(),
            head.stderr.trim()
        ),
    }
}

/// Fetches reef at a ref into the sources, returning its commit and the manifest it holds
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory
/// * `repo` - Where reef is fetched from
/// * `reference` - The branch, tag or commit to fetch
pub fn fetch_reef(
    runner: &mut dyn Runner,
    src: &Path,
    repo: &str,
    reference: &str,
) -> Result<(String, Manifest)> {
    // the sources directory holds every checkout
    let dir = src.join("reef");
    if !crate::exec::exists(runner, src, crate::exec::As::Me)? {
        runner.run(&Cmd::new("mkdir").arg("-p").arg(src))?;
    }
    let rev = checkout(
        runner,
        &dir,
        repo,
        reference,
        "FETCH_HEAD",
        &BTreeMap::new(),
    )?;
    // the manifest of the commit fetched, or in a dry run before any fetch, this reefadm's own
    let path = dir.join("central/sources.toml");
    let manifest = match crate::exec::read(runner, &path, crate::exec::As::Me)? {
        Some(text) => Manifest::read(&text).with_context(|| format!("in reef's commit {rev}"))?,
        None if runner.dry() => {
            runner.say("(a dry run reads central/sources.toml from this reefadm, not the commit)");
            Manifest::built_in()
        }
        None => bail!(
            "reef's commit {rev} has no central/sources.toml, so it predates reefadm: deploy a \
             later one with --ref"
        ),
    };
    Ok((rev, manifest))
}

/// When a reef commit was made, in seconds since the epoch, as its checkout says, which the CLI
/// is stamped with, or `None` in a dry run, whose checkout may not be fetched yet
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory
/// * `rev` - reef's commit, as [`fetch_reef`] returned it
pub fn committed(runner: &mut dyn Runner, src: &Path, rev: &str) -> Result<Option<i64>> {
    // the committer's time, which orders one release's CLI against another's
    let shown = runner.probe(
        &git(&src.join("reef"), &BTreeMap::new())
            .args(["show", "-s", "--format=%ct"])
            .arg(rev),
    )?;
    match (shown.stdout.trim().parse::<i64>(), runner.dry()) {
        (Ok(seconds), _) if shown.ok() => Ok(Some(seconds)),
        (_, true) => Ok(None),
        _ => bail!(
            "reefadm couldn't read when reef's commit {rev} was made, which its CLI is stamped \
             with: {}",
            shown.stderr.trim()
        ),
    }
}

/// Fetches shoal and glommio at their revs beside reef, returning every commit
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory
/// * `reef` - reef's commit
/// * `manifest` - The manifest that pins them
/// * `overrides` - Where `--source` says to fetch a sibling from instead
pub fn fetch_siblings(
    runner: &mut dyn Runner,
    src: &Path,
    reef: &str,
    manifest: &Manifest,
    overrides: &BTreeMap<String, String>,
) -> Result<Revs> {
    let mut revs = BTreeMap::new();
    for (name, sibling) in [("shoal", &manifest.shoal), ("glommio", &manifest.glommio)] {
        // its URL, unless told another, at its branch, then its pinned rev
        let dir = src.join(name);
        let url = overrides.get(name).unwrap_or(&sibling.url);
        let rev = checkout(
            runner,
            &dir,
            url,
            &sibling.branch,
            &sibling.rev,
            &manifest.rewrite,
        )
        .with_context(|| format!("fetching {name} at {}", sibling.rev))?;
        // and its submodules, glommio's liburing among them, from where the manifest says
        if sibling.submodules {
            runner.run(&git(&dir, &manifest.rewrite).args([
                "submodule",
                "update",
                "--quiet",
                "--init",
                "--recursive",
                "--force",
            ]))?;
        }
        revs.insert(name, rev);
    }
    Ok(Revs {
        reef: reef.to_string(),
        shoal: revs.remove("shoal").unwrap_or_default(),
        glommio: revs.remove("glommio").unwrap_or_default(),
    })
}

/// Where glommio's liburing is checked out, which its build compiles instead of fetching it
///
/// # Arguments
///
/// * `src` - The sources directory
pub fn liburing(src: &Path) -> PathBuf {
    src.join("glommio/glommio/liburing")
}

#[cfg(test)]
mod tests {
    use super::{Manifest, fetch_siblings};
    use crate::exec::{Runner, Script};
    use std::collections::BTreeMap;
    use std::path::Path;

    /// The manifest checked in pins each sibling at a full commit, and a nightly by its date
    #[test]
    fn the_manifest_pins_each_sibling() {
        let manifest = Manifest::built_in();
        for sibling in [&manifest.shoal, &manifest.glommio] {
            assert_eq!(sibling.rev.len(), 40, "{sibling:?}");
            assert!(sibling.url.starts_with("https://"), "{sibling:?}");
        }
        assert!(manifest.toolchain.starts_with("nightly-20"));
        // and a key it doesn't know is refused rather than ignored
        assert!(Manifest::read("toolchain = \"x\"\nnightly = 1\n").is_err());
    }

    /// A sibling is cloned the first time, fetched after, and put at its pinned rev
    #[test]
    fn siblings_are_cloned_then_fetched_at_their_rev() {
        let manifest = Manifest::built_in();
        let mut script = Script::default();
        // shoal is there already, its branch holding its pin; glommio isn't, and its branch was
        // rewritten past its pin
        let git = "git -c url.https://github.com/axboe/liburing.git.insteadOf=git://git.kernel.dk/liburing";
        script.prints(format!("{git} -C /src/shoal rev-parse --git-dir"), ".git\n");
        script.prints(
            format!(
                "{git} -C /src/shoal cat-file -e '{}^{{commit}}'",
                manifest.shoal.rev
            ),
            "",
        );
        script.prints(format!("{git} -C /src/shoal rev-parse HEAD"), "aaa\n");
        script.prints(format!("{git} -C /src/glommio rev-parse HEAD"), "bbb\n");
        let revs = fetch_siblings(
            &mut script,
            Path::new("/src"),
            "ccc",
            &manifest,
            &BTreeMap::from([("shoal".to_string(), "/mirror/shoal.git".to_string())]),
        )
        .unwrap();
        assert_eq!(
            (
                revs.reef.as_str(),
                revs.shoal.as_str(),
                revs.glommio.as_str()
            ),
            ("ccc", "aaa", "bbb")
        );
        let steps = script.steps();
        let git = format!("run: {git}");
        assert_eq!(
            steps,
            [
                format!("{git} -C /src/shoal remote set-url origin /mirror/shoal.git"),
                format!("{git} -C /src/shoal fetch --quiet origin ScanSupport"),
                format!(
                    "{git} -C /src/shoal checkout --quiet --force --detach {}",
                    manifest.shoal.rev
                ),
                format!(
                    "{git} clone --quiet --no-checkout {} /src/glommio",
                    manifest.glommio.url
                ),
                format!("{git} -C /src/glommio fetch --quiet origin ZeroCopyDmaStreamWriter"),
                format!(
                    "{git} -C /src/glommio fetch --quiet origin {}",
                    manifest.glommio.rev
                ),
                format!(
                    "{git} -C /src/glommio checkout --quiet --force --detach {}",
                    manifest.glommio.rev
                ),
                format!(
                    "{git} -C /src/glommio submodule update --quiet --init --recursive --force"
                ),
            ]
        );
        assert!(!script.dry());
        // and it says why the pin was fetched by itself
        assert!(
            script
                .said
                .iter()
                .any(|line| line.starts_with("ZeroCopyDmaStreamWriter at ")
                    && line.contains("no longer holds")),
            "{:?}",
            script.said
        );
    }
}
