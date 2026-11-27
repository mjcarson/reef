//! `reefadm --host HOST …`: any command run on another host over ssh, its prompts on this terminal
//!
//! reefadm runs there as it would here, with every argument but `--host`: it is a copy of this
//! binary, kept in `~/.cache/reefadm/reefadm` on HOST and sent again over ssh's input when it
//! differs. Where the copy can't run, on another architecture or an older glibc, reefadm is
//! built there instead, from the reef the command names (`--repo`, `--ref`), after asking:
//! a C compiler, curl and git from the host's packages if it lacks them, rustup, then `cargo
//! install`. ssh is the system's, so `~/.ssh/config`, the agent and ProxyJump all apply, and it
//! gets a terminal when this one has one, for sudo's prompts and reef's.
//!
//! Paths name files on HOST, but for `--password-file`, which is read here and sent to a file
//! only the admin can read there, removed once the command is done.

use crate::exec::{Cmd, quote};
use crate::{Cx, Exit, sources};
use anyhow::{Context, Result, bail};
use std::io::IsTerminal;

/// Where reefadm's copy is kept on the other host, under its home
const COPY: &str = ".cache/reefadm/reefadm";

/// Where reefadm is built on the other host when the copy can't run there, under its home
const BUILT: &str = ".cache/reefadm/cargo";

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
fn ssh(host: &str, script: &str) -> Cmd {
    Cmd::new("ssh").args(["-o", "BatchMode=yes", host, "--", script])
}

/// Runs a command line on another host, ending as it ended there
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `host` - The host, as ssh reaches it
/// * `args` - The command line's arguments, after the program's name
pub fn run(cx: &mut Cx, host: &str, args: &[String]) -> Result<()> {
    let mut args = forwarded(args);
    // the host, reached without asking, and where its home is
    let found = cx
        .runner
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
        lines.next().unwrap_or_default(),
        lines.next().unwrap_or_default(),
    );
    let here = format!("Linux {}", std::env::consts::ARCH);
    // reefadm there: this binary's copy, else one built there
    let reefadm = match system.trim() == here {
        true => copied(cx, host, home)?,
        false => None,
    };
    let reefadm = match reefadm {
        Some(reefadm) => reefadm,
        None => built(cx, host, home, system.trim(), &args)?,
    };
    // a password file is read here, and sent where only the admin can read it there
    let secret = match value(&args, "--password-file") {
        Some(file) => {
            let password =
                std::fs::read(file).with_context(|| format!("reading the password file {file}"))?;
            let there = format!("{home}/.cache/reefadm/password-{}", std::process::id());
            let script = format!(
                "umask 077 && mkdir -p {} && cat > {}",
                quote(&format!("{home}/.cache/reefadm")),
                quote(&there)
            );
            cx.runner.run(&ssh(host, &script).stdin(password))?;
            args = replaced(&args, "--password-file", &there);
            Some(there)
        }
        None => None,
    };
    // the command there, with a terminal when this one has one, for sudo's and reef's prompts
    let mut cmd = Cmd::new("ssh");
    if std::io::stdin().is_terminal() {
        cmd = cmd.arg("-t");
    }
    let code = cx
        .runner
        .interact(&cmd.args([host, "--", &line(&reefadm, &args)]));
    // the password gone again, however it ended
    if let Some(there) = secret {
        let _ = cx
            .runner
            .run(&ssh(host, &format!("rm -f {}", quote(&there))));
    }
    match code? {
        Some(0) => Ok(()),
        Some(code) => Err(Exit(u8::try_from(code).unwrap_or(1)).into()),
        None => bail!("reefadm on {host} was ended by a signal"),
    }
}

/// This binary's copy on the host, sent again if it differs, or none if it can't run there
///
/// # Arguments
///
/// * `cx` - The runner
/// * `host` - The host
/// * `home` - The admin's home there
fn copied(cx: &mut Cx, host: &str, home: &str) -> Result<Option<String>> {
    let path = format!("{home}/{COPY}");
    // this binary's digest, and its copy's there
    let exe = std::env::current_exe().context("finding reefadm's own binary")?;
    let here = cx.runner.probe(&Cmd::new("sha256sum").arg(&exe))?;
    let digest = here
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let there = cx.runner.probe(&ssh(
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
        cx.runner.say(&format!("copying reefadm to {host}:{path}"));
        cx.runner.run(&ssh(host, &script).stdin(bytes))?;
    }
    // a copy that runs there is the one used
    let runs = cx
        .runner
        .probe(&ssh(host, &format!("{} --version", quote(&path))))?;
    match runs.ok() || cx.runner.dry() {
        true => Ok(Some(path)),
        false => {
            // the last line says why, since ssh's own warnings come first
            let why = runs
                .stderr
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .unwrap_or("it exited without saying why");
            cx.runner
                .say(&format!("reefadm's copy can't run on {host}: {why}"));
            Ok(None)
        }
    }
}

/// reefadm built on the host, from the reef the command names, after asking
///
/// # Arguments
///
/// * `cx` - The runner
/// * `host` - The host
/// * `home` - The admin's home there
/// * `system` - What `uname -sm` says there
/// * `args` - The command's arguments, whose `--repo` and `--ref` name the reef to build
fn built(cx: &mut Cx, host: &str, home: &str, system: &str, args: &[String]) -> Result<String> {
    let repo = value(args, "--repo").unwrap_or(sources::REPO);
    let reference = value(args, "--ref").unwrap_or(sources::REF);
    cx.runner.say(&format!(
        "this reefadm can't run on {host} ({system}), so one is built there from {repo} at \
         {reference}"
    ));
    let question = format!(
        "build reefadm on {host}, installing a C compiler, curl, git and rustup there if it lacks \
         them?"
    );
    if !cx.confirm(&question)? {
        bail!("reefadm can't run on {host}: install it there, then run it there");
    }
    // a commit is fetched as a rev, anything else as a branch
    let which = match reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit()) {
        true => "--rev",
        false => "--branch",
    };
    let root = format!("{home}/{BUILT}");
    let script = format!(
        "set -e; \
         if ! command -v cc >/dev/null || ! command -v curl >/dev/null || ! command -v git >/dev/null; then \
           if command -v apt-get >/dev/null; then sudo apt-get update -q && \
             sudo DEBIAN_FRONTEND=noninteractive apt-get install -q -y build-essential curl git; \
           elif command -v dnf >/dev/null; then sudo dnf install -q -y gcc curl git; fi; \
         fi; \
         if [ ! -x \"$HOME/.cargo/bin/cargo\" ]; then \
           curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable; \
         fi; \
         \"$HOME/.cargo/bin/cargo\" install --locked --git {repo} {which} {reference} reefadm --root {root}",
        repo = quote(repo),
        reference = quote(reference),
        root = quote(&root),
    );
    let mut cmd = Cmd::new("ssh");
    if std::io::stdin().is_terminal() {
        cmd = cmd.arg("-t");
    }
    match cx.runner.interact(&cmd.args([host, "--", &script]))? {
        Some(0) => Ok(format!("{root}/bin/reefadm")),
        code => bail!(
            "building reefadm on {host} failed{}",
            code.map(|code| format!(" with exit code {code}"))
                .unwrap_or_default()
        ),
    }
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
