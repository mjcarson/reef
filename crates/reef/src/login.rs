//! Signing in from the terminal: `reef login`, `logout` and `whoami`, and adding and listing
//! users
//!
//! - **A password is asked for on the terminal,** with echo off, never taken as an argument,
//!   which shells keep in their history. `--password-file` reads one from a file, or from
//!   standard input as `-`, for a script.
//! - **A login keeps a token, never the password,** in the credentials file, a server each.
//! - **A logout revokes the token at the service, then forgets it here,** even when the service
//!   can't be reached, so a token never outlives the login it was kept for on this host.
//! - **The first user is added from the service's own host** while it has none, and is an admin.

use reef_api::{ErrorCode, NewUser, SignIn, UserAdded};
use std::io::{BufRead, Write};
use std::path::Path;

use crate::client::{Client, is_code};
use crate::credentials::{self, Held, Login};

/// The credentials file a login is kept in, or why there is none
///
/// # Arguments
///
/// * `client` - The service's client
fn kept_in(client: &Client) -> anyhow::Result<&Path> {
    match client.held() {
        Held::File(path) => Ok(path),
        // a token given outright wins over any file, so a login kept there would never be sent
        Held::Token(_) => anyhow::bail!(
            "`${}` is set, and wins over any login kept: unset it to log in",
            credentials::TOKEN_VAR
        ),
        Held::Nowhere => anyhow::bail!(
            "there is no home to keep a login in: name a file with `--credentials PATH`"
        ),
    }
}

/// The name a login or a new user takes when none is given: the user's own on this host
fn own_name() -> anyhow::Result<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("no user name is set here: name one with `--user NAME`"))
}

/// Reads a password from a file, from standard input as `-`, or else from the terminal with
/// echo off
///
/// # Arguments
///
/// * `prompt` - What to ask on the terminal
/// * `file` - The file to read it from instead, or `-` for standard input
pub fn read_password(prompt: &str, file: Option<&Path>) -> anyhow::Result<String> {
    // a file's first line, its line ending left off
    if let Some(file) = file {
        let text = match file == Path::new("-") {
            true => {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                line
            }
            false => std::fs::read_to_string(file)
                .map_err(|error| anyhow::anyhow!("reading {}: {error}", file.display()))?,
        };
        return Ok(first_line(&text));
    }
    // else the terminal itself, which a pipe into standard input can't stand in for
    ask_terminal(prompt)
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
fn ask_terminal(prompt: &str) -> anyhow::Result<String> {
    // the controlling terminal, read and written directly
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            anyhow::anyhow!(
                "there is no terminal to ask for a password on: pass `--password-file PATH`, or \
                 `-` for standard input"
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
    fn new(tty: &'a std::fs::File) -> anyhow::Result<Quiet<'a>> {
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

/// Logs in: trades a user's name and password for a token, and keeps it for the service
///
/// # Arguments
///
/// * `client` - The service's client
/// * `user` - The user's name, else this host's user's
/// * `password_file` - Where to read the password from instead of the terminal
/// * `label` - What the token is for, else `reef on <host>`
/// * `out` - Where to say who is logged in
pub async fn login<W: Write>(
    client: &Client,
    user: Option<&str>,
    password_file: Option<&Path>,
    label: Option<&str>,
    out: &mut W,
) -> anyhow::Result<()> {
    // where the login is kept, which must be a file
    let file = kept_in(client)?.to_path_buf();
    let name = match user {
        Some(user) => user.to_string(),
        None => own_name()?,
    };
    let prompt = format!("password for `{name}` at {}: ", client.base());
    let password = read_password(&prompt, password_file)?;
    // the token, labelled so a list of them can tell this host's apart
    let label = match label {
        Some(label) => label.to_string(),
        None => format!("reef on {}", crate::run::host_name()),
    };
    let request = SignIn {
        name: name.clone(),
        password,
        label: Some(label),
    };
    let issued = match client.sign_in(&request).await {
        Ok(issued) => issued,
        // a wrong name or password, said as the service says it
        Err(error) if is_code(&error, ErrorCode::Unauthorized) => {
            anyhow::bail!("{error}, at {}", client.base())
        }
        Err(error) => return Err(error),
    };
    // kept a server each, never the password
    let login = Login {
        user: issued.user.name.clone(),
        token: issued.token,
    };
    credentials::keep(&file, client.base(), login)?;
    let admin = if issued.user.admin { ", an admin" } else { "" };
    writeln!(
        out,
        "logged in to {} as `{}`{admin}; the token is kept in {}",
        client.base(),
        issued.user.name,
        file.display()
    )?;
    Ok(())
}

/// Logs out: revokes the token at the service, then forgets it here, even if the service can't
/// be reached
///
/// # Arguments
///
/// * `client` - The service's client
/// * `out` - Where to say what was done
pub async fn logout<W: Write>(client: &Client, out: &mut W) -> anyhow::Result<()> {
    // a token given outright is revoked, but only unsetting it stops it being sent
    if let Held::Token(_) = client.held() {
        let revoked = client.sign_out().await?;
        let said = if revoked.revoked {
            "revoked"
        } else {
            "held nothing for"
        };
        writeln!(
            out,
            "the service {said} the token in `${}`: unset it",
            credentials::TOKEN_VAR
        )?;
        return Ok(());
    }
    // nothing kept for this service is nothing to do
    let file = kept_in(client)?.to_path_buf();
    if credentials::login_for(&file, client.base())?.is_none() {
        writeln!(out, "not logged in to {}", client.base())?;
        return Ok(());
    }
    // revoked there first, then forgotten here whatever the service said
    let revoked = client.sign_out().await;
    credentials::forget(&file, client.base())?;
    match revoked {
        Ok(_) => writeln!(out, "logged out of {}", client.base())?,
        Err(error) => writeln!(
            out,
            "forgot the token for {}, which the service didn't revoke: {error:#}",
            client.base()
        )?,
    }
    Ok(())
}

/// Says who the service takes this `reef` for, and whether it asks callers to sign in
///
/// # Arguments
///
/// * `client` - The service's client
/// * `out` - Where to say it
pub async fn whoami<W: Write>(client: &Client, out: &mut W) -> anyhow::Result<()> {
    let me = client.me().await?;
    match (&me.user, me.required) {
        (Some(user), _) => {
            let admin = if user.admin { ", an admin" } else { "" };
            writeln!(out, "`{}`{admin}, at {}", user.name, client.base())?;
        }
        (None, true) => writeln!(
            out,
            "nobody: {} asks every caller to sign in, so run `reef login`",
            client.base()
        )?,
        (None, false) => writeln!(out, "nobody: {} asks no caller to sign in", client.base())?,
    }
    Ok(())
}

/// Adds a user: the first from the service's own host, as an admin, and the rest as an admin
///
/// # Arguments
///
/// * `client` - The service's client
/// * `name` - The new user's name
/// * `admin` - Whether they may add other users
/// * `password_file` - Where to read their password from instead of the terminal
/// * `out` - Where to say who was added
pub async fn user_add<W: Write>(
    client: &Client,
    name: &str,
    admin: bool,
    password_file: Option<&Path>,
    out: &mut W,
) -> anyhow::Result<()> {
    // the password once from a file, or twice on the terminal, so a slip isn't kept
    let password = match password_file {
        Some(file) => read_password("", Some(file))?,
        None => {
            let first = read_password(&format!("password for `{name}`: "), None)?;
            let again = read_password("the same again: ", None)?;
            anyhow::ensure!(
                first == again,
                "the two passwords differ, so no user was added"
            );
            first
        }
    };
    let new = NewUser {
        name: name.to_string(),
        password,
        admin,
    };
    match client.add_user(&new).await? {
        UserAdded::Added(user) => {
            let admin = if user.admin { ", an admin," } else { "" };
            writeln!(out, "added `{}`{admin} to {}", user.name, client.base())?;
            Ok(())
        }
        UserAdded::NameTaken => anyhow::bail!("a user named `{name}` already exists"),
    }
}

/// Lists every user, an admin's marked so
///
/// # Arguments
///
/// * `client` - The service's client
/// * `out` - Where to list them
pub async fn user_ls<W: Write>(client: &Client, out: &mut W) -> anyhow::Result<()> {
    let users = client.users().await?;
    if users.is_empty() {
        writeln!(out, "{} has no users", client.base())?;
    }
    for user in users {
        let admin = if user.admin { "  admin" } else { "" };
        writeln!(out, "{}{admin}", user.name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{first_line, read_password};

    /// A password file's first line is the password, its line ending left off
    #[test]
    fn a_password_file_loses_its_newline() {
        // each line ending, and none
        assert_eq!(first_line("hunter22\n"), "hunter22");
        assert_eq!(first_line("hunter22\r\n"), "hunter22");
        assert_eq!(first_line("hunter22"), "hunter22");
        assert_eq!(first_line("one\ntwo\n"), "one");
        assert_eq!(first_line(""), "");
        // read from a file, the same
        let file = std::env::temp_dir().join(format!("reef-password-{}", std::process::id()));
        std::fs::write(&file, "correct horse\n").unwrap();
        assert_eq!(read_password("", Some(&file)).unwrap(), "correct horse");
        let _ = std::fs::remove_file(&file);
    }
}
