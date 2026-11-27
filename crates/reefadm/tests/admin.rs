//! `reefadm status`, `logs`, `backup`, `restore` and `recover`, step by step, against a host
//! reefadm deployed

mod common;

use common::{OLD, PROJECT_ROW, deployed, old, run};
use reefadm::exec::{Output, Script};

/// The tarball a backup taken at 2026-11-27 07:22:58 of the old release writes
fn tarball() -> String {
    format!("/var/backups/reef/reef-2026-11-27T072258-{OLD}.tar.gz")
}

/// A deployed host whose unit runs, as systemd says
fn running() -> Script {
    let mut script = deployed(old(PROJECT_ROW));
    script.prints(
        "systemctl show reef.service --property=ActiveState,SubState,ActiveEnterTimestamp,UnitFileState,LimitMEMLOCK",
        "ActiveState=active\nSubState=running\nActiveEnterTimestamp=Fri 2026-11-27 09:16:40 UTC\n\
         UnitFileState=enabled\nLimitMEMLOCK=infinity\n",
    );
    script.prints("cat /etc/reef/reef-ca.pem", "PEM");
    script.prints("sudo test -e /var/lib/reef/certs/ca.pem", "");
    script.prints(
        "sudo cmp -s /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem",
        "",
    );
    script.prints(
        "sudo du -sh /var/lib/reef/shoal",
        "1.2M\t/var/lib/reef/shoal\n",
    );
    script.prints(
        "sudo du -sh /var/lib/reef/clones",
        "4.0K\t/var/lib/reef/clones\n",
    );
    script.prints(
        "df -Ph /var/lib/reef",
        "Filesystem Size Used Avail Use% Mounted on\n/dev/sda1 100G 60G 40G 60% /\n",
    );
    script.prints(
        "sudo cat /var/lib/reef/certs/leaf.toml",
        "names = [\"localhost\"]\nuntil = 1830000000\n",
    );
    script
}

/// Status says what runs and where it answers, and succeeds while it serves
#[test]
fn status_says_how_a_serving_reef_is() {
    let mut script = running();
    run(&mut script, &["status"]).unwrap();
    // it changes nothing
    assert!(script.steps().is_empty(), "{:?}", script.steps());
    let said = script.said.join("\n");
    for line in [
        format!(
            "reef here: a team server running reef-app, release {OLD} (reef 111111111111 at CentralizedPlanner)"
        ),
        "unit reef.service: active (running) since Fri 2026-11-27 09:16:40 UTC, enabled"
            .to_string(),
        "answering https://127.0.0.1:12180: callers sign in".to_string(),
        "answering https://reef.example.com:12443: callers sign in".to_string(),
        "data /var/lib/reef/shoal: 1.2M; clones /var/lib/reef/clones: 4.0K; 40G free".to_string(),
        "reef's own certificate is good until 2027-12-28".to_string(),
        "locked memory: infinity".to_string(),
    ] {
        assert!(said.contains(&line), "{line} isn't in:\n{said}");
    }
}

/// Status exits 1 when the unit is stopped, naming it, and each port that doesn't answer
#[test]
fn status_fails_when_the_unit_is_stopped() {
    let mut script = running();
    script.prints(
        "systemctl show reef.service --property=ActiveState,SubState,ActiveEnterTimestamp,UnitFileState,LimitMEMLOCK",
        "ActiveState=inactive\nSubState=dead\nActiveEnterTimestamp=\nUnitFileState=enabled\n",
    );
    script.mes.push_back(Err("connection refused".into()));
    script.mes.push_back(Err("connection refused".into()));
    let error = run(&mut script, &["status"]).unwrap_err().to_string();
    assert!(
        error.contains("the unit reef.service isn't running"),
        "{error}"
    );
    assert!(
        error.contains("https://127.0.0.1:12180/api/v1/me doesn't answer"),
        "{error}"
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line == "unit reef.service: inactive (dead), enabled")
    );
}

/// Logs show the journal, as root for a team server, following it when asked
#[test]
fn logs_show_the_journal() {
    let mut script = running();
    run(&mut script, &["logs", "-f", "-n", "20"]).unwrap();
    assert_eq!(
        script.steps(),
        ["run: sudo journalctl -u reef.service -b --no-pager -n 20 --follow"]
    );
}

/// A backup stops reef, writes the tarball, and starts it again
#[test]
fn a_backup_stops_reef_around_the_copy() {
    let mut script = running();
    run(&mut script, &["backup"]).unwrap();
    let steps = script.steps();
    assert_eq!(
        steps.first(),
        Some(&"run: sudo systemctl stop reef.service")
    );
    assert!(
        steps
            .iter()
            .any(|step| step.contains(&format!("tar -czf {}", tarball()))),
        "{steps:#?}"
    );
    assert!(steps.contains(&"run: sudo systemctl start reef.service"));
    assert!(
        script
            .said
            .iter()
            .any(|line| *line == format!("backed up reef's data to {}", tarball()))
    );
}

/// A restore asks first, warns when the backup's schema or shards differ, and moves the data aside
#[test]
fn a_restore_asks_then_moves_the_data_aside() {
    // a backup written by a release whose ProjectRow differs, with four shards
    let manifest = "cores = 4\npaths = [\"/var/lib/reef/shoal\"]\n\n[release]\nname = \"older\"\nreef = \"0\"\n\
         shoal = \"1\"\nglommio = \"2\"\ntoolchain = \"n\"\ndeployed = 0\n\n[release.tables]\nProjectRow = \"7\"\n";
    let mut script = running();
    script.answer(
        format!("sudo tar -xzOf {} reefadm-backup.toml", tarball()),
        Output::printed(manifest),
    );
    run(&mut script, &["restore", &tarball()]).unwrap();
    let asked = script
        .said
        .iter()
        .find(|line| line.starts_with("asked: "))
        .unwrap();
    assert!(asked.contains("written by release older"), "{asked}");
    assert!(asked.contains("its schema changed ProjectRow"), "{asked}");
    assert!(asked.contains("written with 4 shards"), "{asked}");
    assert_eq!(
        script.steps(),
        [
            "run: sudo systemctl stop reef.service".to_string(),
            "run: sudo mv -T /var/lib/reef/shoal /var/lib/reef/shoal.before-restore-20261127072258"
                .to_string(),
            format!(
                "run: sudo tar -xzf {} -C / --exclude reefadm-backup.toml",
                tarball()
            ),
            "run: sudo chown -R reef:reef /var/lib/reef/shoal".to_string(),
            "run: sudo systemctl reset-failed reef.service".to_string(),
            "run: sudo systemctl start reef.service".to_string(),
        ]
    );
    // and no is no
    let mut script = running();
    script.yes = false;
    script.answer(
        format!("sudo tar -xzOf {} reefadm-backup.toml", tarball()),
        Output::printed("cores = 2\npaths = []\n\n[release]\nname = \"r\"\nreef = \"0\"\nshoal = \"1\"\nglommio = \"2\"\ntoolchain = \"n\"\ndeployed = 0\n"),
    );
    assert!(run(&mut script, &["restore", &tarball()]).is_err());
    assert!(script.steps().is_empty());
}

/// Recover stops reef, runs reef-serve's recover as the account, and starts reef again
#[test]
fn recover_runs_as_the_account_with_reef_stopped() {
    let mut script = running();
    run(&mut script, &["recover", "ada"]).unwrap();
    assert_eq!(
        script.steps(),
        [
            "run: sudo systemctl stop reef.service",
            "run: sudo -u reef /opt/reef/current/bin/reef-serve recover ada --conf /etc/reef/config.toml --password-file -",
            "run: sudo systemctl start reef.service",
        ]
    );
    // and a recovery that fails says why, with reef started again
    let mut script = running();
    script.attempts.push((
        "sudo -u reef /opt/reef/current/bin/reef-serve recover*".into(),
        Output::failed("reef-serve: the store is locked"),
    ));
    let error = run(&mut script, &["recover", "ada"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("the store is locked"), "{error}");
    assert_eq!(
        script.steps().last(),
        Some(&"run: sudo systemctl start reef.service")
    );
}
