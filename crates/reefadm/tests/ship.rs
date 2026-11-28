//! `reefadm --host` for the commands that build, `deploy`, `deploy --adopt` and `upgrade`: probed
//! there, built here for the target's CPU, shipped and installed there, against a scripted pair
//! of hosts; and `--shipped`, the install a target's reefadm makes of what was shipped to it

mod common;

use common::{OLD, PROJECT_ROW, SRC, deployed, env, fresh, now, old, run};
use reefadm::exec::{Dry, Script};
use reefadm::layout::{Kind, Mode};
use reefadm::release::Record;
use reefadm::ship::Facts;
use reefadm::state::{Release, State};

/// The admin's home on the target, which isn't the builder's
const THERE: &str = "/home/ops";

/// The release a build here at 2026-11-27 07:22:58 is called
const NEW: &str = "992226c4a1b2-20261127072258";

/// The flags the target's features below make, as this machine's CPU and the nightly share them
const FLAGS: &str = "-C target-cpu=x86-64 -C target-feature=+aes";

/// reefadm's copy on the target
fn copy() -> String {
    format!("{THERE}/.cache/reefadm/reefadm")
}

/// Where a release is unpacked on the target
fn shipped() -> String {
    format!("{THERE}/.cache/reefadm/shipped")
}

/// What the target's reefadm says of a plain x86-64 CPU with AES-NI
fn facts() -> Facts {
    Facts {
        arch: std::env::consts::ARCH.to_string(),
        os: Some("Ubuntu 26.04 LTS".into()),
        ubuntu: true,
        user: Some("ops".into()),
        home: THERE.into(),
        features: ["fxsr", "sse", "sse2", "aes", "pclmulqdq"]
            .map(String::from)
            .into(),
        kind: Some(Kind::App),
        // a server there with users, so no first admin is asked for unless a case says
        users: Some(true),
        ..Facts::default()
    }
}

/// A fresh builder, and europa reached with a key, reefadm's copy current there, saying its facts
///
/// # Arguments
///
/// * `asked` - The command its facts are asked with, after its copy
/// * `facts` - What it says
fn hosts(asked: &str, facts: &Facts) -> Script {
    let mut script = fresh();
    script.prints(
        "ssh -o BatchMode=yes europa -- 'uname -sm*",
        format!("Linux {}\n{THERE}\n", std::env::consts::ARCH),
    );
    script.prints("sha256sum *", "abc123  reefadm\n");
    script.prints(
        format!(
            "ssh -o BatchMode=yes europa -- 'sha256sum {} 2>/dev/null'",
            copy()
        ),
        format!("abc123  {}\n", copy()),
    );
    script.prints(
        format!("ssh -o BatchMode=yes europa -- '{} --version'", copy()),
        "reefadm 0.1.0\n",
    );
    script.prints(
        format!("ssh -o BatchMode=yes europa -- '{} {asked}'", copy()),
        serde_json::to_string(facts).unwrap(),
    );
    // the commit's own schema pin, for the checks before the build
    script.prints(
        format!("cat {SRC}/reef/central/reef-service/src/schema.rs"),
        include_str!("../../../central/reef-service/src/schema.rs"),
    );
    script
}

/// Runs a command line from the builder against europa
///
/// # Arguments
///
/// * `script` - Both hosts
/// * `line` - The command line, after `reefadm`
fn there(script: &mut dyn reefadm::exec::Runner, line: &[&str]) -> anyhow::Result<()> {
    let args: Vec<String> = line.iter().map(|word| word.to_string()).collect();
    reefadm::run_on_with("europa", &args, true, script, env(), now(), &[])
}

/// The index of the first step something matches, failing the test where none does
///
/// # Arguments
///
/// * `steps` - The steps
/// * `matches` - What it matches
fn first(steps: &[&str], matches: impl Fn(&str) -> bool) -> usize {
    steps
        .iter()
        .position(|step| matches(step))
        .unwrap_or_else(|| panic!("no such step in {steps:#?}"))
}

/// `--host` deploy, upgrade and adopt build here before anything changes on the target, ship the
/// release, and install it through `--shipped`
#[test]
fn a_host_is_deployed_with_a_release_built_here_for_its_cpu() {
    let mut script = hosts("deploy --facts --name reef.example.com", &facts());
    there(
        &mut script,
        &[
            "--host",
            "europa",
            "deploy",
            "--name",
            "reef.example.com",
            "--ref",
            "v2",
            "--src",
            SRC,
        ],
    )
    .unwrap();
    let steps = script.steps();
    // built here for what the target's CPU has, and staged here
    assert!(
        steps
            .iter()
            .any(|step| step.contains(&format!("RUSTFLAGS='{FLAGS}'"))
                && step.contains("build --release --locked -p reef-service")),
        "{steps:#?}"
    );
    let built = steps
        .iter()
        .rposition(|step| step.contains("cargo +") || step.contains("dx build"))
        .unwrap();
    let out = "/home/ada/.local/share/reefadm/out";
    let staged = first(&steps, |step| step == format!("run: rm -rf {out}"));
    // then, and only then, anything sent there: the release, the password and the install
    let sent = first(&steps, |step| step.contains("ssh"));
    assert!(built < staged && staged < sent, "{steps:#?}");
    let password = format!("{THERE}/.cache/reefadm/password-{}", std::process::id());
    assert_eq!(
        steps[sent..],
        [
            format!(
                "run: sh -c 'tar -C {out}/{NEW} -cf - . | ssh -o BatchMode=yes europa -- '\\''rm \
                 -rf {s} && mkdir -p {s} && tar -C {s} -xf -'\\'''",
                s = shipped()
            ),
            format!(
                "run: ssh -o BatchMode=yes europa -- 'umask 077 && mkdir -p {THERE}/.cache/reefadm \
                 && cat > {password}'"
            ),
            format!(
                "run: ssh europa -- '{} --yes deploy --name reef.example.com --first-admin ops \
                 --shipped {} --password-file {password}'",
                copy(),
                shipped()
            ),
            format!(
                "run: ssh -o BatchMode=yes europa -- 'rm -rf {} {password}'",
                shipped()
            ),
        ]
    );
    // the record shipped says what it was built for
    let record = script
        .files
        .iter()
        .find(|file| file.path.ends_with("release.toml"))
        .unwrap();
    let record: Record = toml::from_str(&record.text).unwrap();
    assert_eq!(record.release.build.as_deref(), Some(FLAGS));
    assert_eq!(record.reference, "v2");
    assert!(
        script
            .said
            .iter()
            .any(|line| line == &format!("building the server for `{FLAGS}`"))
    );
}

/// A dry run still copies reefadm there and asks its facts, shows the build and the ship, and
/// runs the install there as a dry run of its own
#[test]
fn a_dry_run_builds_and_ships_nothing() {
    let mut dry = Dry {
        inner: hosts("deploy --facts", &facts()),
    };
    there(&mut dry, &["--host", "europa", "--dry-run", "deploy"]).unwrap();
    let script = dry.inner;
    assert_eq!(
        script.steps(),
        [format!(
            "run: ssh europa -- '{} --dry-run --yes deploy --first-admin ops --shipped {}'",
            copy(),
            shipped()
        )]
    );
    for shown in ["cargo +stable build", "sh -c 'tar -C"] {
        assert!(
            script
                .said
                .iter()
                .any(|line| line.starts_with("would run: ") && line.contains(shown)),
            "{shown}: {:#?}",
            script.said
        );
    }
    assert!(
        script
            .probed
            .iter()
            .any(|probe| probe.contains("deploy --facts"))
    );
}

/// An upgrade there of a server with no user asks its first admin's password here before the
/// build, by the name the host's admin has, sends it there, and has the install add them
#[test]
fn a_host_with_no_user_gets_its_first_admin() {
    // the release there, at this build's commits, for another class, and no user yet
    let mut running = old(PROJECT_ROW);
    running.reef = "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90".into();
    running.build = Some("-C target-cpu=x86-64-v2".into());
    let mut said = facts();
    said.users = Some(false);
    said.state = Some(State {
        mode: Mode::System,
        kind: Kind::App,
        repo: "https://github.com/mjcarson/reef.git".into(),
        reference: "CentralizedPlanner".into(),
        release: running,
        previous: None,
        first_admin: None,
        src: None,
    });
    let mut script = hosts("upgrade --facts", &said);
    there(&mut script, &["--host", "europa", "upgrade"]).unwrap();
    // the password asked before anything is built
    let asked = script
        .log
        .iter()
        .position(|line| line.starts_with("password: "))
        .expect("the first admin's password was asked");
    let built = script
        .log
        .iter()
        .position(|line| line.contains("cargo +"))
        .expect("the release was built");
    assert!(asked < built, "{:#?}", script.log);
    // sent there where only the admin reads it, and named to the install as ops's
    let steps = script.steps();
    let sent = steps
        .iter()
        .position(|step| step.contains("cat > /home/ops/.cache/reefadm/password-"))
        .unwrap_or_else(|| panic!("the password wasn't sent: {steps:#?}"));
    let installed = steps
        .iter()
        .position(|step| step.starts_with("run: ssh europa"))
        .unwrap();
    assert!(sent < installed, "{steps:#?}");
    let (before, after) = steps[installed].split_once(" --password-file ").unwrap();
    assert_eq!(
        before,
        format!(
            "run: ssh europa -- '{} --yes upgrade --first-admin ops --shipped {}",
            copy(),
            shipped()
        )
    );
    assert!(
        after.starts_with("/home/ops/.cache/reefadm/password-"),
        "{after}"
    );
}

/// What would stop the deploy there stops it before anything is built
#[test]
fn a_problem_there_stops_it_before_the_build() {
    let mut facts = facts();
    facts.problems = vec!["port 12443 is taken: stop what listens there, or move reef's".into()];
    let mut script = hosts("deploy --facts", &facts);
    let error = there(&mut script, &["--host", "europa", "deploy"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("port 12443 is taken"), "{error}");
    assert!(script.steps().is_empty(), "{:#?}", script.steps());
}

/// An upgrade there is built here against the tables its record holds, refused before any build
/// when they changed, and skipped when its commits and its class are the running release's
#[test]
fn a_host_is_upgraded_with_a_release_built_here() {
    // the release there, at this build's commits, for another class
    let mut running = old(PROJECT_ROW);
    running.reef = "992226c4a1b2c3d4e5f60718293a4b5c6d7e8f90".into();
    running.build = Some("-C target-cpu=x86-64-v2".into());
    let state = State {
        mode: Mode::System,
        kind: Kind::App,
        repo: "https://github.com/mjcarson/reef.git".into(),
        reference: "CentralizedPlanner".into(),
        release: running,
        previous: None,
        first_admin: Some("ops".into()),
        src: Some("/home/ops/src".into()),
    };
    let mut said = facts();
    said.state = Some(state.clone());
    let mut script = hosts("upgrade --facts", &said);
    there(&mut script, &["--host", "europa", "upgrade"]).unwrap();
    let steps = script.steps();
    // built in this machine's checkouts, never the target's own
    assert!(
        steps.iter().all(|step| !step.contains("/home/ops/src")),
        "{steps:#?}"
    );
    assert!(
        steps
            .iter()
            .any(|step| step.contains("(in /home/ada/.local/share/reefadm/src/reef/central)"))
    );
    let installed = steps
        .iter()
        .find(|step| step.starts_with("run: ssh europa"))
        .unwrap();
    assert_eq!(
        *installed,
        format!(
            "run: ssh europa -- '{} --yes upgrade --shipped {}'",
            copy(),
            shipped()
        )
    );
    // the same commits for the same class are already there
    let mut same = state.clone();
    same.release.build = Some(FLAGS.into());
    let mut said = facts();
    said.state = Some(same);
    let mut script = hosts("upgrade --facts", &said);
    there(&mut script, &["--host", "europa", "upgrade"]).unwrap();
    // fetched, to know its commit, and nothing built or sent
    assert!(
        !script
            .steps()
            .iter()
            .any(|step| step.contains("cargo") || step.contains("ssh")),
        "{:#?}",
        script.steps()
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("on europa is at 992226c4a1b2 already"))
    );
    // and a schema that changed a table the data was written with is refused before any build
    let mut changed = state;
    changed
        .release
        .tables
        .insert("ProjectRow".into(), "7".into());
    let mut said = facts();
    said.state = Some(changed);
    let mut script = hosts("upgrade --facts", &said);
    let error = there(&mut script, &["--host", "europa", "upgrade"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("changed ProjectRow"), "{error}");
    assert!(
        !script.steps().iter().any(|step| step.contains("cargo")),
        "{:#?}",
        script.steps()
    );
}

/// An install made by hand there is taken over with a release built here for its server, the
/// install's own record shipped beside it
#[test]
fn a_hand_made_install_there_is_adopted_with_a_release_built_here() {
    let mut said = facts();
    said.kind = Some(Kind::Serve);
    said.tables = [("ProjectRow".to_string(), PROJECT_ROW.to_string())].into();
    let mut script = hosts("deploy --adopt --facts", &said);
    there(&mut script, &["--host", "europa", "deploy", "--adopt"]).unwrap();
    let steps = script.steps();
    assert!(
        !steps.iter().any(|step| step.contains("dx build")),
        "{steps:#?}"
    );
    let adopted = script
        .files
        .iter()
        .find(|file| file.path.ends_with("adopted.toml"))
        .unwrap();
    let adopted: Release = toml::from_str(&adopted.text).unwrap();
    assert_eq!(adopted.name, "adopted");
    assert_eq!(adopted.tables["ProjectRow"], PROJECT_ROW);
    assert!(
        steps.contains(
            &format!(
                "run: ssh europa -- '{} --yes deploy --adopt --shipped {}'",
                copy(),
                shipped()
            )
            .as_str()
        ),
        "{steps:#?}"
    );
}

/// A release shipped here is installed with no build: staged from where it was unpacked, its
/// class recorded
#[test]
fn a_shipped_release_is_installed_without_building() {
    let dir = "/home/ada/.cache/reefadm/shipped";
    let record = Record {
        repo: "https://github.com/mjcarson/reef.git".into(),
        reference: "CentralizedPlanner".into(),
        kind: Kind::App,
        release: Release {
            name: NEW.into(),
            build: Some(FLAGS.into()),
            ..old(PROJECT_ROW)
        },
    };
    let mut script = fresh();
    script.prints(
        format!("cat {dir}/release.toml"),
        toml::to_string(&record).unwrap(),
    );
    script.prints(
        format!("{dir}/bin/reef-serve version --json"),
        r#"{"schema":1,"tables":[],"features":["sse2"]}"#,
    );
    run(&mut script, &["deploy", "--shipped", dir]).unwrap();
    let steps = script.steps();
    for never in ["cargo", "git ", "rustup", "dx build"] {
        assert!(
            !steps.iter().any(|step| step.contains(never)),
            "{never}: {steps:#?}"
        );
    }
    assert!(
        steps.contains(
            &format!(
                "run: sudo install -m 0755 {dir}/bin/reef /opt/reef/releases/{NEW}.new/bin/reef"
            )
            .as_str()
        )
    );
    assert!(
        steps.contains(
            &format!(
                "run: sudo cp -R {dir}/app/server {dir}/app/public /opt/reef/releases/{NEW}.new/app"
            )
            .as_str()
        )
    );
    let state = script
        .files
        .iter()
        .find(|file| file.path.as_path() == std::path::Path::new("/etc/reef/reefadm.toml"))
        .unwrap();
    let state: State = toml::from_str(&state.text).unwrap();
    assert_eq!(state.release.build.as_deref(), Some(FLAGS));
    assert_eq!(state.release.name, NEW);
    // and an upgrade shipped here hands over to the new release's own reefadm, as one built here
    let mut script = deployed(old(PROJECT_ROW));
    script.prints(
        format!("cat {dir}/release.toml"),
        toml::to_string(&record).unwrap(),
    );
    script.prints(
        format!("{dir}/bin/reef-serve version --json"),
        r#"{"schema":1,"tables":[],"features":["sse2"]}"#,
    );
    run(&mut script, &["upgrade", "--shipped", dir]).unwrap();
    assert_eq!(
        script.steps().last().copied(),
        Some(format!("run: /opt/reef/releases/{NEW}/bin/reefadm upgrade --finish {NEW}").as_str())
    );
    assert!(!script.steps().iter().any(|step| step.contains("cargo")));
    let _ = OLD;
}

/// A record written before releases said their class still reads
#[test]
fn a_record_without_a_class_reads() {
    let text = format!(
        "mode = \"system\"\nkind = \"app\"\nrepo = \"r\"\nref = \"main\"\n\n[release]\nname = \"{OLD}\"\n\
         reef = \"a\"\nshoal = \"b\"\nglommio = \"c\"\ntoolchain = \"n\"\ndeployed = 0\n"
    );
    let state: State = toml::from_str(&text).unwrap();
    assert_eq!(state.release.build, None);
}
