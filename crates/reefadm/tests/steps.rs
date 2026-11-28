//! Each command's whole list of steps, against a scripted host
//!
//! A [`Script`] answers every probe a command makes from a table, as a host in some state
//! would, and records every change, so a test reads the deploy a host would get, step by step,
//! without one.

mod common;

use common::{COMMITTED, REWRITE, SRC, env, fresh, git, now, run};
use reefadm::Cli;
use reefadm::exec::{Dry, Output};

/// A team server deploys on a fresh host as the book's Deploying chapter would, step by step, its
/// servers built for the CPU named, here as for a fleet, so the steps don't hang on this one's
#[test]
fn a_team_server_deploys_on_a_fresh_host() {
    let mut script = fresh();
    run(&mut script, &["deploy", "--target-cpu", "x86-64-v3"]).unwrap();
    let steps = script.steps().join("\n");
    let manifest = reefadm::sources::Manifest::built_in();
    let release = "/opt/reef/releases/992226c4a1b2-20261127072258";
    let class = reefadm::cpu::Class::named("x86-64-v3");
    let target = format!("{SRC}/reef/central/target/{}", class.dir());
    let web = format!("{target}/dx/reef-app/release/web");
    let liburing = format!("GLOMMIO_LIBURING_DIR={SRC}/glommio/glommio/liburing");
    let for_class = format!(
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='{}' CARGO_TARGET_DIR={target}",
        class.flags
    );
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
        // the four builds: reefadm, the static CLI stamped with its commit, and the servers
        format!("run: {liburing} cargo +stable build --release --locked -p reefadm  (in {SRC}/reef)"),
        format!(
            "run: {liburing} REEF_COMMIT=992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90 \
             REEF_COMMITTED={COMMITTED} CARGO_PROFILE_RELEASE_STRIP=symbols cargo +stable build \
             --release --locked --target x86_64-unknown-linux-musl -p reef  (in {SRC}/reef)"
        ),
        format!(
            "run: {liburing} {for_class} cargo +{tc} build --release --locked -p reef-service \
             --features embedded --bin reef-serve  (in {SRC}/reef/central)",
            tc = manifest.toolchain
        ),
        format!(
            "run: {liburing} {for_class} RUSTUP_TOOLCHAIN={tc} dx build --release @server \
             --platform server --features embedded  (in {SRC}/reef/central/reef-app)",
            tc = manifest.toolchain
        ),
        // the release, made current, with the commands through it
        format!("run: sudo rm -rf {release}.new"),
        format!("run: sudo install -d -m 0755 {release}.new/bin"),
        format!(
            "run: sudo install -m 0755 {SRC}/reef/target/x86_64-unknown-linux-musl/release/reef \
             {release}.new/bin/reef"
        ),
        format!("run: sudo install -m 0755 {target}/release/reef-serve {release}.new/bin/reef-serve"),
        format!("run: sudo install -m 0755 {SRC}/reef/target/release/reefadm {release}.new/bin/reefadm"),
        format!("run: sudo install -d -m 0755 {release}.new/app"),
        format!("run: sudo cp -R {web}/server {web}/public {release}.new/app"),
        format!("write: sudo write {release}.new/release.toml (644)"),
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
    assert!(
        state.contains(&format!("build = \"{}\"", class.flags)),
        "{state}"
    );
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

/// A host other than Ubuntu 26.04 is told it is untested, and deployed all the same
#[test]
fn a_host_other_than_ubuntu_2604_is_told_so() {
    let told = |os_release: Option<&str>| {
        let mut script = fresh();
        if let Some(text) = os_release {
            script.prints("cat /etc/os-release", text);
        }
        run(&mut script, &["deploy"]).unwrap();
        script
            .said
            .into_iter()
            .find(|line| line.starts_with("reefadm is made for Ubuntu 26.04"))
    };
    assert_eq!(
        told(Some(
            "PRETTY_NAME=\"Ubuntu 24.04.3 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\n"
        ))
        .as_deref(),
        Some(
            "reefadm is made for Ubuntu 26.04, and this host runs Ubuntu 24.04.3 LTS: it goes on, untested here"
        )
    );
    assert!(
        told(Some(
            "PRETTY_NAME=\"Fedora Linux 43\"\nID=fedora\nVERSION_ID=43\n"
        ))
        .is_some()
    );
    // 26.04 itself, and a host that doesn't say, are deployed without a word
    assert_eq!(
        told(Some(
            "PRETTY_NAME=\"Ubuntu Resolute Raccoon\"\nVERSION_ID=\"26.04\"\nID=ubuntu\n"
        )),
        None
    );
    assert_eq!(told(None), None);
}

/// A user unit deploys with no sudo, no account and no config, on loopback alone, its first
/// admin the user, added and signed in by the release's CLI with no config to name
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
    // asked on loopback alone, its password before the build, and the user added as its first
    // admin and signed in, by the release's CLI with no config to name
    let asked: Vec<_> = script
        .log
        .iter()
        .filter(|line| line.starts_with("me: "))
        .collect();
    assert_eq!(asked, ["me: https://127.0.0.1:12180/api/v1/me"]);
    let password = script
        .log
        .iter()
        .position(|line| line.starts_with("password: "))
        .expect("the first admin's password was asked");
    let fetched = script
        .log
        .iter()
        .position(|line| line.contains("git"))
        .expect("reef was fetched");
    assert!(password < fetched, "{:#?}", script.log);
    let cli = "run: /home/ada/.local/share/reef/current/bin/reef";
    let admin: Vec<&str> = steps
        .iter()
        .copied()
        .filter(|step| step.starts_with(cli))
        .collect();
    assert_eq!(admin.len(), 2, "{steps:#?}");
    for (step, command) in admin.iter().zip(["user add ada", "login --user ada"]) {
        let (before, after) = step.split_once(" --password-file ").unwrap();
        assert_eq!(before, format!("{cli} {command}"));
        assert!(after.ends_with("/password"), "{step}");
    }
    assert_eq!(
        steps.last(),
        Some(&"write: write /home/ada/.local/share/reef/reefadm.toml (644)")
    );
    let state = &script.files.last().unwrap().text;
    assert!(state.contains("first_admin = \"ada\""), "{state}");
}

/// A user unit over a config of the user's own that turns sign-in off has the key taken out
/// before it starts, the file kept beside it, its comments kept, and its first admin added with
/// the config named
#[test]
fn a_user_unit_over_a_config_turning_sign_in_off() {
    let mut script = fresh();
    let config = "/home/ada/.config/reef/config.toml";
    script.prints(
        format!("cat {config}"),
        "# mine\n[auth]\n# off for now\nrequired = false\n",
    );
    script.prints(format!("test -e {config}"), "");
    run(&mut script, &["deploy", "--user", "--api-only"]).unwrap();
    let steps = script.steps();
    // kept beside it, then written without the key, before the unit starts
    let kept = steps
        .iter()
        .position(|step| {
            *step
                == format!("run: cp -a {config} /home/ada/.config/reef/config.toml.before-sign-in")
        })
        .unwrap_or_else(|| panic!("the config wasn't kept: {steps:#?}"));
    let written = steps
        .iter()
        .position(|step| *step == format!("write: write {config} (644)"))
        .unwrap_or_else(|| panic!("the config wasn't written: {steps:#?}"));
    let started = steps
        .iter()
        .position(|step| *step == "run: systemctl --user enable --now reef.service")
        .unwrap();
    assert!(kept < written && written < started, "{steps:#?}");
    let text = &script
        .files
        .iter()
        .find(|file| file.path.display().to_string() == config)
        .unwrap()
        .text;
    assert!(
        !text
            .lines()
            .any(|line| line.trim_start().starts_with("required")),
        "{text}"
    );
    assert!(
        text.starts_with("# mine\n[auth]\n# off for now\n"),
        "{text}"
    );
    // and its first admin added by the CLI reading it
    let added =
        format!("run: /home/ada/.local/share/reef/current/bin/reef --conf {config} user add ada");
    assert!(
        steps.iter().any(|step| step.starts_with(&added)),
        "{steps:#?}"
    );
}

/// A user unit told to add no admin adds none, and asks no password
#[test]
fn a_user_unit_with_no_admin_adds_none() {
    let mut script = fresh();
    run(
        &mut script,
        &["deploy", "--user", "--api-only", "--no-admin"],
    )
    .unwrap();
    assert!(
        !script.log.iter().any(|line| line.starts_with("password: ")),
        "{:#?}",
        script.log
    );
    assert!(
        !script.steps().iter().any(|step| step.contains("user add")),
        "{:#?}",
        script.steps()
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
