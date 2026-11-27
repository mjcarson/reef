//! What building reef needs on the host, installed where it is missing
//!
//! git 2.29 or later, a C compiler (glommio compiles liburing), curl for rustup, and tar for
//! backups come from the host's packages, through apt-get or dnf after asking. rustup comes
//! from its installer after asking, then the toolchains `central/sources.toml` names: a stable
//! at least as new as it says, and the pinned nightly. The app also needs the wasm target and
//! dx 0.7.10, which cargo builds, against openssl's headers, found through pkg-config.

use crate::exec::{Cmd, Runner};
use crate::layout::Kind;
use crate::sources::Manifest;
use anyhow::{Result, bail};
use std::path::Path;

/// The dx the app is built with, which reef-app's Dioxus version needs exactly
pub const DX: &str = "0.7.10";

/// The oldest git that clones remote projects as the service needs
const GIT_AT_LEAST: (u32, u32) = (2, 29);

/// Something a build needs, and the package each package manager installs it from
struct Needed {
    /// What it is called, to the admin
    what: &'static str,
    /// The command that succeeds once it is there
    probe: &'static [&'static str],
    /// apt-get's package for it
    apt: &'static str,
    /// dnf's package for it
    dnf: &'static str,
}

/// Every command a build needs from the host's packages
const NEEDED: [Needed; 4] = [
    Needed {
        what: "git",
        probe: &["git", "--version"],
        apt: "git",
        dnf: "git",
    },
    Needed {
        what: "cc",
        probe: &["cc", "--version"],
        apt: "build-essential",
        dnf: "gcc",
    },
    Needed {
        what: "curl",
        probe: &["curl", "--version"],
        apt: "curl",
        dnf: "curl",
    },
    Needed {
        what: "tar",
        probe: &["tar", "--version"],
        apt: "tar",
        dnf: "tar",
    },
];

/// What building dx needs beyond them: openssl's headers, found through pkg-config
const DX_NEEDED: [Needed; 2] = [
    Needed {
        what: "pkg-config",
        probe: &["pkg-config", "--version"],
        apt: "pkg-config",
        dnf: "pkgconf-pkg-config",
    },
    Needed {
        what: "openssl's headers",
        probe: &["pkg-config", "--exists", "openssl"],
        apt: "libssl-dev",
        dnf: "openssl-devel",
    },
];

/// A version's leading numbers, from the first word that starts with a digit
///
/// # Arguments
///
/// * `text` - What `--version` printed, such as `git version 2.43.0`
///
/// # Examples
///
/// ```
/// use reefadm::toolchain::version;
///
/// assert_eq!(version("git version 2.43.0"), Some(vec![2, 43, 0]));
/// assert_eq!(version("rustc 1.98.0 (88d9e12ae 2026-08-18)"), Some(vec![1, 98, 0]));
/// assert_eq!(version("dioxus 0.7.10 (57d6794)"), Some(vec![0, 7, 10]));
/// assert_eq!(version("none"), None);
/// ```
pub fn version(text: &str) -> Option<Vec<u32>> {
    let word = text
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    // `1.98.0-nightly` and `2.43.0.windows.1` alike: the dotted numbers it starts with
    Some(
        word.split(['.', '-'])
            .map_while(|part| part.parse().ok())
            .collect(),
    )
}

/// Installs whatever building reef needs and the host lacks
///
/// # Arguments
///
/// * `runner` - The runner
/// * `manifest` - The toolchains reef's commit builds with
/// * `kind` - The server being built, since only the app needs wasm and dx
/// * `src` - The sources directory, where dx is built
/// * `yes` - Whether to install without asking
pub fn ensure(
    runner: &mut dyn Runner,
    manifest: &Manifest,
    kind: Kind,
    src: &Path,
    yes: bool,
) -> Result<()> {
    packages(runner, &NEEDED, yes)?;
    git_new_enough(runner)?;
    rust(runner, manifest, yes)?;
    if kind == Kind::App {
        app(runner, manifest, src, yes)?;
    }
    Ok(())
}

/// Installs what a build needs from the host's packages, where it is missing
///
/// # Arguments
///
/// * `runner` - The runner
/// * `needed` - What the build needs
/// * `yes` - Whether to install without asking
fn packages(runner: &mut dyn Runner, needed: &[Needed], yes: bool) -> Result<()> {
    // each thing whose probe fails
    let mut missing = Vec::new();
    for needed in needed {
        let probe = Cmd::new(needed.probe[0]).args(&needed.probe[1..]);
        if !runner.probe(&probe)?.ok() {
            missing.push(needed);
        }
    }
    if !missing.is_empty() {
        // from whichever package manager the host has
        let apt = runner.probe(&Cmd::new("apt-get").arg("--version"))?.ok();
        let dnf = !apt && runner.probe(&Cmd::new("dnf").arg("--version"))?.ok();
        let names: Vec<&str> = missing.iter().map(|needed| needed.what).collect();
        let packages: Vec<&str> = missing
            .iter()
            .map(|needed| if apt { needed.apt } else { needed.dnf })
            .collect();
        let manager = match (apt, dnf) {
            (true, _) => "apt-get",
            (_, true) => "dnf",
            _ => bail!(
                "building reef needs {}, which this host lacks: install them, then run this again",
                names.join(", ")
            ),
        };
        let question = format!(
            "building reef needs {}: install {} with {manager}?",
            names.join(", "),
            packages.join(" ")
        );
        if !yes && !runner.confirm(&question)? {
            bail!(
                "building reef needs {}: install them, then run this again",
                names.join(", ")
            );
        }
        runner.elevate()?;
        // apt's lists may be empty on a fresh host
        let install = match apt {
            true => {
                runner.run(
                    &Cmd::new("apt-get")
                        .args(["update", "-q"])
                        .env("DEBIAN_FRONTEND", "noninteractive")
                        .root(),
                )?;
                Cmd::new("apt-get")
                    .args(["install", "-q", "-y"])
                    .env("DEBIAN_FRONTEND", "noninteractive")
            }
            false => Cmd::new("dnf").args(["install", "-q", "-y"]),
        };
        runner.run(&install.args(&packages).root())?;
    }
    Ok(())
}

/// Refuses a git older than the service needs to clone remote projects
///
/// # Arguments
///
/// * `runner` - The runner
fn git_new_enough(runner: &mut dyn Runner) -> Result<()> {
    // git clones remote projects as the service needs only from 2.29
    let git = runner.probe(&Cmd::new("git").arg("--version"))?;
    if let Some(found) = version(&git.stdout)
        && found.as_slice() < [GIT_AT_LEAST.0, GIT_AT_LEAST.1].as_slice()
    {
        bail!(
            "reef needs git {}.{} or later, and this host has {}",
            GIT_AT_LEAST.0,
            GIT_AT_LEAST.1,
            git.stdout.trim()
        );
    }
    Ok(())
}

/// Installs rustup, a new enough stable and the pinned nightly
///
/// # Arguments
///
/// * `runner` - The runner
/// * `manifest` - The toolchains reef's commit builds with
/// * `yes` - Whether to install rustup without asking
fn rust(runner: &mut dyn Runner, manifest: &Manifest, yes: bool) -> Result<()> {
    // rustup, from its own installer, with the smallest toolchains
    if !runner.probe(&rustup().arg("--version"))?.ok() {
        let question = "building reef needs rustup: install it from https://sh.rustup.rs?";
        if !yes && !runner.confirm(question)? {
            bail!(
                "building reef needs rustup: install it from https://rustup.rs, then run this again"
            );
        }
        runner.run(&Cmd::new("sh").args([
            "-c",
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable",
        ]))?;
    }
    // the toolchains there are, listed rather than run, since `rustup run` installs a missing one
    let listed = toolchains(runner)?;
    // a stable at least as new as the manifest says, for the CLI and reefadm
    let at_least = version(&manifest.stable_at_least).unwrap_or_default();
    let new_enough = match listed.iter().any(|name| is(name, "stable")) {
        true => {
            let stable = runner.probe(&rustup().args(["run", "stable", "rustc", "--version"]))?;
            stable.ok()
                && version(&stable.stdout)
                    .is_some_and(|found| found.as_slice() >= at_least.as_slice())
        }
        false => false,
    };
    if !new_enough {
        runner.run(&Cmd::new("rustup").args([
            "toolchain",
            "install",
            "stable",
            "--profile",
            "minimal",
        ]))?;
    }
    // and the nightly the server builds on, by its date
    if !listed.iter().any(|name| is(name, &manifest.toolchain)) {
        runner.run(&Cmd::new("rustup").args([
            "toolchain",
            "install",
            &manifest.toolchain,
            "--profile",
            "minimal",
        ]))?;
    }
    Ok(())
}

/// rustup, told never to install a toolchain a probe names
fn rustup() -> Cmd {
    Cmd::new("rustup").env("RUSTUP_AUTO_INSTALL", "0")
}

/// The toolchains rustup has, by their full names, such as `stable-x86_64-unknown-linux-gnu`
///
/// # Arguments
///
/// * `runner` - The runner
fn toolchains(runner: &mut dyn Runner) -> Result<Vec<String>> {
    let listed = runner.probe(&rustup().args(["toolchain", "list"]))?;
    Ok(listed
        .stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(String::from)
        .collect())
}

/// Whether a toolchain's full name is the one named, its host after it
///
/// # Arguments
///
/// * `full` - The full name, such as `nightly-2026-08-21-x86_64-unknown-linux-gnu`
/// * `name` - The name asked for, such as `nightly-2026-08-21`
///
/// # Examples
///
/// ```
/// use reefadm::toolchain::is;
///
/// assert!(is("stable-x86_64-unknown-linux-gnu", "stable"));
/// assert!(is("nightly-2026-08-21-x86_64-unknown-linux-gnu", "nightly-2026-08-21"));
/// assert!(!is("nightly-x86_64-unknown-linux-gnu", "nightly-2026-08-21"));
/// ```
pub fn is(full: &str, name: &str) -> bool {
    full == name || full.starts_with(&format!("{name}-"))
}

/// Installs what only the app's build needs: the wasm target, and dx
///
/// # Arguments
///
/// * `runner` - The runner
/// * `manifest` - The toolchains reef's commit builds with
/// * `src` - The sources directory, where dx is built
/// * `yes` - Whether to install packages without asking
fn app(runner: &mut dyn Runner, manifest: &Manifest, src: &Path, yes: bool) -> Result<()> {
    // the browser's half is built for wasm, with the server's nightly, once it is there
    let there = toolchains(runner)?
        .iter()
        .any(|name| is(name, &manifest.toolchain));
    let installed = match there {
        true => {
            runner
                .probe(&rustup().args([
                    "target",
                    "list",
                    "--installed",
                    "--toolchain",
                    &manifest.toolchain,
                ]))?
                .stdout
        }
        false => String::new(),
    };
    if !installed
        .lines()
        .any(|line| line.trim() == "wasm32-unknown-unknown")
    {
        runner.run(&Cmd::new("rustup").args([
            "target",
            "add",
            "wasm32-unknown-unknown",
            "--toolchain",
            &manifest.toolchain,
        ]))?;
    }
    // dx at the version reef-app's Dioxus needs, built where the sources are, since the
    // system's temporary directory may be in memory
    let dx = runner.probe(&Cmd::new("dx").arg("--version"))?;
    let wanted = version(DX);
    if !dx.ok() || version(&dx.stdout) != wanted {
        packages(runner, &DX_NEEDED, yes)?;
        runner.run(
            &Cmd::new("cargo")
                .args([
                    "+stable",
                    "install",
                    "dioxus-cli",
                    "--version",
                    DX,
                    "--locked",
                    "--target-dir",
                ])
                .arg(src.join("target-dx")),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ensure, version};
    use crate::exec::{Output, Script};
    use crate::layout::Kind;
    use crate::sources::Manifest;
    use std::path::Path;

    /// A host with every command and toolchain installs nothing
    #[test]
    fn a_ready_host_installs_nothing() {
        let manifest = Manifest::built_in();
        let mut script = Script::default();
        // every command answers, at versions new enough
        for command in ["git", "cc", "curl", "tar"] {
            script.prints(format!("{command} --version"), "git version 2.43.0\n");
        }
        script.prints("RUSTUP_AUTO_INSTALL=0 rustup --version", "rustup 1.28.2\n");
        script.prints(
            "RUSTUP_AUTO_INSTALL=0 rustup toolchain list",
            format!(
                "stable-x86_64-unknown-linux-gnu (default)\n{}-x86_64-unknown-linux-gnu\n",
                manifest.toolchain
            ),
        );
        script.prints(
            "RUSTUP_AUTO_INSTALL=0 rustup run stable rustc --version",
            "rustc 1.98.0 (88d9e12ae 2026-08-18)\n",
        );
        script.prints(
            format!(
                "RUSTUP_AUTO_INSTALL=0 rustup target list --installed --toolchain {}",
                manifest.toolchain
            ),
            "wasm32-unknown-unknown\nx86_64-unknown-linux-gnu\n",
        );
        script.prints("dx --version", "dioxus 0.7.10 (57d6794)\n");
        ensure(&mut script, &manifest, Kind::App, Path::new("/src"), false).unwrap();
        assert!(script.steps().is_empty(), "{:?}", script.steps());
    }

    /// A bare host gets its packages, rustup, both toolchains, the wasm target and dx
    #[test]
    fn a_bare_host_gets_everything_after_asking() {
        let manifest = Manifest::built_in();
        let mut script = Script {
            yes: true,
            ..Script::default()
        };
        // only apt-get and an old git answer
        script.prints("apt-get --version", "apt 2.9\n");
        script.answer("git --version", Output::failed(""));
        let error = ensure(&mut script, &manifest, Kind::App, Path::new("/src"), false);
        // the packages it asked for, then everything rust needs
        assert!(error.is_ok(), "{error:?}");
        let tc = &manifest.toolchain;
        assert_eq!(
            script.steps(),
            [
                "run: sudo DEBIAN_FRONTEND=noninteractive apt-get update -q".to_string(),
                "run: sudo DEBIAN_FRONTEND=noninteractive apt-get install -q -y git build-essential curl tar".to_string(),
                "run: sh -c 'curl --proto '\\''=https'\\'' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable'".to_string(),
                "run: rustup toolchain install stable --profile minimal".to_string(),
                format!("run: rustup toolchain install {tc} --profile minimal"),
                format!("run: rustup target add wasm32-unknown-unknown --toolchain {tc}"),
                "run: sudo DEBIAN_FRONTEND=noninteractive apt-get update -q".to_string(),
                "run: sudo DEBIAN_FRONTEND=noninteractive apt-get install -q -y pkg-config libssl-dev".to_string(),
                "run: cargo +stable install dioxus-cli --version 0.7.10 --locked --target-dir /src/target-dx".to_string(),
            ]
        );
        assert!(
            script
                .said
                .iter()
                .any(|line| line.starts_with("asked: building reef needs git, cc, curl, tar"))
        );
    }

    /// An answer of no stops before anything is installed
    #[test]
    fn no_installs_nothing() {
        let mut script = Script::default();
        script.prints("apt-get --version", "apt 2.9\n");
        let error = ensure(
            &mut script,
            &Manifest::built_in(),
            Kind::Serve,
            Path::new("/src"),
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("install them"), "{error}");
        assert!(script.steps().is_empty());
        // and a git too old is refused by name
        assert_eq!(version("git version 2.25.1"), Some(vec![2, 25, 1]));
    }
}
