//! Each command's whole list of steps, against a scripted host
//!
//! A [`Script`] answers every probe a command makes from a table, as a host in some state
//! would, and records every change, so a test reads the deploy a host would get, step by step,
//! without one.

use reefadm::exec::{Dry, Output, Script};
use reefadm::{Cli, Env};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// When every scripted deploy runs: 2026-11-27 07:22:58 UTC
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_795_764_178)
}

/// The admin's environment on the scripted host
fn env() -> Env {
    Env::from_pairs([("HOME", "/home/ada"), ("USER", "ada"), ("PATH", "/usr/bin")])
}

/// The sources directory on the scripted host
const SRC: &str = "/home/ada/.local/share/reefadm/src";

/// How the manifest has git fetch glommio's liburing
const REWRITE: &str =
    "-c url.https://github.com/axboe/liburing.git.insteadOf=git://git.kernel.dk/liburing";

/// `git` as reefadm runs it in a checkout: reef's plain, its siblings' with the manifest's rewrites
///
/// # Arguments
///
/// * `dir` - The checkout, under the sources
fn git(dir: &str) -> String {
    match dir {
        "reef" => format!("git -C {SRC}/{dir}"),
        _ => format!("git {REWRITE} -C {SRC}/{dir}"),
    }
}

/// A host with systemd, the tools a build needs and nothing of reef's yet, whose checkouts
/// answer once they are cloned
fn fresh() -> Script {
    let mut script = Script {
        yes: true,
        secret: "hunter2".into(),
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
    script.prints(
        format!("{} rev-parse HEAD", git("shoal")),
        "dbfb82e25b8143aa8a6e3e8748022c5194990e5f\n",
    );
    script.prints(
        format!("{} rev-parse HEAD", git("glommio")),
        "25d2c35240d0ef753cfdcd3a58078f3b17e132dd\n",
    );
    script.prints(
        format!("cat {SRC}/reef/central/sources.toml"),
        include_str!("../../../central/sources.toml"),
    );
    // every tool and toolchain a build needs
    let manifest = reefadm::sources::Manifest::built_in();
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
        "wasm32-unknown-unknown\n",
    );
    script.prints("dx --version", "dioxus 0.7.10 (57d6794)\n");
    script.prints(
        format!("git -C {SRC}/reef diff --quiet -- central/Cargo.lock"),
        "",
    );
    // the release's reef-serve says its schema
    script.prints(
        "/opt/reef/releases/992226c4a1b2-20261127072258/bin/reef-serve version --json",
        r#"{"schema":13509127152932721421,"tables":[["ProjectRow",12832186597557680855]]}"#,
    );
    script.prints(
        "/home/ada/.local/share/reef/releases/992226c4a1b2-20261127072258/bin/reef-serve version --json",
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
fn run(script: &mut Script, line: &[&str]) -> anyhow::Result<()> {
    let cli = Cli::from_args(std::iter::once("reefadm").chain(line.iter().copied()))?;
    reefadm::run(cli, script, env(), now())
}

/// A team server deploys on a fresh host as the book's Deploying chapter would, step by step
#[test]
fn a_team_server_deploys_on_a_fresh_host() {
    let mut script = fresh();
    run(&mut script, &["deploy"]).unwrap();
    let steps = script.steps().join("\n");
    let manifest = reefadm::sources::Manifest::built_in();
    let release = "/opt/reef/releases/992226c4a1b2-20261127072258";
    let web = format!("{SRC}/reef/central/target/dx/reef-app/release/web");
    let liburing = format!("GLOMMIO_LIBURING_DIR={SRC}/glommio/glommio/liburing");
    let expected = [
        // the checkouts: reef, then its siblings at their pins
        format!("run: mkdir -p {SRC}"),
        format!("run: git clone --quiet --no-checkout https://github.com/mjcarson/reef.git {SRC}/reef"),
        format!("run: {} fetch --quiet origin CentralizedPlanner", git("reef")),
        format!("run: {} checkout --quiet --force --detach FETCH_HEAD", git("reef")),
        format!("run: git {REWRITE} clone --quiet --no-checkout {} {SRC}/shoal", manifest.shoal.url),
        format!("run: {} fetch --quiet origin ScanSupport", git("shoal")),
        format!("run: {} checkout --quiet --force --detach {}", git("shoal"), manifest.shoal.rev),
        format!("run: git {REWRITE} clone --quiet --no-checkout {} {SRC}/glommio", manifest.glommio.url),
        format!("run: {} fetch --quiet origin ZeroCopyDmaStreamWriter", git("glommio")),
        format!("run: {} checkout --quiet --force --detach {}", git("glommio"), manifest.glommio.rev),
        format!("run: {} submodule update --quiet --init --recursive --force", git("glommio")),
        // the three builds
        format!("run: {liburing} cargo +stable build --release --locked -p reef -p reefadm  (in {SRC}/reef)"),
        format!(
            "run: {liburing} cargo +{tc} build --release --locked -p reef-service --features embedded \
             --bin reef-serve  (in {SRC}/reef/central)",
            tc = manifest.toolchain
        ),
        format!(
            "run: {liburing} RUSTUP_TOOLCHAIN={tc} dx build --release @server --platform server \
             --features embedded  (in {SRC}/reef/central/reef-app)",
            tc = manifest.toolchain
        ),
        // the release, made current, with the commands through it
        format!("run: sudo rm -rf {release}.new"),
        format!("run: sudo install -d -m 0755 {release}.new/bin"),
        format!("run: sudo install -m 0755 {SRC}/reef/target/release/reef {release}.new/bin/reef"),
        format!("run: sudo install -m 0755 {SRC}/reef/central/target/release/reef-serve {release}.new/bin/reef-serve"),
        format!("run: sudo install -m 0755 {SRC}/reef/target/release/reefadm {release}.new/bin/reefadm"),
        format!("run: sudo install -d -m 0755 {release}.new/app"),
        format!("run: sudo cp -R {web}/server {web}/public {release}.new/app"),
        format!("run: sudo mv -T {release}.new {release}"),
        format!("run: sudo ln -sfnT {release} /opt/reef/current.new"),
        "run: sudo mv -T /opt/reef/current.new /opt/reef/current".to_string(),
        "run: sudo install -d -m 0755 /usr/local/bin".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reef /usr/local/bin/reef".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reef-serve /usr/local/bin/reef-serve".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reefadm /usr/local/bin/reefadm".to_string(),
        // the account, the config and the account's directories
        "run: sudo useradd --system --home-dir /var/lib/reef --create-home --shell /usr/sbin/nologin reef".to_string(),
        "run: sudo install -d -m 0755 /etc/reef".to_string(),
        "write: sudo write /etc/reef/config.toml (644)".to_string(),
        "run: sudo install -d -o reef -g reef -m 0750 /var/lib/reef /var/lib/reef/shoal /var/lib/reef/clones /var/lib/reef/certs".to_string(),
        // the unit, started
        "write: sudo write /etc/systemd/system/reef.service (644)".to_string(),
        "run: sudo systemctl daemon-reload".to_string(),
        "run: sudo systemctl enable --now reef.service".to_string(),
        // reef's CA copied out, the first admin added and signed in
        "run: sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem".to_string(),
        "PASSWORD".to_string(),
        "PASSWORD".to_string(),
        // and the record, last
        "write: sudo write /etc/reef/reefadm.toml (644)".to_string(),
    ];
    // the password file's name is the process's own, so it is matched by its shape
    let found: Vec<&str> = script.steps();
    assert_eq!(found.len(), expected.len(), "{steps}");
    for (found, expected) in found.iter().zip(&expected) {
        match expected.as_str() {
            "PASSWORD" => {
                let (before, after) = found.split_once("--password-file ").unwrap();
                assert!(
                    before
                        == "run: /opt/reef/current/bin/reef --conf /etc/reef/config.toml user add ada "
                        || before
                            == "run: /opt/reef/current/bin/reef --conf /etc/reef/config.toml login --user ada ",
                    "{found}"
                );
                assert!(after.ends_with("/password"), "{found}");
            }
            expected => assert_eq!(*found, expected, "in:\n{steps}"),
        }
    }
    // the server was asked on both ports, and the record says what was deployed
    assert!(
        script
            .log
            .contains(&"me: https://127.0.0.1:12180/api/v1/me".to_string())
    );
    assert!(
        script
            .log
            .contains(&"me: https://reef.example.com:12443/api/v1/me".to_string())
    );
    let state = &script.files.last().unwrap().text;
    assert!(state.contains("mode = \"system\""), "{state}");
    assert!(
        state.contains("reef = \"992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90\""),
        "{state}"
    );
    assert!(
        state.contains("ProjectRow = \"12832186597557680855\""),
        "{state}"
    );
    assert!(state.contains("first_admin = \"ada\""), "{state}");
    // and the admin is told where it is
    assert!(
        script
            .said
            .iter()
            .any(|line| line.starts_with("reef is up at https://reef.example.com:12443")),
        "{:?}",
        script.said
    );
}

/// A deploy that stopped partway, run again, keeps what it did and does the rest
#[test]
fn a_deploy_that_stopped_partway_carries_on() {
    // the account, the config and reefadm's unit are there, and the unit runs; no record yet
    let mut script = fresh();
    script.prints("id -u reef", "998\n");
    let config = {
        let mut first = fresh();
        run(&mut first, &["deploy"]).unwrap();
        let unit = first
            .files
            .iter()
            .find(|file| file.path.ends_with("reef.service"))
            .unwrap()
            .text
            .clone();
        let config = first
            .files
            .iter()
            .find(|file| file.path.ends_with("config.toml"))
            .unwrap()
            .text
            .clone();
        script.prints("cat /etc/systemd/system/reef.service", unit);
        config
    };
    script.prints("cat /etc/reef/config.toml", config);
    script.prints("systemctl is-active --quiet reef.service", "");
    // its own unit's ports are no problem
    script.prints(
        "ss -Hltn",
        "LISTEN 0 4096 127.0.0.1:12180 0.0.0.0:*\nLISTEN 0 4096 0.0.0.0:12443 0.0.0.0:*\n",
    );
    // and the admin was added before it stopped
    script.attempts.push((
        "/opt/reef/current/bin/reef --conf /etc/reef/config.toml user add*".into(),
        Output::failed("reef: 409 Conflict: a user is named ada already"),
    ));
    run(&mut script, &["deploy"]).unwrap();
    let steps = script.steps();
    // nothing it did before is done again
    assert!(
        !steps.iter().any(|step| step.contains("useradd")),
        "{steps:#?}"
    );
    assert!(
        !steps
            .iter()
            .any(|step| step.contains("write /etc/reef/config.toml")),
        "{steps:#?}"
    );
    assert!(
        !steps.iter().any(|step| step.contains("reef.service (644)")),
        "{steps:#?}"
    );
    // the rest is, the running unit restarted onto the new release
    assert!(
        steps.contains(&"run: sudo systemctl restart reef.service"),
        "{steps:#?}"
    );
    assert!(
        steps.iter().any(|step| step.contains(" login --user ada ")),
        "{steps:#?}"
    );
    assert_eq!(
        steps.last(),
        Some(&"write: sudo write /etc/reef/reefadm.toml (644)")
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("keeping /etc/reef/config.toml"))
    );
}

/// A deployment reefadm made, or a unit made by hand, stops a deploy before anything changes
#[test]
fn a_host_reef_is_on_already_stops_a_deploy() {
    // reefadm's own record
    let mut script = fresh();
    script.prints("test -e /etc/reef/reefadm.toml", "");
    let error = run(&mut script, &["deploy"]).unwrap_err();
    assert!(error.to_string().contains("`reefadm upgrade`"), "{error}");
    assert!(script.steps().is_empty(), "{:?}", script.steps());
    // a unit made by hand, and its ports taken
    let mut script = fresh();
    script.prints(
        "cat /etc/systemd/system/reef.service",
        "[Unit]\nDescription=reef's central planner\n",
    );
    script.prints("ss -Hltn", "LISTEN 0 4096 127.0.0.1:12180 0.0.0.0:*\n");
    let error = format!("{:#}", run(&mut script, &["deploy"]).unwrap_err());
    assert!(error.contains("wasn't written by reefadm"), "{error}");
    assert!(error.contains("port 12180 is taken"), "{error}");
    assert!(script.steps().is_empty());
    // a disk too small for a first build
    let mut script = fresh();
    script.prints(
        "df -Pk /home/ada",
        "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100 90 10485760 90% /\n",
    );
    let error = run(&mut script, &["deploy"]).unwrap_err();
    assert!(error.to_string().contains("10 GiB free"), "{error}");
}

/// A user unit deploys with no sudo, no account and no config, on loopback alone
#[test]
fn a_user_unit_deploys_without_root() {
    let mut script = fresh();
    run(&mut script, &["deploy", "--user", "--api-only"]).unwrap();
    let steps = script.steps();
    // nothing runs as root, nothing for a team server is made
    assert!(
        !steps.iter().any(|step| step.contains("sudo")),
        "{steps:#?}"
    );
    assert!(
        !steps
            .iter()
            .any(|step| step.contains("useradd") || step.contains("config.toml"))
    );
    assert!(
        !steps.iter().any(|step| step.contains("dx build")),
        "{steps:#?}"
    );
    // its unit, under the user's config, runs reef-serve from the current release
    let unit = script
        .files
        .iter()
        .find(|file| file.path.ends_with("systemd/user/reef.service"))
        .unwrap();
    assert!(
        unit.text
            .contains("ExecStart=/home/ada/.local/share/reef/current/bin/reef-serve\n")
    );
    assert!(steps.contains(&"run: systemctl --user enable --now reef.service"));
    assert!(steps.contains(
        &"run: ln -sfn /home/ada/.local/share/reef/current/bin/reef /home/ada/.cargo/bin/reef"
    ));
    // asked on loopback alone, and no admin added
    let asked: Vec<_> = script
        .log
        .iter()
        .filter(|line| line.starts_with("me: "))
        .collect();
    assert_eq!(asked, ["me: https://127.0.0.1:12180/api/v1/me"]);
    assert!(!steps.iter().any(|step| step.contains("user add")));
    assert_eq!(
        steps.last(),
        Some(&"write: write /home/ada/.local/share/reef/reefadm.toml (644)")
    );
}

/// A dry run lists every step of a deploy and makes none of them
#[test]
fn a_dry_run_lists_every_step_and_changes_nothing() {
    // the same deploy, through a dry runner over the same host
    let mut done = fresh();
    run(&mut done, &["deploy"]).unwrap();
    let mut dry = Dry { inner: fresh() };
    let cli = Cli::from_args(["reefadm", "--dry-run", "deploy", "--password-file", "pw"]).unwrap();
    reefadm::run(cli, &mut dry, env(), now()).unwrap();
    // nothing was run or written
    assert!(dry.inner.log.is_empty(), "{:?}", dry.inner.log);
    assert!(dry.inner.files.is_empty());
    // and each step the deploy took is said, in the same order
    let said: Vec<&str> = dry
        .inner
        .said
        .iter()
        .filter_map(|line| {
            line.strip_prefix("would run: ")
                .or_else(|| line.strip_prefix("would "))
        })
        .collect();
    let taken: Vec<String> = done
        .steps()
        .iter()
        .map(|step| {
            step.strip_prefix("run: ")
                .map(String::from)
                .or_else(|| step.strip_prefix("write: ").map(|file| format!("{file}:")))
                .unwrap()
        })
        .collect();
    let shown: Vec<&str> = said
        .iter()
        .copied()
        .filter(|line| !line.starts_with("wait for ") && !line.starts_with("read the password"))
        .collect();
    assert_eq!(shown.len(), taken.len(), "{shown:#?}");
    for (shown, taken) in shown.iter().zip(&taken) {
        // the password file is only named in a dry run
        match taken.split_once("--password-file ") {
            Some((before, _)) => assert!(shown.starts_with(before), "{shown} / {taken}"),
            None => assert_eq!(shown, taken),
        }
    }
}
