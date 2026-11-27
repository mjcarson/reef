//! `reefadm deploy --adopt`, step by step, against a host where reef was installed by hand as
//! the book's Deploying chapter does it

mod common;

use common::{PROJECT_ROW, SRC, fresh, layout, run};
use reefadm::Env;
use reefadm::commands::adopt::{self, Named};
use reefadm::exec::Script;
use reefadm::layout::{Kind, Layout, Mode};
use reefadm::served::Served;
use reefadm::state::State;
use reefadm::systemd::Systemd;
use std::path::PathBuf;

/// The unit's properties, as adopt asks systemd for them
const SHOW: &str = "systemctl show reef.service --property=LoadState,FragmentPath,DropInPaths,ExecStart,User,Environment,ActiveState,UnitFileState";

/// The unit the book's "A systemd unit" page writes by hand
const UNIT: &str = "[Unit]\nDescription=reef's central planner\nWants=network-online.target\n\
                    After=network-online.target\n\n[Service]\nUser=reef\nGroup=reef\n\
                    Environment=REEF_CONFIG=/etc/reef/config.toml\nExecStart=/opt/reef/app/server\n\
                    Restart=on-failure\nTimeoutStopSec=10\nLimitMEMLOCK=infinity\n\n[Install]\n\
                    WantedBy=multi-user.target\n";

/// The config the book's Configuring page writes by hand, every path named
const CONFIG: &str = "[shoal]\ndata = \"/var/lib/reef/shoal\"\ncores = 2\nmemory = \"256MiB\"\n\n\
                      [api]\naddress = \"127.0.0.1:12180\"\ncerts = \"/var/lib/reef/certs\"\n\
                      ca = \"/etc/reef/reef-ca.pem\"\n\n[api.tls]\naddress = \"0.0.0.0:12443\"\n\n\
                      [auth]\nrequired = true\n\n[remotes]\nclones = \"/var/lib/reef/clones\"\n\
                      fetch_every = \"5m\"\n";

/// What `systemctl show` says of a unit
///
/// # Arguments
///
/// * `fragment` - Where systemd loads it from
/// * `argv` - What its ExecStart runs
/// * `user` - Its `User=`
/// * `environment` - Its `Environment=`
fn shown(fragment: &str, argv: &str, user: &str, environment: &str) -> String {
    format!(
        "LoadState=loaded\nFragmentPath={fragment}\nDropInPaths=\nExecStart={{ path={program} ; \
         argv[]={argv} ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; \
         code=(null) ; status=0/0 }}\nUser={user}\nEnvironment={environment}\n\
         ActiveState=active\nUnitFileState=enabled\n",
        program = argv.split_whitespace().next().unwrap_or_default()
    )
}

/// A host where reef-app was installed by hand as a team server, serving, whose checkout of reef
/// answers once it is cloned
fn hand_made() -> Script {
    let mut script = fresh();
    // the unit, its server's public/, its account, and its config
    script.prints(
        SHOW,
        shown(
            "/etc/systemd/system/reef.service",
            "/opt/reef/app/server",
            "reef",
            "REEF_CONFIG=/etc/reef/config.toml",
        ),
    );
    script.prints("cat /etc/systemd/system/reef.service", UNIT);
    script.prints("test -e /opt/reef/app/public", "");
    script.prints(
        "getent passwd reef",
        "reef:x:999:999::/var/lib/reef:/usr/sbin/nologin\n",
    );
    script.prints("sudo cat /etc/reef/config.toml", CONFIG);
    // the commands it installed, as files of their own
    script.prints("test -e /usr/local/bin/reef-serve", "");
    for command in ["reef", "reef-serve"] {
        script.prints(format!("sudo test -e /usr/local/bin/{command}"), "");
    }
    // the new commit's schema pin
    script.prints(
        format!("cat {SRC}/reef/central/reef-service/src/schema.rs"),
        include_str!("../../../central/reef-service/src/schema.rs"),
    );
    // the data a backup holds
    for path in ["shoal", "clones", "certs"] {
        script.prints(format!("sudo test -e /var/lib/reef/{path}"), "");
    }
    // it serves HTTPS, so plain HTTP isn't answered
    script
        .mes
        .push_back(Err("redirected to an untrusted certificate".into()));
    script
}

/// Adopt keeps a hand-made install's config and data, and swaps in reefadm's unit
#[test]
fn adopt_keeps_the_config_and_data_and_swaps_in_reefadms_unit() {
    let mut script = hand_made();
    run(&mut script, &["deploy", "--adopt"]).unwrap();
    let steps = script.steps();
    let at = |step: &str| {
        steps
            .iter()
            .position(|taken| taken.starts_with(step))
            .unwrap_or_else(|| panic!("{step} isn't among {steps:#?}"))
    };
    // stopped, backed up, kept, then reefadm's unit started
    let stopped = at("run: sudo systemctl stop reef.service");
    let backed_up = at(
        "run: sudo sh -c 'umask 077 && tar -czf /var/backups/reef/reef-2026-11-27T072258-adopted.tar.gz",
    );
    let unit_kept = at(
        "run: sudo cp -a /etc/systemd/system/reef.service /etc/systemd/system/reef.service.before-reefadm",
    );
    let commands_kept = [
        at("run: sudo cp -a /usr/local/bin/reef /opt/reef/releases/adopted/bin/reef"),
        at("run: sudo cp -a /usr/local/bin/reef-serve /opt/reef/releases/adopted/bin/reef-serve"),
        at("run: sudo cp -a /opt/reef/app /opt/reef/releases/adopted/app"),
    ];
    let unit_written = at("write: sudo write /etc/systemd/system/reef.service (644)");
    let started = at("run: sudo systemctl enable --now reef.service");
    assert!(stopped < backed_up && backed_up < unit_kept, "{steps:#?}");
    assert!(
        commands_kept
            .iter()
            .all(|kept| backed_up < *kept && *kept < unit_written)
    );
    assert!(unit_kept < unit_written && unit_written < started);
    // the config is kept as it is, and the data is only read: backed up, and its CA copied out
    assert!(
        !steps
            .iter()
            .any(|step| step.contains("/etc/reef/config.toml")),
        "{steps:#?}"
    );
    for step in steps.iter().filter(|step| step.contains("var/lib/reef/")) {
        assert!(
            step.contains("tar -czf")
                || step.contains("install -m 0644 /var/lib/reef/certs/ca.pem"),
            "{step}"
        );
    }
    // reefadm's unit on the new release, and its record, the install kept as the one before
    let written = |path: &str| {
        script
            .files
            .iter()
            .find(|file| file.path.as_path() == std::path::Path::new(path))
            .unwrap_or_else(|| panic!("{path} wasn't written"))
            .text
            .clone()
    };
    assert_eq!(
        written("/etc/systemd/system/reef.service"),
        reefadm::unit::text(&layout(), Kind::App, false)
    );
    let state: State = toml::from_str(&written("/etc/reef/reefadm.toml")).unwrap();
    assert_eq!(state.kind, Kind::App);
    assert_eq!(
        state.previous.map(|release| release.name).as_deref(),
        Some("adopted")
    );
    assert!(state.release.tables.contains_key("ProjectRow"));
    let said = script.said.join("\n");
    for line in [
        "found reef installed by hand: the system unit reef.service, as reef running /opt/reef/app/server",
        "its config /etc/reef/config.toml is kept as it is",
        "its unit is kept as /etc/systemd/system/reef.service.before-reefadm",
        "/opt/reef/app isn't used any more",
        "can't tell which schema the install's data was written with",
    ] {
        assert!(said.contains(line), "{line} isn't in:\n{said}");
    }
    // it served HTTPS already, so there is nothing to say of plain HTTP, and it has its projects
    assert!(!said.contains("plain HTTP"), "{said}");
    assert!(!said.contains("reef project add"), "{said}");
}

/// A unit made by hand with settings of its own, as europa's keeps reef off a benchmark's cores
const EUROPA: &str = "# reef's central planner: reef-app, run as the `reef` account.\n#\n\
                      # NOT a shoal benchmark or lab node: never wipe /var/lib/reef.\n[Unit]\n\
                      Description=reef's central planner (not a shoal node)\n\
                      Wants=network-online.target\nAfter=network-online.target\n\n[Service]\n\
                      User=reef\nGroup=reef\nSlice=reef.slice\n\
                      Environment=REEF_CONFIG=/etc/reef/config.toml\n\
                      WorkingDirectory=/var/lib/reef\nExecStart=/opt/reef/app/server\n\
                      Restart=on-failure\nTimeoutStopSec=10\n# glommio's rings and buffers\n\
                      LimitMEMLOCK=infinity\n# core 15's first thread, which the config's \
                      exclude_cores\n# leaves the shard\nCPUAffinity=15\nAllowedCPUs=15\n\
                      # a bound, so a runaway reef can't take a benchmark's memory\n\
                      MemoryHigh=768M\nMemoryMax=1G\n\n[Install]\nWantedBy=multi-user.target\n";

/// The settings a unit made by hand has and reefadm's lacks are kept in a drop-in, with the
/// comments that say why, and taken away again if the install goes back
#[test]
fn a_units_own_settings_are_kept_in_a_drop_in() {
    let mut script = hand_made();
    script.prints("cat /etc/systemd/system/reef.service", EUROPA);
    run(&mut script, &["deploy", "--adopt"]).unwrap();
    let steps = script.steps();
    let at = |step: &str| {
        steps
            .iter()
            .position(|taken| *taken == step)
            .unwrap_or_else(|| panic!("{step}: {steps:#?}"))
    };
    let written = at("write: sudo write /etc/systemd/system/reef.service.d/adopted.conf (644)");
    assert!(at("run: sudo install -d -m 0755 /etc/systemd/system/reef.service.d") < written);
    assert!(written < at("run: sudo systemctl daemon-reload"));
    assert!(written < at("run: sudo systemctl enable --now reef.service"));
    let drop_in = &script
        .files
        .iter()
        .find(|file| {
            file.path.as_path()
                == std::path::Path::new("/etc/systemd/system/reef.service.d/adopted.conf")
        })
        .unwrap()
        .text;
    for kept in [
        "# NOT a shoal benchmark or lab node: never wipe /var/lib/reef.",
        "[Unit]\nDescription=reef's central planner (not a shoal node)\n",
        "Slice=reef.slice\nWorkingDirectory=/var/lib/reef\n",
        "# core 15's first thread, which the config's exclude_cores\n# leaves the shard\nCPUAffinity=15\nAllowedCPUs=15\n",
        "MemoryHigh=768M\nMemoryMax=1G\n",
    ] {
        assert!(drop_in.contains(kept), "{kept} isn't in:\n{drop_in}");
    }
    // what reefadm's unit says already, or says itself, stays out
    for left in [
        "ExecStart",
        "REEF_CONFIG",
        "LimitMEMLOCK",
        "User=",
        "WantedBy",
        "[Install]",
        "glommio's rings",
    ] {
        assert!(!drop_in.contains(left), "{left} is in:\n{drop_in}");
    }
    assert!(
        script.said.iter().any(|line| line
            == "  its unit's own settings are kept in /etc/systemd/system/reef.service.d/adopted.conf: \
                Description, Slice, WorkingDirectory, CPUAffinity, AllowedCPUs, MemoryHigh, MemoryMax"),
        "{:#?}",
        script.said
    );
    // and an install put back loses the drop-in before systemd reloads
    let mut script = hand_made();
    script.prints("cat /etc/systemd/system/reef.service", EUROPA);
    for _ in 0..180 {
        script.mes.push_back(Err("connection refused".into()));
    }
    script.prints(
        "sudo tar -xzOf /var/backups/reef/reef-2026-11-27T072258-adopted.tar.gz reefadm-backup.toml",
        "cores = 2\npaths = []\n\n[release]\nname = \"adopted\"\nreef = \"unknown\"\nshoal = \"unknown\"\n\
         glommio = \"unknown\"\ntoolchain = \"unknown\"\ndeployed = 0\n",
    );
    assert!(run(&mut script, &["deploy", "--adopt"]).is_err());
    let steps = script.steps();
    let removed = steps
        .iter()
        .rposition(|step| {
            *step == "run: sudo rm -f /etc/systemd/system/reef.service.d/adopted.conf"
        })
        .unwrap_or_else(|| panic!("{steps:#?}"));
    assert_eq!(steps[removed + 1], "run: sudo systemctl daemon-reload");
}

/// A release that doesn't come up puts the install back: its unit, its commands, its data
#[test]
fn adopt_puts_the_install_back_if_the_release_doesnt_come_up() {
    let mut script = hand_made();
    for _ in 0..180 {
        script.mes.push_back(Err("connection refused".into()));
    }
    let backup = "/var/backups/reef/reef-2026-11-27T072258-adopted.tar.gz";
    script.prints(
        format!("sudo tar -xzOf {backup} reefadm-backup.toml"),
        "cores = 2\npaths = [\"/var/lib/reef/shoal\"]\n\n[release]\nname = \"adopted\"\n\
         reef = \"unknown\"\nshoal = \"unknown\"\nglommio = \"unknown\"\ntoolchain = \"unknown\"\n\
         deployed = 0\n",
    );
    let error = run(&mut script, &["deploy", "--adopt"])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("didn't come up, so the install made by hand is back as it was"),
        "{error}"
    );
    // after the start, its CA copied out, then the ports that never answered: everything as it
    // was, and the old unit started again
    let steps = script.steps();
    let started = steps
        .iter()
        .position(|step| *step == "run: sudo systemctl enable --now reef.service")
        .unwrap();
    assert_eq!(
        steps[started + 1],
        "run: sudo install -m 0644 /var/lib/reef/certs/ca.pem /etc/reef/reef-ca.pem"
    );
    assert_eq!(
        steps[started + 2..],
        [
            "run: sudo systemctl stop reef.service".to_string(),
            "run: sudo mv -f -T /etc/systemd/system/reef.service.before-reefadm /etc/systemd/system/reef.service"
                .to_string(),
            "run: sudo systemctl daemon-reload".to_string(),
            "run: sudo cp -a --remove-destination /opt/reef/releases/adopted/bin/reef /usr/local/bin/reef"
                .to_string(),
            "run: sudo cp -a --remove-destination /opt/reef/releases/adopted/bin/reef-serve /usr/local/bin/reef-serve"
                .to_string(),
            "run: sudo rm -f /usr/local/bin/reefadm".to_string(),
            "run: sudo mv -T /var/lib/reef/shoal /var/lib/reef/shoal.failed-20261127072258"
                .to_string(),
            format!("run: sudo tar -xzf {backup} -C / --exclude reefadm-backup.toml"),
            "run: sudo chown -R reef:reef /var/lib/reef/shoal".to_string(),
            "run: sudo systemctl reset-failed reef.service".to_string(),
            "run: sudo systemctl start reef.service".to_string(),
        ]
    );
    // and nothing says reefadm deployed it
    assert!(
        !script
            .files
            .iter()
            .any(|file| file.path.as_path() == std::path::Path::new("/etc/reef/reefadm.toml"))
    );
}

/// A config that leaves its paths to reef's defaults is written where reefadm reads it, naming
/// each as reef found it under the account's home, and every other key kept
#[test]
fn a_config_leaving_paths_to_defaults_is_written_naming_them() {
    let mut script = hand_made();
    // reef-serve, given a config of its own, and one that names no path
    script.prints(
        SHOW,
        shown(
            "/etc/systemd/system/reef.service",
            "/usr/local/bin/reef-serve --conf /etc/reef/reef.toml",
            "reef",
            "",
        ),
    );
    script.prints(
        "sudo cat /etc/reef/reef.toml",
        "# reef on europa: never wipe /var/lib/reef\n[api] # loopback\naddress = \"127.0.0.1:12180\"\n\n\
         [api.tls]\naddress = \"0.0.0.0:12443\"\n\n[auth]\nrequired = true\n",
    );
    let layout = layout();
    let found = adopt::find(&mut script, &layout, &Systemd::of(&layout)).unwrap();
    assert_eq!(found.kind, Kind::Serve);
    assert_eq!(found.config, PathBuf::from("/etc/reef/reef.toml"));
    let home = PathBuf::from("/var/lib/reef/.local/share/reef");
    assert_eq!(
        found.named,
        [
            Named {
                table: "shoal",
                key: "data",
                path: home.join("shoal")
            },
            Named {
                table: "remotes",
                key: "clones",
                path: home.join("clones")
            },
            Named {
                table: "api",
                key: "certs",
                path: home.join("certs")
            },
        ]
    );
    // the paths reef found, named, and the install's own keys and comments kept
    let text = found.rewritten(&layout).unwrap().unwrap();
    assert!(text.starts_with("# Written by `reefadm deploy --adopt` from /etc/reef/reef.toml"));
    assert!(
        text.contains(
            "# reef on europa: never wipe /var/lib/reef\n[api] # loopback\n\
             certs = \"/var/lib/reef/.local/share/reef/certs\"\naddress = \"127.0.0.1:12180\"\n"
        ),
        "{text}"
    );
    assert!(text.ends_with("\n[shoal]\ndata = \"/var/lib/reef/.local/share/reef/shoal\"\n\n[remotes]\nclones = \"/var/lib/reef/.local/share/reef/clones\"\n"), "{text}");
    let served = Served::read(&text, &layout.config).unwrap();
    assert_eq!(served.data(&layout), home.join("shoal"));
    assert_eq!(served.clones(&layout), home.join("clones"));
    assert_eq!(served.certs(&layout), home.join("certs"));
    assert_eq!(served.tls_port(), Some(12443));
    assert_eq!(served.auth.required, Some(true));
    // and the book's own config, every path named, is read the same as it is
    let mut script = hand_made();
    let found = adopt::find(&mut script, &layout, &Systemd::of(&layout)).unwrap();
    assert!(found.named.is_empty());
    assert_eq!(found.rewritten(&layout).unwrap(), None);
}

/// A user unit made as the book's localhost page does keeps its reef-serve, and needs no config
#[test]
fn a_user_unit_made_by_hand_is_taken_over_without_root() {
    let mut script = fresh();
    script.prints(
        SHOW.replace("systemctl show", "systemctl --user show"),
        shown(
            "/home/ada/.config/systemd/user/reef.service",
            "/home/ada/.cargo/bin/reef-serve",
            "",
            "",
        ),
    );
    script.prints(
        "cat /home/ada/.config/systemd/user/reef.service",
        "[Unit]\nDescription=reef's central planner\n\n[Service]\nExecStart=%h/.cargo/bin/reef-serve\n",
    );
    for command in ["reef", "reef-serve"] {
        script.prints(format!("test -e /home/ada/.cargo/bin/{command}"), "");
    }
    script.prints(
        format!("cat {SRC}/reef/central/reef-service/src/schema.rs"),
        include_str!("../../../central/reef-service/src/schema.rs"),
    );
    for path in ["shoal", "clones", "certs"] {
        script.prints(format!("test -e /home/ada/.local/share/reef/{path}"), "");
    }
    script.mes.push_back(Err("redirected".into()));
    run(&mut script, &["deploy", "--user", "--adopt"]).unwrap();
    let steps = script.steps();
    assert!(
        steps.iter().all(|step| !step.contains("sudo")),
        "{steps:#?}"
    );
    assert!(steps.contains(
        &"run: cp -a /home/ada/.cargo/bin/reef-serve /home/ada/.local/share/reef/releases/adopted/bin/reef-serve"
    ));
    // reefadm's user unit runs reef-serve from the new release, with no config written
    let user = Layout::new(Mode::User, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
    let unit = script
        .files
        .iter()
        .find(|file| file.path == user.unit)
        .unwrap();
    assert_eq!(unit.text, reefadm::unit::text(&user, Kind::Serve, false));
    assert!(!script.files.iter().any(|file| file.path == user.config));
    // the reef-serve it ran is the link reefadm makes now, so there is nothing to remove
    assert!(!script.said.join("\n").contains("isn't used any more"));
}

/// What adopt can't take over, or shouldn't, stops it before anything changes
#[test]
fn what_adopt_cant_take_over_stops_it() {
    let cases: [(&str, String, &str); 3] = [
        // nothing installed
        (
            "no unit",
            "LoadState=not-found\nFragmentPath=\n".to_string(),
            "there is no system unit reef.service to take over",
        ),
        // a server run as root, rather than reef's account
        (
            "root",
            shown(
                "/etc/systemd/system/reef.service",
                "/opt/reef/app/server",
                "",
                "",
            ),
            "runs reef as root",
        ),
        // something that isn't reef
        (
            "not reef",
            shown(
                "/etc/systemd/system/reef.service",
                "/usr/bin/python3",
                "reef",
                "",
            ),
            "neither reef-app's server nor reef-serve",
        ),
    ];
    for (case, properties, said) in cases {
        let mut script = hand_made();
        script.prints(SHOW, properties);
        let error = run(&mut script, &["deploy", "--adopt"])
            .unwrap_err()
            .to_string();
        assert!(error.contains(said), "{case}: {error}");
        assert!(script.steps().is_empty(), "{case}: {:?}", script.steps());
    }
    // a unit reefadm wrote is upgraded
    let mut script = hand_made();
    script.prints(
        "cat /etc/systemd/system/reef.service",
        reefadm::unit::text(&layout(), Kind::App, false),
    );
    let error = run(&mut script, &["deploy", "--adopt"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("`reefadm upgrade`"), "{error}");
    // a commit whose schema can't read what the install's wrote is refused before the build
    let mut script = hand_made();
    script.prints(
        format!("git -C {SRC}/reef show 0123abc:central/reef-service/src/schema.rs"),
        include_str!("../../../central/reef-service/src/schema.rs").replace(PROJECT_ROW, "7"),
    );
    let error = run(&mut script, &["deploy", "--adopt", "--from-rev", "0123abc"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("changed ProjectRow"), "{error}");
    assert!(
        !script.steps().iter().any(|step| step.contains("cargo")),
        "{:?}",
        script.steps()
    );
}

/// An install that served plain HTTP, as reef did before C87, is told its clients need HTTPS
#[test]
fn an_install_on_plain_http_is_told_its_clients_need_https() {
    let mut script = hand_made();
    script.mes.clear();
    run(&mut script, &["deploy", "--adopt"]).unwrap();
    let said = script.said.join("\n");
    assert!(said.contains("it served plain HTTP"), "{said}");
    assert!(said.contains("/etc/reef/reef-ca.pem"), "{said}");
}
