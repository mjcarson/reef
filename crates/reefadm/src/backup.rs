//! A backup: the data, the clones and reef's certificates, copied while the server is stopped
//!
//! It is one tarball, `reef-<date>T<time>-<release>.tar.gz`, written under `umask 077` into a
//! directory only its owner can enter, since it holds password and token hashes, private clones
//! and reef's CA key. Each path is kept absolute, as the book's Backups page says to restore
//! them where they were, since every project keeps its clone's path. A team server's backup holds
//! the `reef` account's `.ssh` and `.gitconfig` too, if it has them, which reach private remotes.
//! A manifest inside, `reefadm-backup.toml`, says which release wrote the data, its schema and
//! its shards, and the paths it holds.

use crate::exec::{self, ACCOUNT, Cmd, File, Runner, quote};
use crate::layout::{Layout, Mode};
use crate::release;
use crate::served::Served;
use crate::state::Release;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The manifest's name, inside every backup
pub const MANIFEST: &str = "reefadm-backup.toml";

/// What a backup holds, and what wrote it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The release whose data it is, with its schema
    pub release: Release,
    /// The shards the data was written with
    pub cores: u16,
    /// Every path it holds, absolute
    pub paths: Vec<PathBuf>,
}

/// The paths a backup of a deployment holds, of those there are
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
fn paths(runner: &mut dyn Runner, layout: &Layout, served: &Served) -> Result<Vec<PathBuf>> {
    // the store, the clones and reef's own certificates
    let mut paths = vec![
        served.data(layout),
        served.clones(layout),
        served.certs(layout),
    ];
    // and the account's keys for private remotes, on a team server
    if layout.mode == Mode::System {
        paths.push(layout.data_home.join(".ssh"));
        paths.push(layout.data_home.join(".gitconfig"));
    }
    let mut there = Vec::new();
    for path in paths {
        if exec::exists(runner, &path, layout.owner())? || runner.dry() {
            there.push(path);
        }
    }
    Ok(there)
}

/// Refuses a backup of a store reef doesn't keep itself
///
/// # Arguments
///
/// * `served` - reef's config
fn in_process(served: &Served) -> Result<()> {
    match &served.shoal.address {
        Some(address) => bail!(
            "reef's store is the shoal node at {address}, which keeps its data where its shoal.yml \
             says: back that up while the node is stopped"
        ),
        None => Ok(()),
    }
}

/// Writes a backup of a stopped server's data, returning the tarball's path
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `release` - The release that wrote the data
/// * `now` - When it is taken
/// * `into` - The directory to write it in, else the deployment's backups
pub fn take(
    runner: &mut dyn Runner,
    layout: &Layout,
    served: &Served,
    release: &Release,
    now: SystemTime,
    into: Option<&Path>,
) -> Result<PathBuf> {
    in_process(served)?;
    let owner = layout.owner();
    let dir = into.unwrap_or(&layout.backups).to_path_buf();
    let file = dir.join(format!(
        "reef-{}-{}.tar.gz",
        release::dated(now),
        release.name
    ));
    // a directory only its owner enters
    runner.run(
        &Cmd::new("install")
            .args(["-d", "-m", "0700"])
            .arg(&dir)
            .by(owner),
    )?;
    // the manifest beside it for a moment, then the tarball holding both
    let paths = paths(runner, layout, served)?;
    let manifest = Manifest {
        release: release.clone(),
        cores: served.cores(),
        paths: paths.clone(),
    };
    runner.write(&File {
        path: dir.join(MANIFEST),
        text: toml::to_string(&manifest).context("writing a backup's manifest")?,
        mode: 0o600,
        runs_as: owner,
        owner: None,
    })?;
    // each path from the root, so it restores where it was
    let relative: Vec<String> = paths
        .iter()
        .map(|path| quote(path.display().to_string().trim_start_matches('/')))
        .collect();
    let script = format!(
        "umask 077 && tar -czf {} -C {} {MANIFEST} -C / {}",
        quote(&file.display().to_string()),
        quote(&dir.display().to_string()),
        relative.join(" ")
    );
    runner.run(&Cmd::new("sh").args(["-c", &script]).by(owner))?;
    runner.run(&Cmd::new("rm").arg("-f").arg(dir.join(MANIFEST)).by(owner))?;
    runner.say(&format!("backed up reef's data to {}", file.display()));
    Ok(file)
}

/// Reads a backup's manifest
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `file` - The tarball
pub fn manifest(runner: &mut dyn Runner, layout: &Layout, file: &Path) -> Result<Manifest> {
    let output = runner.probe(
        &Cmd::new("tar")
            .args(["-xzOf"])
            .arg(file)
            .arg(MANIFEST)
            .by(layout.owner()),
    )?;
    if !output.ok() {
        bail!(
            "{} isn't a backup reefadm wrote: {}",
            file.display(),
            output.stderr.trim()
        );
    }
    toml::from_str(&output.stdout).with_context(|| format!("reading {}'s manifest", file.display()))
}

/// Puts a backup back over a stopped server's data, moving what is there aside, and returns
/// where each path went
///
/// # Arguments
///
/// * `runner` - The runner
/// * `layout` - The deployment's paths
/// * `served` - reef's config
/// * `file` - The tarball
/// * `aside` - What to add to each path moved aside, such as `before-restore-20261127090000`
pub fn restore(
    runner: &mut dyn Runner,
    layout: &Layout,
    served: &Served,
    file: &Path,
    aside: &str,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    in_process(served)?;
    let owner = layout.owner();
    // the paths it holds, as its manifest says, else what a backup of this config would hold
    let paths = match manifest(runner, layout, file) {
        Ok(manifest) => manifest.paths,
        Err(_) if runner.dry() => paths(runner, layout, served)?,
        Err(error) => return Err(error),
    };
    // what is there now, moved aside rather than lost
    let mut moved = Vec::new();
    for path in &paths {
        if exec::exists(runner, path, owner)? {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let away = path.with_file_name(format!("{name}.{aside}"));
            runner.run(&Cmd::new("mv").arg("-T").arg(path).arg(&away).by(owner))?;
            moved.push((path.clone(), away));
        }
    }
    // then the backup, where it was taken from
    runner.run(
        &Cmd::new("tar")
            .args(["-xzf"])
            .arg(file)
            .args(["-C", "/", "--exclude", MANIFEST])
            .by(owner),
    )?;
    // owned by the account here, whatever its id where the backup was taken
    if layout.mode == Mode::System {
        let owned = format!("{ACCOUNT}:{ACCOUNT}");
        runner.run(&Cmd::new("chown").args(["-R", &owned]).args(&paths).root())?;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::{restore, take};
    use crate::Env;
    use crate::exec::{Output, Script};
    use crate::layout::{Layout, Mode};
    use crate::served::Served;
    use crate::state::Release;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    /// The release whose data is backed up
    fn release() -> Release {
        Release {
            name: "abc-20261127072258".into(),
            reef: "abc".into(),
            shoal: "def".into(),
            glommio: "ghi".into(),
            toolchain: "nightly".into(),
            deployed: 0,
            schema: None,
            tables: Default::default(),
        }
    }

    /// A team server's backup holds its data, clones, certificates and the account's keys, private
    #[test]
    fn a_backup_is_private_and_holds_every_path() {
        let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
        let mut script = Script::default();
        for path in ["shoal", "clones", "certs", ".ssh"] {
            script.prints(format!("sudo test -e /var/lib/reef/{path}"), "");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_795_764_178);
        let file = take(
            &mut script,
            &layout,
            &Served::default(),
            &release(),
            now,
            None,
        )
        .unwrap();
        assert_eq!(
            file,
            Path::new("/var/backups/reef/reef-2026-11-27T072258-abc-20261127072258.tar.gz")
        );
        assert_eq!(
            script.steps(),
            [
                "run: sudo install -d -m 0700 /var/backups/reef",
                "write: sudo write /var/backups/reef/reefadm-backup.toml (600)",
                "run: sudo sh -c 'umask 077 && tar -czf /var/backups/reef/reef-2026-11-27T072258-abc-20261127072258.tar.gz \
                 -C /var/backups/reef reefadm-backup.toml -C / var/lib/reef/shoal var/lib/reef/clones var/lib/reef/certs \
                 var/lib/reef/.ssh'",
                "run: sudo rm -f /var/backups/reef/reefadm-backup.toml",
            ]
        );
        assert!(script.files[0].text.contains("cores = 2"));
        // an external node's data isn't reef's to back up
        let mut served = Served::default();
        served.shoal.address = Some("127.0.0.1:12100".into());
        assert!(take(&mut script, &layout, &served, &release(), now, None).is_err());
    }

    /// A restore moves what is there aside, unpacks the backup where it was, and gives it to reef
    #[test]
    fn a_restore_moves_the_data_aside_first() {
        let layout = Layout::new(Mode::System, &Env::from_pairs([("HOME", "/home/ada")])).unwrap();
        let mut script = Script::default();
        let manifest = "cores = 2\npaths = [\"/var/lib/reef/shoal\", \"/var/lib/reef/clones\"]\n\n\
                        [release]\nname = \"r\"\nreef = \"a\"\nshoal = \"b\"\nglommio = \"c\"\n\
                        toolchain = \"n\"\ndeployed = 0\n";
        script.answer(
            "sudo tar -xzOf /b.tar.gz reefadm-backup.toml",
            Output::printed(manifest),
        );
        script.prints("sudo test -e /var/lib/reef/shoal", "");
        let moved = restore(
            &mut script,
            &layout,
            &Served::default(),
            Path::new("/b.tar.gz"),
            "failed-1",
        )
        .unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(
            script.steps(),
            [
                "run: sudo mv -T /var/lib/reef/shoal /var/lib/reef/shoal.failed-1",
                "run: sudo tar -xzf /b.tar.gz -C / --exclude reefadm-backup.toml",
                "run: sudo chown -R reef:reef /var/lib/reef/shoal /var/lib/reef/clones",
            ]
        );
    }
}
