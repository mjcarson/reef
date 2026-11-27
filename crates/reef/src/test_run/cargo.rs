//! cargo's side of `reef test`: the command line split where cargo's arguments end and
//! libtest's begin, the build that names each test binary, and each binary's tests listed
//! before any runs
//!
//! - **The build is `cargo test --no-run --message-format=json`,** whose artifacts name each
//!   test binary's executable, package and target, and whether its crate has doc tests.
//! - **Each binary is listed with `--list --format terse`** and libtest's arguments, which
//!   honour a filter, `--ignored`, `--skip` and `--exact`, so its count is what the run will
//!   run. A target with `harness = false` isn't libtest's, and is never run to list it.
//! - **The build shows what cargo shows while it builds,** its own last lines left out, since
//!   the run that follows says them again: `Executable`, and any errors. Its `Finished` is
//!   shown in place of the run's own, which times a build that found everything built.

use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use super::libtest::{Planned, visible, waiting};

/// How long one binary's listing may take before its count is left a guess
const LIST_WAIT: Duration = Duration::from_secs(10);

/// cargo's flags that pick which targets run, without which it runs doc tests too
const TARGET_PICKS: &[&str] = &[
    "--lib",
    "--bin",
    "--bins",
    "--example",
    "--examples",
    "--test",
    "--tests",
    "--bench",
    "--benches",
    "--all-targets",
];

/// libtest's options a listing can't take beside its own `--list --format terse`, each with
/// whether it takes a value
const UNLISTED: &[(&str, bool)] = &[
    ("--format", true),
    ("--logfile", true),
    ("--color", true),
    ("--list", false),
    ("-q", false),
    ("--quiet", false),
];

/// A `cargo test` command line, split where cargo's arguments end and libtest's begin
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The toolchain a leading `+toolchain` names, which goes before `test`
    pub toolchain: Option<OsString>,
    /// cargo's own arguments, before any `--`
    pub cargo: Vec<OsString>,
    /// Everything from the first `--` on, the `--` included, as typed
    pub rest: Vec<OsString>,
}

impl Args {
    /// Splits a command line typed after `reef test`
    ///
    /// # Arguments
    ///
    /// * `typed` - The arguments, as typed
    ///
    /// # Examples
    ///
    /// ```
    /// use reef::test_run::cargo::Args;
    ///
    /// let args = Args::parse(["+nightly", "-p", "reef", "--", "--ignored"].map(Into::into).to_vec());
    /// assert_eq!(args.toolchain, Some("+nightly".into()));
    /// assert_eq!(args.libtest(), ["--ignored"]);
    /// ```
    pub fn parse(typed: Vec<OsString>) -> Args {
        let mut typed = typed.into_iter().peekable();
        // a toolchain comes first, as it does after `cargo`
        let toolchain = typed.next_if(|arg| arg.to_string_lossy().starts_with('+'));
        // then cargo's arguments, up to the first `--`
        let mut cargo = Vec::new();
        while let Some(arg) = typed.next_if(|arg| arg != "--") {
            cargo.push(arg);
        }
        Args {
            toolchain,
            cargo,
            rest: typed.collect(),
        }
    }

    /// libtest's arguments, after the first `--`
    pub fn libtest(&self) -> &[OsString] {
        self.rest.get(1..).unwrap_or_default()
    }

    /// The command as a run names it: cargo, its toolchain, `test`, then the arguments
    pub fn command(&self) -> Vec<String> {
        let mut command = vec!["cargo".to_string()];
        command.extend(
            self.toolchain
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        command.push("test".to_string());
        command.extend(
            self.cargo
                .iter()
                .chain(&self.rest)
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        command
    }

    /// The arguments cargo is run with for the tests: as typed, `test` after the toolchain
    pub fn run_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = self.toolchain.iter().cloned().collect();
        args.push("test".into());
        args.extend(self.cargo.iter().chain(&self.rest).cloned());
        args
    }

    /// The arguments cargo is run with to build and name the test binaries
    pub fn build_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = self.toolchain.iter().cloned().collect();
        args.push("test".into());
        args.extend(self.cargo.iter().cloned());
        args.push("--no-run".into());
        args.push("--message-format=json".into());
        args
    }

    /// Whether cargo is given one of its own flags, alone or with its value after `=`
    ///
    /// # Arguments
    ///
    /// * `flags` - The flags, such as `--no-run`
    fn has(&self, flags: &[&str]) -> bool {
        self.cargo.iter().any(|arg| {
            let arg = arg.to_string_lossy();
            flags.iter().any(|flag| {
                arg == *flag
                    || arg
                        .strip_prefix(flag)
                        .is_some_and(|rest| rest.starts_with('='))
            })
        })
    }

    /// Whether the binaries are built and listed first: not for a build alone, nor for doc
    /// tests alone, which cargo won't build without running, nor where JSON is cargo's output
    pub fn lists(&self) -> bool {
        !self.has(&["--no-run", "--doc", "--message-format"])
    }

    /// Whether cargo runs doc tests: with `--doc`, or with no flag that picks targets
    pub fn runs_doc_tests(&self) -> bool {
        self.has(&["--doc"]) || !self.has(TARGET_PICKS)
    }

    /// Whether cargo runs quietly, naming no binary as it runs it
    pub fn quiet(&self) -> bool {
        self.has(&["-q", "--quiet"])
    }

    /// libtest's arguments for a listing: those that pick tests, without the options a
    /// listing's own `--list --format terse` can't sit beside
    pub fn list_args(&self) -> Vec<OsString> {
        let mut kept = Vec::new();
        let mut args = self.libtest().iter();
        while let Some(arg) = args.next() {
            let text = arg.to_string_lossy();
            // an option left out, alone or with its value
            let unlisted = UNLISTED.iter().find(|(option, _)| {
                text == *option
                    || text
                        .strip_prefix(option)
                        .is_some_and(|rest| rest.starts_with('='))
            });
            match unlisted {
                Some((option, true)) if text == *option => {
                    args.next();
                }
                Some(_) => {}
                None => kept.push(arg.clone()),
            }
        }
        kept
    }
}

/// The program cargo runs as: `$CARGO`, which cargo sets for whatever it runs, unless a
/// toolchain is named, which only rustup's `cargo` takes
///
/// # Arguments
///
/// * `args` - The command line
pub fn program(args: &Args) -> OsString {
    match (&args.toolchain, std::env::var_os("CARGO")) {
        (None, Some(cargo)) if !cargo.is_empty() => cargo,
        _ => "cargo".into(),
    }
}

/// One line of the build's JSON, as far as naming a test binary needs
#[derive(Debug, Deserialize)]
struct Message {
    /// What the line is, `compiler-artifact` for a unit built
    reason: String,
    /// The package's id, which names it where its manifest can't be read
    #[serde(default)]
    package_id: String,
    /// The package's manifest
    #[serde(default)]
    manifest_path: PathBuf,
    /// The target built
    target: Option<Target>,
    /// The profile it was built with
    profile: Option<Profile>,
    /// The executable built, for a test binary
    executable: Option<PathBuf>,
}

/// A target, as the build's JSON names it
#[derive(Debug, Deserialize)]
struct Target {
    /// Its kinds, such as `lib`, `bin` or `test`
    kind: Vec<String>,
    /// Its name
    name: String,
    /// Its root source file
    src_path: PathBuf,
    /// Whether it has doc tests
    #[serde(default)]
    doctest: bool,
}

/// A profile, as the build's JSON names it
#[derive(Debug, Deserialize)]
struct Profile {
    /// Whether the unit is built as a test
    test: bool,
}

/// A test binary the build made
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The package it tests
    pub package: String,
    /// Where it comes in cargo's order among its package's binaries: a lib's first, then a
    /// bin's, then integration tests, benches and examples
    pub rank: u8,
    /// Its target's name
    pub name: String,
    /// What cargo runs it as, such as `unittests src/lib.rs` or `tests/scan.rs`
    pub target: String,
    /// Its executable
    pub executable: PathBuf,
    /// Its package's directory, which cargo runs it from
    pub dir: PathBuf,
    /// The crate whose doc tests its package has, if it is a lib with them
    pub doc_crate: Option<String>,
    /// Whether libtest runs it, so it can be listed
    pub libtest: bool,
}

impl Built {
    /// The binary as a run plans it, counted if it could be listed
    ///
    /// # Arguments
    ///
    /// * `total` - How many tests its listing found, if it could be listed
    pub fn planned(&self, total: Option<u64>) -> Planned {
        let file = self
            .executable
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        Planned {
            binary: waiting(
                &self.package,
                &self.target,
                total.unwrap_or(0),
                total.is_none(),
            ),
            file,
            doc_crate: None,
            libtest: self.libtest,
        }
    }
}

/// The manifests read so far, each package's name and the targets libtest doesn't run
#[derive(Debug, Default)]
pub struct Manifests {
    /// What each manifest said, by its path
    read: HashMap<PathBuf, Manifest>,
}

/// What a manifest says that naming a test binary needs
#[derive(Debug, Clone, Default)]
struct Manifest {
    /// The package's name
    name: Option<String>,
    /// The targets with `harness = false`, each by its section and its name
    unharnessed: Vec<(String, String)>,
}

impl Manifest {
    /// Reads a manifest's package name and the targets it gives `harness = false`
    ///
    /// # Arguments
    ///
    /// * `text` - The manifest's TOML
    fn parse(text: &str) -> Manifest {
        let Ok(table) = text.parse::<toml::Table>() else {
            return Manifest::default();
        };
        let name = table
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(|name| name.as_str())
            .map(String::from);
        // the lib's own table, and each array of targets
        let unharnessed_table = |target: &toml::Value| {
            target.get("harness").and_then(toml::Value::as_bool) == Some(false)
        };
        let mut unharnessed = Vec::new();
        if let Some(lib) = table.get("lib")
            && unharnessed_table(lib)
        {
            unharnessed.push(("lib".to_string(), String::new()));
        }
        for section in ["bin", "test", "bench", "example"] {
            let targets = table.get(section).and_then(toml::Value::as_array);
            for target in targets.into_iter().flatten() {
                if unharnessed_table(target) {
                    let name = target
                        .get("name")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("");
                    unharnessed.push((section.to_string(), name.to_string()));
                }
            }
        }
        Manifest { name, unharnessed }
    }

    /// Whether libtest runs a target: unless the manifest gives it `harness = false`
    ///
    /// # Arguments
    ///
    /// * `section` - The manifest's section the target is in, such as `lib` or `test`
    /// * `name` - The target's name
    fn libtest(&self, section: &str, name: &str) -> bool {
        !self
            .unharnessed
            .iter()
            .any(|(kind, target)| kind == section && (section == "lib" || target == name))
    }
}

impl Manifests {
    /// What a manifest says, read once
    ///
    /// # Arguments
    ///
    /// * `path` - The manifest's path
    fn get(&mut self, path: &Path) -> &Manifest {
        self.read.entry(path.to_path_buf()).or_insert_with(|| {
            std::fs::read_to_string(path)
                .map(|text| Manifest::parse(&text))
                .unwrap_or_default()
        })
    }
}

/// A package's name from its id, where its manifest can't say: `path+file:///src/reef#0.1.0`
/// names `reef`, and `path+file:///src/crates/api#reef-api@0.1.0` names `reef-api`
///
/// # Arguments
///
/// * `id` - The package's id
///
/// # Examples
///
/// ```
/// use reef::test_run::cargo::package_name;
///
/// assert_eq!(package_name("path+file:///src/reef#0.1.0"), "reef");
/// assert_eq!(package_name("path+file:///src/crates/api#reef-api@0.1.0"), "reef-api");
/// ```
pub fn package_name(id: &str) -> String {
    let (path, fragment) = id.rsplit_once('#').unwrap_or((id, ""));
    match fragment.split_once('@') {
        Some((name, _)) => name.to_string(),
        None => path.rsplit('/').next().unwrap_or(path).to_string(),
    }
}

/// The test binary a line of the build's JSON names, if it names one
///
/// # Arguments
///
/// * `line` - The line
/// * `manifests` - The manifests read so far
pub fn built(line: &[u8], manifests: &mut Manifests) -> Option<Built> {
    // a unit built as a test, with an executable to run
    let message: Message = serde_json::from_slice(line).ok()?;
    if message.reason != "compiler-artifact" || !message.profile?.test {
        return None;
    }
    let executable = message.executable?;
    let target = message.target?;
    // its kind, in cargo's order, and the manifest's section for it
    let kinds = &target.kind;
    let has = |kind: &str| kinds.iter().any(|known| known == kind);
    let (rank, section) = if has("bin") {
        (1, "bin")
    } else if has("test") {
        (2, "test")
    } else if has("bench") {
        (3, "bench")
    } else if has("example") {
        (4, "example")
    } else {
        (0, "lib")
    };
    // what cargo runs it as: a lib's and a bin's unit tests, else the target's source
    let dir = message
        .manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let source = target
        .src_path
        .strip_prefix(&dir)
        .unwrap_or(&target.src_path)
        .to_string_lossy()
        .into_owned();
    let shown = match rank {
        0 | 1 => format!("unittests {source}"),
        _ => source,
    };
    // its package's name and whether libtest runs it, as its manifest says
    let manifest = manifests.get(&message.manifest_path);
    let package = manifest
        .name
        .clone()
        .unwrap_or_else(|| package_name(&message.package_id));
    let libtest = manifest.libtest(section, &target.name);
    let doc_crate = (rank == 0 && target.doctest).then(|| target.name.replace('-', "_"));
    Some(Built {
        package,
        rank,
        name: target.name,
        target: shown,
        executable,
        dir,
        doc_crate,
        libtest,
    })
}

/// The binaries a build made, in the order cargo runs them, each counted where it could be
/// listed, with each package's doc tests after them if cargo runs those
///
/// # Arguments
///
/// * `built` - The binaries, with their listings' counts
/// * `doc_tests` - Whether cargo runs doc tests
pub fn plan(mut built: Vec<(Built, Option<u64>)>, doc_tests: bool) -> Vec<Planned> {
    // cargo runs its test binaries sorted by package, then kind, then name
    built.sort_by(|(a, _), (b, _)| {
        (&a.package, a.rank, &a.name).cmp(&(&b.package, b.rank, &b.name))
    });
    let mut planned: Vec<Planned> = built
        .iter()
        .map(|(built, total)| built.planned(*total))
        .collect();
    // then each package's doc tests, in the same order
    if doc_tests {
        for (built, _) in &built {
            if let Some(doc_crate) = &built.doc_crate {
                planned.push(Planned::doc_tests(&built.package, doc_crate));
            }
        }
    }
    planned
}

/// Counts a test binary's tests as libtest lists them, or `None` where it couldn't
///
/// # Arguments
///
/// * `built` - The binary
/// * `args` - libtest's arguments for the listing
pub async fn list(built: &Built, args: &[OsString]) -> Option<u64> {
    // a binary libtest doesn't run might do anything with `--list`
    if !built.libtest {
        return None;
    }
    let mut command = tokio::process::Command::new(&built.executable);
    command
        .args(["--list", "--format", "terse"])
        .args(args)
        .current_dir(&built.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // a listing that fails or hangs leaves the count a guess
    let output = tokio::time::timeout(LIST_WAIT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(count_listed(&output.stdout))
}

/// How many tests a terse listing names, each a line ending `: test` or `: benchmark`
///
/// # Arguments
///
/// * `listing` - The listing's output
pub fn count_listed(listing: &[u8]) -> u64 {
    String::from_utf8_lossy(listing)
        .lines()
        .filter(|line| line.ends_with(": test") || line.ends_with(": benchmark"))
        .count() as u64
}

/// What a filter does with a line that starts with one of its words
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    /// Leaves the line out
    Drop,
    /// Leaves the line out, keeping it for the run to show in its own line's place
    Keep,
    /// Leaves out the line and everything after it
    Rest,
    /// Shows the line the build kept in this one's place, then everything after as it comes
    Swap,
}

/// The build's own lines that the run says again: `Finished`, kept for the run to show, since
/// the build's time is the one `cargo test` would show; `Executable`; and its errors
const BUILD_SAYS: &[(&str, Then)] = &[
    ("Finished ", Then::Keep),
    ("Executable ", Then::Drop),
    ("warning: build failed", Then::Drop),
    ("error", Then::Rest),
];

/// The run's own `Finished`, which times a build that found everything built
const RUN_SAYS: &[(&str, Then)] = &[("Finished ", Then::Swap)];

/// Where a filter is in a line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// At a line's start, holding its bytes until it is clear whether it is shown
    Start,
    /// In a line that is shown
    Shown,
    /// In a line left out
    Left {
        /// Whether its bytes are kept
        keep: bool,
        /// Whether everything after it is shown as it comes
        then_through: bool,
    },
    /// Past an error, leaving out everything after it
    Rest,
    /// Showing everything as it comes
    Through,
}

/// Leaves cargo's own lines out of what it shows as they come, where `reef test`'s two runs of
/// cargo would say them twice: the build's `Finished`, `Executable` and errors, which the run
/// says again, and the run's own `Finished`, shown as the build said it
#[derive(Debug)]
pub struct Quieted {
    /// The words of the lines it leaves out, and what it does with each
    words: &'static [(&'static str, Then)],
    /// Where it is in a line
    held: Held,
    /// The bytes of a line's start, held until it is clear whether it is shown
    start: Vec<u8>,
    /// The build's `Finished` line, kept whole
    kept: Vec<u8>,
    /// The line to show in place of the run's own `Finished`
    swap: Vec<u8>,
}

impl Quieted {
    /// A filter of the build's standard error
    pub fn build() -> Quieted {
        Quieted {
            words: BUILD_SAYS,
            held: Held::Start,
            start: Vec::new(),
            kept: Vec::new(),
            swap: Vec::new(),
        }
    }

    /// A filter of the run's own lines, its `Finished` shown as the build said it
    ///
    /// # Arguments
    ///
    /// * `finished` - The build's `Finished` line, whole
    pub fn run(finished: Vec<u8>) -> Quieted {
        Quieted {
            words: RUN_SAYS,
            swap: finished,
            ..Quieted::build()
        }
    }

    /// Takes the build's `Finished` line, if it said one
    pub fn kept(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.kept)
    }

    /// Takes a chunk of cargo's output, returning the bytes to show of it
    ///
    /// # Arguments
    ///
    /// * `bytes` - The chunk, as read
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut shown = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match self.held {
                // everything after an error is left out, and everything after a swap shown
                Held::Rest => {}
                Held::Through => shown.push(byte),
                Held::Left { keep, then_through } => {
                    if keep {
                        self.kept.push(byte);
                    }
                    if byte == b'\n' {
                        self.held = match then_through {
                            true => Held::Through,
                            false => Held::Start,
                        };
                    }
                }
                Held::Shown => {
                    shown.push(byte);
                    if byte == b'\n' || byte == b'\r' {
                        self.held = Held::Start;
                    }
                }
                Held::Start => self.start(byte, &mut shown),
            }
        }
        shown
    }

    /// The bytes still held at the stream's end, a line that never said what it was
    pub fn finish(&mut self) -> Vec<u8> {
        match self.held {
            Held::Start => std::mem::take(&mut self.start),
            _ => Vec::new(),
        }
    }

    /// Takes a byte at a line's start, deciding once its text says whether it is shown
    ///
    /// # Arguments
    ///
    /// * `byte` - The byte
    /// * `shown` - The bytes to show
    fn start(&mut self, byte: u8, shown: &mut Vec<u8>) {
        self.start.push(byte);
        // a line that ended before saying is shown, and the next starts
        if byte == b'\n' || byte == b'\r' {
            shown.append(&mut self.start);
            return;
        }
        let text = visible(&self.start);
        let text = text.trim_start();
        if text.is_empty() {
            return;
        }
        // a line that starts with one of the words, whole, is left out, but not the escapes
        // that clear what a progress bar drew
        if let Some((_, then)) = self.words.iter().find(|(word, _)| text.starts_with(word)) {
            shown.extend(escapes(&self.start));
            let start = std::mem::take(&mut self.start);
            self.held = match then {
                Then::Drop => Held::Left {
                    keep: false,
                    then_through: false,
                },
                Then::Keep => {
                    self.kept = start;
                    Held::Left {
                        keep: true,
                        then_through: false,
                    }
                }
                Then::Rest => Held::Rest,
                Then::Swap => {
                    shown.extend_from_slice(&self.swap);
                    Held::Left {
                        keep: false,
                        then_through: true,
                    }
                }
            };
            return;
        }
        // still the start of one such word, it waits for more
        if self.words.iter().any(|(word, _)| word.starts_with(text)) {
            return;
        }
        shown.append(&mut self.start);
        self.held = Held::Shown;
    }
}

/// The escape sequences and carriage returns among a line's bytes, its text left out
///
/// # Arguments
///
/// * `bytes` - The line's bytes
fn escapes(bytes: &[u8]) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            // an escape's bytes, to its final byte
            0x1b => {
                let start = index;
                index += 1;
                if bytes.get(index) == Some(&b'[') {
                    index += 1;
                    while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                        index += 1;
                    }
                }
                index += 1;
                kept.extend_from_slice(&bytes[start..index.min(bytes.len())]);
            }
            b'\r' => {
                kept.push(b'\r');
                index += 1;
            }
            _ => index += 1,
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{Args, Manifest, Manifests, Quieted, built, count_listed, plan};
    use crate::test_run::libtest::visible;
    use std::ffi::OsString;

    /// A command line, as typed after `reef test`
    ///
    /// # Arguments
    ///
    /// * `typed` - Its arguments
    fn args(typed: &[&str]) -> Args {
        Args::parse(typed.iter().map(OsString::from).collect())
    }

    /// A command line splits where libtest's arguments begin, a toolchain going before `test`
    #[test]
    fn a_command_line_splits_at_its_first_dashes() {
        let split = args(&["+nightly", "-p", "reef", "--", "filter", "--", "x"]);
        assert_eq!(split.toolchain, Some("+nightly".into()));
        assert_eq!(split.cargo, ["-p", "reef"]);
        assert_eq!(split.libtest(), ["filter", "--", "x"]);
        assert_eq!(
            split.command(),
            [
                "cargo", "+nightly", "test", "-p", "reef", "--", "filter", "--", "x"
            ]
        );
        assert_eq!(
            split.run_args(),
            ["+nightly", "test", "-p", "reef", "--", "filter", "--", "x"]
        );
        // the build takes cargo's arguments alone
        assert_eq!(
            split.build_args(),
            [
                "+nightly",
                "test",
                "-p",
                "reef",
                "--no-run",
                "--message-format=json"
            ]
        );
        // a bare `--` leaves libtest nothing
        assert!(args(&["--"]).libtest().is_empty());
        assert!(args(&[]).libtest().is_empty());
    }

    /// Doc tests run unless a flag picks targets, the build is skipped where it can't list, and
    /// a quiet cargo is known
    #[test]
    fn flags_say_what_runs() {
        assert!(args(&["-p", "reef"]).runs_doc_tests());
        assert!(!args(&["--lib"]).runs_doc_tests());
        assert!(!args(&["--test=scan"]).runs_doc_tests());
        assert!(args(&["--doc"]).runs_doc_tests());
        // a flag after `--` is libtest's, not cargo's
        assert!(args(&["--", "--lib"]).runs_doc_tests());
        assert!(args(&["--workspace"]).lists());
        assert!(!args(&["--no-run"]).lists());
        assert!(!args(&["--doc"]).lists());
        assert!(!args(&["--message-format=short"]).lists());
        assert!(args(&["-q"]).quiet());
        assert!(!args(&["--", "-q"]).quiet());
    }

    /// A listing keeps libtest's arguments that pick tests, and leaves out those beside which
    /// `--list --format terse` can't run
    #[test]
    fn a_listing_keeps_what_picks_tests() {
        let split = args(&[
            "--",
            "scan",
            "--ignored",
            "--format",
            "pretty",
            "--skip",
            "slow",
            "-q",
            "--color=always",
            "--exact",
            "--test-threads",
            "1",
        ]);
        assert_eq!(
            split.list_args(),
            [
                "scan",
                "--ignored",
                "--skip",
                "slow",
                "--exact",
                "--test-threads",
                "1"
            ]
        );
    }

    /// The build's JSON names each test binary, what cargo runs it as and whether its crate has
    /// doc tests, and a binary with `harness = false` isn't libtest's
    #[test]
    fn the_build_names_its_test_binaries() {
        let dir = std::env::temp_dir().join(format!("reef-built-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"reef-api\"\n\n[[test]]\nname = \"ui\"\nharness = false\n",
        )
        .unwrap();
        let artifact = |kind: &str, name: &str, source: &str, test: bool, exe: Option<&str>| {
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///elsewhere#other@0.1.0",
                "manifest_path": manifest,
                "target": {"kind": [kind], "name": name, "src_path": dir.join(source), "doctest": kind == "lib"},
                "profile": {"test": test},
                "executable": exe.map(|exe| dir.join(exe)),
            })
            .to_string()
        };
        let mut manifests = Manifests::default();
        // the lib's unit tests, its doc tests' crate named
        let lib = built(
            artifact("lib", "reef_api", "src/lib.rs", true, Some("reef_api-1")).as_bytes(),
            &mut manifests,
        )
        .expect("a lib's test binary");
        assert_eq!(
            (
                lib.package.as_str(),
                lib.target.as_str(),
                lib.rank,
                lib.libtest
            ),
            ("reef-api", "unittests src/lib.rs", 0, true)
        );
        assert_eq!(lib.doc_crate.as_deref(), Some("reef_api"));
        // an integration test, and one libtest doesn't run
        let scan = built(
            artifact("test", "scan", "tests/scan.rs", true, Some("scan-2")).as_bytes(),
            &mut manifests,
        )
        .unwrap();
        assert_eq!(
            (scan.target.as_str(), scan.rank, scan.libtest),
            ("tests/scan.rs", 2, true)
        );
        let ui = built(
            artifact("test", "ui", "tests/ui.rs", true, Some("ui-3")).as_bytes(),
            &mut manifests,
        )
        .unwrap();
        assert!(!ui.libtest);
        // the lib built for others to use, and a line that isn't an artifact, name none
        assert!(
            built(
                artifact("lib", "reef_api", "src/lib.rs", false, None).as_bytes(),
                &mut manifests
            )
            .is_none()
        );
        assert!(
            built(
                br#"{"reason":"build-finished","success":true}"#,
                &mut manifests
            )
            .is_none()
        );
        // planned in cargo's order, the doc tests last
        let planned = plan(vec![(ui, None), (scan, Some(3)), (lib, Some(4))], true);
        let order: Vec<_> = planned
            .iter()
            .map(|planned| {
                (
                    planned.binary.target.as_str(),
                    planned.binary.total,
                    planned.binary.guessed,
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("unittests src/lib.rs", 4, false),
                ("tests/scan.rs", 3, false),
                ("tests/ui.rs", 0, true),
                ("doc-tests", 0, true),
            ]
        );
        assert_eq!(planned[0].file.as_deref(), Some("reef_api-1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A manifest's lib with `harness = false` is no libtest binary either, and a manifest that
    /// can't be read names nothing
    #[test]
    fn a_manifest_says_what_libtest_runs() {
        let manifest = Manifest::parse("[package]\nname = \"x\"\n[lib]\nharness = false\n");
        assert!(!manifest.libtest("lib", "x"));
        assert!(manifest.libtest("test", "x"));
        let broken = Manifest::parse("not = [toml");
        assert!(broken.name.is_none() && broken.libtest("lib", "x"));
    }

    /// A terse listing counts its tests and benchmarks
    #[test]
    fn a_listing_counts_its_tests() {
        let listing = b"tests::adds: test\ntests::slow: test\nbench::scan: benchmark\n";
        assert_eq!(count_listed(listing), 3);
        assert_eq!(count_listed(b""), 0);
    }

    /// The build shows what it builds, its progress bar and colours too, but not the lines the
    /// run says again, and the run shows its `Finished` as the build said it
    #[test]
    fn the_build_leaves_out_what_the_run_says_again() {
        let mut quieted = Quieted::build();
        let mut shown = Vec::new();
        // a coloured line, a progress bar a byte at a time, then the build's own end, whose
        // escapes stay, so the bar is cleared and the colour reset
        let stream: &[u8] = b"\x1b[1m\x1b[92m   Compiling\x1b[0m fixture v0.1.0\n\
            \x1b[1m\x1b[36m    Building\x1b[0m [=>  ] 1/2: fixture\r\
            \x1b[K\x1b[1m\x1b[92m    Finished\x1b[0m `test` profile target(s) in 6.42s\n\
            \x1b[1m\x1b[92m  Executable\x1b[0m unittests src/lib.rs (target/x)\n";
        for byte in stream {
            shown.extend(quieted.push(std::slice::from_ref(byte)));
        }
        shown.extend(quieted.finish());
        let expected: &[u8] = b"\x1b[1m\x1b[92m   Compiling\x1b[0m fixture v0.1.0\n\
            \x1b[1m\x1b[36m    Building\x1b[0m [=>  ] 1/2: fixture\r\
            \x1b[K\x1b[1m\x1b[92m\x1b[0m\x1b[1m\x1b[92m\x1b[0m";
        assert_eq!(
            String::from_utf8_lossy(&shown),
            String::from_utf8_lossy(expected)
        );
        // the build's `Finished` is kept whole
        let finished = quieted.kept();
        assert_eq!(
            visible(finished.strip_suffix(b"\n").unwrap()),
            "    Finished `test` profile target(s) in 6.42s"
        );
        // and shown in place of the run's own, after the warnings the run replays
        let mut run = Quieted::run(finished);
        let shown = run.push(
            b"warning: unused\n    Finished `test` profile target(s) in 0.02s\n     Running unittests\n",
        );
        assert_eq!(
            String::from_utf8_lossy(&shown),
            "warning: unused\n\x1b[K\x1b[1m\x1b[92m    Finished\x1b[0m `test` profile target(s) in 6.42s\n     Running unittests\n"
        );
        // after which everything passes as it comes, a later `Finished` too
        assert_eq!(run.push(b"Finished x\n"), b"Finished x\n");
        // an error leaves out the rest, which the run says again
        let mut quieted = Quieted::build();
        let shown =
            quieted.push(b"   Compiling a\nerror: could not compile `a`\n\nCaused by:\n  x\n");
        assert_eq!(shown, b"   Compiling a\n");
    }
}
