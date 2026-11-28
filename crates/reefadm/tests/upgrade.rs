//! `reefadm upgrade` and `reefadm uninstall`, step by step, against a host reefadm deployed

mod common;

use common::{OLD, PROJECT_ROW, SRC, deployed, old, run};
use reefadm::exec::{Me, Script};
use reefadm::layout::Kind;
use reefadm::release::Record;
use reefadm::sources::Manifest;
use reefadm::state::{Release, State};

/// The release an upgrade at 2026-11-27 07:22:58 makes of reef's commit `992226c4a1b2…`
const NEW: &str = "992226c4a1b2-20261127072258";

/// An upgrade fetches and builds the new commit, stages it, and hands over to its reefadm
#[test]
fn an_upgrade_builds_then_hands_over_to_the_new_reefadm() {
    let mut script = deployed(old(PROJECT_ROW));
    run(&mut script, &["upgrade"]).unwrap();
    let steps = script.steps();
    // the new commit fetched where it was deployed from, then built
    assert_eq!(
        steps[0],
        format!(
            "run: git -C {SRC}/reef remote set-url origin https://github.com/mjcarson/reef.git"
        )
    );
    assert!(
        steps
            .iter()
            .any(|step| step.contains("cargo +stable build"))
    );
    // staged, but made current by the new release's own reefadm
    assert!(steps.contains(
        &format!("write: sudo write /opt/reef/releases/{NEW}.new/release.toml (644)").as_str()
    ));
    assert_eq!(
        steps.last().copied(),
        Some(format!("run: /opt/reef/releases/{NEW}/bin/reefadm upgrade --finish {NEW}").as_str())
    );
    assert!(
        !steps.iter().any(|step| step.contains("systemctl")),
        "{steps:#?}"
    );
    // the tables the new schema adds are named, and the skill's change
    assert!(
        script
            .said
            .iter()
            .any(|line| line.starts_with("its schema adds PlanRow, NodeRow")),
        "{:?}",
        script.said
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("reef claude install"))
    );
}

/// A commit whose schema changed a table the data was written with is refused before the build
#[test]
fn a_schema_that_changed_a_table_is_refused_before_the_build() {
    let mut script = deployed(old("1"));
    let error = run(&mut script, &["upgrade"]).unwrap_err().to_string();
    assert!(error.contains("changed ProjectRow"), "{error}");
    assert!(error.contains("--fresh"), "{error}");
    assert!(
        !script.steps().iter().any(|step| step.contains("cargo")),
        "{:?}",
        script.steps()
    );
}

/// The commits and the CPU class the running release was built from and for need no upgrade, and
/// another class does
#[test]
fn the_same_commits_need_no_upgrade() {
    let manifest = Manifest::built_in();
    let mut running = old(PROJECT_ROW);
    running.reef = "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90".into();
    running.shoal = manifest.shoal.rev;
    running.glommio = manifest.glommio.rev;
    running.build = Some(reefadm::cpu::Class::named("x86-64-v3").flags);
    let mut script = deployed(running.clone());
    run(&mut script, &["upgrade", "--target-cpu", "x86-64-v3"]).unwrap();
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("nothing to upgrade"))
    );
    assert!(!script.steps().iter().any(|step| step.contains("cargo")));
    // the same commits for another CPU are built again
    let mut script = deployed(running);
    run(&mut script, &["upgrade", "--target-cpu", "x86-64-v4"]).unwrap();
    assert!(
        script
            .steps()
            .iter()
            .any(|step| step.contains("target-cpu=x86-64-v4") && step.contains("cargo +")),
        "{:#?}",
        script.steps()
    );
}

/// A deployed host, its new release staged as the first half of an upgrade left it
fn staged() -> Script {
    let mut script = deployed(old(PROJECT_ROW));
    let record = Record {
        repo: "https://github.com/mjcarson/reef.git".into(),
        reference: "CentralizedPlanner".into(),
        kind: Kind::App,
        release: Release {
            name: NEW.into(),
            reef: "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90".into(),
            ..old(PROJECT_ROW)
        },
    };
    script.prints(
        format!("cat /opt/reef/releases/{NEW}/release.toml"),
        toml::to_string(&record).unwrap(),
    );
    script.prints("ls -1 /opt/reef/releases", format!("{OLD}\n{NEW}\nolder\n"));
    script
}

/// The new release's reefadm backs up, makes it current, starts it and keeps the old one
#[test]
fn finishing_makes_the_new_release_current() {
    let mut script = staged();
    run(&mut script, &["upgrade", "--finish", NEW]).unwrap();
    let backup = format!("/var/backups/reef/reef-2026-11-27T072258-{OLD}.tar.gz");
    let steps = script.steps().join("\n");
    let expected = [
        "run: sudo systemctl stop reef.service".to_string(),
        "run: sudo install -d -m 0700 /var/backups/reef".to_string(),
        "write: sudo write /var/backups/reef/reefadm-backup.toml (600)".to_string(),
        format!(
            "run: sudo sh -c 'umask 077 && tar -czf {backup} -C /var/backups/reef reefadm-backup.toml \
             -C / var/lib/reef/shoal var/lib/reef/clones var/lib/reef/certs'"
        ),
        "run: sudo rm -f /var/backups/reef/reefadm-backup.toml".to_string(),
        format!("run: sudo ln -sfnT /opt/reef/releases/{NEW} /opt/reef/current.new"),
        "run: sudo mv -T /opt/reef/current.new /opt/reef/current".to_string(),
        "run: sudo install -d -m 0755 /usr/local/bin".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reef /usr/local/bin/reef".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reef-serve /usr/local/bin/reef-serve".to_string(),
        "run: sudo ln -sfn /opt/reef/current/bin/reefadm /usr/local/bin/reefadm".to_string(),
        "run: sudo systemctl start reef.service".to_string(),
        "run: sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem".to_string(),
        "write: sudo write /etc/reef/reefadm.toml (644)".to_string(),
        "run: sudo rm -rf /opt/reef/releases/older".to_string(),
    ];
    assert_eq!(steps, expected.join("\n"));
    // the record keeps the old release as the way back
    let state: State = toml::from_str(
        script
            .files
            .last()
            .unwrap()
            .text
            .split_once('\n')
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(state.release.name, NEW);
    assert_eq!(
        state.previous.map(|release| release.name),
        Some(OLD.to_string())
    );
}

/// The two steps the first admin is added and signed in by, the release's CLI reading the team
/// server's config, their password's file matched by its shape, since its name is the process's
///
/// # Arguments
///
/// * `steps` - The steps run
/// * `name` - The first admin's name
fn added_admin(steps: &[&str], name: &str) {
    let cli = "run: /opt/reef/current/bin/reef --conf /etc/reef/config.toml";
    for (step, command) in steps
        .iter()
        .zip([format!("user add {name}"), format!("login --user {name}")])
    {
        let (before, after) = step
            .split_once(" --password-file ")
            .unwrap_or_else(|| panic!("no password file: {step}"));
        assert_eq!(before, format!("{cli} {command}"));
        assert!(after.ends_with("/password"), "{step}");
    }
}

/// Finishing an upgrade of a server with no user adds the first admin it was handed and signs
/// them in, once the new release is up and the record names it
#[test]
fn finishing_adds_the_first_admin_to_a_server_with_none() {
    let mut script = staged();
    script.users = Some(Ok(false));
    let line = [
        "upgrade",
        "--finish",
        NEW,
        "--first-admin",
        "ada",
        "--password-file",
        "/tmp/handed",
    ];
    run(&mut script, &line).unwrap();
    let steps = script.steps();
    // the record and the prune first, then ada added and signed in
    let record = steps
        .iter()
        .position(|step| *step == "write: sudo write /etc/reef/reefadm.toml (644)")
        .unwrap();
    assert_eq!(
        steps[record + 1],
        "run: sudo rm -rf /opt/reef/releases/older"
    );
    added_admin(&steps[record + 2..], "ada");
    assert_eq!(steps.len(), record + 4, "{steps:#?}");
    // the password read from the file it was handed, and the server asked twice
    assert!(
        script
            .log
            .contains(&"password: from /tmp/handed".to_string()),
        "{:#?}",
        script.log
    );
    let asked = script
        .probed
        .iter()
        .filter(|probe| *probe == "users: https://127.0.0.1:12180/api/v1/users")
        .count();
    assert_eq!(asked, 2, "{:#?}", script.probed);
}

/// A first admin's password the service wouldn't keep, such as the empty one a bare Enter gives,
/// stops an upgrade as it is asked, before anything is built or handed over, while a short one
/// is kept
#[test]
fn a_password_the_service_refuses_stops_it_before_the_build() {
    let mut script = deployed(old(PROJECT_ROW));
    script.users = Some(Ok(false));
    script.secret = String::new();
    let error = run(&mut script, &["upgrade"]).unwrap_err().to_string();
    assert!(error.contains("can't be empty"), "{error}");
    assert!(
        !script.steps().iter().any(|step| step.contains("cargo")),
        "{:#?}",
        script.steps()
    );
    // a short one goes on to the build
    let mut script = deployed(old(PROJECT_ROW));
    script.users = Some(Ok(false));
    script.secret = "short".into();
    run(&mut script, &["upgrade"]).unwrap();
    assert!(
        script.steps().iter().any(|step| step.contains("cargo")),
        "{:#?}",
        script.steps()
    );
}

/// Finishing an upgrade over a config that turns sign-in off takes the key out before the new
/// release starts, which would refuse it, keeping the file as it was beside it
#[test]
fn finishing_takes_sign_in_off_out_of_the_config() {
    let mut script = staged();
    let config = "# europa's\n[api]\naddress = \"127.0.0.1:12180\"\n\n[auth]\nrequired = false\n";
    script.prints("cat /etc/reef/config.toml", config);
    run(&mut script, &["upgrade", "--finish", NEW]).unwrap();
    let steps = script.steps();
    // after the backup and before the swap, kept then written
    let backed_up = steps
        .iter()
        .position(|step| *step == "run: sudo rm -f /var/backups/reef/reefadm-backup.toml")
        .unwrap();
    assert_eq!(
        steps[backed_up + 1],
        "run: sudo cp -a /etc/reef/config.toml /etc/reef/config.toml.before-sign-in"
    );
    assert_eq!(
        steps[backed_up + 2],
        "write: sudo write /etc/reef/config.toml (644)"
    );
    assert_eq!(
        steps[backed_up + 3],
        format!("run: sudo ln -sfnT /opt/reef/releases/{NEW} /opt/reef/current.new")
    );
    // the key gone, and the rest as it was
    let written = &script
        .files
        .iter()
        .find(|file| file.path.display().to_string() == "/etc/reef/config.toml")
        .unwrap()
        .text;
    let served = reefadm::served::Served::read(written, std::path::Path::new("c.toml")).unwrap();
    assert_eq!(served.auth.required, None);
    assert!(written.starts_with("# europa's\n[api]\n"), "{written}");
}

/// An upgrade of a server with no user asks the first admin's password before the build, and
/// hands it to the new release's reefadm as a file, with the admin's name
#[test]
fn the_first_admins_password_is_asked_before_the_build() {
    let mut script = deployed(old(PROJECT_ROW));
    script.users = Some(Ok(false));
    run(&mut script, &["upgrade"]).unwrap();
    // asked before anything is built
    let asked = script
        .log
        .iter()
        .position(|line| line.starts_with("password: "))
        .expect("the first admin's password was asked");
    let built = script
        .log
        .iter()
        .position(|line| line.contains("cargo +stable build"))
        .expect("the release was built");
    assert!(asked < built, "{:#?}", script.log);
    // and handed over with the admin running reefadm as the first admin
    let steps = script.steps();
    let last = steps.last().copied().unwrap_or_default();
    let (before, after) = last.split_once(" --password-file ").unwrap();
    assert_eq!(
        before,
        format!(
            "run: /opt/reef/releases/{NEW}/bin/reefadm upgrade --finish {NEW} --first-admin ada"
        )
    );
    assert!(after.ends_with("/password"), "{last}");
}

/// The same commits as the running release need no build, but a server with no user still gets
/// its first admin, so an upgrade run again adds one an earlier one couldn't
#[test]
fn the_same_commits_add_a_first_admin_to_a_server_with_none() {
    let manifest = Manifest::built_in();
    let mut running = old(PROJECT_ROW);
    running.reef = "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90".into();
    running.shoal = manifest.shoal.rev;
    running.glommio = manifest.glommio.rev;
    running.build = Some(reefadm::cpu::Class::named("x86-64-v3").flags);
    let mut script = deployed(running);
    script.users = Some(Ok(false));
    run(&mut script, &["upgrade", "--target-cpu", "x86-64-v3"]).unwrap();
    let steps = script.steps();
    assert!(
        !steps.iter().any(|step| step.contains("cargo")),
        "{steps:#?}"
    );
    let start = steps
        .iter()
        .position(|step| step.contains("user add"))
        .unwrap_or_else(|| panic!("no first admin: {steps:#?}"));
    added_admin(&steps[start..], "ada");
}

/// A new release that doesn't come up is rolled back, the backup restored over what it wrote
#[test]
fn a_release_that_doesnt_come_up_is_rolled_back() {
    let mut script = staged();
    // loopback never answers the new release, then answers the old one at once
    for _ in 0..180 {
        script.mes.push_back(Err("connection refused".into()));
    }
    script.mes.push_back(Ok(Me {
        required: true,
        user: None,
    }));
    let backup = format!("/var/backups/reef/reef-2026-11-27T072258-{OLD}.tar.gz");
    let manifest = format!(
        "cores = 2\npaths = [\"/var/lib/reef/shoal\", \"/var/lib/reef/clones\", \"/var/lib/reef/certs\"]\n\n\
         [release]\nname = \"{OLD}\"\nreef = \"1\"\nshoal = \"2\"\nglommio = \"3\"\ntoolchain = \"n\"\ndeployed = 0\n"
    );
    script.prints(
        format!("sudo tar -xzOf {backup} reefadm-backup.toml"),
        manifest,
    );
    let error = run(&mut script, &["upgrade", "--finish", NEW])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&format!("release {NEW} didn't come up")),
        "{error}"
    );
    assert!(error.contains(&format!("back on release {OLD}")), "{error}");
    // after the start that failed: the old release back, with its unit and the data restored
    let steps = script.steps();
    let failed = steps
        .iter()
        .position(|step| *step == "run: sudo systemctl start reef.service")
        .unwrap();
    let after: Vec<&str> = steps[failed + 2..].to_vec();
    assert_eq!(
        after,
        [
            "run: sudo systemctl stop reef.service".to_string(),
            format!("run: sudo ln -sfnT /opt/reef/releases/{OLD} /opt/reef/current.new"),
            "run: sudo mv -T /opt/reef/current.new /opt/reef/current".to_string(),
            "run: sudo install -d -m 0755 /usr/local/bin".to_string(),
            "run: sudo ln -sfn /opt/reef/current/bin/reef /usr/local/bin/reef".to_string(),
            "run: sudo ln -sfn /opt/reef/current/bin/reef-serve /usr/local/bin/reef-serve".to_string(),
            "run: sudo ln -sfn /opt/reef/current/bin/reefadm /usr/local/bin/reefadm".to_string(),
            "run: sudo mv -T /var/lib/reef/shoal /var/lib/reef/shoal.failed-20261127072258".to_string(),
            "run: sudo mv -T /var/lib/reef/clones /var/lib/reef/clones.failed-20261127072258".to_string(),
            "run: sudo mv -T /var/lib/reef/certs /var/lib/reef/certs.failed-20261127072258".to_string(),
            format!("run: sudo tar -xzf {backup} -C / --exclude reefadm-backup.toml"),
            "run: sudo chown -R reef:reef /var/lib/reef/shoal /var/lib/reef/clones /var/lib/reef/certs".to_string(),
            "run: sudo systemctl reset-failed reef.service".to_string(),
            "run: sudo systemctl start reef.service".to_string(),
            "run: sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem".to_string(),
        ]
    );
    // and the record still names the old release
    assert!(!steps.iter().any(|step| step.contains("reefadm.toml")));
}

/// Uninstall leaves the data and the config; `--purge` leaves only the backups
#[test]
fn uninstall_keeps_the_data_and_purge_only_the_backups() {
    // links into current are reefadm's to remove
    let links = |script: &mut Script| {
        for command in ["reef", "reef-serve", "reefadm"] {
            script.prints(
                format!("readlink /usr/local/bin/{command}"),
                format!("/opt/reef/current/bin/{command}\n"),
            );
        }
    };
    let mut script = deployed(old(PROJECT_ROW));
    links(&mut script);
    run(&mut script, &["uninstall"]).unwrap();
    let removed = [
        "run: sudo systemctl disable --now reef.service",
        "run: sudo rm -f /etc/systemd/system/reef.service",
        "run: sudo systemctl daemon-reload",
        "run: sudo rm -f /usr/local/bin/reef",
        "run: sudo rm -f /usr/local/bin/reef-serve",
        "run: sudo rm -f /usr/local/bin/reefadm",
        "run: sudo rm -f /opt/reef/current",
        "run: sudo rm -rf /opt/reef/releases",
        "run: sudo rmdir --ignore-fail-on-non-empty /opt/reef",
        "run: sudo rm -f /etc/reef/reef-ca.pem",
        "run: sudo rm -f /etc/reef/reefadm.toml",
    ];
    assert_eq!(script.steps(), removed);
    // with --purge, the data, the config, the account and the checkouts too, but no backup
    let mut script = deployed(old(PROJECT_ROW));
    links(&mut script);
    script.prints("id -u reef", "998\n");
    script.prints("sudo test -e /var/backups/reef", "");
    run(&mut script, &["uninstall", "--purge"]).unwrap();
    let purged: Vec<&str> = script.steps()[removed.len()..].to_vec();
    assert_eq!(
        purged,
        [
            "run: sudo rm -rf /var/lib/reef/shoal".to_string(),
            "run: sudo rm -rf /var/lib/reef/clones".to_string(),
            "run: sudo rm -rf /var/lib/reef/certs".to_string(),
            "run: sudo rm -rf /etc/reef".to_string(),
            "run: sudo userdel --remove reef".to_string(),
            "run: sudo rm -rf /var/lib/reef".to_string(),
            format!("run: rm -rf {SRC}"),
        ]
    );
    assert!(
        !script
            .steps()
            .iter()
            .any(|step| step.contains("/var/backups"))
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("its backups are still in /var/backups/reef"))
    );
    // and no is no
    let mut script = deployed(old(PROJECT_ROW));
    script.yes = false;
    assert!(run(&mut script, &["uninstall"]).is_err());
    assert!(script.steps().is_empty());
}
