//! The unit's commands: systemctl and journalctl, for a system unit or a user's

use crate::exec::{Cmd, Runner};
use crate::layout::{Layout, Mode, UNIT};
use anyhow::Result;

/// reef's unit, as a system unit or a user's
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Systemd {
    /// Whether it is the admin's user unit
    pub user: bool,
}

impl Systemd {
    /// The unit a layout's deployment runs as
    ///
    /// # Arguments
    ///
    /// * `layout` - The deployment's paths
    pub fn of(layout: &Layout) -> Self {
        Systemd {
            user: layout.mode == Mode::User,
        }
    }

    /// `systemctl` with `--user` for a user unit, run as root for a system one
    ///
    /// # Arguments
    ///
    /// * `args` - What to ask it
    pub fn ctl(&self, args: &[&str]) -> Cmd {
        match self.user {
            true => Cmd::new("systemctl").arg("--user").args(args),
            false => Cmd::new("systemctl").args(args).root(),
        }
    }

    /// `systemctl` to read the unit's state, which needs no root
    ///
    /// # Arguments
    ///
    /// * `args` - What to ask it
    fn look(&self, args: &[&str]) -> Cmd {
        match self.user {
            true => Cmd::new("systemctl").arg("--user").args(args),
            false => Cmd::new("systemctl").args(args),
        }
    }

    /// Reloads units from disk
    pub fn daemon_reload(&self) -> Cmd {
        self.ctl(&["daemon-reload"])
    }

    /// Enables the unit at boot and starts it
    pub fn enable_now(&self) -> Cmd {
        self.ctl(&["enable", "--now", UNIT])
    }

    /// Restarts the unit
    pub fn restart(&self) -> Cmd {
        self.ctl(&["restart", UNIT])
    }

    /// Whether the unit is running
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    pub fn is_active(&self, runner: &mut dyn Runner) -> Result<bool> {
        Ok(runner
            .probe(&self.look(&["is-active", "--quiet", UNIT]))?
            .ok())
    }

    /// Whether systemd answers at all: its system manager, or the admin's user manager
    ///
    /// # Arguments
    ///
    /// * `runner` - The runner
    pub fn answers(&self, runner: &mut dyn Runner) -> Result<bool> {
        Ok(runner
            .probe(&self.look(&["show", "--property=Version"]))?
            .ok())
    }

    /// The unit's journal since boot, its last lines, read as root for a system unit, since only
    /// some groups may read it otherwise
    ///
    /// # Arguments
    ///
    /// * `lines` - How many lines from the end
    pub fn journal(&self, lines: u32) -> Cmd {
        let unit = Cmd::new("journalctl");
        let unit = match self.user {
            true => unit.arg("--user"),
            false => unit.root(),
        };
        unit.args(["-u", UNIT, "-b", "--no-pager", "-n", &lines.to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::Systemd;

    /// A system unit is changed as root, a user's with `--user` as the admin
    #[test]
    fn a_system_unit_is_changed_as_root() {
        let system = Systemd { user: false };
        assert_eq!(
            system.enable_now().shown(),
            "sudo systemctl enable --now reef.service"
        );
        assert_eq!(
            system.journal(30).shown(),
            "sudo journalctl -u reef.service -b --no-pager -n 30"
        );
        let user = Systemd { user: true };
        assert_eq!(
            user.restart().shown(),
            "systemctl --user restart reef.service"
        );
    }
}
