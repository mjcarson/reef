//! `reefadm recover`: a lost admin got back from the store itself, with the unit stopped
//!
//! It wraps `reef-serve recover`, which sets the user's password, makes them an admin and
//! revokes their tokens, or adds them as an admin if no user has the name. The store is locked
//! while reef runs, so the unit is stopped around it, and started again whatever recover did. The
//! password is asked here, or read from `--password-file`, and handed to reef-serve on its
//! standard input, since the account it runs as can't read the admin's files.

use crate::cli::RecoverArgs;
use crate::exec::{As, Cmd};
use crate::layout::Mode;
use crate::systemd::Systemd;
use crate::{Cx, health};
use anyhow::{Result, bail};

/// Recovers an admin from the store of the reef reefadm deployed
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What recover was told
pub fn recover(cx: &mut Cx, args: &RecoverArgs) -> Result<()> {
    let (layout, _) = super::deployed(cx, args.user)?;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    // the password first, so nothing stops until it is known
    let prompt = format!("{}'s new password", args.name);
    let password = cx.runner.password(&prompt, args.password_file.as_deref())?;
    if layout.mode == Mode::System {
        cx.runner.elevate()?;
    }
    // reef-serve as the account that owns the store, reading the password on its input
    let serve = layout.current.join("bin/reef-serve");
    let mut recover = Cmd::new(serve.display().to_string()).args(["recover", &args.name]);
    if layout.mode == Mode::System || crate::exec::exists(cx.runner, &layout.config, As::Me)? {
        recover = recover.arg("--conf").arg(&layout.config);
    }
    let recover = recover
        .args(["--password-file", "-"])
        .stdin(format!("{password}\n"))
        .by(match layout.mode {
            Mode::System => As::Account,
            Mode::User => As::Me,
        });
    // an in-process store is locked while reef runs, so it stops around the recovery
    let external = served.shoal.address.is_some();
    if !external {
        super::stop(cx.runner, &systemd)?;
    }
    let recovered = cx.runner.attempt(&recover)?;
    if !external {
        super::start(cx.runner, &systemd)?;
    }
    let hostname = super::hostname(cx.runner)?;
    health::wait(cx.runner, &layout, &served, &hostname)?;
    match recovered.ok() {
        true => {
            cx.runner.say(&format!(
                "{} is an admin with the new password: `reef login --user {}`",
                args.name, args.name
            ));
            Ok(())
        }
        false => bail!(
            "recovering {} failed: {}",
            args.name,
            format!("{}{}", recovered.stdout, recovered.stderr).trim()
        ),
    }
}
