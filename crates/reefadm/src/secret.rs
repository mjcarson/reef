//! A password reefadm read once, handed to reef's CLI as a file only the admin can read
//!
//! `reef user add` and `reef login` read `--password-file`, and the CLI joins that path to its
//! working directory, so `-` names a file rather than standard input (C114). reefadm writes
//! the password to a 0600 file in a 0700 directory of its own, under `$XDG_RUNTIME_DIR` where
//! there is one, and removes both when it is done.

use crate::Env;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many secrets this process has written, so each gets a directory of its own
static WRITTEN: AtomicUsize = AtomicUsize::new(0);

/// A password written where only the admin can read it, removed when dropped
pub struct Secret {
    /// The directory it was written in, if it was written
    dir: Option<PathBuf>,
    /// The file to name as `--password-file`
    path: PathBuf,
}

impl Secret {
    /// Writes a password to a file of its own, or in a dry run, names one without writing it
    ///
    /// # Arguments
    ///
    /// * `env` - The environment, for `$XDG_RUNTIME_DIR`
    /// * `password` - The password
    /// * `dry` - Whether changes are only shown
    pub fn write(env: &Env, password: &str, dry: bool) -> Result<Self> {
        if dry {
            return Ok(Secret {
                dir: None,
                path: PathBuf::from("PASSWORD-FILE"),
            });
        }
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        // a directory only the admin can enter, in memory where the system keeps one
        let root = env
            .dir("XDG_RUNTIME_DIR")
            .unwrap_or_else(std::env::temp_dir);
        let nth = WRITTEN.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("reefadm-{}-secret-{nth}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("making {}", dir.display()))?;
        // and the password in it, readable by the admin alone
        let path = dir.join("password");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        std::io::Write::write_all(&mut file, format!("{password}\n").as_bytes())?;
        Ok(Secret {
            dir: Some(dir),
            path,
        })
    }

    /// The file to name as `--password-file`
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Secret {
    /// Removes the password and its directory
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;
    use crate::Env;
    use std::os::unix::fs::PermissionsExt;

    /// A password is written for the admin alone, and gone once dropped
    #[test]
    fn a_password_is_the_admins_alone_until_dropped() {
        let scratch =
            std::env::temp_dir().join(format!("reefadm-secret-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let env = Env::from_pairs([("XDG_RUNTIME_DIR", scratch.display().to_string())]);
        let secret = Secret::write(&env, "hunter2", false).unwrap();
        let path = secret.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hunter2\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir & 0o777, 0o700);
        drop(secret);
        assert!(!path.exists());
        // and a dry run writes nothing
        assert!(!Secret::write(&env, "x", true).unwrap().path().exists());
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
