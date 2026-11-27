//! A release: one build's binaries and app, under a name of its own, with `current` pointing at
//! the one in use
//!
//! A release is staged as `<name>.new`, then renamed, so one that is half copied is never used.
//! `current` is swapped by making a new link beside it and renaming it over the old, which is
//! atomic, and `reef`, `reef-serve` and `reefadm` are linked to `current/bin`, so an upgrade
//! moves one link and the commands move with it. The unit runs its server under `current` too;
//! a running server keeps the release it started from, since the kernel resolves the link once.

use crate::compile::Built;
use crate::exec::{Cmd, Runner};
use crate::layout::Layout;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The commands every release holds, linked into the layout's bin
pub const COMMANDS: [&str; 3] = ["reef", "reef-serve", "reefadm"];

/// A release's name: its reef commit's first 12 characters, then when it was built, in UTC
///
/// # Arguments
///
/// * `reef` - reef's commit
/// * `now` - When it was built
///
/// # Examples
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
///
/// let now = UNIX_EPOCH + Duration::from_secs(1_795_000_000);
/// assert_eq!(
///     reefadm::release::name("992226c4a1b2c3d4e5f6", now),
///     "992226c4a1b2-20261118110640"
/// );
/// ```
pub fn name(reef: &str, now: SystemTime) -> String {
    let short: String = reef.chars().take(12).collect();
    format!("{short}-{}", stamp(now))
}

/// A moment as `YYYYmmddHHMMSS`, in UTC
///
/// # Arguments
///
/// * `now` - The moment
pub fn stamp(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(now);
    format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}")
}

/// A moment as `YYYY-mm-ddTHHMMSS`, in UTC, for a file's name
///
/// # Arguments
///
/// * `now` - The moment
pub fn dated(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(now);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}{minute:02}{second:02}")
}

/// A moment's UTC date and time, by Howard Hinnant's days-to-civil algorithm
///
/// # Arguments
///
/// * `now` - The moment
fn civil(now: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    // seconds since the epoch, split into days and the time of day
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // the day's date, counting eras of 400 years from 0000-03-01
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3600) as u32,
        (rest % 3600 / 60) as u32,
        (rest % 60) as u32,
    )
}

/// Copies a build into a new release, swaps `current` to it and links the commands, returning
/// the release's directory
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `built` - What the build made
/// * `name` - The release's name
pub fn install(
    runner: &mut dyn Runner,
    layout: &Layout,
    built: &Built,
    name: &str,
) -> Result<PathBuf> {
    let owner = layout.owner();
    let release = layout.releases.join(name);
    let staged = layout.releases.join(format!("{name}.new"));
    // whatever a stage that stopped partway left, gone first
    runner.run(&Cmd::new("rm").arg("-rf").arg(&staged).by(owner))?;
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-m", "0755"])
            .arg(staged.join("bin"))
            .by(owner),
    )?;
    // the three commands, then the app's server and public/ together
    for (command, from) in COMMANDS
        .iter()
        .zip([&built.reef, &built.serve, &built.reefadm])
    {
        runner.run(
            &Cmd::new("install")
                .args(["-m", "0755"])
                .arg(from)
                .arg(staged.join("bin").join(command))
                .by(owner),
        )?;
    }
    if let Some(web) = &built.web {
        runner.run(
            &Cmd::new("install")
                .args(["-d", "-m", "0755"])
                .arg(staged.join("app"))
                .by(owner),
        )?;
        runner.run(
            &Cmd::new("cp")
                .arg("-R")
                .arg(web.join("server"))
                .arg(web.join("public"))
                .arg(staged.join("app"))
                .by(owner),
        )?;
    }
    // a whole release, under its own name
    runner.run(
        &Cmd::new("mv")
            .arg("-T")
            .arg(&staged)
            .arg(&release)
            .by(owner),
    )?;
    swap(runner, layout, &release)?;
    // the commands, through current, so the next swap moves them too
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-m", "0755"])
            .arg(&layout.bin)
            .by(owner),
    )?;
    for command in COMMANDS {
        runner.run(
            &Cmd::new("ln")
                .arg("-sfn")
                .arg(layout.current.join("bin").join(command))
                .arg(layout.bin.join(command))
                .by(owner),
        )?;
    }
    Ok(release)
}

/// Points `current` at a release, by renaming a new link over the old one
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `release` - The release's directory
pub fn swap(runner: &mut dyn Runner, layout: &Layout, release: &Path) -> Result<()> {
    let owner = layout.owner();
    let next = layout.current.with_file_name("current.new");
    runner.run(
        &Cmd::new("ln")
            .arg("-sfnT")
            .arg(release)
            .arg(&next)
            .by(owner),
    )?;
    runner.run(
        &Cmd::new("mv")
            .arg("-T")
            .arg(&next)
            .arg(&layout.current)
            .by(owner),
    )
}

#[cfg(test)]
mod tests {
    use super::{dated, install, name};
    use crate::Env;
    use crate::compile::Built;
    use crate::exec::Script;
    use crate::layout::{Layout, Mode};
    use std::time::{Duration, UNIX_EPOCH};

    /// The date of a moment is the calendar's, across a leap day and the epoch
    #[test]
    fn moments_are_dated_by_the_calendar() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(dated(at(0)), "1970-01-01T000000");
        // 2024-02-29 12:34:56 UTC
        assert_eq!(dated(at(1_709_210_096)), "2024-02-29T123456");
        // 2026-11-27 07:22:58 UTC, when C109 started
        assert_eq!(dated(at(1_795_764_178)), "2026-11-27T072258");
        assert_eq!(name("abc", at(1_795_764_178)), "abc-20261127072258");
    }

    /// A release is staged, renamed whole, made current, and its commands linked through current
    #[test]
    fn a_release_is_staged_then_made_current() {
        let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
        let built = Built {
            reef: "/b/reef".into(),
            reefadm: "/b/reefadm".into(),
            serve: "/b/reef-serve".into(),
            web: Some("/b/web".into()),
        };
        let mut script = Script::default();
        let release = install(&mut script, &layout, &built, "r1").unwrap();
        assert_eq!(release, std::path::Path::new("/opt/reef/releases/r1"));
        assert_eq!(
            script.steps(),
            [
                "run: sudo rm -rf /opt/reef/releases/r1.new",
                "run: sudo install -d -m 0755 /opt/reef/releases/r1.new/bin",
                "run: sudo install -m 0755 /b/reef /opt/reef/releases/r1.new/bin/reef",
                "run: sudo install -m 0755 /b/reef-serve /opt/reef/releases/r1.new/bin/reef-serve",
                "run: sudo install -m 0755 /b/reefadm /opt/reef/releases/r1.new/bin/reefadm",
                "run: sudo install -d -m 0755 /opt/reef/releases/r1.new/app",
                "run: sudo cp -R /b/web/server /b/web/public /opt/reef/releases/r1.new/app",
                "run: sudo mv -T /opt/reef/releases/r1.new /opt/reef/releases/r1",
                "run: sudo ln -sfnT /opt/reef/releases/r1 /opt/reef/current.new",
                "run: sudo mv -T /opt/reef/current.new /opt/reef/current",
                "run: sudo install -d -m 0755 /usr/local/bin",
                "run: sudo ln -sfn /opt/reef/current/bin/reef /usr/local/bin/reef",
                "run: sudo ln -sfn /opt/reef/current/bin/reef-serve /usr/local/bin/reef-serve",
                "run: sudo ln -sfn /opt/reef/current/bin/reefadm /usr/local/bin/reefadm",
            ]
        );
    }
}
