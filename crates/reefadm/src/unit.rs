//! The systemd unit reef runs as, written as the book's Deploying chapter gives it
//!
//! A team server's is `docs/src/central/deploy/systemd.md`'s and a user's is
//! `docs/src/central/deploy/localhost.md`'s, with ExecStart on the current release. Its first
//! line says reefadm wrote it, which is how a later deploy tells its own unit from one made by
//! hand, and that changes belong in a drop-in, since every upgrade writes it again.

use crate::layout::{Kind, Layout, Mode};

/// The first line of every unit reefadm writes
pub const MARK: &str = "# Written by reefadm, which writes it again on every upgrade:";

/// The unit's text
///
/// # Arguments
///
/// * `layout` - The deployment's paths
/// * `kind` - The server it runs
/// * `low_port` - Whether a port it serves is under 1024, which needs a capability to bind
///
/// # Examples
///
/// ```
/// use reefadm::Env;
/// use reefadm::layout::{Kind, Layout, Mode};
///
/// let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/root")])).unwrap();
/// let text = reefadm::unit::text(&layout, Kind::App, false);
/// assert!(text.contains("ExecStart=/opt/reef/current/app/server\n"));
/// assert!(reefadm::unit::ours(&text));
/// ```
pub fn text(layout: &Layout, kind: Kind, low_port: bool) -> String {
    // the server under the current release, so an upgrade only moves the link
    let exec = layout.current.join(kind.program());
    match layout.mode {
        // a team server's, as systemd.md gives it
        Mode::System => {
            let capability = match low_port {
                true => "AmbientCapabilities=CAP_NET_BIND_SERVICE\n",
                false => "",
            };
            format!(
                "{MARK}\n# put changes in a drop-in, with `sudo systemctl edit reef`.\n\
                 [Unit]\n\
                 Description=reef's central planner\n\
                 Wants=network-online.target\n\
                 After=network-online.target\n\
                 \n\
                 [Service]\n\
                 User=reef\n\
                 Group=reef\n\
                 Environment=REEF_CONFIG={config}\n\
                 ExecStart={exec}\n\
                 Restart=on-failure\n\
                 TimeoutStopSec=10\n\
                 LimitMEMLOCK=infinity\n\
                 {capability}\
                 \n\
                 [Install]\n\
                 WantedBy=multi-user.target\n",
                config = layout.config.display(),
                exec = exec.display(),
            )
        }
        // a user's, as localhost.md gives it, seeing the paths the admin's shell does
        Mode::User => {
            let env: String = layout
                .unit_env
                .iter()
                .map(|(key, value)| format!("Environment={key}={value}\n"))
                .collect();
            format!(
                "{MARK}\n# put changes in a drop-in, with `systemctl --user edit reef`.\n\
                 [Unit]\n\
                 Description=reef's central planner\n\
                 \n\
                 [Service]\n\
                 {env}\
                 ExecStart={exec}\n\
                 Restart=on-failure\n\
                 TimeoutStopSec=10\n\
                 \n\
                 [Install]\n\
                 WantedBy=default.target\n",
                exec = exec.display(),
            )
        }
    }
}

/// Whether a unit's text is one reefadm wrote
///
/// # Arguments
///
/// * `text` - The unit file's text
pub fn ours(text: &str) -> bool {
    text.starts_with(MARK)
}

#[cfg(test)]
mod tests {
    use super::{ours, text};
    use crate::Env;
    use crate::layout::{Kind, Layout, Mode};

    /// Each `Key=Value` line of the first ini block on a book page, but ExecStart's
    ///
    /// # Arguments
    ///
    /// * `page` - The page's markdown
    fn documented(page: &str) -> Vec<String> {
        let start = page.find("```ini\n").unwrap() + "```ini\n".len();
        page[start..][..page[start..].find("```").unwrap()]
            .lines()
            .filter(|line| line.contains('=') && !line.starts_with("ExecStart="))
            .map(String::from)
            .collect()
    }

    /// A team server's unit sets every setting systemd.md's does, and runs the current release
    #[test]
    fn a_team_unit_has_every_documented_setting() {
        let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
        for kind in [Kind::App, Kind::Serve] {
            let unit = text(&layout, kind, false);
            let page = include_str!("../../../docs/src/central/deploy/systemd.md");
            for line in documented(page) {
                assert!(
                    unit.lines().any(|have| have == line),
                    "{line} is missing:\n{unit}"
                );
            }
            assert!(ours(&unit));
            assert!(!unit.contains("AmbientCapabilities"));
        }
        // the server each kind runs
        assert!(
            text(&layout, Kind::App, false).contains("ExecStart=/opt/reef/current/app/server\n")
        );
        assert!(
            text(&layout, Kind::Serve, false)
                .contains("ExecStart=/opt/reef/current/bin/reef-serve\n")
        );
        // and a port under 1024 can be bound
        assert!(
            text(&layout, Kind::App, true).contains("AmbientCapabilities=CAP_NET_BIND_SERVICE\n")
        );
    }

    /// A user's unit sets every setting localhost.md's does, and sees the admin's XDG paths
    #[test]
    fn a_user_unit_has_every_documented_setting() {
        let env = Env::from_pairs([("HOME", "/home/ada"), ("XDG_DATA_HOME", "/data")]);
        let layout = Layout::new(Mode::User, &env).unwrap();
        let unit = text(&layout, Kind::Serve, false);
        let page = include_str!("../../../docs/src/central/deploy/localhost.md");
        for line in documented(page) {
            assert!(
                unit.lines().any(|have| have == line),
                "{line} is missing:\n{unit}"
            );
        }
        assert!(unit.contains("ExecStart=/data/reef/current/bin/reef-serve\n"));
        assert!(unit.contains("Environment=XDG_DATA_HOME=/data\n"));
        // a unit made by hand isn't reefadm's
        assert!(!ours("[Unit]\nDescription=reef's central planner\n"));
    }
}
