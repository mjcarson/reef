//! Checks on shoal's config and data directories before shoal starts, each naming its fix
//!
//! Shoal and glommio meet a bad config with obscure errors, or none:
//!
//! - **No memory budget.** With no `resources` the budget is 0, and a shard evicts forever.
//! - **A relative data path** fails to open the directory with EBADF.
//! - **A data path on tmpfs** silently loses O_DIRECT under glommio, and the data a reboot.
//! - **A low locked-memory limit.** Below 512 KiB glommio refuses to start. Below what its
//!   executors' io buffers need, 30 MiB for each shard and 30 more, it only logs that it skipped
//!   registering them, and every read and write is slower.
//!
//! The same checks cover an in-process pool, built from reef's config, and `reef-shoal`, which
//! reads a shoal config of its own. Each problem's fix names the key to change in the file it is
//! set in.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::conf::Conf;

/// The filesystem type statfs reports for tmpfs
const TMPFS_MAGIC: i64 = 0x0102_1994;

/// The filesystem type statfs reports for ramfs
const RAMFS_MAGIC: i64 = 0x8584_58f6;

/// The least locked memory glommio starts with, as `glommio/src/sys/uring.rs` requires
pub const GLOMMIO_MIN_MEMLOCK: u64 = 512 * 1024;

/// The locked memory each executor's io buffers take: 10 MiB registered in each of three rings
///
/// A node runs one executor a shard and one more beside them, so it needs `cores + 1` of these.
/// Measured under `prlimit --memlock` with reef-shoal: one shard registers its buffers at 62 MiB
/// and skips them at 58, two at 92 and 88, three at 122 and 116.
pub const EXECUTOR_MEMLOCK: u64 = 3 * 10 * 1024 * 1024;

/// One thing wrong with shoal's config, and what to change
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What is wrong
    pub what: String,
    /// What to change
    pub fix: String,
    /// Whether shoal must not start with it; otherwise it starts, worse off
    pub fatal: bool,
}

impl Problem {
    /// A problem shoal must not start with
    ///
    /// # Arguments
    ///
    /// * `what` - What is wrong
    /// * `fix` - What to change
    fn fatal(what: String, fix: String) -> Problem {
        Problem {
            what,
            fix,
            fatal: true,
        }
    }

    /// A problem shoal starts with, worse off
    ///
    /// # Arguments
    ///
    /// * `what` - What is wrong
    /// * `fix` - What to change
    fn warning(what: String, fix: String) -> Problem {
        Problem {
            what,
            fix,
            fatal: false,
        }
    }
}

impl std::fmt::Display for Problem {
    /// Shows what is wrong, then what to change
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; {}", self.what, self.fix)
    }
}

/// The problems that kept shoal from starting
#[derive(Debug)]
pub struct Refused(pub Vec<Problem>);

impl std::fmt::Display for Refused {
    /// Lists every problem and its fix
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "shoal can't start:")?;
        for problem in &self.0 {
            write!(f, "\n- {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Refused {}

/// Where shoal's settings were written, so a fix names the key to change
#[derive(Debug, Clone, Copy)]
pub enum Keys<'a> {
    /// reef's own config, which sets every data path through `shoal.data`
    Reef(&'a Conf),
    /// A shoal config, such as the one `reef-shoal` reads
    ShoalYml(&'a Path),
}

impl Keys<'_> {
    /// Names the key that sets the memory budget
    fn memory(&self) -> String {
        match self {
            Keys::Reef(conf) => conf.key("shoal.memory"),
            Keys::ShoalYml(file) => yml_key("resources.memory", file),
        }
    }

    /// Names the key that sets how many shards run
    fn cores(&self) -> String {
        match self {
            Keys::Reef(conf) => conf.key("shoal.cores"),
            Keys::ShoalYml(file) => yml_key("resources.cores", file),
        }
    }

    /// Names the key that sets a data path
    ///
    /// # Arguments
    ///
    /// * `key` - The path's key in a shoal config
    fn path(&self, key: &str) -> String {
        match self {
            Keys::Reef(conf) => conf.key("shoal.data"),
            Keys::ShoalYml(file) => yml_key(key, file),
        }
    }
}

/// Names a key in a shoal config
///
/// # Arguments
///
/// * `key` - The key's dotted name
/// * `file` - The config it is set in
fn yml_key(key: &str, file: &Path) -> String {
    format!("`{key}` in {}", file.display())
}

/// Checks a shoal config, and the process's limits, before shoal starts with it
///
/// # Arguments
///
/// * `conf` - The config shoal would start with
/// * `keys` - Where its settings were written
pub fn check(conf: &shoal::Conf, keys: Keys<'_>) -> Vec<Problem> {
    let mut problems = Vec::new();
    // a budget of 0 is what a config with no `resources` gets
    problems.extend(memory_problem(conf.resources.memory, keys));
    // a config with no cores pins a shard to every cpu but cpu 0
    let cores = match conf.resources.cores {
        Some(cores) => cores,
        None => {
            problems.push(Problem::warning(
                "shoal's cores are unset, so it runs a shard on every cpu but cpu 0".to_string(),
                format!("set {} to the shards to run, such as 2", keys.cores()),
            ));
            std::thread::available_parallelism().map_or(1, usize::from)
        }
    };
    // every data path must be absolute and on a disk
    for (key, path) in data_paths(conf) {
        problems.extend(path_problem(&key, &path, keys));
    }
    // and the process must be able to lock what glommio needs
    match memlock_limit() {
        Ok(limit) => problems.extend(memlock_problem(limit, cores)),
        Err(error) => tracing::warn!(%error, "could not read the locked memory limit"),
    }
    problems
}

/// Checks the shoal config in a file, read the way shoal's node program reads it
///
/// # Arguments
///
/// * `file` - The shoal config
pub fn check_file(file: &Path) -> anyhow::Result<Vec<Problem>> {
    // shoal reads the file with its `SHOAL_*` variables on top
    let path = file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("{} is not a utf-8 path", file.display()))?;
    let conf = shoal::Conf::from_file(path)
        .map_err(|error| anyhow::anyhow!("reading {}: {error:?}", file.display()))?;
    Ok(check(&conf, Keys::ShoalYml(file)))
}

/// Every data path a shoal config names, each with its key
///
/// # Arguments
///
/// * `conf` - The config
fn data_paths(conf: &shoal::Conf) -> Vec<(String, PathBuf)> {
    // the default settings' two writers
    let default = &conf.storage.default.filesystem;
    let mut paths = vec![
        (
            "storage.default.filesystem.latency_sensitive.path".to_string(),
            default.latency_sensitive.path.clone(),
        ),
        (
            "storage.default.filesystem.throughput_sensitive.path".to_string(),
            default.throughput_sensitive.path.clone(),
        ),
    ];
    // then each table's own, in a stable order
    let mut tables: Vec<_> = conf.storage.tables.iter().collect();
    tables.sort_by_key(|(name, _)| name.as_str());
    for (name, settings) in tables {
        let shoal::server::conf::TableSettings::FS(fs) = settings;
        paths.push((
            format!("storage.tables.{name}.FS.latency_sensitive.path"),
            fs.latency_sensitive.path.clone(),
        ));
        paths.push((
            format!("storage.tables.{name}.FS.throughput_sensitive.path"),
            fs.throughput_sensitive.path.clone(),
        ));
    }
    paths
}

/// A memory budget of 0, which a config with no `resources` gets
///
/// # Arguments
///
/// * `memory` - Each shard's budget in bytes
/// * `keys` - Where the budget was set
fn memory_problem(memory: usize, keys: Keys<'_>) -> Option<Problem> {
    // any budget at all lets a shard stop evicting
    (memory == 0).then(|| {
        Problem::fatal(
            "shoal's memory budget is 0, so each shard evicts forever".to_string(),
            format!(
                "set {} to each shard's budget, such as \"256MiB\"",
                keys.memory()
            ),
        )
    })
}

/// A data path that is relative, or on a filesystem held in memory
///
/// # Arguments
///
/// * `key` - The path's key in a shoal config
/// * `path` - The path
/// * `keys` - Where it was set
fn path_problem(key: &str, path: &Path, keys: Keys<'_>) -> Option<Problem> {
    // a relative path fails to open with EBADF, wherever shoal was started
    if !path.is_absolute() {
        return Some(Problem::fatal(
            format!(
                "shoal's data path {} is relative, and shoal can't open it",
                path.display()
            ),
            format!("set {} to an absolute path", keys.path(key)),
        ));
    }
    // a filesystem in memory loses O_DIRECT, silently, and the data on a reboot
    match in_memory_fs(path) {
        Ok(Some(fs)) => Some(Problem::fatal(
            format!(
                "shoal's data path {} is on {fs}, where glommio silently loses O_DIRECT and a \
                 reboot loses the data",
                path.display()
            ),
            format!(
                "set {} to a directory on a disk, such as under ~/.local/share",
                keys.path(key)
            ),
        )),
        Ok(None) => None,
        Err(error) => {
            // a path that can't be looked at is shoal's to refuse, by name
            tracing::warn!(path = %path.display(), %error, "could not tell what holds a data path");
            None
        }
    }
}

/// A locked-memory limit too low for glommio to start, or to register each shard's buffers
///
/// # Arguments
///
/// * `limit` - The process's soft `RLIMIT_MEMLOCK` in bytes
/// * `cores` - How many shards will run
pub fn memlock_problem(limit: u64, cores: usize) -> Option<Problem> {
    // how to raise it, in a shell, a unit or for a login
    let raise = "raise it with `ulimit -l unlimited`, `LimitMEMLOCK=infinity` in a systemd unit, \
                 or a `memlock` line in /etc/security/limits.conf";
    // below glommio's minimum no shard starts
    if limit < GLOMMIO_MIN_MEMLOCK {
        return Some(Problem::fatal(
            format!(
                "the locked memory limit is {} KiB, below the {} KiB glommio needs to start",
                limit / 1024,
                GLOMMIO_MIN_MEMLOCK / 1024
            ),
            raise.to_string(),
        ));
    }
    // below what every executor's buffers take, they start without them, and slower
    let wanted = EXECUTOR_MEMLOCK.saturating_mul(cores as u64 + 1);
    (limit < wanted).then(|| {
        Problem::warning(
            format!(
                "the locked memory limit is {} MiB, below the {} MiB that {cores} shards and \
                 shoal's other executor register for io, so glommio skips registering it and \
                 every read and write is slower",
                limit / (1024 * 1024),
                wanted / (1024 * 1024)
            ),
            raise.to_string(),
        )
    })
}

/// The process's soft limit on locked memory, in bytes
fn memlock_limit() -> std::io::Result<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit only writes the struct it is given
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut limit) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(limit.rlim_cur)
}

/// The filesystem held in memory that a path is on, if it is on one
///
/// A path that doesn't exist yet is judged by its nearest ancestor that does, so this creates
/// nothing.
///
/// # Arguments
///
/// * `path` - The absolute path to look at
pub fn in_memory_fs(path: &Path) -> std::io::Result<Option<&'static str>> {
    // the nearest part of the path that exists, which is what the directory will be made on
    let existing = path
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .unwrap_or(Path::new("/"));
    let c_path = CString::new(existing.as_os_str().as_bytes())?;
    // SAFETY: statfs reads a nul-terminated path and only writes the struct it is given
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // tmpfs and ramfs keep their files in memory
    #[allow(clippy::unnecessary_cast)]
    Ok(match stat.f_type as i64 {
        TMPFS_MAGIC => Some("tmpfs"),
        RAMFS_MAGIC => Some("ramfs"),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Keys, check, in_memory_fs, memlock_problem};
    use crate::conf::Conf;
    use shoal::server::conf::{DefaultStorageSettings, Resources, Storage};
    use shoal::server::tables::storage::fs::conf::{
        FileSystemLatencyWriterConf, FileSystemTableConf, FileSystemThroughputWriterConf,
    };
    use std::path::{Path, PathBuf};

    /// A shoal config with a budget and two cores, its data in one directory
    ///
    /// # Arguments
    ///
    /// * `data` - The data directory
    fn conf(data: &Path) -> shoal::Conf {
        // a budget and cores, so only the paths are in question
        let resources = Resources::default()
            .cores(2)
            .memory("256MiB")
            .expect("a budget parses");
        // both writers in the one directory
        let storage = Storage::default().default_settings(
            DefaultStorageSettings::default().filesystem(
                FileSystemTableConf::default()
                    .latency_sensitive(FileSystemLatencyWriterConf::default().path(data))
                    .throughput_sensitive(FileSystemThroughputWriterConf::default().path(data)),
            ),
        );
        shoal::Conf::default().resources(resources).storage(storage)
    }

    /// A directory on a disk: the crate's own, which holds the checkout
    fn on_disk() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("data-that-is-never-made")
    }

    /// The first filesystem held in memory this host mounts where tests look for one
    fn tmpfs() -> PathBuf {
        // /dev/shm is tmpfs nearly everywhere, then /tmp and the login's runtime directory
        let uid = unsafe { libc::getuid() };
        let candidates = [
            PathBuf::from("/dev/shm"),
            PathBuf::from("/tmp"),
            PathBuf::from(format!("/run/user/{uid}")),
        ];
        candidates
            .into_iter()
            .find(|dir| matches!(in_memory_fs(dir), Ok(Some(_))))
            .expect("this test needs a tmpfs at /dev/shm, /tmp or /run/user/$UID")
            .join("reef-check-never-made")
    }

    /// A good config has no problems, and a fatal one names its key in reef's config
    #[test]
    fn zero_memory_named() {
        // the same config with no budget
        let mut zero = conf(&on_disk());
        zero.resources.memory = 0;
        let reef = Conf::default();
        let problems = check(&zero, Keys::Reef(&reef));
        // is refused, and the fix names reef's key
        let memory = problems
            .iter()
            .find(|problem| problem.what.contains("memory budget"))
            .expect("a budget of 0 is a problem");
        assert!(memory.fatal);
        assert!(memory.fix.contains("`shoal.memory`"), "{}", memory.fix);
        // while the config with a budget has no problems at all on this host
        let fine = check(&conf(&on_disk()), Keys::Reef(&reef));
        assert!(fine.iter().all(|problem| !problem.fatal), "{fine:?}");
    }

    /// A relative data path is refused, naming the key that set it in shoal's config
    #[test]
    fn relative_path_named() {
        // a config whose data sits wherever shoal was started
        let relative = conf(Path::new("data/shoal"));
        let problems = check(&relative, Keys::ShoalYml(Path::new("/etc/reef/shoal.yml")));
        // both writers' paths are refused, each by its own key
        let named: Vec<&str> = problems
            .iter()
            .filter(|problem| problem.fatal && problem.what.contains("relative"))
            .map(|problem| problem.fix.as_str())
            .collect();
        assert_eq!(named.len(), 2, "{problems:?}");
        assert!(named[0].contains("`storage.default.filesystem.latency_sensitive.path`"));
        assert!(named[1].contains("`storage.default.filesystem.throughput_sensitive.path`"));
        assert!(named[0].contains("/etc/reef/shoal.yml"));
    }

    /// A data path on tmpfs is refused before it is made
    #[test]
    fn tmpfs_path_named() {
        // a directory that doesn't exist, under a tmpfs
        let dir = tmpfs();
        let reef = Conf::default();
        let problems = check(&conf(&dir), Keys::Reef(&reef));
        // is refused by the filesystem it would be made on
        let held = problems
            .iter()
            .find(|problem| problem.what.contains("O_DIRECT"))
            .expect("a data path on tmpfs is a problem");
        assert!(held.fatal);
        assert!(held.fix.contains("`shoal.data`"), "{}", held.fix);
        // and judging it made nothing
        assert!(!dir.exists());
    }

    /// A locked-memory limit below glommio's minimum is refused
    #[test]
    fn memlock_below_glommio_refused() {
        // 256 KiB, which no shard starts under
        let problem = memlock_problem(256 * 1024, 2).expect("256 KiB is a problem");
        assert!(problem.fatal);
        assert!(problem.fix.contains("ulimit -l"), "{}", problem.fix);
    }

    /// A limit that starts glommio but can't hold every shard's buffers is a warning alone
    #[test]
    fn memlock_below_buffers_warned() {
        // a login's 8 MiB starts two shards without their buffers
        let problem = memlock_problem(8 * 1024 * 1024, 2).expect("8 MiB is a problem");
        assert!(!problem.fatal);
        // and so does 64 MiB, which holds one shard's and the other executor's but not two
        assert!(memlock_problem(64 * 1024 * 1024, 2).is_some_and(|problem| !problem.fatal));
        // while enough for both and the other executor, or no limit at all, is none
        assert_eq!(memlock_problem(90 * 1024 * 1024, 2), None);
        assert_eq!(memlock_problem(u64::MAX, 2), None);
    }
}
