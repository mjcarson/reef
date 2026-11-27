//! Which build this `reef` is, as `reef version` prints it, and replacing it with a newer one
//! central reef serves
//!
//! A release's `reef` is stamped when reefadm builds it: `REEF_COMMIT` names the commit it was
//! built from and `REEF_COMMITTED` that commit's time, in seconds since the epoch, both read at
//! compile time. rustc records the variables a crate reads, so cargo builds it again when either
//! changes. A build made without them, as `cargo install` or a checkout's `cargo build` makes, is
//! not stamped, and says so.
//!
//! ```text
//! $ reef version
//! reef 0.1.0 dc5e68c1a2b3 (2026-11-27)
//! $ reef version --json
//! {"version":"0.1.0","commit":"dc5e68c1a2b3…","committed":1795800000,"target":"x86_64-linux-musl"}
//! ```
//!
//! - **Noticing.** Central reef names the build it hands its users on every answer
//!   (`reef-client`), which [`Watch::notice`] reads. A stamped build that sees one made after its
//!   own commit, with `[client] update` on and a binary it may replace ([`may_replace`]), spawns
//!   `reef update --auto` once a process, detached, its log `update.log` in reef's state
//!   directory, and goes on with what it was asked.
//! - **Ordering.** Builds order by their commits' times ([`direction`]), newer only. An older or
//!   other build needs `reef update --force`, and an unstamped one, as `cargo install` builds,
//!   never replaces itself, but `reef update` typed by hand swaps it for the service's.
//! - **The updater** (`--auto`) is quiet and newer-only. It takes `update.lock`, so every other
//!   updater started meanwhile leaves at once, and keeps `update.toml`, so a commit whose install
//!   failed isn't tried again within the hour.
//! - **Installing** ([`install`]) streams the service's binary into a file beside the target,
//!   makes it executable and syncs it, runs it to check it says the commit wanted, and renames it
//!   over the target. Anything that fails removes the new file and leaves the old binary.
//! - **A reefadm release's own `reef`** is never replaced: `reefadm upgrade` moves it with the
//!   server.
//! - **Replaced while running.** A process whose binary was replaced finds its path again
//!   ([`own_exe`]), so a long-lived `reef mcp` spawns its followers from the new binary.

use reef_api::Build;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::clear;
use crate::client::Client;

/// Whether this process has spawned its updater, or is one, so it spawns no other
static SPAWNED: AtomicBool = AtomicBool::new(false);

/// How long a commit whose install failed waits before an updater tries it again
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// How long a binary has to say its build before it is taken for one that can't
const ASK_WAIT: Duration = Duration::from_secs(10);

/// The updater's log, in reef's state directory
const LOG: &str = "update.log";

/// The lock an updater holds while it runs, in reef's state directory
const LOCK: &str = "update.lock";

/// The last failed install, in reef's state directory
const FAILED: &str = "update.toml";

/// This binary's build: its version, the commit and time it was stamped with, if any, and the
/// target it was compiled for
pub static BUILD: LazyLock<Build> = LazyLock::new(|| {
    stamped(
        option_env!("REEF_COMMIT"),
        option_env!("REEF_COMMITTED"),
        &target(),
    )
});

/// A build of this version, stamped with what the compiler was given, if it was given both
///
/// # Arguments
///
/// * `commit` - `REEF_COMMIT` as it was at compile time, if it was set
/// * `committed` - `REEF_COMMITTED` as it was at compile time, if it was set
/// * `target` - The target it was compiled for
///
/// # Examples
///
/// ```
/// use reef::update::stamped;
///
/// let build = stamped(Some("dc5e68c"), Some("1795800000"), "x86_64-linux-musl");
/// assert_eq!(build.stamp(), Some(("dc5e68c", 1_795_800_000)));
/// assert!(!stamped(Some("dc5e68c"), Some("soon"), "x86_64-linux-gnu").is_stamped());
/// ```
pub fn stamped(commit: Option<&str>, committed: Option<&str>, target: &str) -> Build {
    // a variable set to nothing is as good as unset
    let commit = commit
        .map(str::trim)
        .filter(|commit| !commit.is_empty())
        .map(str::to_string);
    // and a time that isn't whole seconds is no time at all
    let committed = committed.and_then(|seconds| seconds.trim().parse().ok());
    Build {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit,
        committed,
        target: target.to_string(),
    }
}

/// The target this binary was compiled for: its architecture, its OS, and its C library where
/// it was linked with musl or glibc, such as `x86_64-linux-musl`
fn target() -> String {
    // the architecture and OS std was built for
    let mut target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    // and the C library, which decides whether the binary runs without the host's
    if cfg!(target_env = "musl") {
        target.push_str("-musl");
    } else if cfg!(target_env = "gnu") {
        target.push_str("-gnu");
    }
    target
}

/// A build as one line: `reef 0.1.0 <commit's first 12> (<its date>)`, or `reef 0.1.0 (not
/// stamped)`
///
/// # Arguments
///
/// * `build` - The build
///
/// # Examples
///
/// ```
/// use reef::update::{describe, stamped};
///
/// let build = stamped(Some("dc5e68c1a2b3c4d5"), Some("1795800000"), "x86_64-linux-musl");
/// assert_eq!(describe(&build), "reef 0.1.0 dc5e68c1a2b3 (2026-11-27)");
/// assert_eq!(describe(&stamped(None, None, "x86_64-linux-gnu")), "reef 0.1.0 (not stamped)");
/// ```
pub fn describe(build: &Build) -> String {
    match build.stamp() {
        // the commit as git abbreviates it, and the day it was made, in UTC
        Some((commit, committed)) => format!(
            "reef {} {} ({})",
            build.version,
            short(commit),
            day(committed)
        ),
        None => format!("reef {} (not stamped)", build.version),
    }
}

/// A commit's first 12 characters, as reefadm names a release's
///
/// # Arguments
///
/// * `commit` - The commit
pub fn short(commit: &str) -> &str {
    // its first 12 characters, or all of a shorter one
    commit
        .char_indices()
        .nth(12)
        .map_or(commit, |(end, _)| &commit[..end])
}

/// The day a time falls on, in UTC, such as `2026-11-27`
///
/// # Arguments
///
/// * `seconds` - The time, in seconds since the epoch
pub fn day(seconds: i64) -> String {
    gix::date::Time::new(seconds, 0).format_or_unix(gix::date::time::format::SHORT)
}

/// Prints this binary's build, as a line or as JSON
///
/// # Arguments
///
/// * `json` - Whether to print it as the JSON [`Build`] reads from
/// * `out` - Where to print it
pub fn version<W: Write>(json: bool, out: &mut W) -> anyhow::Result<()> {
    match json {
        // the build whole, for a server or an updater to read
        true => writeln!(out, "{}", serde_json::to_string(&*BUILD)?)?,
        // or the line a person reads
        false => writeln!(out, "{}", describe(&BUILD))?,
    }
    Ok(())
}

/// Which way the service's build lies from this one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The same commit
    Same,
    /// A commit made after this build's
    Newer,
    /// A commit made no later than this build's: older, or another of the same time
    NotNewer,
    /// This build isn't stamped, so nothing orders it against another
    Unstamped,
}

/// Which way a stamped build lies from another, by when their commits were made
///
/// # Arguments
///
/// * `own` - This build
/// * `commit` - The other build's commit
/// * `committed` - When that commit was made, in seconds since the epoch
///
/// # Examples
///
/// ```
/// use reef::update::{Direction, direction, stamped};
///
/// let own = stamped(Some("aaa"), Some("100"), "x86_64-linux-musl");
/// assert_eq!(direction(&own, "bbb", 200), Direction::Newer);
/// assert_eq!(direction(&own, "aaa", 100), Direction::Same);
/// assert_eq!(direction(&own, "ccc", 50), Direction::NotNewer);
/// ```
pub fn direction(own: &Build, commit: &str, committed: i64) -> Direction {
    match own.stamp() {
        // a build `cargo install` made has no commit to order by
        None => Direction::Unstamped,
        Some((ours, _)) if ours == commit => Direction::Same,
        Some((_, ours)) if committed > ours => Direction::Newer,
        Some(_) => Direction::NotNewer,
    }
}

/// Whether a binary is one reef may replace: anything but a reefadm release's own, its link
/// resolved, which is `releases/<name>/bin/reef` with the release's `release.toml` beside `bin`
///
/// `reefadm upgrade` moves that one with the server, and `/usr/local/bin/reef` on the server's
/// host links to it.
///
/// # Arguments
///
/// * `path` - The binary
pub fn may_replace(path: &Path) -> bool {
    // the file the path leads to, through any link, as `/usr/local/bin/reef` leads into a release
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // `bin` in a release, which holds its record, in reefadm's directory of releases
    let bin = real
        .parent()
        .filter(|bin| bin.file_name() == Some("bin".as_ref()));
    let release = bin.and_then(Path::parent);
    let in_release = release.is_some_and(|release| {
        release.join("release.toml").is_file()
            && release
                .parent()
                .is_some_and(|releases| releases.file_name() == Some("releases".as_ref()))
    });
    !in_release
}

/// This binary's path, as it was started, even after it was replaced while running
///
/// Linux names a binary renamed over while it runs by its old path with ` (deleted)` after it,
/// where the new binary now is.
pub fn own_exe() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(undeleted)
}

/// A binary's path with the ` (deleted)` Linux adds to one replaced while it runs taken off, if
/// it was replaced
///
/// # Arguments
///
/// * `path` - The path `current_exe` named
///
/// # Examples
///
/// ```
/// use reef::update::undeleted;
/// use std::path::PathBuf;
///
/// let gone = PathBuf::from("/nowhere/bin/reef (deleted)");
/// assert_eq!(undeleted(gone), PathBuf::from("/nowhere/bin/reef"));
/// ```
pub fn undeleted(path: PathBuf) -> PathBuf {
    // a path that is there is the binary's own, whatever it is called
    if path.exists() {
        return path;
    }
    match path
        .to_str()
        .and_then(|text| text.strip_suffix(" (deleted)"))
    {
        Some(text) => PathBuf::from(text),
        None => path,
    }
}

/// What a client needs to update itself as it goes: whether it may, and what its updater is
/// passed so it calls the same service, signed in as this `reef` is
#[derive(Debug, Clone, Default)]
pub struct Watch {
    /// Whether `[client] update` lets this `reef` update itself
    pub enabled: bool,
    /// The config this `reef` read, if it read one
    pub conf: Option<PathBuf>,
    /// The credentials file this `reef` was named, if it was named one
    pub credentials: Option<PathBuf>,
}

impl Watch {
    /// Reads the build an answer names, and starts the updater if it is newer than this one and
    /// this process hasn't started one yet
    ///
    /// Nothing it finds or fails at stops the command whose answer it read.
    ///
    /// # Arguments
    ///
    /// * `headers` - The answer's headers
    pub fn notice(&self, headers: &reqwest::header::HeaderMap) {
        // the build named, if the service names one
        let Some((commit, committed)) = named(headers) else {
            return;
        };
        // newer than this one, which is stamped and allowed to follow it, and only once a process
        if !wanted(&BUILD, committed, self.enabled) || SPAWNED.swap(true, Ordering::SeqCst) {
            return;
        }
        // a binary reef may replace, and a commit whose install didn't just fail
        let Ok(exe) = own_exe() else {
            return;
        };
        if !may_replace(&exe) || failed_lately(&commit, now()) {
            return;
        }
        // in the background, saying nothing, since the command goes on either way
        let _ = spawn(&exe, self.conf.as_deref(), self.credentials.as_deref());
    }
}

/// The commit and its time an answer's `reef-client` header names, if it names one that can be
/// ordered
///
/// # Arguments
///
/// * `headers` - The answer's headers
pub fn named(headers: &reqwest::header::HeaderMap) -> Option<(String, i64)> {
    headers
        .get(reef_api::paths::CLIENT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(reef_api::build::read_header)
}

/// Whether a build should replace itself with one made at a time: it is stamped, older, and
/// allowed to
///
/// # Arguments
///
/// * `own` - This build
/// * `committed` - When the service's build's commit was made, in seconds since the epoch
/// * `enabled` - Whether `[client] update` lets it
///
/// # Examples
///
/// ```
/// use reef::update::{stamped, wanted};
///
/// let own = stamped(Some("aaa"), Some("100"), "x86_64-linux-musl");
/// assert!(wanted(&own, 200, true));
/// assert!(!wanted(&own, 200, false));
/// assert!(!wanted(&own, 100, true));
/// assert!(!wanted(&stamped(None, None, "x86_64-linux-gnu"), 200, true));
/// ```
pub fn wanted(own: &Build, committed: i64, enabled: bool) -> bool {
    // a build cargo made never replaces itself unasked
    enabled && own.stamp().is_some_and(|(_, ours)| committed > ours)
}

/// Starts `reef update --auto` from a binary, in the background, in a process group of its own,
/// its output appended to its log
///
/// # Arguments
///
/// * `exe` - The binary to run, this one
/// * `conf` - The config this `reef` read, passed on so the updater calls the same service
/// * `credentials` - The credentials file this `reef` was named, passed on so it signs in as this
///   one does
pub fn spawn(exe: &Path, conf: Option<&Path>, credentials: Option<&Path>) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    // what it says goes to its log, since nothing would show it, or nowhere if that can't open
    let log = clear::open_log(LOG)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    // this same binary, on the same service, signed in as this one is
    let mut command = std::process::Command::new(exe);
    if let Some(conf) = conf {
        command.arg("--conf").arg(conf);
    }
    if let Some(credentials) = credentials {
        command.arg("--credentials").arg(credentials);
    }
    command
        .args(["update", "--auto"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0);
    let mut child = command.spawn()?;
    // waited on in the background, so a long-lived process leaves no zombie, and init reaps it
    // for one that ends first
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// What `reef update` was asked
#[derive(Debug, Clone, Default)]
pub struct Asked {
    /// Only say what it would do
    pub check: bool,
    /// Install the service's build whichever way it lies from this one
    pub force: bool,
    /// Run as the updater a command started: quietly, and newer builds alone
    pub auto: bool,
    /// The binary to replace, else this one
    pub into: Option<PathBuf>,
}

/// Replaces this binary with the build the service hands its users, as `reef update` asks
///
/// # Arguments
///
/// * `client` - The service's client
/// * `asked` - What was asked
/// * `out` - Where to say what came of it: the terminal, or the updater's log
pub async fn update<W: Write>(client: &Client, asked: &Asked, out: &mut W) -> anyhow::Result<()> {
    // this process starts no updater of its own, whatever its answers name
    SPAWNED.store(true, Ordering::SeqCst);
    match asked.auto {
        true => auto(client, out).await,
        false => by_hand(client, asked, out).await,
    }
}

/// `reef update` typed by hand: says why it won't, or installs the service's build
///
/// # Arguments
///
/// * `client` - The service's client
/// * `asked` - What was asked
/// * `out` - Where to say what came of it
async fn by_hand<W: Write>(client: &Client, asked: &Asked, out: &mut W) -> anyhow::Result<()> {
    // the binary to replace, which reefadm's release keeps for itself
    let target = match &asked.into {
        Some(into) => into.clone(),
        None => own_exe()?,
    };
    if !may_replace(&target) {
        anyhow::bail!(
            "{} is a reefadm release's own `reef`, which `reefadm upgrade` replaces with the \
             server: run that instead",
            target.display()
        );
    }
    // the build the service hands its users, which it must hand them, stamped, for this host
    let Some(served) = client.served_client().await? else {
        anyhow::bail!(
            "reef's service at {} hands its users no `reef`: it is older than reef's updates, or \
             its release holds no stamped client",
            client.base()
        );
    };
    let (commit, committed) = stamp_of(&served)?;
    check_target(&served)?;
    // which way it lies, and whether that is reason enough to install it
    match direction(&BUILD, commit, committed) {
        Direction::Same if !asked.force => {
            writeln!(out, "reef is up to date: {}", describe(&BUILD))?;
            return Ok(());
        }
        Direction::NotNewer if !asked.force => {
            writeln!(
                out,
                "reef's service hands out {}, which isn't newer than this one, {}: `reef update \
                 --force` installs it anyway",
                describe(&served),
                describe(&BUILD)
            )?;
            return Ok(());
        }
        // newer, or forced, or a build cargo made, which only a hand replaces
        _ => {}
    }
    // only said, when only asked
    if asked.check {
        writeln!(
            out,
            "reef's service hands out {}, and this is {}: `reef update` installs it over {}",
            describe(&served),
            describe(&BUILD),
            target.display()
        )?;
        return Ok(());
    }
    install(client, &target, &served).await?;
    writeln!(
        out,
        "updated {} to {}, from {}",
        target.display(),
        describe(&served),
        describe(&BUILD)
    )?;
    Ok(())
}

/// The updater a command started: installs a newer build the service hands out, or leaves at
/// once, saying in its log only what it did or failed at
///
/// # Arguments
///
/// * `client` - The service's client
/// * `log` - The updater's log
async fn auto<W: Write>(client: &Client, log: &mut W) -> anyhow::Result<()> {
    // one updater at a time: any other started meanwhile leaves
    let Some(_held) = hold()? else {
        return Ok(());
    };
    // a service that hands out no client, or an old one, is no update
    let served = match client.served_client().await {
        Ok(Some(served)) => served,
        Ok(None) => return Ok(()),
        Err(error) => {
            writeln!(log, "{} asking for reef's client: {error:#}", stamp_now())?;
            return Ok(());
        }
    };
    // a newer build, for this host, over a binary reef may replace, whose install didn't just fail
    let Ok((commit, committed)) = stamp_of(&served) else {
        return Ok(());
    };
    if direction(&BUILD, commit, committed) != Direction::Newer || check_target(&served).is_err() {
        return Ok(());
    }
    let target = own_exe()?;
    if !may_replace(&target) || failed_lately(commit, now()) {
        return Ok(());
    }
    // installed, or kept as failed so the next updater waits
    match install(client, &target, &served).await {
        Ok(()) => writeln!(
            log,
            "{} updated {} to {}, from {}",
            stamp_now(),
            target.display(),
            describe(&served),
            describe(&BUILD)
        )?,
        Err(error) => {
            writeln!(
                log,
                "{} updating {} to {} failed: {error:#}",
                stamp_now(),
                target.display(),
                describe(&served)
            )?;
            if let Err(error) = record_failure(commit, &format!("{error:#}"), now()) {
                writeln!(log, "{} keeping that failure: {error:#}", stamp_now())?;
            }
        }
    }
    Ok(())
}

/// The commit and its time a served build is stamped with, which a service only serves stamped
///
/// # Arguments
///
/// * `served` - The build the service hands out
fn stamp_of(served: &Build) -> anyhow::Result<(&str, i64)> {
    served.stamp().ok_or_else(|| {
        anyhow::anyhow!("reef's service hands out a `reef` with no commit, which nothing can order")
    })
}

/// Refuses a build for another architecture or OS than this one's
///
/// # Arguments
///
/// * `served` - The build the service hands out
fn check_target(served: &Build) -> anyhow::Result<()> {
    anyhow::ensure!(
        served.runs_beside(&BUILD),
        "reef's service hands out a `reef` for {}, and this host is {}",
        served.target,
        BUILD.target
    );
    Ok(())
}

/// Installs the build the service hands out over a binary: streamed beside it, made executable
/// and synced, run to check it says the commit wanted, then renamed over it, the binary its link
/// leads to if it is a link
///
/// Anything that fails removes the new file, and leaves the binary as it was.
///
/// # Arguments
///
/// * `client` - The service's client
/// * `into` - The binary to replace
/// * `wanted` - The build the service said it hands out
pub async fn install(client: &Client, into: &Path, wanted: &Build) -> anyhow::Result<()> {
    let (commit, _) = stamp_of(wanted)?;
    // the file itself, through any link, so the link keeps leading to it
    let target = std::fs::canonicalize(into).unwrap_or_else(|_| into.to_path_buf());
    let dir = target
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // beside it, so the rename is one filesystem's and so whole
    let temp = dir.join(format!(".reef-update-{}", std::process::id()));
    let installed = put(client, &temp, &target, commit).await;
    // whatever failed leaves nothing behind
    if installed.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    installed
}

/// Streams the service's binary into a file, checks it, and renames it over the target
///
/// # Arguments
///
/// * `client` - The service's client
/// * `temp` - The file to stream it into, beside the target
/// * `target` - The binary to replace
/// * `commit` - The commit it must say it was built from
async fn put(client: &Client, temp: &Path, target: &Path, commit: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    // a file of this process's own, an earlier one's left by a crash cleared first
    let _ = std::fs::remove_file(temp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(temp)
        .map_err(|error| anyhow::anyhow!("writing beside {}: {error}", target.display()))?;
    // the binary, a chunk at a time, as many bytes as the service said
    let mut response = client.client_binary().await?;
    let length = response.content_length();
    let mut written = 0u64;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
    }
    if let Some(length) = length {
        anyhow::ensure!(
            written == length,
            "reef's service sent {written} of the client's {length} bytes"
        );
    }
    // executable by anyone, as an installed binary is, and on disk before it is run
    file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    file.sync_all()?;
    drop(file);
    // the build it says it is, which must be the one the service named
    let said = ask(temp).await?;
    anyhow::ensure!(
        said.commit.as_deref() == Some(commit),
        "the `reef` reef's service sent says it is {}, not the {} it named",
        describe(&said),
        short(commit)
    );
    // over the target in one step, and the directory synced so the rename lasts
    std::fs::rename(temp, target)
        .map_err(|error| anyhow::anyhow!("replacing {}: {error}", target.display()))?;
    if let Some(dir) = target.parent()
        && let Ok(dir) = std::fs::File::open(dir)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Asks a binary which build it is, through `version --json`, within a few seconds
///
/// # Arguments
///
/// * `path` - The binary
async fn ask(path: &Path) -> anyhow::Result<Build> {
    // a file just written may be busy a moment, while a fork elsewhere still holds it open
    let mut tries = 0;
    let output = loop {
        let asked = run_version(path).await;
        match asked {
            Err(error) if is_busy(&error) && tries < BUSY_TRIES => {
                tries += 1;
                tokio::time::sleep(BUSY_PAUSE).await;
            }
            asked => {
                break asked
                    .map_err(|error| error.context("running the `reef` reef's service sent"))?;
            }
        }
    };
    anyhow::ensure!(
        output.status.success(),
        "the `reef` reef's service sent answered `version --json` with {}",
        output.status
    );
    serde_json::from_slice(&output.stdout).map_err(|error| {
        anyhow::anyhow!("the `reef` reef's service sent said a build that isn't one: {error}")
    })
}

/// How many times a binary that is busy is run again
const BUSY_TRIES: usize = 10;

/// How long to wait before running a busy binary again
const BUSY_PAUSE: Duration = Duration::from_millis(50);

/// Whether running a binary failed because it was still open for writing somewhere
///
/// # Arguments
///
/// * `error` - What running it failed with
fn is_busy(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::ExecutableFileBusy)
}

/// Runs a binary's `version --json`, never waiting past the timeout
///
/// # Arguments
///
/// * `path` - The binary
async fn run_version(path: &Path) -> anyhow::Result<std::process::Output> {
    let asked = tokio::process::Command::new(path)
        .args(["version", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(ASK_WAIT, asked)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the `reef` reef's service sent didn't say its build within {} s",
                ASK_WAIT.as_secs()
            )
        })?
        .map_err(anyhow::Error::from)
}

/// The updater's lock, held until it is dropped, or `None` if another updater holds it
fn hold() -> anyhow::Result<Option<std::fs::File>> {
    // in reef's state directory, made if it isn't there
    let dir = clear::state_dir(|name| std::env::var(name).ok())?;
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))?;
    // held by this process alone, or by another, which this one leaves to it
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

/// The install that failed last, kept in reef's state directory so an updater waits before it
/// tries the same commit again
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    /// The commit whose install failed
    pub commit: String,
    /// When it failed, in seconds since the epoch
    pub at: i64,
    /// What it failed with
    pub error: String,
}

impl Failed {
    /// Whether this failure still holds a commit back at a time: the same commit, within the hour
    ///
    /// # Arguments
    ///
    /// * `commit` - The commit an updater would install
    /// * `now` - The time, in seconds since the epoch
    pub fn holds_back(&self, commit: &str, now: i64) -> bool {
        self.commit == commit && now.saturating_sub(self.at) < RETRY_AFTER.as_secs() as i64
    }
}

/// Whether a commit's install failed within the hour, as reef's state directory keeps it
///
/// # Arguments
///
/// * `commit` - The commit an updater would install
/// * `now` - The time, in seconds since the epoch
fn failed_lately(commit: &str, now: i64) -> bool {
    // nothing kept, or nothing that reads, holds nothing back
    let Ok(dir) = clear::state_dir(|name| std::env::var(name).ok()) else {
        return false;
    };
    std::fs::read_to_string(dir.join(FAILED))
        .ok()
        .and_then(|text| toml::from_str::<Failed>(&text).ok())
        .is_some_and(|failed| failed.holds_back(commit, now))
}

/// Keeps a commit's failed install in reef's state directory
///
/// # Arguments
///
/// * `commit` - The commit whose install failed
/// * `error` - What it failed with
/// * `now` - The time, in seconds since the epoch
fn record_failure(commit: &str, error: &str, now: i64) -> anyhow::Result<()> {
    let dir = clear::state_dir(|name| std::env::var(name).ok())?;
    std::fs::create_dir_all(&dir)?;
    let failed = Failed {
        commit: commit.to_string(),
        at: now,
        error: error.to_string(),
    };
    std::fs::write(dir.join(FAILED), toml::to_string(&failed)?)?;
    Ok(())
}

/// The time now, in seconds since the epoch
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// The time now as a log line starts with it, in UTC, such as `2026-11-27 20:56:00 +0000`
fn stamp_now() -> String {
    gix::date::Time::new(now(), 0).format_or_unix(gix::date::time::format::ISO8601)
}

#[cfg(test)]
mod tests {
    use super::{
        BUILD, Direction, Failed, describe, direction, install, may_replace, named, short, stamped,
        undeleted, version, wanted,
    };
    use crate::client::Client;
    use crate::conf::Conf;
    use reef_api::Build;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A directory of a test's own, empty
    ///
    /// # Arguments
    ///
    /// * `name` - What the test is
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reef-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes an executable script
    ///
    /// # Arguments
    ///
    /// * `path` - Where to write it
    /// * `text` - The script
    fn executable(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A script that says it is a build, as `reef version --json` would
    ///
    /// # Arguments
    ///
    /// * `build` - The build it says it is
    fn saying(build: &Build) -> String {
        format!(
            "#!/bin/sh\necho '{}'\n",
            serde_json::to_string(build).unwrap()
        )
    }

    /// Serves some bytes over plain HTTP on a loopback port, to every request, as the client's
    /// route sends a binary, returning the root to call
    ///
    /// # Arguments
    ///
    /// * `body` - The bytes every answer carries
    async fn serving(body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    // the request's head, which a blank line ends
                    let mut head = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        let read = stream.read(&mut buffer).await?;
                        if read == 0 {
                            break;
                        }
                        head.extend_from_slice(&buffer[..read]);
                    }
                    // the bytes, whole, then the connection closed
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(answer.as_bytes()).await?;
                    stream.write_all(&body).await?;
                    stream.shutdown().await
                });
            }
        });
        format!("http://{address}")
    }

    /// A stamped build says its commit and day, an unstamped one says it isn't stamped, and both
    /// print as the JSON a build reads back from
    #[test]
    fn a_build_prints_its_stamp_or_its_lack() {
        // stamped: the commit's first 12, and its day in UTC
        let build = stamped(
            Some("dc5e68c1a2b3c4d5e6f7"),
            Some("1795800000"),
            "x86_64-linux-musl",
        );
        assert_eq!(describe(&build), "reef 0.1.0 dc5e68c1a2b3 (2026-11-27)");
        // unstamped, or stamped with half of it, or with nothing at all
        for unstamped in [
            stamped(None, None, "x86_64-linux-gnu"),
            stamped(Some("dc5e68c"), None, "x86_64-linux-gnu"),
            stamped(Some(" "), Some("1795800000"), "x86_64-linux-gnu"),
        ] {
            assert_eq!(describe(&unstamped), "reef 0.1.0 (not stamped)");
        }
        // this test's own build, which nothing stamped, prints as JSON that reads back
        let mut out = Vec::new();
        version(true, &mut out).unwrap();
        let read: Build = serde_json::from_slice(&out).unwrap();
        assert_eq!(read, *BUILD);
        assert!(read.target.starts_with(std::env::consts::ARCH), "{read:?}");
        // and as the line
        let mut out = Vec::new();
        version(false, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), describe(&BUILD));
        // a short commit is kept whole
        assert_eq!(short("dc5e68c"), "dc5e68c");
    }

    /// Builds order by when their commits were made, newer only, and a build cargo made orders
    /// against nothing
    #[test]
    fn builds_order_by_their_commits_time() {
        let own = stamped(Some("aaa"), Some("1000"), "x86_64-linux-musl");
        // newer, the same, older, and another commit of the same time
        assert_eq!(direction(&own, "bbb", 2000), Direction::Newer);
        assert_eq!(direction(&own, "aaa", 1000), Direction::Same);
        assert_eq!(direction(&own, "ccc", 500), Direction::NotNewer);
        assert_eq!(direction(&own, "ddd", 1000), Direction::NotNewer);
        // only a newer one is wanted, and only while updates are on
        assert!(wanted(&own, 2000, true));
        assert!(!wanted(&own, 2000, false));
        assert!(!wanted(&own, 1000, true) && !wanted(&own, 500, true));
        // unstamped, as cargo builds it: ordered against nothing, and never wanted unasked
        let cargo = stamped(None, None, "x86_64-linux-gnu");
        assert_eq!(direction(&cargo, "bbb", 2000), Direction::Unstamped);
        assert!(!wanted(&cargo, 2000, true));
    }

    /// An answer's header reads as a commit and its time, and anything else as nothing
    #[test]
    fn the_header_reads_as_a_commit_and_its_time() {
        let mut headers = reqwest::header::HeaderMap::new();
        // no header at all
        assert_eq!(named(&headers), None);
        // the build a service names
        headers.insert("reef-client", "dc5e68c 1795800000".parse().unwrap());
        assert_eq!(
            named(&headers),
            Some(("dc5e68c".to_string(), 1_795_800_000))
        );
        // and one that can't be ordered
        for junk in ["dc5e68c", "dc5e68c tomorrow", "", "a 1 b"] {
            headers.insert("reef-client", junk.parse().unwrap());
            assert_eq!(named(&headers), None, "{junk:?}");
        }
    }

    /// A reefadm release's own `reef`, or a link into one, is never replaced, and any other is
    #[test]
    fn a_releases_own_reef_is_never_replaced() {
        let dir = scratch("release");
        // a release, with its record beside `bin`, and the host's link into it
        let release = dir.join("opt/reef/releases/992226c4a1b2-20261127072258");
        std::fs::create_dir_all(release.join("bin")).unwrap();
        std::fs::write(release.join("release.toml"), "").unwrap();
        executable(&release.join("bin/reef"), "#!/bin/sh\n");
        let link = dir.join("usr/local/bin/reef");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(release.join("bin/reef"), &link).unwrap();
        assert!(!may_replace(&release.join("bin/reef")));
        assert!(!may_replace(&link));
        // a `bin/reef` with no record beside it, or outside reefadm's releases, is a user's
        let bare = dir.join("home/ada/releases/r1");
        std::fs::create_dir_all(bare.join("bin")).unwrap();
        executable(&bare.join("bin/reef"), "#!/bin/sh\n");
        assert!(may_replace(&bare.join("bin/reef")));
        let elsewhere = dir.join("home/ada/r2");
        std::fs::create_dir_all(elsewhere.join("bin")).unwrap();
        std::fs::write(elsewhere.join("release.toml"), "").unwrap();
        executable(&elsewhere.join("bin/reef"), "#!/bin/sh\n");
        assert!(may_replace(&elsewhere.join("bin/reef")));
        // and a user's own install
        let own = dir.join("home/ada/.local/bin/reef");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        executable(&own, "#!/bin/sh\n");
        assert!(may_replace(&own));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A binary replaced while running is found at its path again, and one that is there keeps
    /// its name
    #[test]
    fn a_replaced_binary_drops_deleted() {
        let dir = scratch("deleted");
        // the path Linux names once the binary was renamed over
        let path = dir.join("reef");
        executable(&path, "#!/bin/sh\n");
        let named = PathBuf::from(format!("{} (deleted)", path.display()));
        assert_eq!(undeleted(named), path);
        // a file that really has the suffix in its name keeps it
        let odd = dir.join("reef (deleted)");
        executable(&odd, "#!/bin/sh\n");
        assert_eq!(undeleted(odd.clone()), odd);
        // and a plain path is left alone
        assert_eq!(undeleted(path.clone()), path);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failed install holds its commit back for an hour, and no other
    #[test]
    fn a_failed_install_waits_an_hour() {
        let failed = Failed {
            commit: "bbb".to_string(),
            at: 10_000,
            error: "disk full".to_string(),
        };
        assert!(failed.holds_back("bbb", 10_000 + 59 * 60));
        assert!(!failed.holds_back("bbb", 10_000 + 60 * 60));
        assert!(!failed.holds_back("ccc", 10_000));
        // and it reads back as it was kept
        let kept = toml::to_string(&failed).unwrap();
        assert_eq!(toml::from_str::<Failed>(&kept).unwrap(), failed);
    }

    /// An install swaps in a binary that says the commit wanted, and refuses one that doesn't,
    /// leaving the binary it would have replaced as it was and nothing beside it
    #[tokio::test]
    async fn an_install_swaps_in_the_commit_wanted_and_refuses_another() {
        let dir = scratch("install");
        // the binary to replace
        let target = dir.join("reef");
        executable(&target, "#!/bin/sh\necho old\n");
        // a service handing out a build of commit `aaa`
        let served = stamped(Some("aaa"), Some("2000"), &BUILD.target);
        let bytes = saying(&served);
        let mut conf = Conf::default();
        conf.api.url = Some(serving(bytes.clone().into_bytes()).await);
        let client = Client::new(&conf).unwrap();
        // wanting another commit, the one sent is refused, and the old binary stays
        let other = stamped(Some("bbb"), Some("3000"), &BUILD.target);
        let refused = install(&client, &target, &other).await.unwrap_err();
        assert!(
            format!("{refused:#}").contains("not the bbb"),
            "{refused:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\necho old\n"
        );
        // wanting the commit sent, it replaces the binary, executable
        install(&client, &target, &served).await.unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), bytes);
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // and no file of either install is left beside it
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["reef"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
