//! `reef test`: `cargo test` run as given, its output shown as cargo would show it and read as
//! it comes, and the run told to central reef where it answers
//!
//! ```text
//! reef test [ARGS]...       # cargo test [ARGS]..., `--` and libtest's arguments included
//! ```
//!
//! - **Built and listed first.** `cargo test --no-run --message-format=json` builds every test
//!   binary and names it, and each is listed with libtest's arguments, so the run's total is
//!   known before a test runs. Then `cargo test` runs as typed, building nothing more.
//! - **Through a pty where a terminal watches,** so cargo's colours, its progress bar and the
//!   order of its lines are what the terminal would show. Through pipes elsewhere, as a tool's
//!   output is, cargo's standard output to `reef test`'s and its standard error to its own.
//! - **Telling central reef is best-effort,** beside the tests: where no project is found or
//!   nothing answers, it says so once and the tests run as they would have.
//! - **It exits with cargo's code,** or, where a signal ended cargo, ends by that signal too, so
//!   a shell stops what it was running as it would have for cargo.

pub mod cargo;
pub mod libtest;
pub mod pty;
pub mod report;

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use cargo::{Args, Built, Manifests, Quieted};
use libtest::{Ending, Lines, Stream, Tally, visible};
use pty::Pty;
use report::{Telling, elapsed_ms};

/// How long cargo's streams may stay open once it exits, held by something it started
const GRACE: Duration = Duration::from_millis(300);

/// How long the run's last report may take once the tests end
const FINAL_WAIT: Duration = Duration::from_secs(3);

/// The most bytes one read of cargo's output takes
const CHUNK: usize = 16 * 1024;

/// A command's own exit code, as `reef test` exits with cargo's: `reef` exits with it,
/// printing nothing more
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub u8);

impl std::fmt::Display for Exit {
    /// Says what the command exited with
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exited with code {}", self.0)
    }
}

impl std::error::Error for Exit {}

/// A command's wish to end by a signal, as `reef test` ends when a signal ended cargo, so a
/// shell stops what it was running too, as it would have for cargo
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Killed(pub i32);

impl std::fmt::Display for Killed {
    /// Says what signal ended the command
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ended by signal {}", self.0)
    }
}

impl std::error::Error for Killed {}

/// Ends this process by a signal, as the cargo it ran ended, falling back to the code a shell
/// reads for that signal if its default leaves the process running
///
/// # Arguments
///
/// * `signal` - The signal
pub fn die_by(signal: i32) -> std::process::ExitCode {
    // SAFETY: putting a signal's default action back and raising it touch no memory of this
    // process's, and nothing else runs by the time `reef` ends
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
    std::process::ExitCode::from(128u8.saturating_add(signal as u8))
}

/// How cargo's output reaches `reef test`'s
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Through a pty, as cargo's terminal: both its streams in the order written, colours and
    /// all, to `reef test`'s standard output
    Pty,
    /// Through a pipe each, its standard output to `reef test`'s and its standard error to its
    /// own
    Pipes,
}

impl Mode {
    /// A pty where `reef test`'s standard output and error are both a terminal, else pipes
    pub fn detect() -> Mode {
        match std::io::stdout().is_terminal() && std::io::stderr().is_terminal() {
            true => Mode::Pty,
            false => Mode::Pipes,
        }
    }
}

/// What `reef test` was asked
#[derive(Debug, Clone)]
pub struct Tests {
    /// The config named on the command line, if one was
    pub conf: Option<PathBuf>,
    /// The credentials file named, if one was
    pub credentials: Option<PathBuf>,
    /// The working directory, which cargo runs in, and which picks the project and the node
    pub cwd: PathBuf,
    /// cargo test's arguments, as typed
    pub args: Vec<OsString>,
    /// How cargo's output reaches `reef test`'s
    pub mode: Mode,
}

/// What the tests and the reports beside them share: the run's tally, and their signals
pub struct Shared {
    /// What the run's output has said
    tally: Mutex<Tally>,
    /// Woken when the tally changes, for the reports
    news: Notify,
    /// What the reports have to say on standard error, once
    notice: Mutex<Option<String>>,
    /// Woken when there is something to say
    noticed: Notify,
}

impl Shared {
    /// The run's tally, locked
    fn tally(&self) -> MutexGuard<'_, Tally> {
        // a tally a panic left half-changed is still the best word on the run
        self.tally
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Changes the tally, and wakes the reports
    ///
    /// # Arguments
    ///
    /// * `change` - The change
    fn change(&self, change: impl FnOnce(&mut Tally)) {
        change(&mut self.tally());
        self.news.notify_one();
    }

    /// Has something said on standard error, once the tests can say it
    ///
    /// # Arguments
    ///
    /// * `text` - What to say
    fn say(&self, text: String) {
        *self
            .notice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(text);
        self.noticed.notify_one();
    }

    /// Takes what there is to say, if anything
    fn take_notice(&self) -> Option<String> {
        self.notice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// Runs `cargo test` as asked, showing its output, telling central reef how far it has got
/// where it answers, and failing with [`Exit`] where cargo exits other than 0
///
/// # Arguments
///
/// * `tests` - What was asked
/// * `out` - Where cargo's standard output goes, and all its output through a pty
/// * `err` - Where cargo's standard error goes through pipes, and what reef says
///
/// # Examples
///
/// ```no_run
/// use reef::test_run::{Mode, Tests};
///
/// # async fn example() -> anyhow::Result<()> {
/// let tests = Tests {
///     conf: None,
///     credentials: None,
///     cwd: std::env::current_dir()?,
///     args: vec!["--workspace".into()],
///     mode: Mode::detect(),
/// };
/// reef::test_run::run(tests, &mut std::io::stdout(), &mut std::io::stderr()).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run<W: Write, E: Write>(tests: Tests, out: &mut W, err: &mut E) -> anyhow::Result<()> {
    let args = Args::parse(tests.args.clone());
    let started = Instant::now();
    let shared = Shared {
        tally: Mutex::new(Tally::new(args.quiet())),
        news: Notify::new(),
        notice: Mutex::new(None),
        noticed: Notify::new(),
    };
    // the tests, and the reports beside them on this task
    let telling = Telling {
        conf: tests.conf.as_deref(),
        credentials: tests.credentials.as_deref(),
        cwd: &tests.cwd,
        args: &args,
    };
    let reports = report::report(telling, &shared, started);
    let work = work(&tests, &args, &shared, started, out, err);
    let code = beside(work, reports).await;
    // anything the reports said after the tests ended
    if let Some(notice) = shared.take_notice() {
        let _ = writeln!(err, "{notice}");
    }
    ended(code?)
}

/// Runs the tests to their end with the reports beside them, giving the reports a little while
/// after the tests end to send their last
///
/// # Arguments
///
/// * `work` - The tests
/// * `reports` - The reports
async fn beside<T>(work: impl Future<Output = T>, reports: impl Future<Output = ()>) -> T {
    tokio::pin!(work);
    tokio::pin!(reports);
    let mut reporting = true;
    // the tests to their end, the reports polled as they go
    let done = loop {
        tokio::select! {
            done = &mut work => break done,
            () = &mut reports, if reporting => reporting = false,
        }
    };
    // and the last report given its while
    if reporting {
        let _ = tokio::time::timeout(FINAL_WAIT, reports).await;
    }
    done
}

/// The signals `reef test` takes while cargo runs, each passed on to cargo
struct Signals {
    /// Ctrl-C, which a terminal sends cargo as well
    interrupt: Signal,
    /// A request to stop
    terminate: Signal,
    /// The terminal closing
    hangup: Signal,
    /// The terminal resized
    resize: Signal,
    /// Whether one of the stopping signals came
    stopped: bool,
}

impl Signals {
    /// Takes the signals, so they no longer stop `reef test` before cargo
    fn take() -> std::io::Result<Signals> {
        Ok(Signals {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
            hangup: signal(SignalKind::hangup())?,
            resize: signal(SignalKind::window_change())?,
            stopped: false,
        })
    }
}

/// Where a chunk of cargo's output came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Its pty, both its streams
    Pty,
    /// Its standard output's pipe
    Out,
    /// Its standard error's pipe
    Err,
}

/// What takes cargo's output as it comes
trait Sink {
    /// Takes a chunk of output
    ///
    /// # Arguments
    ///
    /// * `source` - Where it came from
    /// * `bytes` - The chunk
    fn chunk(&mut self, source: Source, bytes: &[u8]);

    /// Says something of reef's on standard error
    ///
    /// # Arguments
    ///
    /// * `text` - What to say
    fn say(&mut self, text: &str);

    /// Takes the end of every stream
    fn finish(&mut self);
}

/// One run of cargo: the child, and its streams
struct Cargo {
    /// The child
    child: Child,
    /// Its pty, if its output goes to one
    pty: Option<Pty>,
    /// Its standard output's pipe, if it has one
    out: Option<ChildStdout>,
    /// Its standard error's pipe, if it has one
    err: Option<ChildStderr>,
}

/// How a run of cargo's streams are wired
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wiring {
    /// Both to a pty
    Pty,
    /// Standard output to a pipe, for JSON, and standard error to a pty
    JsonPty,
    /// Each to a pipe of its own
    Pipes,
}

impl Cargo {
    /// Starts cargo with its streams wired
    ///
    /// # Arguments
    ///
    /// * `program` - cargo's program
    /// * `cwd` - The directory it runs in
    /// * `args` - Its arguments
    /// * `wiring` - Where its streams go
    fn spawn(
        program: &OsStr,
        cwd: &Path,
        args: &[OsString],
        wiring: Wiring,
    ) -> anyhow::Result<Cargo> {
        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(cwd).stdin(Stdio::inherit());
        // a pty for the streams that go to one, a pipe for the rest
        let pty = match wiring {
            Wiring::Pty => {
                let (pty, slave) = Pty::open()?;
                command
                    .stdout(Stdio::from(slave.try_clone()?))
                    .stderr(Stdio::from(slave));
                Some(pty)
            }
            Wiring::JsonPty => {
                let (pty, slave) = Pty::open()?;
                command.stdout(Stdio::piped()).stderr(Stdio::from(slave));
                Some(pty)
            }
            Wiring::Pipes => {
                command.stdout(Stdio::piped()).stderr(Stdio::piped());
                None
            }
        };
        let mut child = command
            .spawn()
            .map_err(|error| anyhow::anyhow!("running {}: {error}", program.to_string_lossy()))?;
        // the slave goes with the command, so the pty ends when cargo and its children do
        drop(command);
        Ok(Cargo {
            out: child.stdout.take(),
            err: child.stderr.take(),
            child,
            pty,
        })
    }

    /// Hands cargo's output to a sink as it comes, until cargo exits and its streams end, the
    /// terminal's size and stopping signals passed on to it
    ///
    /// # Arguments
    ///
    /// * `sink` - What takes the output
    /// * `signals` - The signals taken while cargo runs
    /// * `shared` - What reports have to say
    async fn pump(
        mut self,
        sink: &mut impl Sink,
        signals: &mut Signals,
        shared: &Shared,
    ) -> std::io::Result<ExitStatus> {
        let mut pty_buf = vec![0u8; CHUNK];
        let mut out_buf = vec![0u8; CHUNK];
        let mut err_buf = vec![0u8; CHUNK];
        let mut status = None;
        let mut grace = None;
        loop {
            // done once cargo exited and its streams ended, or its grace ran out
            let open = self.pty.is_some() || self.out.is_some() || self.err.is_some();
            if status.is_some() && !open {
                break;
            }
            let until =
                grace.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
            tokio::select! {
                read = read_pty(self.pty.as_ref(), &mut pty_buf), if self.pty.is_some() => match read {
                    Ok(0) | Err(_) => self.pty = None,
                    Ok(read) => sink.chunk(Source::Pty, &pty_buf[..read]),
                },
                read = read_pipe(self.out.as_mut(), &mut out_buf), if self.out.is_some() => match read {
                    Ok(0) | Err(_) => self.out = None,
                    Ok(read) => sink.chunk(Source::Out, &out_buf[..read]),
                },
                read = read_err(self.err.as_mut(), &mut err_buf), if self.err.is_some() => match read {
                    Ok(0) | Err(_) => self.err = None,
                    Ok(read) => sink.chunk(Source::Err, &err_buf[..read]),
                },
                exited = self.child.wait(), if status.is_none() => {
                    status = Some(exited?);
                    grace = Some(tokio::time::Instant::now() + GRACE);
                },
                () = tokio::time::sleep_until(until), if grace.is_some() => break,
                _ = signals.interrupt.recv() => self.pass_on(rustix::process::Signal::INT, signals),
                _ = signals.terminate.recv() => self.pass_on(rustix::process::Signal::TERM, signals),
                _ = signals.hangup.recv() => self.pass_on(rustix::process::Signal::HUP, signals),
                _ = signals.resize.recv() => {
                    if let Some(pty) = &self.pty {
                        pty.resize();
                    }
                },
                () = shared.noticed.notified() => {
                    if let Some(notice) = shared.take_notice() {
                        sink.say(&notice);
                    }
                },
            }
        }
        sink.finish();
        // a child whose streams outlived it was waited on already
        match status {
            Some(status) => Ok(status),
            None => self.child.wait().await,
        }
    }

    /// Passes a stopping signal on to cargo, noting the run was stopped
    ///
    /// # Arguments
    ///
    /// * `sent` - The signal
    /// * `signals` - The signals taken, which note it
    fn pass_on(&self, sent: rustix::process::Signal, signals: &mut Signals) {
        signals.stopped = true;
        let pid = self
            .child
            .id()
            .and_then(|id| rustix::process::Pid::from_raw(id as i32));
        if let Some(pid) = pid {
            let _ = rustix::process::kill_process(pid, sent);
        }
    }
}

/// Reads cargo's pty, which the select only polls while it has one
///
/// # Arguments
///
/// * `pty` - The pty
/// * `buf` - Where to read to
async fn read_pty(pty: Option<&Pty>, buf: &mut [u8]) -> std::io::Result<usize> {
    match pty {
        Some(pty) => pty.read(buf).await,
        None => Ok(0),
    }
}

/// Reads cargo's standard output's pipe, which the select only polls while it has one
///
/// # Arguments
///
/// * `pipe` - The pipe
/// * `buf` - Where to read to
async fn read_pipe(pipe: Option<&mut ChildStdout>, buf: &mut [u8]) -> std::io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(buf).await,
        None => Ok(0),
    }
}

/// Reads cargo's standard error's pipe, which the select only polls while it has one
///
/// # Arguments
///
/// * `pipe` - The pipe
/// * `buf` - Where to read to
async fn read_err(pipe: Option<&mut ChildStderr>, buf: &mut [u8]) -> std::io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(buf).await,
        None => Ok(0),
    }
}

/// How `reef test` ends for how cargo ended: as it did, with its exit code, or by the signal
/// that ended it
///
/// # Arguments
///
/// * `status` - How cargo ended
fn ended(status: ExitStatus) -> anyhow::Result<()> {
    match (status.code(), status.signal()) {
        (Some(0), _) => Ok(()),
        (Some(code), _) => Err(Exit((code & 0xff) as u8).into()),
        (None, Some(signal)) => Err(Killed(signal).into()),
        (None, None) => Err(Exit(1).into()),
    }
}

/// Runs the build, which names and counts the test binaries, and then the tests, returning
/// how the cargo that ran last ended
///
/// # Arguments
///
/// * `tests` - What was asked
/// * `args` - The command line, split
/// * `shared` - The run's tally
/// * `started` - When the run started
/// * `out` - Where cargo's standard output goes
/// * `err` - Where its standard error goes
async fn work<W: Write, E: Write>(
    tests: &Tests,
    args: &Args,
    shared: &Shared,
    started: Instant,
    out: &mut W,
    err: &mut E,
) -> anyhow::Result<ExitStatus> {
    // signals are taken before cargo starts, so it is stopped through reef
    let mut signals = Signals::take()?;
    let program = cargo::program(args);
    let mut finished = Vec::new();
    // the build, which names each test binary, each counted as it is built
    if args.lists() {
        let wiring = match tests.mode {
            Mode::Pty => Wiring::JsonPty,
            Mode::Pipes => Wiring::Pipes,
        };
        let build = Cargo::spawn(&program, &tests.cwd, &args.build_args(), wiring)?;
        let mut building = Building {
            out: &mut *out,
            err: &mut *err,
            through_pty: wiring == Wiring::JsonPty,
            json: Lines::default(),
            quieted: Quieted::build(),
            manifests: Manifests::default(),
            listings: JoinSet::new(),
            built: Vec::new(),
            list_args: args.list_args(),
        };
        let status = build.pump(&mut building, &mut signals, shared).await?;
        // a build stopped by a signal stops the run there
        if signals.stopped || status.signal().is_some() {
            shared.change(|tally| {
                tally.end(
                    Ending {
                        passed: false,
                        interrupted: true,
                    },
                    elapsed_ms(started),
                )
            });
            return Ok(status);
        }
        // the build's `Finished`, for the run to show in place of its own
        finished = building.quieted.kept();
        let planned = cargo::plan(building.counted().await, args.runs_doc_tests());
        shared.change(|tally| tally.plan(planned));
    }
    // then the tests, as typed
    let wiring = match tests.mode {
        Mode::Pty => Wiring::Pty,
        Mode::Pipes => Wiring::Pipes,
    };
    let run = Cargo::spawn(&program, &tests.cwd, &args.run_args(), wiring)?;
    let mut shown = Shown {
        out,
        err,
        swap: (!finished.is_empty()).then(|| Quieted::run(finished)),
        through_pty: tests.mode == Mode::Pty,
        lines: [Lines::default(), Lines::default(), Lines::default()],
        shared,
        started,
    };
    let status = run.pump(&mut shown, &mut signals, shared).await?;
    // the run ends as cargo did
    let ending = Ending {
        passed: status.success(),
        interrupted: signals.stopped || status.signal().is_some(),
    };
    shared.change(|tally| tally.end(ending, elapsed_ms(started)));
    Ok(status)
}

/// The build's sink: its JSON read for test binaries, each listed as it is built, and what it
/// shows passed on, its last lines left out
struct Building<'a, W: Write, E: Write> {
    /// Where the build's standard error goes through a pty
    out: &'a mut W,
    /// Where it goes through a pipe, and what reef says
    err: &'a mut E,
    /// Whether the build's standard error comes through a pty
    through_pty: bool,
    /// The JSON's lines
    json: Lines,
    /// What the build shows, its last lines left out
    quieted: Quieted,
    /// The manifests read so far
    manifests: Manifests,
    /// Each binary's listing, by its place among those built
    listings: JoinSet<(usize, Option<u64>)>,
    /// The binaries built so far
    built: Vec<Built>,
    /// libtest's arguments for a listing
    list_args: Vec<OsString>,
}

impl<W: Write, E: Write> Building<'_, W, E> {
    /// The binaries built, each with its listing's count once every listing ended
    async fn counted(mut self) -> Vec<(Built, Option<u64>)> {
        let mut counts = vec![None; self.built.len()];
        while let Some(listed) = self.listings.join_next().await {
            if let Ok((place, count)) = listed {
                counts[place] = count;
            }
        }
        self.built.into_iter().zip(counts).collect()
    }

    /// Takes one line of the build's JSON, listing the test binary it names
    ///
    /// # Arguments
    ///
    /// * `line` - The line
    fn json_line(&mut self, line: &[u8]) {
        let Some(built) = cargo::built(line, &mut self.manifests) else {
            return;
        };
        // listed at once, beside the rest of the build
        let place = self.built.len();
        let listed = built.clone();
        let args = self.list_args.clone();
        self.listings
            .spawn(async move { (place, cargo::list(&listed, &args).await) });
        self.built.push(built);
    }
}

impl<W: Write, E: Write> Sink for Building<'_, W, E> {
    /// Takes a chunk of the build's output
    ///
    /// # Arguments
    ///
    /// * `source` - Where it came from
    /// * `bytes` - The chunk
    fn chunk(&mut self, source: Source, bytes: &[u8]) {
        match source {
            // the JSON names the test binaries
            Source::Out => {
                for line in self.json.push(bytes) {
                    self.json_line(&line);
                }
            }
            // and what the build shows passes on, its last lines left out
            Source::Pty => write_all(self.out, &self.quieted.push(bytes)),
            Source::Err => write_all(self.err, &self.quieted.push(bytes)),
        }
    }

    /// Says something of reef's on standard error
    ///
    /// # Arguments
    ///
    /// * `text` - What to say
    fn say(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// Takes the end of every stream
    fn finish(&mut self) {
        if let Some(line) = self.json.finish() {
            self.json_line(&line);
        }
        let held = self.quieted.finish();
        match self.through_pty {
            true => write_all(self.out, &held),
            false => write_all(self.err, &held),
        }
    }
}

/// The tests' sink: their output shown as it comes, and read into the tally a line at a time
struct Shown<'a, W: Write, E: Write> {
    /// Where cargo's standard output goes, and all its output through a pty
    out: &'a mut W,
    /// Where its standard error goes through a pipe, and what reef says
    err: &'a mut E,
    /// What shows the build's `Finished` in place of the run's own, on the stream cargo's own
    /// lines come on, if the build said one
    swap: Option<Quieted>,
    /// Whether cargo's output comes through a pty
    through_pty: bool,
    /// The lines of the pty, the standard output and the standard error
    lines: [Lines; 3],
    /// The run's tally
    shared: &'a Shared,
    /// When the run started
    started: Instant,
}

impl<W: Write, E: Write> Shown<'_, W, E> {
    /// Reads lines into the tally, and wakes the reports
    ///
    /// # Arguments
    ///
    /// * `stream` - Where they came from
    /// * `lines` - The lines, each as read
    fn read(&self, stream: Stream, lines: Vec<Vec<u8>>) {
        if lines.is_empty() {
            return;
        }
        let at = elapsed_ms(self.started);
        self.shared.change(|tally| {
            for line in &lines {
                tally.line(stream, &visible(line), at);
            }
        });
    }
}

impl<W: Write, E: Write> Sink for Shown<'_, W, E> {
    /// Takes a chunk of the tests' output, shown at once
    ///
    /// # Arguments
    ///
    /// * `source` - Where it came from
    /// * `bytes` - The chunk
    fn chunk(&mut self, source: Source, bytes: &[u8]) {
        // cargo's own lines come through the pty, else on standard error
        let swapped = match (&mut self.swap, source) {
            (Some(swap), Source::Pty | Source::Err) => Some(swap.push(bytes)),
            _ => None,
        };
        let shown = swapped.as_deref().unwrap_or(bytes);
        let (place, stream) = match source {
            Source::Pty => {
                write_all(self.out, shown);
                (0, Stream::Both)
            }
            Source::Out => {
                write_all(self.out, shown);
                (1, Stream::Out)
            }
            Source::Err => {
                write_all(self.err, shown);
                (2, Stream::Err)
            }
        };
        let lines = self.lines[place].push(bytes);
        self.read(stream, lines);
    }

    /// Says something of reef's on standard error
    ///
    /// # Arguments
    ///
    /// * `text` - What to say
    fn say(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// Takes the end of every stream, reading each one's last unfinished line
    fn finish(&mut self) {
        // what the swap still held is shown where it would have gone
        if let Some(swap) = &mut self.swap {
            let held = swap.finish();
            match self.through_pty {
                true => write_all(self.out, &held),
                false => write_all(self.err, &held),
            }
        }
        for (place, stream) in [(0, Stream::Both), (1, Stream::Out), (2, Stream::Err)] {
            if let Some(line) = self.lines[place].finish() {
                self.read(stream, vec![line]);
            }
        }
    }
}

/// Writes bytes where they go at once, dropping them if it can't take them, as a closed
/// terminal can't
///
/// # Arguments
///
/// * `to` - Where they go
/// * `bytes` - The bytes
fn write_all(to: &mut impl Write, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let _ = to.write_all(bytes);
    let _ = to.flush();
}

#[cfg(test)]
mod tests {
    use super::{Exit, Killed, ended};
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    /// `reef test` ends as cargo did: with its exit code, or by the signal that ended it
    #[test]
    fn it_ends_as_cargo_did() {
        assert!(ended(ExitStatus::from_raw(0)).is_ok());
        let failed = ended(ExitStatus::from_raw(101 << 8)).unwrap_err();
        assert_eq!(failed.downcast_ref::<Exit>(), Some(&Exit(101)));
        let interrupted = ended(ExitStatus::from_raw(2)).unwrap_err();
        assert_eq!(interrupted.downcast_ref::<Killed>(), Some(&Killed(2)));
    }
}
