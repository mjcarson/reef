//! Asking for a password: on the terminal with echo off, or from a file a script names
//!
//! The CLI asks for one to log in, to add a user or to change a password, and `reef-serve` to
//! recover a lost admin, so both ask the same way. A password is never taken as an argument,
//! which shells keep in their history. A file's first line is read instead, or standard input's
//! as `-`, for a script.

use std::io::{BufRead, Write};
use std::path::Path;

/// Reads a password from a file, from standard input as `-`, or else from the terminal with
/// echo off
///
/// # Arguments
///
/// * `prompt` - What to ask on the terminal
/// * `file` - The file to read it from instead, or `-` for standard input
pub fn read_password(prompt: &str, file: Option<&Path>) -> std::io::Result<String> {
    // a file's first line, its line ending left off
    if let Some(file) = file {
        let text = match file == Path::new("-") {
            true => {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                line
            }
            false => std::fs::read_to_string(file).map_err(|error| {
                std::io::Error::new(error.kind(), format!("reading {}: {error}", file.display()))
            })?,
        };
        return Ok(first_line(&text));
    }
    // else the terminal itself, which a pipe into standard input can't stand in for
    ask_terminal(prompt)
}

/// Reads a new password: once from a file, or twice on the terminal so a slip isn't kept
///
/// # Arguments
///
/// * `prompt` - What to ask on the terminal the first time
/// * `file` - The file to read it from instead, or `-` for standard input
pub fn read_new_password(prompt: &str, file: Option<&Path>) -> std::io::Result<String> {
    // a file is read once, since a script wrote it
    if file.is_some() {
        return read_password(prompt, file);
    }
    // the terminal twice, the two the same
    let first = read_password(prompt, None)?;
    let again = read_password("the same again: ", None)?;
    match first == again {
        true => Ok(first),
        false => Err(std::io::Error::other("the two passwords differ")),
    }
}

/// A file's first line, its line ending left off
///
/// # Arguments
///
/// * `text` - The file's text
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

/// Asks for a password on the terminal, with echo off while it is typed
///
/// # Arguments
///
/// * `prompt` - What to ask
fn ask_terminal(prompt: &str) -> std::io::Result<String> {
    // the controlling terminal, read and written directly
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            std::io::Error::other(
                "there is no terminal to ask for a password on: pass `--password-file PATH`, or \
                 `-` for standard input",
            )
        })?;
    write!(tty, "{prompt}")?;
    tty.flush()?;
    // echo off until the line is read, and back on however reading ends
    let line = {
        let _quiet = Quiet::new(&tty)?;
        let mut line = String::new();
        std::io::BufReader::new(&tty).read_line(&mut line)?;
        line
    };
    // the newline the user typed wasn't echoed, so the next output starts a line of its own
    writeln!(tty)?;
    Ok(first_line(&line))
}

/// The terminal's echo turned off, and turned back on when this is dropped
struct Quiet<'a> {
    /// The terminal
    tty: &'a std::fs::File,
    /// Its settings before echo was turned off
    was: rustix::termios::Termios,
}

impl<'a> Quiet<'a> {
    /// Turns the terminal's echo off
    ///
    /// # Arguments
    ///
    /// * `tty` - The terminal
    fn new(tty: &'a std::fs::File) -> std::io::Result<Quiet<'a>> {
        use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
        // what it was, to put back
        let was = tcgetattr(tty)?;
        let mut quiet = was.clone();
        // nothing typed shown, though a newline still ends the line
        quiet.local_modes -= LocalModes::ECHO;
        quiet.local_modes |= LocalModes::ECHONL;
        tcsetattr(tty, OptionalActions::Now, &quiet)?;
        Ok(Quiet { tty, was })
    }
}

impl Drop for Quiet<'_> {
    /// Turns the terminal's echo back on, as it was
    fn drop(&mut self) {
        use rustix::termios::{OptionalActions, tcsetattr};
        let _ = tcsetattr(self.tty, OptionalActions::Now, &self.was);
    }
}

#[cfg(test)]
mod tests {
    use super::{first_line, read_new_password, read_password};

    /// A password file's first line is the password, its line ending left off
    #[test]
    fn a_password_file_loses_its_newline() {
        // each line ending, and none
        assert_eq!(first_line("hunter22\n"), "hunter22");
        assert_eq!(first_line("hunter22\r\n"), "hunter22");
        assert_eq!(first_line("hunter22"), "hunter22");
        assert_eq!(first_line("one\ntwo\n"), "one");
        assert_eq!(first_line(""), "");
        // read from a file, the same, and a new one read from it once
        let file = std::env::temp_dir().join(format!("reef-password-{}", std::process::id()));
        std::fs::write(&file, "correct horse\n").unwrap();
        assert_eq!(read_password("", Some(&file)).unwrap(), "correct horse");
        assert_eq!(read_new_password("", Some(&file)).unwrap(), "correct horse");
        let _ = std::fs::remove_file(&file);
        // a file that isn't there is named
        let missing = std::env::temp_dir().join("reef-password-that-is-not-there");
        let error = read_password("", Some(&missing)).unwrap_err().to_string();
        assert!(error.contains("reef-password-that-is-not-there"), "{error}");
    }
}
