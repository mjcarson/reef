//! reefadm's commands, one module each, and what they share

pub mod adopt;
pub mod backups;
pub mod deploy;
pub mod hosted;
pub mod recover;
pub mod status;
pub mod uninstall;
pub mod upgrade;

use crate::exec::{self, As, Cmd, File, Runner};
use crate::layout::{Kind, Layout, Mode};
use crate::served::{Served, port_of};
use crate::state::State;
use crate::systemd::Systemd;
use crate::{Cx, unit};
use anyhow::{Result, bail};

/// The deployment reefadm made here, and its record: the admin's user unit with `--user`, else
/// the team server, else the admin's user unit if that is all there is
///
/// # Arguments
///
/// * `cx` - The runner and the environment
/// * `user` - Whether `--user` was given
pub fn deployed(cx: &mut Cx, user: bool) -> Result<(Layout, State)> {
    let modes: &[Mode] = match user {
        true => &[Mode::User],
        false => &[Mode::System, Mode::User],
    };
    for mode in modes {
        let layout = Layout::new(*mode, &cx.env)?;
        if let Some(state) = State::read(cx.runner, &layout)? {
            return Ok((layout, state));
        }
    }
    bail!(
        "reefadm hasn't deployed reef here{}: `reefadm deploy{}` does",
        if user { " for you" } else { "" },
        if user { " --user" } else { "" }
    )
}

/// The config a deploy before this one left, if there is one
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
pub(crate) fn kept_config(runner: &mut dyn Runner, layout: &Layout) -> Result<Option<Served>> {
    // a team server's is readable by everyone, since it holds no secret
    match exec::read(runner, &layout.config, As::Me)? {
        Some(text) => Ok(Some(Served::read(&text, &layout.config)?)),
        None => Ok(None),
    }
}

/// This host's name: its fully qualified one, else its own
///
/// # Arguments
///
/// * `runner` - The runner
pub(crate) fn hostname(runner: &mut dyn Runner) -> Result<String> {
    // `hostname -f` fails, or says localhost, on a host whose /etc/hosts doesn't name it
    for cmd in [Cmd::new("hostname").arg("-f"), Cmd::new("hostname")] {
        let output = runner.probe(&cmd)?;
        let name = output.stdout.trim();
        if output.ok() && !name.is_empty() && !name.starts_with("localhost") {
            return Ok(name.to_string());
        }
    }
    Ok("localhost".to_string())
}

/// Whether a port the config serves is under 1024, which needs a capability to bind
///
/// # Arguments
///
/// * `served` - reef's config
pub(crate) fn low_port(served: &Served) -> bool {
    [served.tls_port(), port_of(&served.loopback())]
        .into_iter()
        .flatten()
        .any(|port| port < 1024)
}

/// Writes the unit if its text changed, then reloads systemd, returning whether it changed
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `systemd` - Its unit
/// * `kind` - The server it runs
/// * `low` - Whether a port it serves is under 1024
pub(crate) fn write_unit(
    runner: &mut dyn Runner,
    layout: &Layout,
    systemd: &Systemd,
    kind: Kind,
    low: bool,
) -> Result<bool> {
    let text = unit::text(layout, kind, low);
    let had = exec::read(runner, &layout.unit, As::Me)?;
    if had.as_deref() == Some(text.as_str()) {
        return Ok(false);
    }
    runner.write(&File {
        path: layout.unit.clone(),
        text,
        mode: 0o644,
        runs_as: layout.owner(),
        owner: None,
    })?;
    runner.run(&systemd.daemon_reload())?;
    Ok(true)
}

/// Stops the unit
///
/// # Arguments
///
/// * `runner` - The runner
/// * `systemd` - The unit
pub(crate) fn stop(runner: &mut dyn Runner, systemd: &Systemd) -> Result<()> {
    runner.run(&systemd.ctl(&["stop", crate::layout::UNIT]))
}

/// Starts the unit
///
/// # Arguments
///
/// * `runner` - The runner
/// * `systemd` - The unit
pub(crate) fn start(runner: &mut dyn Runner, systemd: &Systemd) -> Result<()> {
    runner.run(&systemd.ctl(&["start", crate::layout::UNIT]))
}
