//! The three builds a release is made of, as the book's Building page runs them
//!
//! - **The CLI and reefadm**, on stable, from reef's root.
//! - **reef-serve**, on the pinned nightly with shoal's engine linked in, from `central`, where
//!   `central/.cargo/config.toml` sets `-Ctarget-cpu=native`.
//! - **reef-app's server and its browser client**, through dx, from `central/reef-app`.
//!
//! Each build's environment drops whatever would override the workspace's own settings or move
//! its artifacts: `RUSTFLAGS` and its kin, `CARGO_TARGET_DIR` and `RUSTUP_TOOLCHAIN`. glommio's
//! build is pointed at the liburing reefadm checked out, so it never fetches it itself. cargo's
//! builds take `--locked`, so a lockfile out of step with the pinned siblings stops the build
//! rather than changing the checkout.

use crate::exec::{Cmd, Runner};
use crate::layout::Kind;
use crate::sources;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The variables that would override the workspaces' own build settings or move their artifacts
const UNSET: [&str; 6] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
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

/// Builds the CLI, reefadm, reef-serve and, for the app, reef-app
///
/// # Arguments
///
/// * `runner` - The runner
/// * `src` - The sources directory
/// * `toolchain` - The nightly the server builds on
/// * `kind` - The server being built
pub fn build(runner: &mut dyn Runner, src: &Path, toolchain: &str, kind: Kind) -> Result<Built> {
    let reef = src.join("reef");
    let central = reef.join("central");
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
    // reef-serve, with shoal's engine, from central, whose cargo config sets the target cpu
    runner.run(
        &clean(Cmd::new("cargo"), src)
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
                &clean(Cmd::new("dx"), src)
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
            Some(central.join("target/dx/reef-app/release/web"))
        }
        Kind::Serve => None,
    };
    Ok(Built {
        reef: reef.join("target/release/reef"),
        reefadm: reef.join("target/release/reefadm"),
        serve: central.join("target/release/reef-serve"),
        web,
    })
}

#[cfg(test)]
mod tests {
    use super::build;
    use crate::exec::Script;
    use crate::layout::Kind;
    use std::path::Path;

    /// The app's three builds run as the Building page runs them, each with a clean environment
    #[test]
    fn the_builds_run_as_the_book_runs_them() {
        let mut script = Script::default();
        // the lockfile stays as committed
        script.prints("git -C /src/reef diff --quiet -- central/Cargo.lock", "");
        let built = build(
            &mut script,
            Path::new("/src"),
            "nightly-2026-08-21",
            Kind::App,
        )
        .unwrap();
        let liburing = "GLOMMIO_LIBURING_DIR=/src/glommio/glommio/liburing";
        assert_eq!(
            script.steps(),
            [
                format!(
                    "run: {liburing} cargo +stable build --release --locked -p reef -p reefadm  (in /src/reef)"
                ),
                format!(
                    "run: {liburing} cargo +nightly-2026-08-21 build --release --locked -p reef-service \
                     --features embedded --bin reef-serve  (in /src/reef/central)"
                ),
                format!(
                    "run: {liburing} RUSTUP_TOOLCHAIN=nightly-2026-08-21 dx build --release @server \
                     --platform server --features embedded  (in /src/reef/central/reef-app)"
                ),
            ]
        );
        assert_eq!(
            built.web.as_deref(),
            Some(Path::new(
                "/src/reef/central/target/dx/reef-app/release/web"
            ))
        );
        assert_eq!(
            built.serve,
            Path::new("/src/reef/central/target/release/reef-serve")
        );
    }

    /// A lockfile dx moved stops the deploy
    #[test]
    fn a_moved_lockfile_stops_it() {
        let mut script = Script::default();
        let error = build(&mut script, Path::new("/src"), "nightly", Kind::App).unwrap_err();
        assert!(error.to_string().contains("central/Cargo.lock"), "{error}");
        // and reef-serve alone builds no app and reads no lockfile
        let mut script = Script::default();
        let built = build(&mut script, Path::new("/src"), "nightly", Kind::Serve).unwrap();
        assert_eq!(built.web, None);
        assert_eq!(script.steps().len(), 2);
    }
}
