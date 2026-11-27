//! A pty for cargo to write to as it would to the terminal `reef test` runs in, so its colours,
//! its progress bar and the order of its lines stay what the terminal would show
//!
//! - **Its size is the terminal's,** and follows it when the terminal is resized.
//! - **Its bytes pass as written:** output processing is off, so a newline isn't made a
//!   carriage return and a newline on the way through.
//! - **No session of its own.** cargo stays in `reef test`'s process group, reading the
//!   terminal's own standard input, so Ctrl-C reaches it as it would without reef.

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use rustix::pty::OpenptFlags;
use rustix::termios::{OptionalActions, OutputModes, Winsize};
use std::ffi::CString;
use std::io;
use std::os::fd::OwnedFd;
use tokio::io::unix::AsyncFd;

/// The size a pty takes where `reef test`'s own output isn't a terminal
const FALLBACK: Winsize = Winsize {
    ws_row: 24,
    ws_col: 80,
    ws_xpixel: 0,
    ws_ypixel: 0,
};

/// A pty's master side, which `reef test` reads what cargo writes from
#[derive(Debug)]
pub struct Pty {
    /// The master, read as it becomes readable
    master: AsyncFd<OwnedFd>,
}

impl Pty {
    /// Opens a pty the terminal's size, returning it and its slave side for cargo to write to
    ///
    /// The slave must be handed to cargo and dropped here, or the master never reads an end.
    pub fn open() -> io::Result<(Pty, OwnedFd)> {
        // the master, and the slave it names
        let master =
            rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let name: CString = rustix::pty::ptsname(&master, Vec::new())?;
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        // bytes pass as cargo wrote them, at the terminal's size
        let mut termios = rustix::termios::tcgetattr(&slave)?;
        termios.output_modes.remove(OutputModes::OPOST);
        rustix::termios::tcsetattr(&slave, OptionalActions::Now, &termios)?;
        rustix::termios::tcsetwinsize(&master, terminal_size())?;
        // the master is read as it becomes readable
        rustix::io::ioctl_fionbio(&master, true)?;
        // SAFETY: the master is an `OwnedFd`, a valid descriptor the `AsyncFd` owns from here,
        // which nothing replaces or closes while it does
        let master = unsafe { AsyncFd::register(master)? };
        Ok((Pty { master }, slave))
    }

    /// Sizes the pty as the terminal is now, after the terminal was resized
    pub fn resize(&self) {
        // a pty that can't be resized keeps its size
        let _ = rustix::termios::tcsetwinsize(self.master.get_ref(), terminal_size());
    }

    /// Reads what cargo wrote, or 0 once every writer has closed the slave
    ///
    /// # Arguments
    ///
    /// * `buf` - Where to read to
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ready = self.master.readable().await?;
            match ready.try_io(|master| Ok(rustix::io::read(master.get_ref(), &mut *buf)?)) {
                Ok(Ok(read)) => return Ok(read),
                // a master whose slave every writer closed says EIO, which is its end
                Ok(Err(error)) if Errno::from_io_error(&error) == Some(Errno::IO) => return Ok(0),
                Ok(Err(error)) => return Err(error),
                // nothing was there after all
                Err(_) => continue,
            }
        }
    }
}

/// The size of the terminal `reef test` writes to, else a usual terminal's
fn terminal_size() -> Winsize {
    rustix::termios::tcgetwinsize(std::io::stdout()).unwrap_or(FALLBACK)
}

#[cfg(test)]
mod tests {
    use super::Pty;
    use std::process::Stdio;

    /// What a child writes under the pty reads back, and the child sees a terminal
    #[tokio::test]
    async fn a_child_under_the_pty_sees_a_terminal() {
        let (pty, slave) = Pty::open().expect("a pty opens");
        // a child whose output is the slave, which this side then lets go of
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "test -t 1 && printf 'a terminal\\n'"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .expect("sh runs");
        // read until every writer is gone
        let mut read = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            let got = pty.read(&mut buf).await.expect("the pty reads");
            if got == 0 {
                break;
            }
            read.extend_from_slice(&buf[..got]);
        }
        assert!(child.wait().await.unwrap().success());
        // a newline came as written, with no carriage return added
        assert_eq!(read, b"a terminal\n");
    }
}
