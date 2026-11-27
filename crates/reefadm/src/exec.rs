//! Every side effect reefadm has goes through one runner
//!
//! A command either reads, as a probe, which runs even in a dry run, or changes something, as a
//! step, which a dry run only shows. Files, the server's health, the admin's answers and their
//! passwords go through the runner too, so a command's whole list of steps can be shown without
//! running it ([`Dry`]) or compared in a test ([`Script`]).
//!
//! - **[`Real`]** runs everything as the admin, putting `sudo` in front only of the steps that
//!   need root or the service's account, and echoes each change as `$ …` before it runs.
//! - **[`Dry`]** runs another runner's probes and prints its changes instead of making them.
//! - **[`Script`]** answers probes from a table and records the changes, for tests.

use anyhow::{Context, Result, bail};
use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The account the service runs as, which owns its data
pub const ACCOUNT: &str = "reef";

/// Whom a command runs as
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum As {
    /// The admin running reefadm
    Me,
    /// root, through sudo unless reefadm is root already
    Root,
    /// The service's account, which owns its data
    Account,
}

/// A command to run
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    /// The program
    pub program: String,
    /// Its arguments
    pub args: Vec<String>,
    /// Variables set for it
    pub env: Vec<(String, String)>,
    /// Variables taken away from it
    pub unset: Vec<String>,
    /// The directory it runs in
    pub cwd: Option<PathBuf>,
    /// Whom it runs as
    pub runs_as: As,
    /// What it reads on standard input instead of the terminal, never shown
    pub stdin: Option<Vec<u8>>,
}

impl Cmd {
    /// A command that runs a program as the admin, with no arguments yet
    ///
    /// # Arguments
    ///
    /// * `program` - The program to run
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::exec::Cmd;
    ///
    /// let cmd = Cmd::new("systemctl").args(["restart", "reef"]).root();
    /// assert_eq!(cmd.shown(), "sudo systemctl restart reef");
    /// ```
    pub fn new(program: impl Into<String>) -> Self {
        Cmd {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            unset: Vec::new(),
            cwd: None,
            runs_as: As::Me,
            stdin: None,
        }
    }

    /// Adds an argument
    ///
    /// # Arguments
    ///
    /// * `arg` - The argument
    pub fn arg(mut self, arg: impl AsRef<std::ffi::OsStr>) -> Self {
        self.args.push(arg.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Adds several arguments
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments, in order
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.args.extend(
            args.into_iter()
                .map(|arg| arg.as_ref().to_string_lossy().into_owned()),
        );
        self
    }

    /// Sets a variable for it
    ///
    /// # Arguments
    ///
    /// * `key` - The variable's name
    /// * `value` - Its value
    pub fn env(mut self, key: impl Into<String>, value: impl AsRef<std::ffi::OsStr>) -> Self {
        self.env
            .push((key.into(), value.as_ref().to_string_lossy().into_owned()));
        self
    }

    /// Takes variables away from it
    ///
    /// # Arguments
    ///
    /// * `keys` - The variables' names
    pub fn unset<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.unset.extend(keys.into_iter().map(Into::into));
        self
    }

    /// Runs it in a directory
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Runs it as root
    pub fn root(mut self) -> Self {
        self.runs_as = As::Root;
        self
    }

    /// Runs it as the service's account
    pub fn account(mut self) -> Self {
        self.runs_as = As::Account;
        self
    }

    /// Runs it as whom a layout's files belong to
    ///
    /// # Arguments
    ///
    /// * `runs_as` - Whom it runs as
    pub fn by(mut self, runs_as: As) -> Self {
        self.runs_as = runs_as;
        self
    }

    /// Feeds it bytes on standard input, which are never shown
    ///
    /// # Arguments
    ///
    /// * `bytes` - What it reads
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    /// The command as an admin would type it: `sudo` where it needs it, set variables, then the
    /// program and its arguments, each quoted for a shell, and the directory it runs in
    pub fn shown(&self) -> String {
        // who runs it comes first, as sudo would be typed
        let mut words: Vec<String> = match self.runs_as {
            As::Me => Vec::new(),
            As::Root => vec!["sudo".into()],
            As::Account => vec!["sudo".into(), "-u".into(), ACCOUNT.into()],
        };
        // then the variables it sets, the program and its arguments
        words.extend(
            self.env
                .iter()
                .map(|(key, value)| format!("{key}={}", quote(value))),
        );
        words.push(quote(&self.program));
        words.extend(self.args.iter().map(|arg| quote(arg)));
        let line = words.join(" ");
        // and where it runs, if not where reefadm does
        match &self.cwd {
            Some(dir) => format!("{line}  (in {})", dir.display()),
            None => line,
        }
    }
}

/// Quotes a word for a POSIX shell, leaving it bare when it holds nothing a shell reads
///
/// # Arguments
///
/// * `word` - The word to quote
///
/// # Examples
///
/// ```
/// use reefadm::exec::quote;
///
/// assert_eq!(quote("reef-serve"), "reef-serve");
/// assert_eq!(quote("it's"), "'it'\\''s'");
/// assert_eq!(quote(""), "''");
/// ```
pub fn quote(word: &str) -> String {
    // a word of safe characters needs no quotes
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./=:+@%,^".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_string();
    }
    // anything else goes in single quotes, each of its own quotes closed, escaped and reopened
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// What a probe printed, and how it ended
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Its exit code, or none if it never ran or a signal ended it
    pub code: Option<i32>,
    /// What it printed on standard output
    pub stdout: String,
    /// What it printed on standard error
    pub stderr: String,
}

impl Output {
    /// A command that succeeded, having printed this
    ///
    /// # Arguments
    ///
    /// * `stdout` - What it printed
    pub fn printed(stdout: impl Into<String>) -> Self {
        Output {
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    /// A command that failed, having said this
    ///
    /// # Arguments
    ///
    /// * `stderr` - What it said
    pub fn failed(stderr: impl Into<String>) -> Self {
        Output {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    /// Whether it exited 0
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// A file to write, whole
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Where it goes
    pub path: PathBuf,
    /// What it holds
    pub text: String,
    /// Its mode, such as 0o644
    pub mode: u32,
    /// Whom it is written as: the admin, or root, who can give it to another owner
    pub runs_as: As,
    /// The user and group that own it, when root writes it
    pub owner: Option<(String, String)>,
}

impl File {
    /// What the step writing it looks like: its path, mode and owner
    pub fn shown(&self) -> String {
        // its owner only when root gives it one
        let owner = match &self.owner {
            Some((user, group)) => format!(", {user}:{group}"),
            None => String::new(),
        };
        let root = match self.runs_as {
            As::Me => "",
            As::Root | As::Account => "sudo ",
        };
        format!(
            "{root}write {} ({:o}{owner})",
            self.path.display(),
            self.mode
        )
    }
}

/// A request for the server's `GET /api/v1/me`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// The route's whole URL
    pub url: String,
    /// The CA to trust beside the system's roots, as PEM
    pub ca: Option<Vec<u8>>,
    /// A name the URL uses, and the address to reach it at instead of asking DNS
    pub resolve: Option<(String, SocketAddr)>,
}

/// What `GET /api/v1/me` answered
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Me {
    /// Whether every caller must sign in
    pub required: bool,
    /// Whom the caller is signed in as, if anyone
    #[serde(default)]
    pub user: Option<MeUser>,
}

/// The user `GET /api/v1/me` names
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct MeUser {
    /// Their name
    pub name: String,
}

/// Everything reefadm does to a host, and everything it asks of the admin
pub trait Runner {
    /// Runs a command that only reads, returning what it printed, even in a dry run
    ///
    /// A program that isn't there comes back as a failure, not an error.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn probe(&mut self, cmd: &Cmd) -> Result<Output>;

    /// Runs a command that changes something, with its output passed through, and fails if it does
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn run(&mut self, cmd: &Cmd) -> Result<()>;

    /// Runs a command that changes something, returning how it ended for the caller to read
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn attempt(&mut self, cmd: &Cmd) -> Result<Output>;

    /// Runs a command on the terminal, as the admin would, returning its exit code
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn interact(&mut self, cmd: &Cmd) -> Result<Option<i32>>;

    /// Writes a file whole
    ///
    /// # Arguments
    ///
    /// * `file` - The file
    fn write(&mut self, file: &File) -> Result<()>;

    /// Asks the server who the caller is, which says it is up
    ///
    /// # Arguments
    ///
    /// * `ask` - Where to ask, and what to trust
    fn me(&mut self, ask: &Ask) -> Result<Me>;

    /// Makes sure root's steps can run without asking again for a while
    fn elevate(&mut self) -> Result<()>;

    /// Asks the admin a yes or no question
    ///
    /// # Arguments
    ///
    /// * `question` - What to ask
    fn confirm(&mut self, question: &str) -> Result<bool>;

    /// Reads a new password: from a file, else asked twice on the terminal
    ///
    /// # Arguments
    ///
    /// * `prompt` - What to ask on the terminal
    /// * `file` - The file to read it from instead
    fn password(&mut self, prompt: &str, file: Option<&Path>) -> Result<String>;

    /// Says something to the admin
    ///
    /// # Arguments
    ///
    /// * `line` - What to say
    fn say(&mut self, line: &str);

    /// Waits a moment before looking again
    ///
    /// # Arguments
    ///
    /// * `duration` - How long
    fn sleep(&mut self, duration: Duration);

    /// Whether changes are only shown, not made
    fn dry(&self) -> bool;
}

/// Runs everything for real, as the admin, with sudo only where a step needs it
pub struct Real {
    /// Whether reefadm runs as root already, so sudo isn't needed
    root: bool,
    /// Whether a terminal is there to answer sudo and reefadm's questions
    terminal: bool,
    /// The PATH every command gets: cargo's bin first, then the admin's
    path: String,
    /// The private directory root's files are staged in before they're installed
    stage: Option<PathBuf>,
    /// How many files have been staged, so each has a name of its own
    staged: usize,
}

impl Real {
    /// A runner for this process: its user, its terminal and its PATH
    ///
    /// # Arguments
    ///
    /// * `env` - reefadm's environment
    pub fn new(env: &crate::Env) -> Self {
        // root needs no sudo
        // SAFETY: geteuid has no preconditions and can't fail
        let root = unsafe { libc::geteuid() } == 0;
        // sudo, and reefadm's own questions, need a terminal to ask on
        let terminal = std::io::stdin().is_terminal();
        Real {
            root,
            terminal,
            path: crate::layout::path_with_cargo(env),
            stage: None,
            staged: 0,
        }
    }

    /// The process a command runs as: through sudo or runuser where it needs another account
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn command(&self, cmd: &Cmd) -> Command {
        // who runs it: the admin as they are, or root or the account through sudo
        let mut argv: Vec<String> = Vec::new();
        let quiet = if self.terminal { None } else { Some("-n") };
        match (cmd.runs_as, self.root) {
            (As::Me, _) | (As::Root, true) => {}
            (As::Root, false) => argv.extend(["sudo"].into_iter().chain(quiet).map(String::from)),
            (As::Account, true) => {
                argv.extend(["runuser", "-u", ACCOUNT, "--"].map(String::from));
            }
            (As::Account, false) => argv.extend(
                ["sudo"]
                    .into_iter()
                    .chain(quiet)
                    .chain(["-u", ACCOUNT])
                    .map(String::from),
            ),
        }
        // sudo clears the environment, so another account's variables go through env
        let other = !argv.is_empty();
        if other && !cmd.env.is_empty() {
            argv.push("env".into());
            argv.extend(cmd.env.iter().map(|(key, value)| format!("{key}={value}")));
        }
        argv.push(cmd.program.clone());
        argv.extend(cmd.args.iter().cloned());
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        // the admin's own commands get cargo's bin on their PATH, and their variables
        if !other {
            for key in &cmd.unset {
                command.env_remove(key);
            }
            command.env("PATH", &self.path);
            command.envs(cmd.env.iter().map(|(key, value)| (key, value)));
        }
        if let Some(dir) = &cmd.cwd {
            command.current_dir(dir);
        }
        command
    }

    /// Runs a command to its end, capturing what it printed or passing it through
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    /// * `capture` - Whether to keep its output rather than show it
    fn exec(&self, cmd: &Cmd, capture: bool) -> Result<Output> {
        let mut command = self.command(cmd);
        // its input: the bytes it is given, else nothing for a probe, else the terminal
        command.stdin(match (&cmd.stdin, capture) {
            (Some(_), _) => Stdio::piped(),
            (None, true) => Stdio::null(),
            (None, false) => Stdio::inherit(),
        });
        let (out, err) = match capture {
            true => (Stdio::piped(), Stdio::piped()),
            false => (Stdio::inherit(), Stdio::inherit()),
        };
        command.stdout(out).stderr(err);
        // a program that isn't there is a failure for the caller to read
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Output {
                    code: None,
                    stdout: String::new(),
                    stderr: format!("{} is not installed", cmd.program),
                });
            }
            Err(error) => return Err(error).with_context(|| format!("running `{}`", cmd.shown())),
        };
        // its input, then its end
        if let (Some(bytes), Some(mut stdin)) = (&cmd.stdin, child.stdin.take()) {
            stdin
                .write_all(bytes)
                .with_context(|| format!("writing to `{}`", cmd.shown()))?;
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("running `{}`", cmd.shown()))?;
        Ok(Output {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// The private directory root's files are staged in, made the first time it is needed
    fn stage(&mut self) -> Result<PathBuf> {
        if let Some(dir) = &self.stage {
            return Ok(dir.clone());
        }
        // a directory only the admin can read, since a staged file may hold a key
        let dir = std::env::temp_dir().join(format!("reefadm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        set_mode(&dir, 0o700)?;
        self.stage = Some(dir.clone());
        Ok(dir)
    }
}

impl Drop for Real {
    /// Removes the staging directory, and whatever is still in it
    fn drop(&mut self) {
        if let Some(dir) = &self.stage {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Sets a path's mode
///
/// # Arguments
///
/// * `path` - The path
/// * `mode` - Its new mode, such as 0o600
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting {}'s mode", path.display()))
}

impl Runner for Real {
    /// Runs a probe, capturing its output
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn probe(&mut self, cmd: &Cmd) -> Result<Output> {
        self.exec(cmd, true)
    }

    /// Echoes a step, runs it with its output shown, and fails if it does
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn run(&mut self, cmd: &Cmd) -> Result<()> {
        self.say(&format!("$ {}", cmd.shown()));
        let output = self.exec(cmd, false)?;
        match output.ok() {
            true => Ok(()),
            false if output.code.is_none() && !output.stderr.is_empty() => {
                bail!("`{}` failed: {}", cmd.shown(), output.stderr)
            }
            false => bail!(
                "`{}` failed{}",
                cmd.shown(),
                output
                    .code
                    .map(|code| format!(" with exit code {code}"))
                    .unwrap_or_default()
            ),
        }
    }

    /// Echoes a step, runs it capturing its output, and shows what it printed
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn attempt(&mut self, cmd: &Cmd) -> Result<Output> {
        self.say(&format!("$ {}", cmd.shown()));
        let output = self.exec(cmd, true)?;
        // what it said is still the admin's to see
        print!("{}", output.stdout);
        eprint!("{}", output.stderr);
        Ok(output)
    }

    /// Echoes a step, runs it on the terminal, and returns its exit code
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn interact(&mut self, cmd: &Cmd) -> Result<Option<i32>> {
        self.say(&format!("$ {}", cmd.shown()));
        let output = self.exec(cmd, false)?;
        if output.code.is_none() && !output.stderr.is_empty() {
            bail!("`{}` failed: {}", cmd.shown(), output.stderr);
        }
        Ok(output.code)
    }

    /// Writes a file in place, through a staged copy and `install` when root writes it
    ///
    /// # Arguments
    ///
    /// * `file` - The file
    fn write(&mut self, file: &File) -> Result<()> {
        match file.runs_as {
            // the admin's own file, written beside itself and moved over the old one
            As::Me => {
                self.say(&format!("$ {}", file.shown()));
                if let Some(parent) = file.path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("making {}", parent.display()))?;
                }
                let name = file
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let next = file.path.with_file_name(format!(".{name}.reefadm"));
                std::fs::write(&next, &file.text)
                    .with_context(|| format!("writing {}", next.display()))?;
                set_mode(&next, file.mode)?;
                std::fs::rename(&next, &file.path)
                    .with_context(|| format!("writing {}", file.path.display()))
            }
            // root's, staged where only the admin can read it, then installed with its owner
            As::Root | As::Account => {
                self.staged += 1;
                let staged = self.stage()?.join(format!("file-{}", self.staged));
                std::fs::write(&staged, &file.text)
                    .with_context(|| format!("staging {}", file.path.display()))?;
                set_mode(&staged, 0o600)?;
                let mut install = Cmd::new("install")
                    .args(["-D", "-m", &format!("{:o}", file.mode)])
                    .root();
                if let Some((user, group)) = &file.owner {
                    install = install.args(["-o", user, "-g", group]);
                }
                let install = install.arg(&staged).arg(&file.path);
                let done = self.run(&install);
                let _ = std::fs::remove_file(&staged);
                done
            }
        }
    }

    /// Asks the server who the caller is, once, with short timeouts
    ///
    /// # Arguments
    ///
    /// * `ask` - Where to ask, and what to trust
    fn me(&mut self, ask: &Ask) -> Result<Me> {
        // short timeouts, since the port takes connections before the store is up
        let mut builder = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4));
        if let Some(ca) = &ask.ca {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(ca).context("reading reef's CA")?,
            );
        }
        if let Some((name, address)) = &ask.resolve {
            builder = builder.resolve(name, *address);
        }
        let client = builder
            .build()
            .context("building the health check's client")?;
        // the route answers without signing in
        let me = client
            .get(&ask.url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .with_context(|| format!("asking {}", ask.url))?
            .json::<Me>()
            .with_context(|| format!("reading {}'s answer", ask.url))?;
        Ok(me)
    }

    /// Asks for sudo's password now, so root's steps that follow don't stop to ask
    fn elevate(&mut self) -> Result<()> {
        if self.root {
            return Ok(());
        }
        // sudo that needs no password, or has one still fresh, asks nothing; `-v` would, since
        // it wants every sudoers entry of the admin's to be NOPASSWD, as a cloud image's isn't
        if self
            .exec(&Cmd::new("sudo").args(["-n", "true"]), true)?
            .ok()
        {
            return Ok(());
        }
        // else it asks now, on the terminal, if there is one
        let asked = match self.terminal {
            true => self.exec(&Cmd::new("sudo").arg("-v"), false)?.ok(),
            false => false,
        };
        match asked {
            true => Ok(()),
            false => bail!(
                "reefadm needs sudo for this, and sudo refused{}",
                match self.terminal {
                    true => String::new(),
                    false => " without a terminal to ask on: run it from one".to_string(),
                }
            ),
        }
    }

    /// Asks on the terminal, refusing when there is none
    ///
    /// # Arguments
    ///
    /// * `question` - What to ask
    fn confirm(&mut self, question: &str) -> Result<bool> {
        if !self.terminal {
            bail!("{question}: there's no terminal to ask on, so pass --yes to say yes");
        }
        // anything but yes is no
        print!("{question} [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
    }

    /// Reads a password as reef's CLI does: from the file, else asked twice with echo off
    ///
    /// # Arguments
    ///
    /// * `prompt` - What to ask on the terminal
    /// * `file` - The file to read it from instead
    fn password(&mut self, prompt: &str, file: Option<&Path>) -> Result<String> {
        if file.is_none() && !self.terminal {
            bail!("{prompt}: there's no terminal to ask on, so give --password-file");
        }
        reef_api::prompt::read_new_password(prompt, file).context("reading the password")
    }

    /// Prints a line
    ///
    /// # Arguments
    ///
    /// * `line` - What to say
    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    /// Sleeps
    ///
    /// # Arguments
    ///
    /// * `duration` - How long
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    /// Changes are made
    fn dry(&self) -> bool {
        false
    }
}

/// Runs another runner's probes, and only shows its changes
pub struct Dry<R> {
    /// The runner whose probes run
    pub inner: R,
}

impl<R: Runner> Runner for Dry<R> {
    /// Runs the probe for real
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn probe(&mut self, cmd: &Cmd) -> Result<Output> {
        self.inner.probe(cmd)
    }

    /// Shows the step
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn run(&mut self, cmd: &Cmd) -> Result<()> {
        self.inner.say(&format!("would run: {}", cmd.shown()));
        Ok(())
    }

    /// Shows the step, as if it succeeded
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn attempt(&mut self, cmd: &Cmd) -> Result<Output> {
        self.inner.say(&format!("would run: {}", cmd.shown()));
        Ok(Output::printed(""))
    }

    /// Shows the step, as if it succeeded
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn interact(&mut self, cmd: &Cmd) -> Result<Option<i32>> {
        self.inner.say(&format!("would run: {}", cmd.shown()));
        Ok(Some(0))
    }

    /// Shows the file, and what it would hold
    ///
    /// # Arguments
    ///
    /// * `file` - The file
    fn write(&mut self, file: &File) -> Result<()> {
        self.inner.say(&format!("would {}:", file.shown()));
        for line in file.text.lines() {
            self.inner.say(&format!("    {line}"));
        }
        Ok(())
    }

    /// Shows where it would ask, and answers as a server asking callers to sign in
    ///
    /// # Arguments
    ///
    /// * `ask` - Where to ask
    fn me(&mut self, ask: &Ask) -> Result<Me> {
        self.inner
            .say(&format!("would wait for {} to answer", ask.url));
        Ok(Me {
            required: true,
            user: None,
        })
    }

    /// Nothing runs as root, so nothing needs sudo
    fn elevate(&mut self) -> Result<()> {
        Ok(())
    }

    /// Shows the question, and goes on as if it were answered yes
    ///
    /// # Arguments
    ///
    /// * `question` - What would be asked
    fn confirm(&mut self, question: &str) -> Result<bool> {
        self.inner.say(&format!("would ask: {question}"));
        Ok(true)
    }

    /// Shows that it would ask, and asks nothing
    ///
    /// # Arguments
    ///
    /// * `prompt` - What would be asked
    /// * `file` - The file it would be read from instead
    fn password(&mut self, prompt: &str, file: Option<&Path>) -> Result<String> {
        match file {
            Some(file) => self
                .inner
                .say(&format!("would read the password from {}", file.display())),
            None => self.inner.say(&format!("would ask: {prompt}")),
        }
        Ok(String::new())
    }

    /// Says it through the inner runner
    ///
    /// # Arguments
    ///
    /// * `line` - What to say
    fn say(&mut self, line: &str) {
        self.inner.say(line);
    }

    /// Doesn't wait, since nothing it waits for is happening
    ///
    /// # Arguments
    ///
    /// * `duration` - How long it would wait
    fn sleep(&mut self, _duration: Duration) {}

    /// Changes are only shown
    fn dry(&self) -> bool {
        true
    }
}

/// Answers probes from a table and records changes, for tests
#[derive(Debug, Default)]
pub struct Script {
    /// Each probe's answer, by the command as shown; a key ending in `*` answers every command
    /// it starts
    pub answers: Vec<(String, Output)>,
    /// What each attempt answers, by the command as shown, else success
    pub attempts: Vec<(String, Output)>,
    /// What the server answers each `me`, in turn, else a server asking callers to sign in
    pub mes: VecDeque<std::result::Result<Me, String>>,
    /// The answer to every question
    pub yes: bool,
    /// The password every prompt gets
    pub secret: String,
    /// Every change, in order: `run: …`, `write: …`, `me: …`
    pub log: Vec<String>,
    /// Every file written, whole
    pub files: Vec<File>,
    /// Every probe, as shown
    pub probed: Vec<String>,
    /// Everything said
    pub said: Vec<String>,
}

impl Script {
    /// Answers a probe, or every probe a pattern ending in `*` starts
    ///
    /// # Arguments
    ///
    /// * `shown` - The command as shown
    /// * `output` - Its answer
    pub fn answer(&mut self, shown: impl Into<String>, output: Output) -> &mut Self {
        self.answers.push((shown.into(), output));
        self
    }

    /// Answers a probe with what it printed, succeeding
    ///
    /// # Arguments
    ///
    /// * `shown` - The command as shown
    /// * `stdout` - What it printed
    pub fn prints(&mut self, shown: impl Into<String>, stdout: impl Into<String>) -> &mut Self {
        self.answer(shown, Output::printed(stdout))
    }

    /// The changes run or written, without the server's answers
    pub fn steps(&self) -> Vec<&str> {
        self.log
            .iter()
            .filter(|line| !line.starts_with("me: "))
            .map(String::as_str)
            .collect()
    }

    /// The answer a table holds for a command, the latest first, so a test can change one
    ///
    /// # Arguments
    ///
    /// * `table` - The answers
    /// * `shown` - The command as shown
    fn find(table: &[(String, Output)], shown: &str) -> Option<Output> {
        table
            .iter()
            .rev()
            .find(|(key, _)| match key.strip_suffix('*') {
                Some(prefix) => shown.starts_with(prefix),
                None => key == shown,
            })
            .map(|(_, output)| output.clone())
    }
}

impl Runner for Script {
    /// Answers from the table, else as a command that failed
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn probe(&mut self, cmd: &Cmd) -> Result<Output> {
        let shown = cmd.shown();
        self.probed.push(shown.clone());
        Ok(Script::find(&self.answers, &shown).unwrap_or_else(|| Output::failed("")))
    }

    /// Records the step
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn run(&mut self, cmd: &Cmd) -> Result<()> {
        self.log.push(format!("run: {}", cmd.shown()));
        Ok(())
    }

    /// Records the step, answering from the attempts' table, else as a success
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn attempt(&mut self, cmd: &Cmd) -> Result<Output> {
        let shown = cmd.shown();
        self.log.push(format!("run: {shown}"));
        Ok(Script::find(&self.attempts, &shown).unwrap_or_else(|| Output::printed("")))
    }

    /// Records the step, ending as the attempts' table says, else with 0
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command
    fn interact(&mut self, cmd: &Cmd) -> Result<Option<i32>> {
        let shown = cmd.shown();
        self.log.push(format!("run: {shown}"));
        Ok(Script::find(&self.attempts, &shown).map_or(Some(0), |output| output.code))
    }

    /// Records the file
    ///
    /// # Arguments
    ///
    /// * `file` - The file
    fn write(&mut self, file: &File) -> Result<()> {
        self.log.push(format!("write: {}", file.shown()));
        self.files.push(file.clone());
        Ok(())
    }

    /// Answers the next of the server's answers
    ///
    /// # Arguments
    ///
    /// * `ask` - Where it asks
    fn me(&mut self, ask: &Ask) -> Result<Me> {
        self.log.push(format!("me: {}", ask.url));
        match self.mes.pop_front() {
            Some(Ok(me)) => Ok(me),
            Some(Err(error)) => bail!("asking {}: {error}", ask.url),
            None => Ok(Me {
                required: true,
                user: None,
            }),
        }
    }

    /// Needs nothing
    fn elevate(&mut self) -> Result<()> {
        Ok(())
    }

    /// Answers as the script says
    ///
    /// # Arguments
    ///
    /// * `question` - What was asked
    fn confirm(&mut self, question: &str) -> Result<bool> {
        self.said.push(format!("asked: {question}"));
        Ok(self.yes)
    }

    /// Answers with the script's password
    ///
    /// # Arguments
    ///
    /// * `prompt` - What was asked
    /// * `file` - The file it would be read from
    fn password(&mut self, _prompt: &str, _file: Option<&Path>) -> Result<String> {
        Ok(self.secret.clone())
    }

    /// Keeps what was said
    ///
    /// # Arguments
    ///
    /// * `line` - What was said
    fn say(&mut self, line: &str) {
        self.said.push(line.to_string());
    }

    /// Doesn't wait
    ///
    /// # Arguments
    ///
    /// * `duration` - How long it would wait
    fn sleep(&mut self, _duration: Duration) {}

    /// Changes are recorded as if made
    fn dry(&self) -> bool {
        false
    }
}

/// Reads a whole file the admin can read, through a probe, or none if it isn't there
///
/// # Arguments
///
/// * `runner` - The runner
/// * `path` - The file
/// * `runs_as` - Whom it is read as
pub fn read(runner: &mut dyn Runner, path: &Path, runs_as: As) -> Result<Option<String>> {
    // `cat` reads it as whoever may, so a probe of root's file works as it would for the admin
    let output = runner.probe(&Cmd::new("cat").arg(path).by(runs_as))?;
    Ok(output.ok().then_some(output.stdout))
}

/// Whether a path exists, through a probe
///
/// # Arguments
///
/// * `runner` - The runner
/// * `path` - The path
/// * `runs_as` - Whom it is looked at as
pub fn exists(runner: &mut dyn Runner, path: &Path, runs_as: As) -> Result<bool> {
    Ok(runner
        .probe(&Cmd::new("test").args(["-e"]).arg(path).by(runs_as))?
        .ok())
}

#[cfg(test)]
mod tests {
    use super::{Cmd, Dry, Output, Runner, Script, quote};

    /// A command shows as typed: sudo, its variables, its words quoted, and where it runs
    #[test]
    fn commands_show_as_typed() {
        // root's and the account's go through sudo
        let cmd = Cmd::new("install").args(["-d", "/var/lib/reef"]).root();
        assert_eq!(cmd.shown(), "sudo install -d /var/lib/reef");
        let cmd = Cmd::new("reef-serve").args(["recover", "ada"]).account();
        assert_eq!(cmd.shown(), "sudo -u reef reef-serve recover ada");
        // a variable and a word with a space, in a directory
        let cmd = Cmd::new("dx")
            .env("RUSTUP_TOOLCHAIN", "nightly")
            .args(["build", "a b"])
            .cwd("/src");
        assert_eq!(
            cmd.shown(),
            "RUSTUP_TOOLCHAIN=nightly dx build 'a b'  (in /src)"
        );
    }

    /// A word is quoted only when a shell would read something in it
    #[test]
    fn words_are_quoted_only_when_needed() {
        assert_eq!(quote("--features"), "--features");
        assert_eq!(quote("@server"), "@server");
        assert_eq!(quote("a;b"), "'a;b'");
        assert_eq!(quote("$HOME"), "'$HOME'");
        assert_eq!(quote("it's"), "'it'\\''s'");
    }

    /// A dry run runs probes but none of the changes, and says each
    #[test]
    fn a_dry_run_shows_changes_and_makes_none() {
        // a probe is answered by the runner under it
        let mut script = Script::default();
        script.prints("uname -s", "Linux\n");
        let mut dry = Dry { inner: script };
        let probed = dry.probe(&Cmd::new("uname").arg("-s")).unwrap();
        assert_eq!(probed, Output::printed("Linux\n"));
        // a step and a file are only said
        dry.run(&Cmd::new("useradd").arg("reef").root()).unwrap();
        dry.write(&super::File {
            path: "/etc/reef/config.toml".into(),
            text: "[auth]\nrequired = true\n".into(),
            mode: 0o644,
            runs_as: super::As::Root,
            owner: None,
        })
        .unwrap();
        assert!(dry.inner.log.is_empty(), "{:?}", dry.inner.log);
        assert_eq!(
            dry.inner.said,
            [
                "would run: sudo useradd reef",
                "would sudo write /etc/reef/config.toml (644):",
                "    [auth]",
                "    required = true",
            ]
        );
    }
}
