//! The three builds a release is made of, as the book's Building page runs them
//!
//! - **The CLI and reefadm**, on stable, from reef's root, for plain x86-64, so one build serves
//!   every target and reefadm's copy runs on any of them.
//! - **reef-serve**, on the pinned nightly with shoal's engine linked in, from `central`.
//! - **reef-app's server and its browser client**, through dx, from `central/reef-app`.
//!
//! The two central builds are for the target's CPU class ([`crate::cpu::Class`]): its flags go in
//! `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS`, which cargo puts after
//! `central/.cargo/config.toml`'s `-Ctarget-cpu=native` for the host's triple alone, so the
//! browser's wasm never gets them, and each class builds in a target directory of its own, since
//! the flags are part of cargo's fingerprint. Afterwards the fingerprints are read back, so a
//! build the flags didn't reach is refused rather than shipped to a CPU it may crash on.
//!
//! Each build's environment drops whatever would override the workspace's own settings or move
//! its artifacts: `RUSTFLAGS` and its kin, `CARGO_TARGET_DIR` and `RUSTUP_TOOLCHAIN`. glommio's
//! build is pointed at the liburing reefadm checked out, so it never fetches it itself. cargo's
//! builds take `--locked`, so a lockfile out of step with the pinned siblings stops the build
//! rather than changing the checkout.

use crate::cpu::Class;
use crate::exec::{Cmd, Runner, quote};
use crate::layout::Kind;
use crate::sources;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The variable a class's flags go in: rustflags for the host's triple alone
pub const TARGET_RUSTFLAGS: &str = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS";

/// How many classes' target directories are kept, the most recently built first
const CLASSES_KEPT: usize = 3;

/// The variables that would override the workspaces' own build settings or move their artifacts
const UNSET: [&str; 7] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    TARGET_RUSTFLAGS,
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "RUSTUP_TOOLCHAIN",
];

/// What a build made, in the checkout it was built in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The CLI
    pub reef: PathBuf,
    /// reefadm
    pub reefadm: PathBuf,
    /// reef-serve
    pub serve: PathBuf,
    /// reef-app's `web` directory, holding its server and `public/`, if the app was built
    pub web: Option<PathBuf>,
}

/// A build command with a clean environment, pointed at the liburing reefadm checked out
///
/// # Arguments
///
/// * `cmd` - The command
/// * `src` - The sources directory
fn clean(cmd: Cmd, src: &Path) -> Cmd {
    cmd.unset(UNSET)
        .env("GLOMMIO_LIBURING_DIR", sources::liburing(src))
}

/// A central build for a class: its flags, and a target directory of its own
///
/// # Arguments
///
/// * `cmd` - The command, with a clean environment
/// * `class` - The CPU class
/// * `target` - The class's target directory
fn for_class(cmd: Cmd, class: &Class, target: &Path) -> Cmd {
    cmd.env(TARGET_RUSTFLAGS, &class.flags)
        .env("CARGO_TARGET_DIR", target)
}

/// The target directory a class's central builds go in
///
/// # Arguments
///
/// * `src` - The sources directory
/// * `class` - The CPU class
pub fn class_dir(src: &Path, class: &Class) -> PathBuf {
    src.join("reef/central/target").join(class.dir())
}

/// Builds the CLI, reefadm, reef-serve and, for the app, reef-app, the last two for a CPU class
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory
/// * `toolchain` - The nightly the server builds on
/// * `kind` - The server being built
/// * `class` - The CPU class the server is built for
pub fn build(
    runner: &mut dyn Runner,
    src: &Path,
    toolchain: &str,
    kind: Kind,
    class: &Class,
) -> Result<Built> {
    let reef = src.join("reef");
    let central = reef.join("central");
    let target = class_dir(src, class);
    // the CLI and reefadm, on stable, from reef's root
    runner.run(
        &clean(Cmd::new("cargo"), src)
            .args([
                "+stable",
                "build",
                "--release",
                "--locked",
                "-p",
                "reef",
                "-p",
                "reefadm",
            ])
            .cwd(&reef),
    )?;
    // reef-serve, with shoal's engine, from central, for the class
    runner.run(
        &for_class(clean(Cmd::new("cargo"), src), class, &target)
            .arg(format!("+{toolchain}"))
            .args([
                "build",
                "--release",
                "--locked",
                "-p",
                "reef-service",
                "--features",
                "embedded",
                "--bin",
                "reef-serve",
            ])
            .cwd(&central),
    )?;
    // reef-app's server and client, `--release` before `@server` so both are release builds
    let web = match kind {
        Kind::App => {
            runner.run(
                &for_class(clean(Cmd::new("dx"), src), class, &target)
                    .env("RUSTUP_TOOLCHAIN", toolchain)
                    .args([
                        "build",
                        "--release",
                        "@server",
                        "--platform",
                        "server",
                        "--features",
                        "embedded",
                    ])
                    .cwd(central.join("reef-app")),
            )?;
            // dx takes no --locked, so a lockfile it moved stops the deploy instead
            let moved = runner.probe(&Cmd::new("git").arg("-C").arg(&reef).args([
                "diff",
                "--quiet",
                "--",
                "central/Cargo.lock",
            ]))?;
            if !moved.ok() && !runner.dry() {
                bail!(
                    "dx changed central/Cargo.lock in {}, so its build isn't the commit's: the \
                     pinned siblings in central/sources.toml are out of step with it",
                    reef.display()
                );
            }
            Some(target.join("dx/reef-app/release/web"))
        }
        Kind::Serve => None,
    };
    // the class's flags reached rustc for each server, else it isn't built for that CPU
    reached(runner, &target, class, "reef-serve", "reef-serve")?;
    if kind == Kind::App {
        reached(runner, &target, class, "reef-app", "reef-app's server")?;
    }
    // the classes built longest ago, gone, since each is a whole build
    prune(runner, &central.join("target"))?;
    Ok(Built {
        reef: reef.join("target/release/reef"),
        reefadm: reef.join("target/release/reefadm"),
        serve: target.join("release/reef-serve"),
        web,
    })
}

/// Refuses a server the class's flags didn't reach, as the newest fingerprint cargo wrote for it
/// says, and says so when it can't tell
///
/// # Arguments
///
/// * `runner` - The runner
/// * `target` - The class's target directory
/// * `class` - The CPU class
/// * `bin` - The binary's name, as cargo's fingerprint has it
/// * `what` - What to call it
fn reached(
    runner: &mut dyn Runner,
    target: &Path,
    class: &Class,
    bin: &str,
    what: &str,
) -> Result<()> {
    // the newest fingerprint of the binary for the host, outside any wasm build
    let script = format!(
        "find {} -path '*/wasm32-*' -prune -o -name 'bin-{bin}.json' -path '*fingerprint*' \
         -printf '%T@ %p\\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2- | xargs -r cat",
        quote(&target.display().to_string())
    );
    let found = runner.probe(&Cmd::new("sh").args(["-c", &script]))?;
    let flags = serde_json::from_str::<serde_json::Value>(found.stdout.trim())
        .ok()
        .and_then(|value| {
            value.get("rustflags")?.as_array().map(|flags| {
                flags
                    .iter()
                    .filter_map(|flag| flag.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
        });
    match (flags, runner.dry()) {
        (Some(flags), _) => match last_cpu(&flags) {
            Some(cpu) if cpu == class.cpu() => Ok(()),
            cpu => bail!(
                "{what} was compiled for {}, not the class's {}, so it isn't built for the \
                 target: its rustflags were `{}`",
                cpu.unwrap_or_else(|| "no CPU of its own".to_string()),
                class.cpu(),
                flags.join(" ")
            ),
        },
        (None, true) => Ok(()),
        (None, false) => {
            runner.say(&format!(
                "reefadm couldn't read the flags {what} was compiled with, to check they were \
                 `{}`",
                class.flags
            ));
            Ok(())
        }
    }
}

/// The CPU the last `-C target-cpu` among some rustflags names, which is the one rustc builds for
///
/// # Arguments
///
/// * `flags` - The rustflags, each word apart or `-C` joined to its value
///
/// # Examples
///
/// ```
/// let flags = ["-C", "target-cpu=native", "-Ctarget-cpu=x86-64-v3", "-C", "opt-level=3"].map(String::from);
/// assert_eq!(reefadm::compile::last_cpu(&flags).as_deref(), Some("x86-64-v3"));
/// ```
pub fn last_cpu(flags: &[String]) -> Option<String> {
    let mut cpu = None;
    let mut words = flags.iter().peekable();
    while let Some(word) = words.next() {
        // `-Ctarget-cpu=X`, or `-C` then `target-cpu=X`
        let value = match word.as_str() {
            "-C" => words.next().map(String::as_str),
            other => other.strip_prefix("-C"),
        };
        if let Some(name) = value.and_then(|value| value.strip_prefix("target-cpu=")) {
            cpu = Some(name.to_string());
        }
    }
    cpu
}

/// Removes every class's target directory but the most recently built
///
/// # Arguments
///
/// * `runner` - The runner
/// * `targets` - central's target directory, which holds one directory per class
fn prune(runner: &mut dyn Runner, targets: &Path) -> Result<()> {
    let listed = runner.probe(&Cmd::new("ls").arg("-1t").arg(targets))?;
    for name in listed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with("cpu-"))
        .skip(CLASSES_KEPT)
    {
        runner.run(&Cmd::new("rm").arg("-rf").arg(targets.join(name)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build, class_dir};
    use crate::cpu::Class;
    use crate::exec::Script;
    use crate::layout::Kind;
    use std::path::Path;

    /// The find that reads a binary's newest fingerprint under a class's directory
    ///
    /// # Arguments
    ///
    /// * `dir` - The class's directory
    /// * `bin` - The binary
    fn fingerprint(dir: &str, bin: &str) -> String {
        format!(
            "sh -c 'find {dir} -path '\\''*/wasm32-*'\\'' -prune -o -name '\\''bin-{bin}.json'\\'' \
             -path '\\''*fingerprint*'\\'' -printf '\\''%T@ %p\\n'\\'' 2>/dev/null | sort -n | tail -1 \
             | cut -d'\\'' '\\'' -f2- | xargs -r cat'"
        )
    }

    /// The central builds take the class's flags for the host's triple alone, in the class's own
    /// target directory, and the CLI's and reefadm's build takes neither
    #[test]
    fn the_central_builds_take_the_class_in_a_directory_of_its_own() {
        let class = Class::named("x86-64-v3");
        let dir = class_dir(Path::new("/src"), &class);
        let shown = dir.display().to_string();
        let mut script = Script::default();
        // the lockfile stays as committed, and each server was compiled with the class's cpu last
        script.prints("git -C /src/reef diff --quiet -- central/Cargo.lock", "");
        for bin in ["reef-serve", "reef-app"] {
            script.prints(
                fingerprint(&shown, bin),
                r#"{"rustflags":["-C","target-cpu=native","-C","target-cpu=x86-64-v3","-C","target-feature=+aes,+sse2"]}"#,
            );
        }
        let built = build(
            &mut script,
            Path::new("/src"),
            "nightly-2026-08-21",
            Kind::App,
            &class,
        )
        .unwrap();
        let liburing = "GLOMMIO_LIBURING_DIR=/src/glommio/glommio/liburing";
        let for_class = format!(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='-C target-cpu=x86-64-v3 -C \
             target-feature=+aes,+sse2' CARGO_TARGET_DIR={shown}"
        );
        assert_eq!(
            script.steps(),
            [
                format!(
                    "run: {liburing} cargo +stable build --release --locked -p reef -p reefadm  (in /src/reef)"
                ),
                format!(
                    "run: {liburing} {for_class} cargo +nightly-2026-08-21 build --release --locked -p \
                     reef-service --features embedded --bin reef-serve  (in /src/reef/central)"
                ),
                format!(
                    "run: {liburing} {for_class} RUSTUP_TOOLCHAIN=nightly-2026-08-21 dx build --release \
                     @server --platform server --features embedded  (in /src/reef/central/reef-app)"
                ),
            ]
        );
        assert!(shown.starts_with("/src/reef/central/target/cpu-x86-64-v3-"));
        assert_eq!(built.web, Some(dir.join("dx/reef-app/release/web")));
        assert_eq!(built.serve, dir.join("release/reef-serve"));
        assert_eq!(built.reefadm, Path::new("/src/reef/target/release/reefadm"));
        // a server the flags didn't reach, `native` last, is refused
        let mut script = Script::default();
        script.prints(
            fingerprint(&shown, "reef-serve"),
            r#"{"rustflags":["-C","target-cpu=x86-64-v3","-C","target-cpu=native"]}"#,
        );
        let error = build(
            &mut script,
            Path::new("/src"),
            "nightly",
            Kind::Serve,
            &class,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("reef-serve was compiled for native, not the class's x86-64-v3"),
            "{error}"
        );
    }

    /// A lockfile dx moved stops the deploy, and classes beyond the three built last are pruned
    #[test]
    fn a_moved_lockfile_stops_it_and_old_classes_go() {
        let class = Class::named("x86-64-v2");
        let mut script = Script::default();
        let error =
            build(&mut script, Path::new("/src"), "nightly", Kind::App, &class).unwrap_err();
        assert!(error.to_string().contains("central/Cargo.lock"), "{error}");
        // reef-serve alone builds no app and reads no lockfile, and the oldest classes go
        let mut script = Script::default();
        script.prints(
            "ls -1t /src/reef/central/target",
            "cpu-a\nrelease\ncpu-b\ncpu-c\ncpu-d\ncpu-e\n",
        );
        let built = build(
            &mut script,
            Path::new("/src"),
            "nightly",
            Kind::Serve,
            &class,
        )
        .unwrap();
        assert_eq!(built.web, None);
        assert_eq!(
            script.steps()[2..],
            [
                "run: rm -rf /src/reef/central/target/cpu-d",
                "run: rm -rf /src/reef/central/target/cpu-e"
            ]
        );
        // and a fingerprint it couldn't read is said, not refused
        assert!(
            script
                .said
                .iter()
                .any(|line| line.starts_with("reefadm couldn't read the flags reef-serve")),
            "{:?}",
            script.said
        );
    }
}
