//! `reefadm --host HOST …`: a command on another host over ssh, its prompts on this terminal
//!
//! reefadm reaches the host with the system's `ssh`, so `~/.ssh/config`, the agent and ProxyJump
//! all apply, and copies this binary to `~/.cache/reefadm/reefadm` there, sent again over ssh's
//! input when it differs. The copy must be built for plain x86-64, so it runs on any target and
//! asks the target's own CPU what it has: a reefadm built with `-C target-cpu=native`, as an
//! exported `RUSTFLAGS` builds it, is refused before it is copied.
//!
//! `deploy`, `deploy --adopt` and `upgrade` are built here and installed there
//! ([`crate::commands::hosted`]). Every other command runs there as it would here, with every
//! argument but `--host`, and a terminal when this one has one, for sudo's prompts and reef's.
//! Paths name files on HOST, but for `--password-file`, which is read here and sent to a file only
//! the admin can read there, removed once the command is done.

use crate::exec::{Cmd, quote};
use crate::{Cx, Exit, sources};
use anyhow::{Context, Result, bail};
use std::io::IsTerminal;

/// Where reefadm's copy is kept on the other host, under its home
const COPY: &str = ".cache/reefadm/reefadm";

/// The arguments to run on the other host: every one but `--host` and its value
///
/// # Arguments
///
/// * `args` - The command line's arguments, after the program's name
///
/// # Examples
///
/// ```
/// use reefadm::remote::forwarded;
///
/// let args = ["--host", "europa", "deploy", "--name=reef.example.com"].map(String::from);
/// assert_eq!(forwarded(&args), ["deploy", "--name=reef.example.com"]);
/// let args = ["--dry-run", "--host=europa", "status"].map(String::from);
/// assert_eq!(forwarded(&args), ["--dry-run", "status"]);
/// ```
pub fn forwarded(args: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        // `--host X` drops both, `--host=X` the one; anything after `--` is kept as it is
        match arg.as_str() {
            "--host" => {
                args.next();
            }
            "--" => {
                kept.push(arg.clone());
                kept.extend(args.by_ref().cloned());
            }
            arg if arg.starts_with("--host=") => {}
            _ => kept.push(arg.clone()),
        }
    }
    kept
}

/// The value a flag is given among some arguments, as `--flag X` or `--flag=X`
///
/// # Arguments
///
/// * `args` - The arguments
/// * `flag` - The flag, such as `--repo`
fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let prefix = format!("{flag}=");
    args.iter()
        .enumerate()
        .find_map(|(at, arg)| match arg.strip_prefix(&prefix) {
            Some(value) => Some(value),
            None if arg == flag => args.get(at + 1).map(String::as_str),
            None => None,
        })
}

/// The arguments with a flag's value replaced, in whichever spelling it was given
///
/// # Arguments
///
/// * `args` - The arguments
/// * `flag` - The flag
/// * `to` - Its new value
fn replaced(args: &[String], flag: &str, to: &str) -> Vec<String> {
    let prefix = format!("{flag}=");
    let mut out = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == flag {
            args.next();
            out.extend([flag.to_string(), to.to_string()]);
        } else if arg.starts_with(&prefix) {
            out.push(format!("{prefix}{to}"));
        } else {
            out.push(arg.clone());
        }
    }
    out
}

/// The command line the other host's shell runs: reefadm and each argument, quoted for it
///
/// # Arguments
///
/// * `reefadm` - reefadm's path there
/// * `args` - The arguments to run it with
///
/// # Examples
///
/// ```
/// use reefadm::remote::line;
///
/// let args = ["deploy", "--name", "it's here"].map(String::from);
/// assert_eq!(line("/home/ada/.cache/reefadm/reefadm", &args), "/home/ada/.cache/reefadm/reefadm deploy --name 'it'\\''s here'");
/// ```
pub fn line(reefadm: &str, args: &[String]) -> String {
    std::iter::once(quote(reefadm))
        .chain(args.iter().map(|arg| quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `ssh` to the host, asking nothing of the terminal, for a probe or a copy
///
/// # Arguments
///
/// * `host` - The host, as ssh reaches it
/// * `script` - What its shell runs
pub fn ssh(host: &str, script: &str) -> Cmd {
    Cmd::new("ssh").args(["-o", "BatchMode=yes", host, "--", script])
}

/// reefadm's copy on a host, and where the admin's home is there
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct There {
    /// The host, as ssh reaches it
    pub host: String,
    /// The admin's home there
    pub home: String,
    /// reefadm's copy there
    pub reefadm: String,
}

/// Reaches a host and copies this reefadm there, refusing a host it can't run on, or a reefadm
/// built for a newer CPU than plain x86-64
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `host` - The host, as ssh reaches it
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
pub fn reach(cx: &mut Cx, host: &str, compiled: &[&str]) -> Result<There> {
    // a copy that runs on any x86-64 CPU, or none
    if !compiled.is_empty() {
        bail!(
            "this reefadm was built for this machine's CPU, with {}, as an exported RUSTFLAGS \
             builds it, so its copy may not run on {host}: rebuild it with RUSTFLAGS unset, as \
             `env -u RUSTFLAGS cargo install --locked --git {} --branch {} reefadm`",
            compiled.join(", "),
            sources::REPO,
            sources::REF
        );
    }
    // the host, reached without asking, and where its home is
    let found = cx
        .runner
        .real()
        .probe(&ssh(host, "uname -sm; printf '%s\\n' \"$HOME\""))?;
    if !found.ok() {
        bail!(
            "`ssh {host}` didn't sign in without asking: {} (reefadm needs a key or an agent \
             that reaches it)",
            found.stderr.trim()
        );
    }
    let mut lines = found.stdout.lines();
    let (system, home) = (
        lines.next().unwrap_or_default().trim().to_string(),
        lines.next().unwrap_or_default().trim().to_string(),
    );
    let here = format!("Linux {}", std::env::consts::ARCH);
    if system != here {
        bail!("{host} is {system}, and reefadm builds for and runs on {here}, as this machine is");
    }
    let reefadm = copied(cx, host, &home)?;
    Ok(There {
        host: host.to_string(),
        home,
        reefadm,
    })
}

/// Runs a command line on another host, ending as it ended there
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `host` - The host, as ssh reaches it
/// * `args` - The command line's arguments, after the program's name
/// * `compiled` - The features this reefadm was compiled with beyond plain x86-64's
pub fn run(cx: &mut Cx, host: &str, args: &[String], compiled: &[&str]) -> Result<()> {
    let mut args = forwarded(args);
    let there = reach(cx, host, compiled)?;
    // a password file is read here, and sent where only the admin can read it there
    let secret = match value(&args, "--password-file") {
        Some(file) => {
            let password =
                std::fs::read(file).with_context(|| format!("reading the password file {file}"))?;
            let there = send(cx, &there, &password)?;
            args = replaced(&args, "--password-file", &there);
            Some(there)
        }
        None => None,
    };
    // the command there, with a terminal when this one has one, for sudo's and reef's prompts
    let code = cx
        .runner
        .real()
        .interact(&terminal(host, &line(&there.reefadm, &args)));
    // the password gone again, however it ended
    if let Some(file) = secret {
        let _ = cx
            .runner
            .real()
            .run(&ssh(host, &format!("rm -f {}", quote(&file))));
    }
    ended(host, code?)
}

/// Sends a secret to a file on a host that only the admin can read, returning its path there
///
/// # Arguments
///
/// * `cx` - The runner
/// * `there` - The host and the admin's home there
/// * `secret` - The secret
pub fn send(cx: &mut Cx, there: &There, secret: &[u8]) -> Result<String> {
    let file = format!(
        "{}/.cache/reefadm/password-{}",
        there.home,
        std::process::id()
    );
    let script = format!(
        "umask 077 && mkdir -p {} && cat > {}",
        quote(&format!("{}/.cache/reefadm", there.home)),
        quote(&file)
    );
    cx.runner
        .real()
        .run(&ssh(&there.host, &script).stdin(secret.to_vec()))?;
    Ok(file)
}

/// `ssh` to the host for a command with prompts: with a terminal when this one has one
///
/// # Arguments
///
/// * `host` - The host, as ssh reaches it
/// * `line` - What its shell runs
pub fn terminal(host: &str, line: &str) -> Cmd {
    let mut cmd = Cmd::new("ssh");
    if std::io::stdin().is_terminal() {
        cmd = cmd.arg("-t");
    }
    cmd.args([host, "--", line])
}

/// Ends as a command on another host ended: its exit code, or the signal it was ended by
///
/// # Arguments
///
/// * `host` - The host
/// * `code` - Its exit code, or none for a signal
pub fn ended(host: &str, code: Option<i32>) -> Result<()> {
    match code {
        Some(0) => Ok(()),
        Some(code) => Err(Exit(u8::try_from(code).unwrap_or(1)).into()),
        None => bail!("reefadm on {host} was ended by a signal"),
    }
}

/// This binary's copy on the host, sent again if it differs, refused if it can't run there
///
/// # Arguments
///
/// * `cx` - The runner
/// * `host` - The host
/// * `home` - The admin's home there
fn copied(cx: &mut Cx, host: &str, home: &str) -> Result<String> {
    let runner = cx.runner.real();
    let path = format!("{home}/{COPY}");
    // this binary's digest, and its copy's there
    let exe = std::env::current_exe().context("finding reefadm's own binary")?;
    let here = runner.probe(&Cmd::new("sha256sum").arg(&exe))?;
    let digest = here
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let there = runner.probe(&ssh(
        host,
        &format!("sha256sum {} 2>/dev/null", quote(&path)),
    ))?;
    let same =
        !digest.is_empty() && there.stdout.split_whitespace().next() == Some(digest.as_str());
    if !same {
        // sent over ssh's input, then renamed into place whole
        let bytes = std::fs::read(&exe).with_context(|| format!("reading {}", exe.display()))?;
        let next = format!("{path}.new");
        let script = format!(
            "mkdir -p {dir} && cat > {next} && chmod 0755 {next} && mv -f {next} {path}",
            dir = quote(&format!("{home}/.cache/reefadm")),
            next = quote(&next),
            path = quote(&path)
        );
        runner.say(&format!("copying reefadm to {host}:{path}"));
        runner.run(&ssh(host, &script).stdin(bytes))?;
    }
    // a copy that runs there is the one used
    let runs = runner.probe(&ssh(host, &format!("{} --version", quote(&path))))?;
    if !runs.ok() {
        // the last line says why, since ssh's own warnings come first
        let why = runs
            .stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("it exited without saying why");
        bail!("reefadm's copy can't run on {host}: {why}: reefadm runs on Ubuntu 26.04");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{forwarded, line, replaced, value};

    /// Arguments, from words
    ///
    /// # Arguments
    ///
    /// * `words` - The words
    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    /// The remote command line keeps every argument but `--host`, quoted for the remote shell
    #[test]
    fn the_remote_line_keeps_every_argument_but_the_host() {
        // --host goes in either spelling, wherever it is, and nothing after `--` is read
        let given = args(&[
            "--host",
            "europa",
            "--dry-run",
            "deploy",
            "--name",
            "reef example",
            "--host=x",
            "--",
            "--host",
        ]);
        let kept = forwarded(&given);
        assert_eq!(
            kept,
            args(&[
                "--dry-run",
                "deploy",
                "--name",
                "reef example",
                "--",
                "--host"
            ])
        );
        // each word quoted for the shell there, so it reads back as given
        assert_eq!(
            line("/home/ada/.cache/reefadm/reefadm", &kept),
            "/home/ada/.cache/reefadm/reefadm --dry-run deploy --name 'reef example' -- --host"
        );
        assert_eq!(
            line("/r", &args(&["recover", "$(rm -rf /)"])),
            "/r recover '$(rm -rf /)'"
        );
    }

    /// A flag's value is read and replaced in whichever spelling it was given
    #[test]
    fn a_flags_value_is_read_and_replaced() {
        let given = args(&["deploy", "--password-file", "pw", "--ref=main"]);
        assert_eq!(value(&given, "--password-file"), Some("pw"));
        assert_eq!(value(&given, "--ref"), Some("main"));
        assert_eq!(value(&given, "--repo"), None);
        assert_eq!(
            replaced(&given, "--password-file", "/h/p"),
            args(&["deploy", "--password-file", "/h/p", "--ref=main"])
        );
        assert_eq!(
            replaced(&given, "--ref", "v2"),
            args(&["deploy", "--password-file", "pw", "--ref=v2"])
        );
    }
}
