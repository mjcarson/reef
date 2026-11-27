//! Reading a test run's output as it comes: cargo's lines naming each test binary as it runs it,
//! and libtest's lines saying how many tests a binary holds, how each ended and what a failure
//! printed
//!
//! - **What a terminal would show is read,** colours, links and lines redrawn after a carriage
//!   return resolved first, so a run through a pty reads as one through pipes does.
//! - **A binary's `test result:` line has the last word,** its counts replacing those its tests'
//!   lines added up to, since a test's own output can break one of those lines.
//! - **A failure's output is its `---- <name> stdout ----` block** under `failures:`, or, with
//!   `--nocapture`, what it printed between its name and its `FAILED`.
//! - **Two pipes can arrive out of order,** so through pipes each stream's lines wait their
//!   turn: libtest's for cargo to name the binary they come from, and cargo's naming the next
//!   binary for the running one's result.

use reef_api::test_run::{Binary, BinaryState, FailedTest, KEPT_FAILURES, RunState, cut_output};
use std::collections::{HashMap, VecDeque};

/// Where a line of cargo's output came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Its standard output, through a pipe of its own: libtest's lines
    Out,
    /// Its standard error, through a pipe of its own: cargo's own lines
    Err,
    /// A pty both write to, which keeps the order they wrote in
    Both,
}

/// Splits a stream's bytes into lines as they come, holding an unfinished line until its end
#[derive(Debug, Default)]
pub struct Lines {
    /// The bytes since the last newline
    partial: Vec<u8>,
}

impl Lines {
    /// Takes a chunk of a stream's bytes, returning each line it finished, its newline left off
    ///
    /// # Arguments
    ///
    /// * `bytes` - The chunk, as read
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut lines = Vec::new();
        // each newline ends the line held so far
        for &byte in bytes {
            match byte {
                b'\n' => lines.push(std::mem::take(&mut self.partial)),
                byte => self.partial.push(byte),
            }
        }
        lines
    }

    /// The stream's last line, which it ended without a newline, if it held one
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        // an empty tail is no line
        match self.partial.is_empty() {
            true => None,
            false => Some(std::mem::take(&mut self.partial)),
        }
    }
}

/// A line as a terminal shows it: escape sequences left out, and only what follows its last
/// carriage return, which a redrawn line such as cargo's progress bar ends with
///
/// # Arguments
///
/// * `raw` - The line's bytes, its newline left off
///
/// # Examples
///
/// ```
/// use reef::test_run::libtest::visible;
///
/// assert_eq!(visible(b"test a ... \x1b[32mok\x1b(B\x1b[m"), "test a ... ok");
/// assert_eq!(visible(b"  Building [=> ] 1/2\r\x1b[K   Compiling a\r"), "   Compiling a");
/// ```
pub fn visible(raw: &[u8]) -> String {
    // a line that ends a carriage return, as a pty's newline does, ends there
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end_matches('\r');
    let mut shown = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // an escape shows nothing
            '\x1b' => skip_escape(&mut chars),
            // a carriage return draws the line again from its start
            '\r' => shown.clear(),
            // nor does any other control character but a tab
            c if c.is_control() && c != '\t' => {}
            c => shown.push(c),
        }
    }
    shown
}

/// Skips the rest of an escape sequence, its `ESC` already read
///
/// # Arguments
///
/// * `chars` - The line's characters, just past the `ESC`
fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        // a control sequence ends at its final byte, such as `m` for a colour
        Some('[') => {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        // an operating system command, such as a link, and the strings like it end at a bell
        // or a string terminator
        Some(']' | 'P' | 'X' | '^' | '_') => {
            while let Some(c) = chars.next() {
                if c == '\x07' {
                    break;
                }
                if c == '\x1b' {
                    chars.next_if_eq(&'\\');
                    break;
                }
            }
        }
        // a character set's designation takes one more character, such as `ESC ( B`
        Some('(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | '%') => {
            chars.next();
        }
        // and any other escape is two characters long
        _ => {}
    }
}

/// A binary known before the run starts: one the build made, counted by listing it, or a
/// crate's doc tests, their count a guess
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The binary as a run reports it
    pub binary: Binary,
    /// Its executable's file name, which cargo's `Running` line names it by
    pub file: Option<String>,
    /// The crate whose doc tests it runs, as cargo's `Doc-tests` line names it
    pub doc_crate: Option<String>,
    /// Whether libtest runs its tests, so it says `running N tests` and `test result:`
    pub libtest: bool,
}

impl Planned {
    /// A crate's doc tests, as many as a guess says until they say how many
    ///
    /// # Arguments
    ///
    /// * `package` - The package they are in
    /// * `doc_crate` - The crate's name, as cargo's `Doc-tests` line names it
    pub fn doc_tests(package: &str, doc_crate: &str) -> Planned {
        Planned {
            binary: waiting(package, DOC_TESTS, 0, true),
            file: None,
            doc_crate: Some(doc_crate.to_string()),
            libtest: true,
        }
    }
}

/// What cargo calls a crate's doc tests among its binaries
pub const DOC_TESTS: &str = "doc-tests";

/// A binary that hasn't run yet
///
/// # Arguments
///
/// * `package` - The package it tests
/// * `target` - What cargo runs it as
/// * `total` - How many tests it holds
/// * `guessed` - Whether that is only a guess
pub fn waiting(package: &str, target: &str, total: u64, guessed: bool) -> Binary {
    Binary {
        package: package.to_string(),
        target: target.to_string(),
        total,
        guessed,
        passed: 0,
        failed: 0,
        ignored: 0,
        state: BinaryState::Waiting,
        started_ms: None,
        ended_ms: None,
    }
}

/// How a run's cargo ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ending {
    /// Whether cargo exited 0
    pub passed: bool,
    /// Whether the run was interrupted, by Ctrl-C or a signal
    pub interrupted: bool,
}

/// One of cargo's own lines that the reading turns on
#[derive(Debug, Clone, PartialEq, Eq)]
enum Said<'a> {
    /// `Running <target> (<path>)`: cargo runs a test binary
    Running {
        /// What it runs it as, such as `unittests src/lib.rs`
        target: &'a str,
        /// Its executable's file name
        file: &'a str,
    },
    /// `Doc-tests <crate>`: cargo runs a crate's doc tests
    DocTests {
        /// The crate's name
        krate: &'a str,
    },
    /// `error: test failed`, or that targets failed: a test failed somewhere
    Failed,
}

impl Said<'_> {
    /// Whether it names a binary that starts running
    fn starts(&self) -> bool {
        matches!(self, Said::Running { .. } | Said::DocTests { .. })
    }
}

/// Reads one of cargo's own lines, if it is one the reading turns on
///
/// # Arguments
///
/// * `line` - The line, as a terminal shows it
fn cargo_said(line: &str) -> Option<Said<'_>> {
    // cargo right-aligns its status words, so they come after spaces
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("Running ")
        && let Some((target, path)) = rest
            .strip_suffix(')')
            .and_then(|rest| rest.rsplit_once(" ("))
    {
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
        return Some(Said::Running { target, file });
    }
    if let Some(krate) = trimmed.strip_prefix("Doc-tests ") {
        return Some(Said::DocTests {
            krate: krate.trim(),
        });
    }
    // a test failed in a binary, or, with --no-fail-fast, in some targets
    let failed = trimmed.strip_prefix("error: ").is_some_and(|rest| {
        rest.starts_with("test failed")
            || rest.starts_with("doctest failed")
            || rest.ends_with("target failed:")
            || rest.ends_with("targets failed:")
    });
    failed.then_some(Said::Failed)
}

/// How many tests a binary says it runs, from libtest's `running N tests`
///
/// # Arguments
///
/// * `line` - The line, as a terminal shows it
fn running_count(line: &str) -> Option<u64> {
    // `running 1 test` or `running 12 tests`, and nothing after
    let rest = line.strip_prefix("running ")?;
    let (count, noun) = rest.split_once(' ')?;
    match noun {
        "test" | "tests" => count.parse().ok(),
        _ => None,
    }
}

/// How one test ended, as libtest's line for it says
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// It passed, or a benchmark ran
    Passed,
    /// It failed
    Failed,
    /// It was ignored
    Ignored,
}

/// How a test ended, from what follows its name and ` ... `
///
/// # Arguments
///
/// * `result` - What follows, such as `ok`, `FAILED` or `ignored, needs a network`
fn outcome(result: &str) -> Option<Outcome> {
    // a result alone, or one followed by its time or a reason
    let word = result.split([' ', ',']).next().unwrap_or(result);
    match word {
        "ok" => Some(Outcome::Passed),
        "FAILED" => Some(Outcome::Failed),
        "ignored" => Some(Outcome::Ignored),
        "bench:" => Some(Outcome::Passed),
        _ => None,
    }
}

/// The counts in libtest's `test result:` line: passed, failed and ignored, a benchmark measured
/// counted as passed
///
/// # Arguments
///
/// * `line` - The line, as a terminal shows it
fn result_counts(line: &str) -> Option<[u64; 3]> {
    // `test result: ok. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; ...`
    let rest = line.strip_prefix("test result: ")?;
    let (_, counts) = rest.split_once(". ")?;
    let mut found = [0u64; 3];
    for part in counts.split("; ") {
        let Some((count, what)) = part.split_once(' ') else {
            continue;
        };
        let Ok(count) = count.parse::<u64>() else {
            continue;
        };
        match what {
            "passed" | "measured" => found[0] += count,
            "failed" => found[1] += count,
            "ignored" => found[2] += count,
            _ => {}
        }
    }
    Some(found)
}

/// Which part of a binary's output its lines are in
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    /// Its tests' own lines
    #[default]
    Tests,
    /// `successes:` with `--show-output`: passing tests' blocks, then their names
    Successes,
    /// `failures:`: failed tests' blocks
    Failures,
    /// The second `failures:`: the failed tests' names
    FailedNames,
}

/// What the running binary's lines have said since cargo named it
#[derive(Debug, Clone, Default)]
struct Reading {
    /// Whether it said how many tests it runs, so a result is still to come
    running: bool,
    /// The part of its output its lines are in
    section: Section,
    /// The test whose output block is being read, and its lines so far
    block: Option<(String, Vec<String>)>,
    /// With `--nocapture`, the test whose result is still to come, and what it printed since
    pending: Option<(String, Vec<String>)>,
    /// Its failed tests, in the order they failed, each with its output
    failed: Vec<(String, String)>,
}

impl Reading {
    /// Notes a failed test, once
    ///
    /// # Arguments
    ///
    /// * `name` - The test's name
    fn failed(&mut self, name: &str) {
        if !self.failed.iter().any(|(failed, _)| failed == name) {
            self.failed.push((name.to_string(), String::new()));
        }
    }

    /// Keeps a failed test's output, its trailing blank lines left off
    ///
    /// # Arguments
    ///
    /// * `name` - The test's name
    /// * `lines` - What it printed
    fn output(&mut self, name: &str, lines: &[String]) {
        // a block found for a test whose FAILED line was missed still names it
        self.failed(name);
        let text = lines.join("\n").trim_end().to_string();
        if let Some((_, output)) = self.failed.iter_mut().find(|(failed, _)| failed == name) {
            *output = text;
        }
    }

    /// Ends the output block being read, keeping it if it is a failure's
    fn close_block(&mut self) {
        if let Some((name, lines)) = self.block.take()
            && self.section == Section::Failures
        {
            self.output(&name, &lines);
        }
    }
}

/// What a run's output has said: each binary and its counts, its failures, and where it stands
#[derive(Debug, Clone)]
pub struct Tally {
    /// The run's binaries, those that ran first, then the rest in the order cargo will run them
    slots: Vec<Planned>,
    /// Its failures, the first [`KEPT_FAILURES`] of them, each output cut
    failures: Vec<FailedTest>,
    /// Where it stands
    state: RunState,
    /// The slot of the binary running, if one is
    current: Option<usize>,
    /// What the running binary has said
    reading: Reading,
    /// libtest's lines from a pipe, waiting for cargo to name their binary
    out: VecDeque<String>,
    /// cargo's lines from a pipe, waiting for the running binary's result
    err: VecDeque<String>,
    /// Whether cargo runs quietly, naming no binary, so libtest's lines take the next in turn
    quiet: bool,
    /// Whether cargo said a test failed
    said_failed: bool,
    /// Whether a binary ended before its result, as one that crashed does
    died: bool,
    /// The doc tests each package had in its last run, which a run guesses they still have
    guesses: HashMap<String, u64>,
}

impl Tally {
    /// A run that is building, its binaries not yet known
    ///
    /// # Arguments
    ///
    /// * `quiet` - Whether cargo runs quietly, so names no binary as it runs it
    pub fn new(quiet: bool) -> Tally {
        Tally {
            slots: Vec::new(),
            failures: Vec::new(),
            state: RunState::Building,
            current: None,
            reading: Reading::default(),
            out: VecDeque::new(),
            err: VecDeque::new(),
            quiet,
            said_failed: false,
            died: false,
            guesses: HashMap::new(),
        }
    }

    /// Where the run stands
    pub fn state(&self) -> RunState {
        self.state
    }

    /// Its binaries as they stand
    pub fn binaries(&self) -> Vec<Binary> {
        self.slots.iter().map(|slot| slot.binary.clone()).collect()
    }

    /// Its failures so far, oldest first
    pub fn failures(&self) -> &[FailedTest] {
        &self.failures
    }

    /// Takes the binaries the build made, each counted, in the order cargo will run them
    ///
    /// # Arguments
    ///
    /// * `planned` - The binaries
    pub fn plan(&mut self, planned: Vec<Planned>) {
        // only a run that hasn't started takes them
        if self
            .slots
            .iter()
            .any(|slot| slot.binary.started_ms.is_some())
        {
            return;
        }
        self.slots = planned;
        let guesses = self.guesses.clone();
        self.guess_doc_tests(guesses);
    }

    /// Guesses each package's doc tests from its last run, for those that haven't said
    ///
    /// # Arguments
    ///
    /// * `guesses` - How many doc tests each package had in its last run, by package
    pub fn guess_doc_tests(&mut self, guesses: HashMap<String, u64>) {
        // each crate's doc tests still waiting, and still guessed
        for slot in &mut self.slots {
            let binary = &mut slot.binary;
            if slot.doc_crate.is_some()
                && binary.guessed
                && binary.state == BinaryState::Waiting
                && let Some(total) = guesses.get(&binary.package)
            {
                binary.total = *total;
            }
        }
        // kept for binaries planned later
        self.guesses.extend(guesses);
    }

    /// Reads one line of cargo's output
    ///
    /// # Arguments
    ///
    /// * `stream` - Where it came from
    /// * `line` - The line, as a terminal shows it
    /// * `at` - When it came, in milliseconds since the run started
    pub fn line(&mut self, stream: Stream, line: &str, at: u64) {
        match stream {
            // a pty keeps the order they were written in
            Stream::Both => self.apply(line, at),
            // and a pipe's line waits its turn
            Stream::Out => {
                self.out.push_back(line.to_string());
                self.pump(false, at);
            }
            Stream::Err => {
                self.err.push_back(line.to_string());
                self.pump(false, at);
            }
        }
    }

    /// Ends the run as cargo ended: what the pipes still hold read, the binary running ended,
    /// and those cargo never ran marked so
    ///
    /// # Arguments
    ///
    /// * `ending` - How cargo ended
    /// * `at` - When, in milliseconds since the run started
    pub fn end(&mut self, ending: Ending, at: u64) {
        // the pipes said all they will
        self.pump(true, at);
        // a binary still running ended with cargo
        if self.current.is_some() {
            let died = self.reading.running;
            self.end_binary(at, died);
        }
        // and those it never ran won't run
        for slot in &mut self.slots {
            if slot.binary.state == BinaryState::Waiting {
                slot.binary.state = BinaryState::NotRun;
            }
        }
        let failed =
            self.said_failed || self.died || self.slots.iter().any(|slot| slot.binary.failed > 0);
        self.state = match (ending.passed, ending.interrupted, failed) {
            (true, _, _) => RunState::Passed,
            (false, true, _) => RunState::Stopped,
            (false, false, true) => RunState::Failed,
            (false, false, false) => RunState::Stopped,
        };
    }

    /// Reads the lines the pipes hold, each once its turn comes
    ///
    /// # Arguments
    ///
    /// * `ended` - Whether both pipes have ended, so nothing more is coming to wait for
    /// * `at` - When, in milliseconds since the run started
    fn pump(&mut self, ended: bool, at: u64) {
        loop {
            // libtest's lines go to the binary running, once cargo has named it
            if let Some(line) = self.out.front() {
                if self.current.is_some() && self.reading.running && running_count(line).is_some() {
                    // a binary says how many tests it runs while the last still owes its
                    // result: the last one died, and cargo's line naming this one comes next
                    self.end_binary(at, true);
                    continue;
                }
                if self.current.is_some() || self.quiet || (ended && self.err.is_empty()) {
                    let line = self.out.pop_front().unwrap_or_default();
                    self.libtest(&line, at);
                    continue;
                }
            }
            // cargo's lines come next, one naming a binary once the running one has its result
            if let Some(line) = self.err.front() {
                let starts = cargo_said(line).is_some_and(|said| said.starts());
                let owed = self.current.is_some() && self.reading.running;
                if starts && owed && !ended {
                    break;
                }
                let line = self.err.pop_front().unwrap_or_default();
                self.cargo(&line, at);
                continue;
            }
            break;
        }
    }

    /// Reads a line from a pty, which is cargo's or libtest's
    ///
    /// # Arguments
    ///
    /// * `line` - The line, as a terminal shows it
    /// * `at` - When, in milliseconds since the run started
    fn apply(&mut self, line: &str, at: u64) {
        match cargo_said(line) {
            Some(_) => self.cargo(line, at),
            None => self.libtest(line, at),
        }
    }

    /// Reads one of cargo's own lines
    ///
    /// # Arguments
    ///
    /// * `line` - The line, as a terminal shows it
    /// * `at` - When, in milliseconds since the run started
    fn cargo(&mut self, line: &str, at: u64) {
        match cargo_said(line) {
            Some(Said::Running { target, file }) => {
                let slot = self.find_running(target, file);
                self.start(slot, at);
            }
            Some(Said::DocTests { krate }) => {
                let slot = self.find_doc_tests(krate);
                self.start(slot, at);
            }
            Some(Said::Failed) => self.said_failed = true,
            None => {}
        }
    }

    /// The slot of the binary cargo's `Running` line names, added if the build didn't make it
    ///
    /// # Arguments
    ///
    /// * `target` - What cargo runs it as
    /// * `file` - Its executable's file name
    fn find_running(&mut self, target: &str, file: &str) -> usize {
        // by its executable, else by what cargo runs it as
        let unstarted = |slot: &Planned| slot.binary.state == BinaryState::Waiting;
        let found = self
            .slots
            .iter()
            .position(|slot| unstarted(slot) && slot.file.as_deref() == Some(file))
            .or_else(|| {
                self.slots.iter().position(|slot| {
                    unstarted(slot) && slot.doc_crate.is_none() && slot.binary.target == target
                })
            });
        if let Some(found) = found {
            return found;
        }
        // else one the build didn't name, its package the executable's crate
        let package = file.rsplit_once('-').map_or(file, |(name, _)| name);
        self.slots.push(Planned {
            binary: waiting(package, target, 0, true),
            file: Some(file.to_string()),
            doc_crate: None,
            libtest: true,
        });
        self.slots.len() - 1
    }

    /// The slot of the crate's doc tests cargo's `Doc-tests` line names, added if none was
    /// planned
    ///
    /// # Arguments
    ///
    /// * `krate` - The crate's name
    fn find_doc_tests(&mut self, krate: &str) -> usize {
        let found = self.slots.iter().position(|slot| {
            slot.binary.state == BinaryState::Waiting && slot.doc_crate.as_deref() == Some(krate)
        });
        if let Some(found) = found {
            return found;
        }
        // else its own, guessed from the last run if the crate's package is its name
        let mut planned = Planned::doc_tests(krate, krate);
        if let Some(total) = self.guesses.get(krate) {
            planned.binary.total = *total;
        }
        self.slots.push(planned);
        self.slots.len() - 1
    }

    /// Starts the binary in a slot, ending the one running and moving this one just after
    /// those that ran, so a failure's binary keeps its place
    ///
    /// # Arguments
    ///
    /// * `slot` - Its slot
    /// * `at` - When, in milliseconds since the run started
    fn start(&mut self, slot: usize, at: u64) {
        // the binary running ended before its result if it owed one
        if self.current.is_some() {
            let died = self.reading.running;
            self.end_binary(at, died);
        }
        // just after those that started, which keep their places
        let place = self
            .slots
            .iter()
            .filter(|slot| slot.binary.started_ms.is_some())
            .count();
        let moved = self.slots.remove(slot);
        let place = place.min(self.slots.len());
        self.slots.insert(place, moved);
        let binary = &mut self.slots[place].binary;
        binary.state = BinaryState::Running;
        binary.started_ms = Some(at);
        self.current = Some(place);
        self.reading = Reading::default();
        self.state = RunState::Running;
    }

    /// Starts the next binary in turn, for a quiet cargo that names none
    ///
    /// # Arguments
    ///
    /// * `at` - When, in milliseconds since the run started
    fn start_next(&mut self, at: u64) {
        // the first still waiting, else one of its own
        let next = self
            .slots
            .iter()
            .position(|slot| slot.binary.state == BinaryState::Waiting);
        let slot = match next {
            Some(next) => next,
            None => {
                self.slots.push(Planned {
                    binary: waiting("", "", 0, true),
                    file: None,
                    doc_crate: None,
                    libtest: true,
                });
                self.slots.len() - 1
            }
        };
        self.start(slot, at);
    }

    /// Ends the binary running: done with its result, else done or not run as its tests say
    ///
    /// # Arguments
    ///
    /// * `at` - When, in milliseconds since the run started
    /// * `died` - Whether it ended before the result it owed
    fn end_binary(&mut self, at: u64, died: bool) {
        let Some(slot) = self.current.take() else {
            return;
        };
        let mut reading = std::mem::take(&mut self.reading);
        reading.close_block();
        let binary = &mut self.slots[slot].binary;
        binary.ended_ms = Some(at);
        // tests it never ended didn't run
        binary.state = match binary.done() < binary.total {
            true => BinaryState::NotRun,
            false => BinaryState::Done,
        };
        self.died |= died;
        self.keep_failures(slot, reading.failed);
    }

    /// Keeps a binary's failures, up to the most a run keeps, each output cut
    ///
    /// # Arguments
    ///
    /// * `slot` - The binary's slot
    /// * `failed` - Its failed tests, each with its output
    fn keep_failures(&mut self, slot: usize, failed: Vec<(String, String)>) {
        for (test, output) in failed {
            if self.failures.len() >= KEPT_FAILURES {
                break;
            }
            self.failures.push(FailedTest {
                binary: slot,
                test,
                output: cut_output(&output),
            });
        }
    }

    /// Reads one of libtest's lines, for the binary running
    ///
    /// # Arguments
    ///
    /// * `line` - The line, as a terminal shows it
    /// * `at` - When, in milliseconds since the run started
    fn libtest(&mut self, line: &str, at: u64) {
        // a quiet cargo names no binary, so one saying how many tests it runs is the next
        if let Some(count) = running_count(line) {
            if self.quiet && (self.current.is_none() || self.reading.running) {
                self.start_next(at);
            }
            if let Some(slot) = self.current {
                let binary = &mut self.slots[slot].binary;
                binary.total = count;
                binary.guessed = false;
                self.reading.running = true;
            }
            return;
        }
        let Some(slot) = self.current else {
            return;
        };
        // its result ends it, its counts the last word
        if let Some([passed, failed, ignored]) = result_counts(line) {
            self.reading.close_block();
            self.reading.pending = None;
            let binary = &mut self.slots[slot].binary;
            binary.passed = passed;
            binary.failed = failed;
            binary.ignored = ignored;
            binary.total = binary.total.max(binary.done());
            self.end_binary(at, false);
            return;
        }
        // the summary's headers, and a test's output block
        match line {
            "failures:" => {
                self.reading.close_block();
                self.reading.section = match self.reading.section {
                    Section::Failures => Section::FailedNames,
                    _ => Section::Failures,
                };
                return;
            }
            "successes:" => {
                self.reading.close_block();
                self.reading.section = Section::Successes;
                return;
            }
            _ => {}
        }
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|rest| rest.strip_suffix(" stdout ----"))
        {
            self.reading.close_block();
            self.reading.block = Some((name.to_string(), Vec::new()));
            return;
        }
        if let Some((_, lines)) = &mut self.reading.block {
            lines.push(line.to_string());
            return;
        }
        // the failed tests named again
        if self.reading.section == Section::FailedNames {
            if let Some(name) = line.strip_prefix("    ") {
                self.reading.failed(name.trim());
            }
            return;
        }
        if self.reading.section != Section::Tests {
            return;
        }
        self.test_line(slot, line);
    }

    /// Reads a line among a binary's tests: one's result, a quiet run's marks, or with
    /// `--nocapture`, what a test printed before its result
    ///
    /// # Arguments
    ///
    /// * `slot` - The binary's slot
    /// * `line` - The line, as a terminal shows it
    fn test_line(&mut self, slot: usize, line: &str) {
        // a test's line, its result after its name
        if let Some((name, result)) = line
            .strip_prefix("test ")
            .and_then(|rest| rest.split_once(" ... "))
        {
            match outcome(result) {
                Some(outcome) => self.count(slot, name, outcome, Vec::new()),
                // with --nocapture, what it printed comes before its result
                None => self.reading.pending = Some((name.to_string(), vec![result.to_string()])),
            }
            return;
        }
        // a result on a line of its own ends the test that printed before it
        if let Some((name, mut printed)) = self.reading.pending.take() {
            match outcome(line.trim()) {
                Some(outcome) => {
                    self.count(slot, &name, outcome, printed);
                }
                None => {
                    printed.push(line.to_string());
                    self.reading.pending = Some((name, printed));
                }
            }
            return;
        }
        // a quiet run marks each test with a character, and its count so far
        let marks = line.split(' ').next().unwrap_or(line);
        if !marks.is_empty() && marks.chars().all(|c| matches!(c, '.' | 'F' | 'i')) {
            let binary = &mut self.slots[slot].binary;
            for mark in marks.chars() {
                match mark {
                    '.' => binary.passed += 1,
                    'F' => binary.failed += 1,
                    _ => binary.ignored += 1,
                }
            }
        }
    }

    /// Counts a test's end, noting a failure with what it printed
    ///
    /// # Arguments
    ///
    /// * `slot` - Its binary's slot
    /// * `name` - The test's name
    /// * `outcome` - How it ended
    /// * `printed` - What it printed before its result, with `--nocapture`
    fn count(&mut self, slot: usize, name: &str, outcome: Outcome, printed: Vec<String>) {
        let binary = &mut self.slots[slot].binary;
        match outcome {
            Outcome::Passed => binary.passed += 1,
            Outcome::Ignored => binary.ignored += 1,
            Outcome::Failed => {
                binary.failed += 1;
                // what it printed stands as its output until a block says more
                match printed.is_empty() {
                    true => self.reading.failed(name),
                    false => self.reading.output(name, &printed),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ending, Lines, Planned, Stream, Tally, visible, waiting};
    use reef_api::test_run::{BinaryState, RunState};

    /// A real `cargo test --color always --no-fail-fast -- --color always --test-threads=1
    /// --show-output` of a crate with a test that prints and passes, one that prints and fails,
    /// an ignored one, an integration test and a doc test, both streams as a terminal got them
    const COLOURED: &[u8] = include_bytes!("fixtures/coloured.ansi");

    /// The same crate's run with `--nocapture` rather than `--show-output`
    const NOCAPTURE: &[u8] = include_bytes!("fixtures/nocapture.ansi");

    /// The binaries the fixture's build makes, each counted by listing it, and its doc tests
    fn planned() -> Vec<Planned> {
        vec![
            Planned {
                binary: waiting("fixture", "unittests src/lib.rs", 4, false),
                file: Some("fixture-7950858fa0e8ef59".to_string()),
                doc_crate: None,
                libtest: true,
            },
            Planned {
                binary: waiting("fixture", "tests/more.rs", 2, false),
                file: Some("more-923d9bcc8af59bc5".to_string()),
                doc_crate: None,
                libtest: true,
            },
            Planned::doc_tests("fixture", "fixture"),
        ]
    }

    /// Reads a whole output through a pty, a chunk at a time as a pty hands them over
    ///
    /// # Arguments
    ///
    /// * `tally` - The run
    /// * `output` - The output
    /// * `chunk` - How many bytes each read takes
    fn read_pty(tally: &mut Tally, output: &[u8], chunk: usize) {
        let mut lines = Lines::default();
        for (at, bytes) in output.chunks(chunk).enumerate() {
            for line in lines.push(bytes) {
                tally.line(Stream::Both, &visible(&line), at as u64);
            }
        }
        if let Some(line) = lines.finish() {
            tally.line(Stream::Both, &visible(&line), 9_999);
        }
    }

    /// libtest's output, coloured and with the tests' own output among it, is read into each
    /// test's result and each failure's output
    #[test]
    fn coloured_output_reads_into_results_and_failures() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        read_pty(&mut tally, COLOURED, 37);
        tally.end(
            Ending {
                passed: false,
                interrupted: false,
            },
            10_000,
        );
        // each binary's counts, the ignored test among them
        let binaries = tally.binaries();
        let counts: Vec<_> = binaries
            .iter()
            .map(|binary| {
                (
                    binary.total,
                    binary.passed,
                    binary.failed,
                    binary.ignored,
                    binary.state,
                )
            })
            .collect();
        assert_eq!(
            counts,
            vec![
                (4, 2, 1, 1, BinaryState::Done),
                (2, 2, 0, 0, BinaryState::Done),
                (1, 1, 0, 0, BinaryState::Done),
            ]
        );
        // the doc tests said how many they were
        assert!(!binaries[2].guessed);
        // the one failure, with what it printed and how it panicked, and no colour; a passing
        // test's block under `successes:` is no failure
        let failures = tally.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].binary, 0);
        assert_eq!(failures[0].test, "tests::prints_then_fails");
        let output = &failures[0].output;
        assert!(output.starts_with("about to fail\n\nthread 'tests::prints_then_fails'"));
        assert!(output.contains("assertion `left == right` failed: one and one"));
        assert!(output.ends_with("to display a backtrace"));
        assert!(!output.contains('\x1b'));
        assert_eq!(tally.state(), RunState::Failed);
    }

    /// With `--nocapture`, a failure's output is what it printed before its `FAILED`
    #[test]
    fn nocapture_output_is_a_failures_output() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        read_pty(&mut tally, NOCAPTURE, 4096);
        tally.end(
            Ending {
                passed: false,
                interrupted: false,
            },
            10_000,
        );
        // the counts are as they were with the output captured
        let binaries = tally.binaries();
        assert_eq!(
            (binaries[0].passed, binaries[0].failed, binaries[0].ignored),
            (2, 1, 1)
        );
        assert_eq!(binaries[1].passed, 2);
        let failures = tally.failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].output.starts_with("about to fail"));
        assert!(failures[0].output.contains("one and one"));
    }

    /// The total is known before the first test runs, and the binary running is named
    #[test]
    fn a_planned_run_reads_its_total_before_its_tests() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        assert_eq!(tally.state(), RunState::Building);
        // cargo names the first binary, which says how many it runs
        tally.line(
            Stream::Both,
            "     Running unittests src/lib.rs (target/debug/deps/fixture-7950858fa0e8ef59)",
            5,
        );
        tally.line(Stream::Both, "running 4 tests", 6);
        let binaries = tally.binaries();
        assert_eq!(tally.state(), RunState::Running);
        assert_eq!(binaries.iter().map(|binary| binary.total).sum::<u64>(), 6);
        assert_eq!(binaries[0].state, BinaryState::Running);
        assert_eq!(binaries[0].started_ms, Some(5));
        assert_eq!(binaries[2].state, BinaryState::Waiting);
    }

    /// Through pipes, cargo's line naming the next binary waits for the last one's result, and
    /// libtest's lines wait for cargo to name their binary
    #[test]
    fn pipes_read_in_turn_whatever_order_they_arrive_in() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        // the first binary's lines come before cargo names it
        tally.line(Stream::Out, "running 1 test", 1);
        assert_eq!(tally.binaries()[0].state, BinaryState::Waiting);
        tally.line(
            Stream::Err,
            "     Running unittests src/lib.rs (target/debug/deps/fixture-7950858fa0e8ef59)",
            2,
        );
        assert_eq!(tally.binaries()[0].state, BinaryState::Running);
        assert_eq!(tally.binaries()[0].total, 1);
        // cargo names the next before the first one's result arrives
        tally.line(
            Stream::Err,
            "     Running tests/more.rs (target/debug/deps/more-923d9bcc8af59bc5)",
            3,
        );
        assert_eq!(tally.binaries()[1].state, BinaryState::Waiting);
        tally.line(Stream::Out, "test tests::adds ... ok", 4);
        tally.line(Stream::Out, "", 4);
        tally.line(
            Stream::Out,
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s",
            5,
        );
        // the first ended with its result, and the second started after it
        let binaries = tally.binaries();
        assert_eq!(
            (binaries[0].state, binaries[0].passed),
            (BinaryState::Done, 1)
        );
        assert_eq!(binaries[1].state, BinaryState::Running);
        assert_eq!(binaries[1].started_ms, Some(5));
    }

    /// A binary that says how many tests it runs while the last still owes its result is the
    /// next, and the last one died
    #[test]
    fn a_binary_that_died_ends_when_the_next_runs() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        tally.line(
            Stream::Err,
            "     Running unittests src/lib.rs (target/debug/deps/fixture-7950858fa0e8ef59)",
            1,
        );
        tally.line(Stream::Out, "running 4 tests", 2);
        tally.line(Stream::Out, "test tests::adds ... ok", 3);
        // it crashed: cargo says so, then names the next, which says how many it runs
        tally.line(Stream::Err, "error: test failed, to rerun pass `--lib`", 4);
        tally.line(
            Stream::Err,
            "     Running tests/more.rs (target/debug/deps/more-923d9bcc8af59bc5)",
            4,
        );
        tally.line(Stream::Out, "running 2 tests", 5);
        let binaries = tally.binaries();
        assert_eq!(binaries[0].state, BinaryState::NotRun);
        assert_eq!(binaries[0].passed, 1);
        assert_eq!(binaries[1].state, BinaryState::Running);
        tally.end(
            Ending {
                passed: false,
                interrupted: false,
            },
            6,
        );
        // the run failed, though no test said FAILED
        assert_eq!(tally.state(), RunState::Failed);
    }

    /// A binary cargo runs ahead of its planned turn moves just after those that ran, and a
    /// binary the build didn't make is added
    #[test]
    fn binaries_take_the_order_cargo_runs_them_in() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        // the integration test first, then one no build named
        tally.line(
            Stream::Both,
            "     Running tests/more.rs (target/debug/deps/more-923d9bcc8af59bc5)",
            1,
        );
        tally.line(Stream::Both, "running 2 tests", 1);
        tally.line(
            Stream::Both,
            "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            1,
        );
        tally.line(
            Stream::Both,
            "     Running tests/other.rs (target/debug/deps/other-0123456789abcdef)",
            2,
        );
        tally.line(Stream::Both, "running 3 tests", 3);
        let targets: Vec<_> = tally
            .binaries()
            .into_iter()
            .map(|binary| (binary.target, binary.state))
            .collect();
        assert_eq!(
            targets,
            vec![
                ("tests/more.rs".to_string(), BinaryState::Done),
                ("tests/other.rs".to_string(), BinaryState::Running),
                ("unittests src/lib.rs".to_string(), BinaryState::Waiting),
                ("doc-tests".to_string(), BinaryState::Waiting),
            ]
        );
        assert_eq!(tally.binaries()[1].total, 3);
    }

    /// A run that ends with binaries cargo never ran, as one that stops at the first failure
    /// does, marks them not run, and an interrupted run is stopped
    #[test]
    fn binaries_cargo_never_ran_are_not_run() {
        let mut tally = Tally::new(false);
        tally.plan(planned());
        tally.line(
            Stream::Both,
            "     Running unittests src/lib.rs (target/debug/deps/fixture-7950858fa0e8ef59)",
            1,
        );
        tally.line(Stream::Both, "running 4 tests", 2);
        tally.end(
            Ending {
                passed: false,
                interrupted: true,
            },
            3,
        );
        let states: Vec<_> = tally
            .binaries()
            .into_iter()
            .map(|binary| binary.state)
            .collect();
        assert_eq!(states, vec![BinaryState::NotRun; 3]);
        assert_eq!(tally.state(), RunState::Stopped);
    }

    /// Doc tests are guessed from a package's last run until they say how many they are
    #[test]
    fn doc_tests_are_guessed_until_they_say() {
        let mut tally = Tally::new(false);
        tally.guess_doc_tests([("fixture".to_string(), 7)].into());
        tally.plan(planned());
        let doc = &tally.binaries()[2];
        assert_eq!((doc.total, doc.guessed), (7, true));
        // they say how many when they run
        tally.line(Stream::Both, "   Doc-tests fixture", 1);
        tally.line(Stream::Both, "running 1 test", 2);
        let doc = &tally.binaries()[0];
        assert_eq!(
            (doc.target.as_str(), doc.total, doc.guessed),
            ("doc-tests", 1, false)
        );
    }

    /// A quiet cargo names no binary, so each `running N tests` is the next in turn
    #[test]
    fn a_quiet_run_takes_binaries_in_turn() {
        let mut tally = Tally::new(true);
        tally.plan(planned());
        tally.line(Stream::Out, "running 4 tests", 1);
        tally.line(Stream::Out, "..Fi", 2);
        tally.line(
            Stream::Out,
            "test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            3,
        );
        tally.line(Stream::Out, "running 2 tests", 4);
        let binaries = tally.binaries();
        assert_eq!((binaries[0].passed, binaries[0].failed), (2, 1));
        assert_eq!(binaries[0].state, BinaryState::Done);
        assert_eq!(
            (binaries[1].total, binaries[1].state),
            (2, BinaryState::Running)
        );
    }

    /// A line is read as a terminal shows it: colours, links and character sets left out, and
    /// what a carriage return drew over gone
    #[test]
    fn lines_read_as_a_terminal_shows_them() {
        // cargo's status, a link in it, and libtest's coloured result
        let finished = b"\x1b[1m\x1b[92m    Finished\x1b[0m \x1b]8;;https://doc.rust-lang.org/\x1b\\`test` profile\x1b]8;;\x1b\\ target(s) in 0.00s";
        assert_eq!(
            visible(finished),
            "    Finished `test` profile target(s) in 0.00s"
        );
        assert_eq!(
            visible(b"test result: \x1b[31mFAILED\x1b(B\x1b[m. 2 passed"),
            "test result: FAILED. 2 passed"
        );
        // a pty's newline, and a link ended by a bell
        assert_eq!(visible(b"running 1 test\r"), "running 1 test");
        assert_eq!(visible(b"\x1b]8;;x\x07link\x1b]8;;\x07"), "link");
        // lines split as they come, the last held until it ends
        let mut lines = Lines::default();
        assert_eq!(lines.push(b"one\ntw"), vec![b"one".to_vec()]);
        assert_eq!(lines.push(b"o\n"), vec![b"two".to_vec()]);
        assert_eq!(lines.push(b"three"), Vec::<Vec<u8>>::new());
        assert_eq!(lines.finish(), Some(b"three".to_vec()));
    }
}
