//! `reefadm --host` for a command that runs there, step by step, against a scripted pair of hosts
//!
//! The commands that build, `deploy` and `upgrade`, are built here and shipped there, which
//! `tests/ship.rs` checks.

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
    // a reefadm built for plain x86-64, whatever this test was compiled for
    reefadm::run_on_with("europa", &args, true, script, env, now, &[])
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
            "backup",
            "--to",
            "/srv/reef backups",
        ],
    )
    .unwrap();
    assert_eq!(
        script.steps(),
        [
            "run: ssh europa -- '/home/ada/.cache/reefadm/reefadm --dry-run backup --to '\\''/srv/reef backups'\\'''"
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
            "recover",
            "ada",
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
                "run: ssh europa -- '/home/ada/.cache/reefadm/reefadm recover ada --password-file {there}'"
            ),
            format!("run: ssh -o BatchMode=yes europa -- 'rm -f {there}'"),
        ]
    );
}

/// A host of another architecture, or one the copy won't run on, is refused, saying why past
/// ssh's own warnings
#[test]
fn a_host_the_copy_cant_run_on_is_refused() {
    let mut script = europa("Linux aarch64", "abc123");
    let error = run(&mut script, &["--host", "europa", "status"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("europa is Linux aarch64"), "{error}");
    assert!(script.steps().is_empty(), "{:?}", script.steps());
    // and one whose glibc is too old for the copy
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "abc123");
    script.answer(
        "ssh -o BatchMode=yes europa -- '/home/ada/.cache/reefadm/reefadm --version'",
        Output::failed(
            "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
             /home/ada/.cache/reefadm/reefadm: /lib/x86_64-linux-gnu/libc.so.6: version \
             `GLIBC_2.39' not found (required by /home/ada/.cache/reefadm/reefadm)\n",
        ),
    );
    let error = run(&mut script, &["--host", "europa", "status"])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("can't run on europa: ") && error.contains("GLIBC_2.39"),
        "{error}"
    );
    assert!(script.steps().is_empty());
}

/// A reefadm built for this machine's CPU, as an exported RUSTFLAGS builds it, isn't copied
/// anywhere, since its copy may not run there
#[test]
fn a_reefadm_built_for_this_cpu_is_never_copied() {
    let mut script = europa(&format!("Linux {}", std::env::consts::ARCH), "old");
    let args: Vec<String> = ["--host", "europa", "status"].map(String::from).to_vec();
    let env = Env::from_pairs([("HOME", "/home/ada")]);
    let now = UNIX_EPOCH + Duration::from_secs(1_795_764_178);
    let error = reefadm::run_on_with(
        "europa",
        &args,
        true,
        &mut script,
        env,
        now,
        &["avx2", "bmi2"],
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("built for this machine's CPU, with avx2, bmi2")
            && error.contains("RUSTFLAGS unset"),
        "{error}"
    );
    assert!(script.steps().is_empty());
    assert!(script.probed.is_empty(), "{:?}", script.probed);
}
