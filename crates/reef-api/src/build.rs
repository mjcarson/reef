//! Which build of `reef` a binary is: its version, the commit it was built from and when that
//! commit was made, and the target it runs on
//!
//! A release's `reef` is stamped with its commit and the commit's time, which `reef version
//! --json` prints as a [`Build`]. Central reef serves its release's client and says that build on
//! `GET /api/v1/client`, and names it in its `reef-client` header on every API response
//! ([`Build::header`]), so a client can tell whether the server it calls serves a newer one
//! ([`read_header`]). A build with no stamp, as `cargo install` makes, has no commit.

use serde::{Deserialize, Serialize};

/// A build of `reef`, as `reef version --json` prints it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The crate's version, such as `0.1.0`
    pub version: String,
    /// The commit it was built from, if it was stamped with one
    pub commit: Option<String>,
    /// When that commit was made, in seconds since the epoch, if it was stamped with it
    pub committed: Option<i64>,
    /// What it runs on: its architecture, its OS and its C library, such as `x86_64-linux-musl`
    pub target: String,
}

impl Build {
    /// The commit and its time, if this build was stamped with both
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::Build;
    ///
    /// let build = Build {
    ///     version: "0.1.0".to_string(),
    ///     commit: Some("dc5e68c".to_string()),
    ///     committed: Some(1_795_000_000),
    ///     target: "x86_64-linux-musl".to_string(),
    /// };
    /// assert_eq!(build.stamp(), Some(("dc5e68c", 1_795_000_000)));
    /// assert_eq!(Build { committed: None, ..build }.stamp(), None);
    /// ```
    pub fn stamp(&self) -> Option<(&str, i64)> {
        // a commit with no time can't be ordered, so it is no stamp at all
        let commit = self.commit.as_deref().filter(|commit| !commit.is_empty())?;
        Some((commit, self.committed?))
    }

    /// Whether this build was stamped with its commit and the commit's time
    pub fn is_stamped(&self) -> bool {
        self.stamp().is_some()
    }

    /// The value of the `reef-client` header that names this build, `<commit> <committed>`, if
    /// it was stamped
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::Build;
    ///
    /// let build = Build {
    ///     version: "0.1.0".to_string(),
    ///     commit: Some("dc5e68c".to_string()),
    ///     committed: Some(1_795_000_000),
    ///     target: "x86_64-linux-musl".to_string(),
    /// };
    /// assert_eq!(build.header().as_deref(), Some("dc5e68c 1795000000"));
    /// ```
    pub fn header(&self) -> Option<String> {
        // the commit and its time, the two a client orders builds by
        self.stamp()
            .map(|(commit, committed)| format!("{commit} {committed}"))
    }

    /// The architecture this build runs on, the first part of its target, such as `x86_64`
    pub fn arch(&self) -> &str {
        // the part before the first `-`, or all of it
        self.target.split('-').next().unwrap_or_default()
    }

    /// The OS this build runs on, the second part of its target, such as `linux`
    pub fn os(&self) -> &str {
        // the part after the first `-`, up to the next
        self.target.split('-').nth(1).unwrap_or_default()
    }

    /// Whether this build runs where another does: the same architecture and OS, whichever C
    /// library each was linked with, since a static build needs none
    ///
    /// # Arguments
    ///
    /// * `other` - The build whose host is asked about
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::Build;
    ///
    /// let on = |target: &str| Build {
    ///     version: "0.1.0".to_string(),
    ///     commit: None,
    ///     committed: None,
    ///     target: target.to_string(),
    /// };
    /// assert!(on("x86_64-linux-musl").runs_beside(&on("x86_64-linux-gnu")));
    /// assert!(!on("aarch64-linux-musl").runs_beside(&on("x86_64-linux-gnu")));
    /// assert!(!on("x86_64-macos").runs_beside(&on("x86_64-linux-gnu")));
    /// ```
    pub fn runs_beside(&self, other: &Build) -> bool {
        self.arch() == other.arch() && self.os() == other.os()
    }
}

/// The commit and its time a `reef-client` header's value names, or `None` if it names nothing
/// that can be ordered
///
/// # Arguments
///
/// * `value` - The header's value, such as `dc5e68c 1795000000`
///
/// # Examples
///
/// ```
/// use reef_api::build::read_header;
///
/// assert_eq!(read_header("dc5e68c 1795000000"), Some(("dc5e68c".to_string(), 1_795_000_000)));
/// assert_eq!(read_header("dc5e68c"), None);
/// assert_eq!(read_header("dc5e68c soon"), None);
/// assert_eq!(read_header("dc5e68c 1 2"), None);
/// ```
pub fn read_header(value: &str) -> Option<(String, i64)> {
    // exactly two words: the commit, then its time in seconds
    let mut words = value.split_whitespace();
    let commit = words.next()?;
    let committed = words.next()?.parse().ok()?;
    match words.next() {
        Some(_) => None,
        None => Some((commit.to_string(), committed)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Build, read_header};

    /// A build of a target, stamped or not
    ///
    /// # Arguments
    ///
    /// * `commit` - The commit it was stamped with, if any
    /// * `committed` - That commit's time, if it was stamped with it
    fn build(commit: Option<&str>, committed: Option<i64>) -> Build {
        Build {
            version: "0.1.0".to_string(),
            commit: commit.map(str::to_string),
            committed,
            target: "x86_64-linux-musl".to_string(),
        }
    }

    /// A build is stamped only with both its commit and the commit's time, and a header names
    /// both
    #[test]
    fn a_stamp_needs_the_commit_and_its_time() {
        // both, and the header that names them
        let stamped = build(Some("dc5e68c"), Some(1_795_000_000));
        assert!(stamped.is_stamped());
        assert_eq!(
            read_header(&stamped.header().unwrap()),
            Some(("dc5e68c".to_string(), 1_795_000_000))
        );
        // either missing, or an empty commit, is no stamp and no header
        for unstamped in [
            build(None, None),
            build(Some("dc5e68c"), None),
            build(None, Some(1)),
            build(Some(""), Some(1)),
        ] {
            assert!(!unstamped.is_stamped(), "{unstamped:?}");
            assert_eq!(unstamped.header(), None);
        }
    }

    /// A build reads back from the JSON `reef version --json` prints
    #[test]
    fn a_build_reads_as_its_json() {
        let text = r#"{"version":"0.1.0","commit":"dc5e68c","committed":1795000000,"target":"x86_64-linux-musl"}"#;
        let read: Build = serde_json::from_str(text).unwrap();
        assert_eq!(read, build(Some("dc5e68c"), Some(1_795_000_000)));
        assert_eq!((read.arch(), read.os()), ("x86_64", "linux"));
        // and an unstamped one says so with nulls
        let unstamped = serde_json::to_string(&build(None, None)).unwrap();
        assert!(unstamped.contains(r#""commit":null"#), "{unstamped}");
    }
}
