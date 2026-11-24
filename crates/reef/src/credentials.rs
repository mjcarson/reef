//! Where `reef` keeps the tokens it signs in with, a token for each server it logged in to
//!
//! `reef login` trades a user's name and password for a token, and keeps it here, never the
//! password. Every request then sends it, from the CLI, the MCP server and the hooks alike, so
//! one login signs in every `reef` this user runs on this host.
//!
//! - **Picked in one order** ([`pick`]): a file named by `--credentials`, else a token in
//!   `$REEF_TOKEN`, else a file named by `$REEF_CREDENTIALS`, else
//!   `$XDG_CONFIG_HOME/reef/credentials.toml`, beside reef's config.
//! - **A token a server, by the server's root**, such as `http://127.0.0.1:12180`, so a login to
//!   one server never signs in to another.
//! - **Read for each request,** so a server or a runner started before `reef login` sends the
//!   token once there is one.
//! - **Private.** The file is written whole, readable by its owner alone, in a directory only
//!   its owner can enter, and swapped in by a rename, so it is never seen half-written.
//!
//! ```toml
//! [servers."http://127.0.0.1:12180"]
//! user = "ada"
//! token = "reef_…"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The environment variable that gives a token outright, winning over any credentials file but
/// one `--credentials` names
pub const TOKEN_VAR: &str = "REEF_TOKEN";

/// The environment variable that names a credentials file
pub const CREDENTIALS_VAR: &str = "REEF_CREDENTIALS";

/// The credentials file's name, beside reef's config
const FILE_NAME: &str = "credentials.toml";

/// Where a client's token comes from
#[derive(Clone, PartialEq, Eq)]
pub enum Held {
    /// A token given outright, in `$REEF_TOKEN`
    Token(String),
    /// A credentials file, which may hold a token for the server
    File(PathBuf),
    /// Nowhere: no file was named, and there is no home to keep one in
    Nowhere,
}

impl std::fmt::Debug for Held {
    /// Shows where the token comes from, with a token given outright left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Held::Token(_) => f.write_str("Token(…)"),
            Held::File(path) => f.debug_tuple("File").field(path).finish(),
            Held::Nowhere => f.write_str("Nowhere"),
        }
    }
}

impl Held {
    /// Where the token comes from, as a message names it
    pub fn describe(&self) -> String {
        match self {
            Held::Token(_) => format!("`${TOKEN_VAR}`"),
            Held::File(path) => path.display().to_string(),
            Held::Nowhere => "nowhere".to_string(),
        }
    }
}

/// Every token kept, a server each
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// Each server's login, by the server's root
    #[serde(default)]
    pub servers: BTreeMap<String, Login>,
}

/// One server's login: who it signs in, and the token that does
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    /// The user's name
    pub user: String,
    /// The token the server issued
    pub token: String,
}

impl std::fmt::Debug for Login {
    /// Shows the login with its token left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("user", &self.user)
            .field("token", &"…")
            .finish()
    }
}

/// Picks where a client's token comes from, first match winning
///
/// # Arguments
///
/// * `named` - The file `--credentials` named, if it named one
/// * `env_token` - `$REEF_TOKEN`, if it is set
/// * `env_file` - `$REEF_CREDENTIALS`, if it is set
/// * `default` - The default file, if there is a home to keep it in
///
/// # Examples
///
/// ```
/// use reef::credentials::{Held, pick};
/// use std::path::PathBuf;
///
/// let held = pick(None, None, None, Some(PathBuf::from("/home/ada/.config/reef/credentials.toml")));
/// assert!(matches!(held, Held::File(_)));
/// ```
pub fn pick(
    named: Option<&Path>,
    env_token: Option<String>,
    env_file: Option<PathBuf>,
    default: Option<PathBuf>,
) -> Held {
    // a file named on the command line wins, so a test or a script decides for itself
    if let Some(named) = named {
        return Held::File(named.to_path_buf());
    }
    // then a token given outright, unless it is empty
    if let Some(token) = env_token.map(|token| token.trim().to_string())
        && !token.is_empty()
    {
        return Held::Token(token);
    }
    // then a file the environment names, else the default beside reef's config
    match env_file.or(default) {
        Some(file) => Held::File(file),
        None => Held::Nowhere,
    }
}

/// Where a client's token comes from, by the command line, the environment and the default
///
/// # Arguments
///
/// * `named` - The file `--credentials` named, if it named one
pub fn held(named: Option<&Path>) -> Held {
    // the environment's, read once here
    let env_token = std::env::var(TOKEN_VAR).ok();
    let env_file = std::env::var_os(CREDENTIALS_VAR)
        .filter(|file| !file.is_empty())
        .map(PathBuf::from);
    pick(named, env_token, env_file, default_path())
}

/// The default credentials file: `credentials.toml` beside reef's default config
pub fn default_path() -> Option<PathBuf> {
    reef_api::config::default_path().map(|config| config.with_file_name(FILE_NAME))
}

/// Reads a credentials file, or none kept if it isn't there
///
/// # Arguments
///
/// * `path` - The file
pub fn read(path: &Path) -> anyhow::Result<Credentials> {
    // a file not written yet keeps no token
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Credentials::default());
        }
        Err(error) => anyhow::bail!("reading {}: {error}", path.display()),
    };
    toml::from_str(&text).map_err(|error| anyhow::anyhow!("parsing {}: {error}", path.display()))
}

/// The login a credentials file keeps for a server, if it keeps one
///
/// # Arguments
///
/// * `path` - The file
/// * `base` - The server's root, such as `http://127.0.0.1:12180`
pub fn login_for(path: &Path, base: &str) -> anyhow::Result<Option<Login>> {
    Ok(read(path)?.servers.remove(base))
}

/// Keeps a server's login, replacing any it kept before
///
/// # Arguments
///
/// * `path` - The file
/// * `base` - The server's root
/// * `login` - Who it signs in, and the token
pub fn keep(path: &Path, base: &str, login: Login) -> anyhow::Result<()> {
    // every other server's login stays as it was
    let mut credentials = read(path)?;
    credentials.servers.insert(base.to_string(), login);
    write(path, &credentials)
}

/// Forgets a server's login, returning whether one was kept
///
/// # Arguments
///
/// * `path` - The file
/// * `base` - The server's root
pub fn forget(path: &Path, base: &str) -> anyhow::Result<bool> {
    // nothing kept is nothing to forget, and leaves no file written
    let mut credentials = read(path)?;
    if credentials.servers.remove(base).is_none() {
        return Ok(false);
    }
    write(path, &credentials)?;
    Ok(true)
}

/// Writes a credentials file whole, private to its owner, swapped in by a rename
///
/// # Arguments
///
/// * `path` - The file
/// * `credentials` - Every login to keep
fn write(path: &Path, credentials: &Credentials) -> anyhow::Result<()> {
    // a directory only its owner can enter, made if it isn't there
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    private_dir(dir)?;
    // the whole file beside it first, readable by its owner alone
    let text = toml::to_string(credentials)?;
    let staged = dir.join(format!(".{FILE_NAME}.{}", std::process::id()));
    let mut file = private_file(&staged)?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| anyhow::anyhow!("writing {}: {error}", staged.display()))?;
    // then swapped in, so a reader sees the old file or the new one
    std::fs::rename(&staged, path).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        anyhow::anyhow!("replacing {}: {error}", path.display())
    })
}

/// Makes a directory only its owner can enter, if it isn't there
///
/// # Arguments
///
/// * `dir` - The directory
fn private_dir(dir: &Path) -> anyhow::Result<()> {
    // one there already is left as its owner set it
    if dir.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| anyhow::anyhow!("making {}: {error}", dir.display()))
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
            .map_err(|error| anyhow::anyhow!("making {}: {error}", dir.display()))
    }
}

/// Opens a new file readable by its owner alone, replacing one left at its path
///
/// # Arguments
///
/// * `path` - The file
fn private_file(path: &Path) -> anyhow::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // its owner alone may read it, from the moment it exists
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| anyhow::anyhow!("writing {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{Held, Login, forget, keep, login_for, pick, read};
    use std::path::{Path, PathBuf};

    /// A scratch directory for one test, empty
    ///
    /// # Arguments
    ///
    /// * `name` - The test's name
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reef-credentials-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A file named on the command line wins, then a token given outright, then the files
    #[test]
    fn the_named_file_wins_then_the_env() {
        let named = Path::new("/named.toml");
        let env_file = || Some(PathBuf::from("/env.toml"));
        let default = || Some(PathBuf::from("/default.toml"));
        // the command line's file, over everything the environment says
        assert_eq!(
            pick(Some(named), Some("reef_t".into()), env_file(), default()),
            Held::File(named.to_path_buf())
        );
        // a token outright, over any file the environment names
        assert_eq!(
            pick(None, Some("reef_t".into()), env_file(), default()),
            Held::Token("reef_t".into())
        );
        // an empty token is none
        assert_eq!(
            pick(None, Some(" ".into()), env_file(), default()),
            Held::File("/env.toml".into())
        );
        // the default file last, and nowhere with no home
        assert_eq!(
            pick(None, None, None, default()),
            Held::File("/default.toml".into())
        );
        assert_eq!(pick(None, None, None, None), Held::Nowhere);
        // a token given outright isn't shown
        assert_eq!(
            format!("{:?}", Held::Token("reef_secret".into())),
            "Token(…)"
        );
    }

    /// Tokens are kept a server each, private to their owner, and forgotten a server at a time
    #[test]
    fn tokens_are_kept_by_server_and_private() {
        let dir = scratch("kept");
        let file = dir.join("reef/credentials.toml");
        // nothing kept reads as no login
        assert_eq!(read(&file).unwrap().servers.len(), 0);
        assert_eq!(login_for(&file, "http://a").unwrap(), None);
        // two servers' logins, each read back alone
        let ada = Login {
            user: "ada".into(),
            token: "reef_a".into(),
        };
        let bob = Login {
            user: "bob".into(),
            token: "reef_b".into(),
        };
        keep(&file, "http://a", ada.clone()).unwrap();
        keep(&file, "http://b", bob.clone()).unwrap();
        assert_eq!(login_for(&file, "http://a").unwrap(), Some(ada));
        assert_eq!(login_for(&file, "http://b").unwrap(), Some(bob.clone()));
        // its owner alone reads the file, and enters its directory
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&file), 0o600);
            assert_eq!(mode(file.parent().unwrap()), 0o700);
        }
        // and no staged copy is left beside it
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1);
        // forgetting one server leaves the other
        assert!(forget(&file, "http://a").unwrap());
        assert!(!forget(&file, "http://a").unwrap());
        assert_eq!(login_for(&file, "http://a").unwrap(), None);
        assert_eq!(login_for(&file, "http://b").unwrap(), Some(bob));
        // a login shows no token
        assert!(!format!("{:?}", read(&file).unwrap()).contains("reef_b"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
