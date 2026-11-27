//! Which build this `reef` is, as `reef version` prints it
//!
//! A release's `reef` is stamped when reefadm builds it: `REEF_COMMIT` names the commit it was
//! built from and `REEF_COMMITTED` that commit's time, in seconds since the epoch, both read at
//! compile time. rustc records the variables a crate reads, so cargo builds it again when either
//! changes. A build made without them, as `cargo install` or a checkout's `cargo build` makes, is
//! not stamped, and says so.
//!
//! ```text
//! $ reef version
//! reef 0.1.0 dc5e68c1a2b3 (2026-11-27)
//! $ reef version --json
//! {"version":"0.1.0","commit":"dc5e68c1a2b3…","committed":1795800000,"target":"x86_64-linux-musl"}
//! ```

use reef_api::Build;
use std::io::Write;
use std::sync::LazyLock;

/// This binary's build: its version, the commit and time it was stamped with, if any, and the
/// target it was compiled for
pub static BUILD: LazyLock<Build> = LazyLock::new(|| {
    stamped(
        option_env!("REEF_COMMIT"),
        option_env!("REEF_COMMITTED"),
        &target(),
    )
});

/// A build of this version, stamped with what the compiler was given, if it was given both
///
/// # Arguments
///
/// * `commit` - `REEF_COMMIT` as it was at compile time, if it was set
/// * `committed` - `REEF_COMMITTED` as it was at compile time, if it was set
/// * `target` - The target it was compiled for
///
/// # Examples
///
/// ```
/// use reef::update::stamped;
///
/// let build = stamped(Some("dc5e68c"), Some("1795800000"), "x86_64-linux-musl");
/// assert_eq!(build.stamp(), Some(("dc5e68c", 1_795_800_000)));
/// assert!(!stamped(Some("dc5e68c"), Some("soon"), "x86_64-linux-gnu").is_stamped());
/// ```
pub fn stamped(commit: Option<&str>, committed: Option<&str>, target: &str) -> Build {
    // a variable set to nothing is as good as unset
    let commit = commit
        .map(str::trim)
        .filter(|commit| !commit.is_empty())
        .map(str::to_string);
    // and a time that isn't whole seconds is no time at all
    let committed = committed.and_then(|seconds| seconds.trim().parse().ok());
    Build {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit,
        committed,
        target: target.to_string(),
    }
}

/// The target this binary was compiled for: its architecture, its OS, and its C library where
/// it was linked with musl or glibc, such as `x86_64-linux-musl`
fn target() -> String {
    // the architecture and OS std was built for
    let mut target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    // and the C library, which decides whether the binary runs without the host's
    if cfg!(target_env = "musl") {
        target.push_str("-musl");
    } else if cfg!(target_env = "gnu") {
        target.push_str("-gnu");
    }
    target
}

/// A build as one line: `reef 0.1.0 <commit's first 12> (<its date>)`, or `reef 0.1.0 (not
/// stamped)`
///
/// # Arguments
///
/// * `build` - The build
///
/// # Examples
///
/// ```
/// use reef::update::{describe, stamped};
///
/// let build = stamped(Some("dc5e68c1a2b3c4d5"), Some("1795800000"), "x86_64-linux-musl");
/// assert_eq!(describe(&build), "reef 0.1.0 dc5e68c1a2b3 (2026-11-27)");
/// assert_eq!(describe(&stamped(None, None, "x86_64-linux-gnu")), "reef 0.1.0 (not stamped)");
/// ```
pub fn describe(build: &Build) -> String {
    match build.stamp() {
        // the commit as git abbreviates it, and the day it was made, in UTC
        Some((commit, committed)) => format!(
            "reef {} {} ({})",
            build.version,
            short(commit),
            day(committed)
        ),
        None => format!("reef {} (not stamped)", build.version),
    }
}

/// A commit's first 12 characters, as reefadm names a release's
///
/// # Arguments
///
/// * `commit` - The commit
pub fn short(commit: &str) -> &str {
    // its first 12 characters, or all of a shorter one
    commit
        .char_indices()
        .nth(12)
        .map_or(commit, |(end, _)| &commit[..end])
}

/// The day a time falls on, in UTC, such as `2026-11-27`
///
/// # Arguments
///
/// * `seconds` - The time, in seconds since the epoch
pub fn day(seconds: i64) -> String {
    gix::date::Time::new(seconds, 0).format_or_unix(gix::date::time::format::SHORT)
}

/// Prints this binary's build, as a line or as JSON
///
/// # Arguments
///
/// * `json` - Whether to print it as the JSON [`Build`] reads from
/// * `out` - Where to print it
pub fn version<W: Write>(json: bool, out: &mut W) -> anyhow::Result<()> {
    match json {
        // the build whole, for a server or an updater to read
        true => writeln!(out, "{}", serde_json::to_string(&*BUILD)?)?,
        // or the line a person reads
        false => writeln!(out, "{}", describe(&BUILD))?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BUILD, describe, short, stamped, version};
    use reef_api::Build;

    /// A stamped build says its commit and day, an unstamped one says it isn't stamped, and both
    /// print as the JSON a build reads back from
    #[test]
    fn a_build_prints_its_stamp_or_its_lack() {
        // stamped: the commit's first 12, and its day in UTC
        let build = stamped(
            Some("dc5e68c1a2b3c4d5e6f7"),
            Some("1795800000"),
            "x86_64-linux-musl",
        );
        assert_eq!(describe(&build), "reef 0.1.0 dc5e68c1a2b3 (2026-11-27)");
        // unstamped, or stamped with half of it, or with nothing at all
        for unstamped in [
            stamped(None, None, "x86_64-linux-gnu"),
            stamped(Some("dc5e68c"), None, "x86_64-linux-gnu"),
            stamped(Some(" "), Some("1795800000"), "x86_64-linux-gnu"),
        ] {
            assert_eq!(describe(&unstamped), "reef 0.1.0 (not stamped)");
        }
        // this test's own build, which nothing stamped, prints as JSON that reads back
        let mut out = Vec::new();
        version(true, &mut out).unwrap();
        let read: Build = serde_json::from_slice(&out).unwrap();
        assert_eq!(read, *BUILD);
        assert!(read.target.starts_with(std::env::consts::ARCH), "{read:?}");
        // and as the line
        let mut out = Vec::new();
        version(false, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), describe(&BUILD));
        // a short commit is kept whole
        assert_eq!(short("dc5e68c"), "dc5e68c");
    }
}
