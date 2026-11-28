//! A scripted host, as each command's step tests start from it

#![allow(dead_code)]

use reefadm::exec::Script;
use reefadm::layout::{Kind, Layout, Mode};
use reefadm::served::{Served, TeamFlags};
use reefadm::sources::Manifest;
use reefadm::state::{Release, State};
use reefadm::{Cli, Env};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// When every scripted deploy runs: 2026-11-27 07:22:58 UTC
pub fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_795_764_178)
}

/// The admin's environment on the scripted host
pub fn env() -> Env {
    Env::from_pairs([("HOME", "/home/ada"), ("USER", "ada"), ("PATH", "/usr/bin")])
}

/// The sources directory on the scripted host
pub const SRC: &str = "/home/ada/.local/share/reefadm/src";

/// When the scripted host's reef commit was made, in seconds since the epoch: 2026-11-26
pub const COMMITTED: i64 = 1_795_700_000;

/// What the nightly prints of the features it builds for: rustc's own, then LLVM's
pub const KNOWN: &str = "Features supported by rustc for this target:\n    aes   - Enable AES.\n    \
                         avx2   - Enable AVX2.\n    sha   - Enable SHA.\n    gfni   - Enable GFNI.\n\n\
                         Code-generation features supported by LLVM for this target:\n    \
                         retpoline   - Retpolines.\n";

/// How the manifest has git fetch glommio's liburing
pub const REWRITE: &str =
    "-c url.https://github.com/axboe/liburing.git.insteadOf=git://git.kernel.dk/liburing";

/// `git` as reefadm runs it in a checkout: reef's plain, its siblings' with the manifest's rewrites
///
/// # Arguments
///
/// * `dir` - The checkout, under the sources
pub fn git(dir: &str) -> String {
    match dir {
        "reef" => format!("git -C {SRC}/{dir}"),
        _ => format!("git {REWRITE} -C {SRC}/{dir}"),
    }
}

/// A host with systemd, the tools a build needs and nothing of reef's yet, whose checkouts
/// answer once they are cloned
pub fn fresh() -> Script {
    let mut script = Script {
        yes: true,
        secret: "correct horse battery".into(),
        ..Script::default()
    };
    // systemd, the disk and the host's name
    script.prints("systemctl show --property=Version", "Version=258\n");
    script.prints("systemctl --user show --property=Version", "Version=258\n");
    script.prints("ss -Hltn", "LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n");
    script.prints("test -e /home/ada", "");
    script.prints(
        "df -Pk /home/ada",
        "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100000000 1000 90000000 1% /\n",
    );
    script.prints("hostname -f", "reef.example.com\n");
    // each checkout's commit once it is fetched, and the manifest reef's holds
    script.prints(
        format!("{} rev-parse HEAD", git("reef")),
        "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90\n",
    );
    // and when reef's was made, which its CLI is stamped with
    script.prints(
        format!(
            "{} show -s --format=%ct 992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90",
            git("reef")
        ),
        format!("{COMMITTED}\n"),
    );
    script.prints(
        format!("{} rev-parse HEAD", git("shoal")),
        "e40b08f27a0367a1aae9b3709fa2185e491cf2ab\n",
    );
    script.prints(
        format!("{} rev-parse HEAD", git("glommio")),
        "25d2c35240d0ef753cfdcd3a58078f3b17e132dd\n",
    );
    // each sibling's branch holds its pinned commit
    let pins = reefadm::sources::Manifest::built_in();
    for (dir, rev) in [("shoal", &pins.shoal.rev), ("glommio", &pins.glommio.rev)] {
        script.prints(format!("{} cat-file -e '{rev}^{{commit}}'", git(dir)), "");
    }
    script.prints(
        format!("cat {SRC}/reef/central/sources.toml"),
        include_str!("../../../../central/sources.toml"),
    );
    // every tool and toolchain a build needs
    let manifest = reefadm::sources::Manifest::built_in();
    for command in ["git", "cc", "musl-gcc", "curl", "tar"] {
        script.prints(format!("{command} --version"), "git version 2.43.0\n");
    }
    script.prints("RUSTUP_AUTO_INSTALL=0 rustup --version", "rustup 1.28.2\n");
    script.prints(
        "RUSTUP_AUTO_INSTALL=0 rustup target list --installed --toolchain stable",
        "x86_64-unknown-linux-gnu\nx86_64-unknown-linux-musl\n",
    );
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
        "wasm32-unknown-unknown\n",
    );
    script.prints("dx --version", "dioxus 0.7.10 (57d6794)\n");
    script.prints(
        format!("git -C {SRC}/reef diff --quiet -- central/Cargo.lock"),
        "",
    );
    // the nightly lists the features it builds for, as rustc's first section
    script.prints(
        format!(
            "RUSTUP_AUTO_INSTALL=0 rustup run {} rustc --print target-features --target \
             x86_64-unknown-linux-gnu",
            manifest.toolchain
        ),
        KNOWN,
    );
    // the reef-serve built for any class says its schema
    script.prints(
        format!("{SRC}/reef/central/target/cpu-*"),
        r#"{"schema":13509127152932721421,"tables":[["ProjectRow",12832186597557680855]]}"#,
    );
    // the server makes its CA once it starts, and nobody is signed in yet
    script.prints("sudo test -e /var/lib/reef/certs/ca.pem", "");
    script.prints("test -e /home/ada/.local/share/reef/certs/ca.pem", "");
    script.prints("cat /etc/reef/reef-ca.pem", "PEM");
    script.prints("cat /home/ada/.local/share/reef/certs/ca.pem", "PEM");
    script.prints(
        "/opt/reef/current/bin/reef --conf /etc/reef/config.toml whoami",
        "nobody: https://127.0.0.1:12180 asks every caller to sign in, so run `reef login`\n",
    );
    script.prints("loginctl show-user ada --property=Linger", "Linger=yes\n");
    script
}

/// Runs a command line against a scripted host
///
/// # Arguments
///
/// * `script` - The host
/// * `line` - The command line, after `reefadm`
pub fn run(script: &mut Script, line: &[&str]) -> anyhow::Result<()> {
    let cli = Cli::from_args(std::iter::once("reefadm").chain(line.iter().copied()))?;
    reefadm::run(cli, script, env(), now())
}

/// The release reefadm deployed before
pub const OLD: &str = "111111111111-20261120000000";

/// ProjectRow's fingerprint, as this commit's schema pins it
pub const PROJECT_ROW: &str = "12832186597557680855";

/// The team server's paths on the scripted host
pub fn layout() -> Layout {
    Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap()
}

/// The old release, with the tables it recorded
///
/// # Arguments
///
/// * `project_row` - ProjectRow's fingerprint as it recorded it
pub fn old(project_row: &str) -> Release {
    Release {
        name: OLD.into(),
        reef: "1111111111111111111111111111111111111111".into(),
        shoal: "e40b08f27a0367a1aae9b3709fa2185e491cf2ab".into(),
        glommio: "25d2c35240d0ef753cfdcd3a58078f3b17e132dd".into(),
        toolchain: Manifest::built_in().toolchain,
        deployed: 1_795_000_000,
        schema: Some("1".into()),
        tables: BTreeMap::from([("ProjectRow".to_string(), project_row.to_string())]),
        build: None,
    }
}

/// A fresh host, then everything a team deploy of the old release left on it
///
/// # Arguments
///
/// * `running` - The release it runs
pub fn deployed(running: Release) -> Script {
    let mut script = fresh();
    let layout = layout();
    // reefadm's record, the config it wrote and its unit
    let state = State {
        mode: Mode::System,
        kind: Kind::App,
        repo: "https://github.com/mjcarson/reef.git".into(),
        reference: "CentralizedPlanner".into(),
        release: running,
        previous: None,
        first_admin: Some("ada".into()),
        src: None,
    };
    script.prints(
        "cat /etc/reef/reefadm.toml",
        toml::to_string(&state).unwrap(),
    );
    let flags = TeamFlags {
        names: vec!["reef.example.com".into()],
        port: 12443,
        own_cert: false,
        cores: 2,
        memory: "256MiB".into(),
    };
    script.prints(
        "cat /etc/reef/config.toml",
        Served::team(&layout, &flags).text().unwrap(),
    );
    script.prints(
        "cat /etc/systemd/system/reef.service",
        reefadm::unit::text(&layout, Kind::App, false),
    );
    // its checkout of reef, and the commit's schema pin
    script.prints(format!("test -e {SRC}"), "");
    script.prints(format!("git -C {SRC}/reef rev-parse --git-dir"), ".git\n");
    script.prints(
        format!("cat {SRC}/reef/central/reef-service/src/schema.rs"),
        include_str!("../../../../central/reef-service/src/schema.rs"),
    );
    // the data a backup holds
    for path in ["shoal", "clones", "certs"] {
        script.prints(format!("sudo test -e /var/lib/reef/{path}"), "");
    }
    script
}
