//! `reefadm uninstall`: what deploy put here, removed
//!
//! The unit, the releases, `current`, the commands' links, the CA's copy and reefadm's record
//! go. The config, the data, the clones, reef's certificates, the `reef` account, the backups
//! and the checkouts stay, so a later deploy picks up where this one was. `--purge` removes
//! those too, but for the backups, which it names.

use crate::Cx;
use crate::cli::UninstallArgs;
use crate::exec::{self, ACCOUNT, As, Cmd};
use crate::layout::{Mode, UNIT};
use crate::release::COMMANDS;
use crate::systemd::Systemd;
use anyhow::{Result, bail};

/// Removes what deploy put here, and with `--purge`, reef's data and config too
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `args` - What uninstall was told
pub fn uninstall(cx: &mut Cx, args: &UninstallArgs) -> Result<()> {
    let (layout, state) = super::deployed(cx, args.user)?;
    let served = super::kept_config(cx.runner, &layout)?.unwrap_or_default();
    let systemd = Systemd::of(&layout);
    let owner = layout.owner();
    // what goes, said before it goes
    let data = [
        served.data(&layout),
        served.clones(&layout),
        served.certs(&layout),
    ];
    let question = match args.purge {
        false => {
            "remove reef's unit, releases and commands from this host, keeping its data and config?"
                .to_string()
        }
        true => format!(
            "remove reef from this host with its data, clones and certificates ({}), its config{}? \
             Only the backups in {} stay",
            data.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            match layout.mode {
                Mode::System => " and the `reef` account, its ssh keys among its home",
                Mode::User => "",
            },
            layout.backups.display()
        ),
    };
    if !cx.confirm(&question)? {
        bail!("nothing was removed");
    }
    if layout.mode == Mode::System {
        cx.runner.elevate()?;
    }
    let runner = &mut *cx.runner;
    // the unit, stopped and gone
    runner.run(&systemd.ctl(&["disable", "--now", UNIT]))?;
    runner.run(&Cmd::new("rm").arg("-f").arg(&layout.unit).by(owner))?;
    runner.run(&systemd.daemon_reload())?;
    // the commands' links, where they are still reefadm's
    for command in COMMANDS {
        let link = layout.bin.join(command);
        let points = runner.probe(&Cmd::new("readlink").arg(&link))?;
        if points.ok()
            && points
                .stdout
                .trim()
                .starts_with(&layout.current.display().to_string())
        {
            runner.run(&Cmd::new("rm").arg("-f").arg(&link).by(owner))?;
        }
    }
    // the releases, the link to the one in use, the CA's copy and the record
    runner.run(&Cmd::new("rm").arg("-f").arg(&layout.current).by(owner))?;
    runner.run(&Cmd::new("rm").arg("-rf").arg(&layout.releases).by(owner))?;
    // and a team server's /opt/reef, once nothing else is in it
    if let (Mode::System, Some(parent)) = (layout.mode, layout.releases.parent()) {
        runner.run(
            &Cmd::new("rmdir")
                .arg("--ignore-fail-on-non-empty")
                .arg(parent)
                .root(),
        )?;
    }
    if let Some(copy) = &layout.ca_copy {
        runner.run(&Cmd::new("rm").arg("-f").arg(copy).by(owner))?;
    }
    runner.run(&Cmd::new("rm").arg("-f").arg(&layout.state).by(owner))?;
    if !args.purge {
        runner.say(&format!(
            "reef is gone from this host but for its data and config: `reefadm deploy{}` puts it \
             back over them",
            if layout.mode == Mode::User {
                " --user"
            } else {
                ""
            }
        ));
        return Ok(());
    }
    // with --purge, the data, the clones, the certificates and the config too
    match layout.mode {
        Mode::System => {
            for path in &data {
                runner.run(&Cmd::new("rm").arg("-rf").arg(path).root())?;
            }
            runner.run(&Cmd::new("rm").arg("-rf").arg("/etc/reef").root())?;
            // the account, and its home with its ssh keys, once nothing runs as it
            if runner.probe(&Cmd::new("id").args(["-u", ACCOUNT]))?.ok() {
                runner.run(&Cmd::new("userdel").args(["--remove", ACCOUNT]).root())?;
            }
            runner.run(&Cmd::new("rm").arg("-rf").arg(&layout.data_home).root())?;
        }
        Mode::User => {
            for path in &data {
                runner.run(&Cmd::new("rm").arg("-rf").arg(path).by(As::Me))?;
            }
            // the user's config only if reefadm wrote it, since it may name another server too
            if let Some(text) = exec::read(runner, &layout.config, As::Me)?
                && text.starts_with("# Written by `reefadm deploy`")
            {
                runner.run(&Cmd::new("rm").arg("-f").arg(&layout.config))?;
            }
        }
    }
    // and the checkouts reef was built from
    let src = state.src.clone().unwrap_or_else(|| layout.src.clone());
    runner.run(&Cmd::new("rm").arg("-rf").arg(&src).by(As::Me))?;
    let kept = exec::exists(runner, &layout.backups, owner)?;
    runner.say(&match kept {
        true => format!(
            "reef is gone from this host; its backups are still in {}",
            layout.backups.display()
        ),
        false => "reef is gone from this host".to_string(),
    });
    Ok(())
}
