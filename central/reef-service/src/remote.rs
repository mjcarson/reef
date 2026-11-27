//! Git remotes: the URLs a project may be registered by, how two spellings of one repo match,
//! and the git commands that keep a clone of one branch of it
//!
//! A remote project's clone is bare and shallow, made by `git init --bare` and filled by the same
//! fetch every later fetch runs, so registering and fetching take one path. The URL is passed to
//! each command rather than kept as the clone's `origin`, so the row the store keeps is the one
//! place it lives. Git runs with the service's own account and environment, so a private remote
//! is reached through that account's ssh keys or credential helpers, never a password in the
//! URL, which every caller of reef would see listed. Git never asks for one either: it has no
//! terminal, and is told not to prompt.
//!
//! This runs git through the CLI, as [`crate::repo`] does, so its transports and credentials are
//! git's own. It touches no shoal.

use gix::bstr::{BStr, ByteSlice};
use gix::url::Scheme;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::instrument;

use crate::repo::GIT_ENV;

/// How long one git command may run before it is stopped
const GIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The longest URL a project may be registered by
const URL_MAX: usize = 2048;

/// The transports git may use for the service, leaving out `ext::` and every remote helper
const ALLOWED_PROTOCOLS: &str = "file:git:http:https:ssh";

/// A URL or branch a project can't be registered by, and why
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadRemote(pub String);

impl std::fmt::Display for BadRemote {
    /// Says what is wrong with the URL or branch, and what to give instead
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BadRemote {}

/// A git command against a remote, or a clone of one, that failed, and what git said
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchFailed(pub String);

impl std::fmt::Display for FetchFailed {
    /// Says what was being done, and what git said of it
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FetchFailed {}

/// Checks a URL a project may be registered by
///
/// It must be a file, git, ssh, http or https URL, or a form git reads as one (`user@host:path`,
/// or an absolute path), carrying no password, and no user at all over http(s), where a token is
/// often put in its place. A local path must be absolute, since the service's working directory
/// is no one's. Nothing in it may be read by git or ssh as an option.
///
/// # Arguments
///
/// * `url` - The URL as given
///
/// # Examples
///
/// ```
/// use reef_service::remote::checked;
///
/// assert!(checked("git@example.com:reef/reef.git").is_ok());
/// assert!(checked("https://ada:hunter2@example.com/reef.git").is_err());
/// assert!(checked("ext::sh -c touch% /tmp/pwned").is_err());
/// ```
pub fn checked(url: &str) -> Result<gix::Url, BadRemote> {
    // a URL no one would write, before git reads it
    if url.len() > URL_MAX {
        return Err(BadRemote(format!(
            "a remote's URL is at most {URL_MAX} bytes, and this one is {}",
            url.len()
        )));
    }
    if url.chars().any(char::is_control) {
        return Err(BadRemote(
            "a remote's URL can't hold control characters".to_string(),
        ));
    }
    // read as git reads it
    let parsed = gix::url::parse(BStr::new(url))
        .map_err(|error| BadRemote(format!("`{url}` is not a URL git can fetch: {error}")))?;
    // only the transports git runs itself, and none that runs a command here
    match &parsed.scheme {
        Scheme::File | Scheme::Git | Scheme::Ssh | Scheme::Http | Scheme::Https => {}
        other => {
            return Err(BadRemote(format!(
                "`{url}` fetches through `{}`, which reef doesn't run: use an https, ssh, git or \
                 file URL",
                other.as_str()
            )));
        }
    }
    // every caller of reef sees a project's URL, so it can't carry a secret
    if parsed.password().is_some() {
        return Err(BadRemote(format!(
            "{} carries a password, which every caller of reef would see: give the service's \
             account an ssh key or a credential helper instead",
            redacted(&parsed)
        )));
    }
    if matches!(parsed.scheme, Scheme::Http | Scheme::Https) && parsed.user().is_some() {
        return Err(BadRemote(format!(
            "{} names a user, where a token is often kept, and every caller of reef would see \
             it: give the service's account a credential helper instead",
            redacted(&parsed)
        )));
    }
    // a local path is the service's, and must name one place whatever its working directory
    if parsed.scheme == Scheme::File && !parsed.path.starts_with(b"/") {
        return Err(BadRemote(format!(
            "`{url}` is a relative path: name the repo by its absolute path on the service's host"
        )));
    }
    // nothing git hands to ssh, or to itself, may read as an option
    let path = parsed.path.strip_prefix(b"/").unwrap_or(&parsed.path);
    let host_unsafe = parsed.host().is_some() && parsed.host_argument_safe().is_none();
    if host_unsafe || path.starts_with(b"-") {
        return Err(BadRemote(format!(
            "`{url}` has a host or path that starts with `-`, which git would read as an option"
        )));
    }
    Ok(parsed)
}

/// A URL with its user and password left out, to name in a message
///
/// # Arguments
///
/// * `url` - The URL to name
fn redacted(url: &gix::Url) -> String {
    // the same URL, with nobody in it
    let mut url = url.clone();
    url.set_user(None);
    url.set_password(None);
    format!("`{}`", url.to_bstring())
}

/// The key every spelling of one repo's URL shares, or `None` for one git can't read
///
/// A repo on a host is its host and path, so its scheme, its user and its port are left out, and
/// so are a trailing `/` and `.git`: `git@host:o/r.git`, `ssh://git@host:22/o/r` and
/// `https://HOST/o/r/` are one repo. A local repo is its path, kept apart from any host's.
///
/// # Arguments
///
/// * `url` - The URL, as a project was registered by or a checkout fetches from
///
/// # Examples
///
/// ```
/// use reef_service::remote::key;
///
/// assert_eq!(key("git@example.com:reef/reef.git"), key("https://example.com/reef/reef"));
/// assert_ne!(key("https://example.com/reef/reef"), key("https://example.com/reef/shoal"));
/// ```
pub fn key(url: &str) -> Option<String> {
    // read as git reads it, its host already lowercased
    let parsed = gix::url::parse(BStr::new(url)).ok()?;
    let path = parsed.path.to_str_lossy();
    // a trailing slash, then `.git`, then a slash before it, are all one repo
    let path = path.trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    match parsed.host() {
        // a repo on a host, however it is reached
        Some(host) => Some(format!("{host}/{}", path.trim_start_matches('/'))),
        // a repo on this host, by its path
        None => Some(format!("file:{path}")),
    }
}

/// The branch a remote's `HEAD` names, its default
///
/// # Arguments
///
/// * `url` - The remote's URL, already checked
#[instrument(name = "remote::default_branch", skip_all, err(Debug))]
pub async fn default_branch(url: &str) -> anyhow::Result<String> {
    // ask the remote what its HEAD points at
    let what = format!("asking {url} for its default branch");
    let said = git(None, ["ls-remote", "--symref", "--", url, "HEAD"], &what).await?;
    // a remote with no symbolic HEAD, such as an empty one, names none
    match symref(&said) {
        Some(branch) => Ok(branch),
        None => Err(BadRemote(format!(
            "{url} names no default branch: name the branch whose docs to read"
        ))
        .into()),
    }
}

/// The branches a remote has, by name, in the order git lists them
///
/// # Arguments
///
/// * `url` - The remote's URL, already checked
#[instrument(name = "remote::heads", skip_all, err(Debug))]
pub async fn heads(url: &str) -> anyhow::Result<Vec<String>> {
    // ask the remote for its branches alone
    let what = format!("asking {url} for its branches");
    let said = git(None, ["ls-remote", "--heads", "--", url], &what).await?;
    Ok(heads_in(&said))
}

/// The branches a `git ls-remote --heads` names: a commit and `refs/heads/<branch>` a line
///
/// # Arguments
///
/// * `said` - What it printed
fn heads_in(said: &str) -> Vec<String> {
    said.lines()
        .filter_map(|line| line.split_once('\t'))
        .filter_map(|(_, name)| name.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .collect()
}

/// The branch a `git ls-remote --symref … HEAD` names, if it names one
///
/// # Arguments
///
/// * `said` - What it printed: `ref: refs/heads/<branch>\tHEAD`, then HEAD's commit
fn symref(said: &str) -> Option<String> {
    // the one line that says where HEAD points
    said.lines().find_map(|line| {
        let target = line.strip_prefix("ref: ")?.strip_suffix("\tHEAD")?;
        target.strip_prefix("refs/heads/").map(str::to_string)
    })
}

/// Checks that a branch's name is one git keeps
///
/// # Arguments
///
/// * `branch` - The branch's name, without `refs/heads/`
#[instrument(name = "remote::check_branch", skip_all, err(Debug))]
pub async fn check_branch(branch: &str) -> anyhow::Result<()> {
    // a name git would read as an option is no branch's
    if branch.is_empty() || branch.starts_with('-') {
        return Err(BadRemote(format!("`{branch}` is not a branch's name")).into());
    }
    // git judges the rest, by the full ref the branch would be
    let full = format!("refs/heads/{branch}");
    match git(
        None,
        ["check-ref-format", full.as_str()],
        "checking a branch",
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(_) => Err(BadRemote(format!("`{branch}` is not a branch's name")).into()),
    }
}

/// Makes an empty bare repo for a clone, keeping no sample hooks
///
/// # Arguments
///
/// * `dir` - Where the clone goes: a directory that doesn't exist yet, or an empty one
/// * `branch` - The branch its `HEAD` names
#[instrument(name = "remote::make", skip_all, err(Debug))]
pub async fn make(dir: &Path, branch: &str) -> anyhow::Result<()> {
    // an empty template, so the clone holds nothing git would run
    let initial = format!("--initial-branch={branch}");
    let args: [&OsStr; 6] = [
        "init".as_ref(),
        "--bare".as_ref(),
        "--quiet".as_ref(),
        "--template=".as_ref(),
        initial.as_ref(),
        dir.as_os_str(),
    ];
    git(None, args, &format!("making a clone at {}", dir.display())).await?;
    Ok(())
}

/// Fetches a branch's tip into a clone, alone and shallow, following a force-push
///
/// # Arguments
///
/// * `clone` - The clone, made by [`make`]
/// * `url` - The remote's URL, already checked
/// * `branch` - The branch to fetch
#[instrument(name = "remote::fetch", skip_all, err(Debug))]
pub async fn fetch(clone: &Path, url: &str, branch: &str) -> anyhow::Result<()> {
    // the branch over its own ref, whatever it was before, with no hook run and no FETCH_HEAD
    let refspec = format!("+refs/heads/{branch}:refs/heads/{branch}");
    let args = [
        "-c",
        "core.hooksPath=/dev/null",
        "fetch",
        "--quiet",
        "--no-tags",
        "--depth=1",
        "--update-head-ok",
        "--no-write-fetch-head",
        "--",
        url,
        refspec.as_str(),
    ];
    git(
        Some(clone),
        args,
        &format!("fetching `{branch}` from {url}"),
    )
    .await?;
    Ok(())
}

/// The commit a clone's branch is at
///
/// # Arguments
///
/// * `clone` - The clone
/// * `branch` - The branch
#[instrument(name = "remote::tip", skip_all, err(Debug))]
pub async fn tip(clone: &Path, branch: &str) -> anyhow::Result<String> {
    // the branch's commit, as its full hex id
    let rev = format!("refs/heads/{branch}^{{commit}}");
    let args = ["rev-parse", "--verify", "--end-of-options", rev.as_str()];
    let said = git(
        Some(clone),
        args,
        &format!("reading `{branch}` in its clone"),
    )
    .await?;
    Ok(said.trim().to_string())
}

/// Runs git with none of the variables that would point it at another repo, and no prompt
///
/// It is stopped after [`GIT_TIMEOUT`], and killed if its caller stops waiting.
///
/// # Arguments
///
/// * `dir` - The repo to run in, or `None` for a command that needs none
/// * `args` - Its arguments
/// * `what` - What it does, for the message if it fails
async fn git<I, S>(dir: Option<&Path>, args: I, what: &str) -> Result<String, FetchFailed>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // git in the repo named, or none, from its arguments alone
    let mut git = Command::new("git");
    if let Some(dir) = dir {
        git.arg("-C").arg(dir);
    }
    git.args(args);
    for var in GIT_ENV {
        git.env_remove(var);
    }
    // no terminal to ask on, no helper that asks, and only the transports git runs itself
    git.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_ALLOW_PROTOCOL", ALLOWED_PROTOCOLS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // run it, but not forever
    let output = match tokio::time::timeout(GIT_TIMEOUT, git.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(FetchFailed(format!("{what}: running git: {error}"))),
        Err(_) => {
            return Err(FetchFailed(format!(
                "{what}: git ran longer than {} s, so it was stopped",
                GIT_TIMEOUT.as_secs()
            )));
        }
    };
    // a failure says what git said of it
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(FetchFailed(format!("{what}: {}", said.trim())));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| FetchFailed(format!("{what}: git printed text that isn't utf-8")))
}

#[cfg(test)]
mod tests {
    use super::{checked, key, symref};

    /// Every spelling of one repo's URL shares a key, and different repos don't
    #[test]
    fn urls_of_one_repo_share_a_key() {
        // scp-like, ssh with a user and port, and https with a trailing slash or `.git`
        let one = [
            "git@example.com:reef/reef.git",
            "ssh://git@example.com:22/reef/reef",
            "https://EXAMPLE.com/reef/reef/",
            "https://example.com/reef/reef.git",
            "http://example.com:8080/reef/reef.git/",
            "git://example.com/reef/reef",
        ];
        let first = key(one[0]).expect("a key");
        assert_eq!(first, "example.com/reef/reef");
        for url in one {
            assert_eq!(key(url).as_deref(), Some(first.as_str()), "{url}");
        }
        // a local repo is its path, by any spelling, and never a host's
        assert_eq!(key("/srv/reef.git"), key("file:///srv/reef"));
        assert_eq!(key("/srv/reef.git").as_deref(), Some("file:/srv/reef"));
        assert_ne!(key("/srv/reef"), key("https://srv/reef"));
        // another repo on the same host, or another host, is another key
        assert_ne!(key("https://example.com/reef/shoal"), Some(first.clone()));
        assert_ne!(key("https://example.org/reef/reef"), Some(first));
        // a path's case is the repo's own
        assert_ne!(
            key("https://example.com/Reef/reef"),
            key("https://example.com/reef/reef")
        );
    }

    /// A URL that would leak a secret, run a command or be read as an option is refused
    #[test]
    fn urls_that_leak_or_run_commands_are_refused() {
        // each transport git runs itself, and the forms it reads as them
        for url in [
            "https://example.com/reef.git",
            "http://example.com/reef.git",
            "ssh://git@example.com/reef.git",
            "git@example.com:reef/reef.git",
            "git://example.com/reef.git",
            "file:///srv/reef.git",
            "/srv/reef.git",
        ] {
            assert!(checked(url).is_ok(), "{url} was refused");
        }
        // a password anywhere, or any user over http(s), where a token is often put
        for url in [
            "https://ada:hunter2@example.com/reef.git",
            "https://ghp_token@example.com/reef.git",
            "ssh://git:hunter2@example.com/reef.git",
        ] {
            let error = checked(url).expect_err(url).to_string();
            assert!(
                !error.contains("hunter2") && !error.contains("ghp_token"),
                "{error}"
            );
        }
        // a transport that runs a command, or a remote helper
        assert!(checked("ext::sh -c touch% /tmp/pwned").is_err());
        assert!(checked("fd::17").is_err());
        assert!(checked("persistent-https://example.com/reef").is_err());
        // a relative path, and anything git or ssh would read as an option
        assert!(checked("../reef").is_err());
        assert!(checked("ssh://-oProxyCommand=touch/reef").is_err());
        assert!(checked("example.com:-reef").is_err());
        // control characters, and a URL too long to be one
        assert!(checked("https://example.com/reef\n.git").is_err());
        let long = format!("https://example.com/{}", "r".repeat(3000));
        assert!(checked(&long).is_err());
    }

    /// A remote's branches are read from the refs `ls-remote --heads` prints
    #[test]
    fn heads_are_read_from_ls_remote() {
        // two branches, one with a slash in its name
        let said = "0123\trefs/heads/main\n4567\trefs/heads/feature/x\n";
        assert_eq!(super::heads_in(said), ["main", "feature/x"]);
        assert!(super::heads_in("").is_empty());
    }

    /// The default branch is read from the symbolic ref `ls-remote` prints, if it prints one
    #[test]
    fn default_branch_is_read_from_ls_remote() {
        // a remote whose HEAD names a branch
        let said = "ref: refs/heads/trunk\tHEAD\n6376987ff439ffc7ac19bb8c1cf7bcd908f5f522\tHEAD\n";
        assert_eq!(symref(said).as_deref(), Some("trunk"));
        // one that names a commit alone, or nothing, names no branch
        assert_eq!(
            symref("6376987ff439ffc7ac19bb8c1cf7bcd908f5f522\tHEAD\n"),
            None
        );
        assert_eq!(symref(""), None);
    }
}
