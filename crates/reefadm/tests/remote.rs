//! `reefadm --host`, step by step, against a scripted pair of hosts

use reefadm::exec::{Output, Script};
use reefadm::{Env, Exit};
use std::time::{Duration, UNIX_EPOCH};

/// Runs a command line on the scripted host `europa`
///
/// # Arguments
///
/// * `script` - Both hosts, as the probes see them
/// * `line` - The command line, after `reefadm`
fn run(script: &mut Script, line: &[&str]) -> anyhow::Result<()> {
    let args: Vec<String> = line.iter().map(|word| word.to_string()).collect();
    let env = Env::from_pairs([("HOME", "/home/ada"), ("USER", "ada")]);
    let now = UNIX_EPOCH + Duration::from_secs(1_795_764_178);
    reefadm::run_on("europa", &args, true, script, env, now)
}

/// europa, reached over ssh with a key, on this host's architecture, with a copy of this reefadm
///
/// # Arguments
///
/// * `system` - What `uname -sm` says there
/// * `copy` - The digest of reefadm's copy there
fn europa(system: &str, copy: &str) -> Script {
    let mut script = Script {
        yes: true,
        ..Script::default()
    };
    script.prints(
        "ssh -o BatchMode=yes europa -- 'uname -sm*",
        format!("{system}\n/home/ada\n"),
    );
    script.prints("sha256sum *", "abc123  reefadm\n");
    script.prints(
        "ssh -o BatchMode=yes europa -- 'sha256sum /home/ada/.cache/reefadm/reefadm 2>/dev/null'",
        format!("{copy}  /home/ada/.cache/reefadm/reefadm\n"),
    );
    script.prints(
        "ssh -o BatchMode=yes europa -- '/home/ada/.cache/reefadm/reefadm --version'",
        "reefadm 0.1.0\n",
    );
    script
}

/// A copy that is this reefadm already runs the command there, every argument but `--host` kept
#[test]
fn a_command_runs_there_through_the_copy() {
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "abc123");
    run(
        &mut script,
        &[
            "--host",
            "europa",
            "--dry-run",
            "deploy",
            "--name",
            "reef example",
        ],
    )
    .unwrap();
    assert_eq!(
        script.steps(),
        [
            "run: ssh europa -- '/home/ada/.cache/reefadm/reefadm --dry-run deploy --name '\\''reef example'\\'''"
        ]
    );
}

/// A copy that differs is sent again before the command, and an exit code there is this one's
#[test]
fn a_stale_copy_is_sent_and_its_exit_code_passed_back() {
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "old");
    script.attempts.push((
        "ssh europa -- '/home/ada/.cache/reefadm/reefadm status'".into(),
        Output {
            code: Some(1),
            ..Output::default()
        },
    ));
    let error = run(&mut script, &["--host=europa", "status"]).unwrap_err();
    assert_eq!(error.downcast_ref::<Exit>(), Some(&Exit(1)));
    let steps = script.steps();
    assert_eq!(steps.len(), 2, "{steps:#?}");
    assert!(
        steps[0].contains("cat > /home/ada/.cache/reefadm/reefadm.new && chmod 0755"),
        "{}",
        steps[0]
    );
}

/// A password file is read here and sent where only the admin can read it there, then removed
#[test]
fn a_password_file_is_sent_and_removed() {
    let file = std::env::temp_dir().join(format!("reefadm-remote-pw-{}", std::process::id()));
    std::fs::write(&file, "hunter2\n").unwrap();
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "abc123");
    run(
        &mut script,
        &[
            "--host",
            "europa",
            "deploy",
            "--password-file",
            &file.display().to_string(),
        ],
    )
    .unwrap();
    std::fs::remove_file(&file).unwrap();
    let there = format!("/home/ada/.cache/reefadm/password-{}", std::process::id());
    assert_eq!(
        script.steps(),
        [
            format!(
                "run: ssh -o BatchMode=yes europa -- 'umask 077 && mkdir -p /home/ada/.cache/reefadm && cat > {there}'"
            ),
            format!(
                "run: ssh europa -- '/home/ada/.cache/reefadm/reefadm deploy --password-file {there}'"
            ),
            format!("run: ssh -o BatchMode=yes europa -- 'rm -f {there}'"),
        ]
    );
}

/// A host the copy can't run on gets reefadm built there, from the reef the command names
#[test]
fn a_host_the_copy_cant_run_on_builds_its_own() {
    let mut script = europa("Linux aarch64", "abc123");
    run(&mut script, &["--host", "europa", "deploy", "--ref", "v2"]).unwrap();
    let steps = script.steps();
    assert_eq!(steps.len(), 2, "{steps:#?}");
    assert!(
        steps[0].contains(
            "install --locked --git https://github.com/mjcarson/reef.git --branch v2 reefadm \
             --root /home/ada/.cache/reefadm/cargo"
        ),
        "{}",
        steps[0]
    );
    assert_eq!(
        steps[1],
        "run: ssh europa -- '/home/ada/.cache/reefadm/cargo/bin/reefadm deploy --ref v2'"
    );
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("can't run on europa (Linux aarch64)"))
    );
    // and one that won't run there is built too, saying why past ssh's own warnings
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "abc123");
    script.answer(
        "ssh -o BatchMode=yes europa -- '/home/ada/.cache/reefadm/reefadm --version'",
        Output::failed(
            "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
             /home/ada/.cache/reefadm/reefadm: /lib/x86_64-linux-gnu/libc.so.6: version \
             `GLIBC_2.39' not found (required by /home/ada/.cache/reefadm/reefadm)\n",
        ),
    );
    run(&mut script, &["--host", "europa", "status"]).unwrap();
    assert!(
        script
            .said
            .iter()
            .any(|line| line.contains("can't run on europa: ") && line.contains("GLIBC_2.39")),
        "{:?}",
        script.said
    );
    assert_eq!(
        script.steps().last().copied(),
        Some("run: ssh europa -- '/home/ada/.cache/reefadm/cargo/bin/reefadm status'")
    );
}
