//! Signing in from the terminal: `reef login`, `logout` and `whoami`, adding, listing and
//! removing users and changing their passwords, and listing and revoking tokens
//!
//! - **A password is asked for on the terminal,** with echo off, never taken as an argument,
//!   which shells keep in their history. `--password-file` reads one from a file, or from
//!   standard input as `-`, for a script ([`reef_api::prompt`]).
//! - **A login keeps a token, never the password,** in the credentials file, a server each.
//! - **A logout revokes the token at the service, then forgets it here,** even when the service
//!   can't be reached, so a token never outlives the login it was kept for on this host.
//! - **The first user is added from the service's own host** while it has none, and is an admin.
//! - **Your own password needs your current one,** and leaves your tokens signing in; an admin
//!   setting another user's revokes every token of theirs.
//! - **A token is named by its id,** cut from its hash, so a listing never shows a token.

use reef_api::prompt::{read_new_password, read_password};
use reef_api::{ErrorCode, NewUser, PasswordChange, SignIn, TokenInfo, UserAdded, UserRemoved};
use std::io::Write;
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
    let password = read_new_password(&format!("password for `{name}`: "), password_file)
        .map_err(|error| anyhow::anyhow!("{error}, so no user was added"))?;
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
        // whether they are an admin, then who added them, if anyone known did
        let mut said: Vec<String> = Vec::new();
        if user.admin {
            said.push("admin".to_string());
        }
        if let Some(added) = crate::history::by_whom("added", &user.added_by) {
            said.push(added);
        }
        match said.is_empty() {
            true => writeln!(out, "{}", user.name)?,
            false => writeln!(out, "{}  {}", user.name, said.join(", "))?,
        }
    }
    Ok(())
}

/// Where a password change reads its passwords from instead of the terminal
#[derive(Debug, Clone, Copy, Default)]
pub struct PasswordFiles<'a> {
    /// The new password's file, or `-` for standard input
    pub new: Option<&'a Path>,
    /// The current password's file, or `-` for standard input
    pub current: Option<&'a Path>,
}

/// Changes a password: your own with your current one, which leaves your tokens, or another
/// user's as an admin, which revokes theirs
///
/// # Arguments
///
/// * `client` - The service's client
/// * `name` - The user whose password to set, else this `reef`'s own user's
/// * `files` - Where to read the passwords from instead of the terminal
/// * `out` - Where to say what was done
pub async fn user_passwd<W: Write>(
    client: &Client,
    name: Option<&str>,
    files: PasswordFiles<'_>,
    out: &mut W,
) -> anyhow::Result<()> {
    // whom this `reef` signs in, which decides whose password is its own
    let me = client.me().await?.user;
    let own = me.as_ref().map(|user| user.name.as_str());
    let Some(name) = name.or(own) else {
        anyhow::bail!(
            "this `reef` signs nobody in at {}: run `reef login`, or name the user",
            client.base()
        );
    };
    // your own needs your current password, and another's doesn't
    let current = match own == Some(name) {
        true => {
            let prompt = format!("current password for `{name}`: ");
            Some(read_password(&prompt, files.current)?)
        }
        false => None,
    };
    let prompt = format!("new password for `{name}`: ");
    let password = read_new_password(&prompt, files.new)
        .map_err(|error| anyhow::anyhow!("{error}, so the password is as it was"))?;
    let own_change = current.is_some();
    let change = PasswordChange { current, password };
    let changed = client.change_password(name, &change).await?;
    // your tokens still sign you in, and another's were revoked
    match own_change {
        true => writeln!(
            out,
            "changed your password at {}; your tokens still sign you in, and `reef token ls` \
             lists them",
            client.base()
        )?,
        false => writeln!(
            out,
            "set the password of `{name}` at {}, revoking {}",
            client.base(),
            tokens(changed.revoked)
        )?,
    }
    Ok(())
}

/// Removes a user and revokes their tokens, which only an admin may, and never the last admin
///
/// # Arguments
///
/// * `client` - The service's client
/// * `name` - The user's name
/// * `out` - Where to say what was done
pub async fn user_rm<W: Write>(client: &Client, name: &str, out: &mut W) -> anyhow::Result<()> {
    match client.remove_user(name).await? {
        UserRemoved::Removed { revoked } => {
            writeln!(
                out,
                "removed `{name}` from {}, revoking {}",
                client.base(),
                tokens(revoked)
            )?;
            Ok(())
        }
        UserRemoved::NoSuchUser => anyhow::bail!("no user is named `{name}` at {}", client.base()),
        UserRemoved::LastAdmin => anyhow::bail!(
            "`{name}` is the last admin of {}, so stays: add another with `reef user add NAME \
             --admin` first",
            client.base()
        ),
    }
}

/// Lists your tokens, or another user's as an admin, the one this `reef` sends marked
///
/// # Arguments
///
/// * `client` - The service's client
/// * `user` - The user whose tokens to list, else your own
/// * `out` - Where to list them
pub async fn token_ls<W: Write>(
    client: &Client,
    user: Option<&str>,
    out: &mut W,
) -> anyhow::Result<()> {
    let listed = client.tokens(user).await?;
    if listed.is_empty() {
        writeln!(out, "no tokens sign {} in", user.unwrap_or("you"))?;
    }
    for token in &listed {
        writeln!(out, "{}", token_line(token))?;
    }
    Ok(())
}

/// One token as a listing shows it: its id, kind, when it was issued and expires, its label,
/// and whether this `reef` sends it
///
/// # Arguments
///
/// * `token` - The token
fn token_line(token: &TokenInfo) -> String {
    // a kind of a fixed width, so the columns after it line up
    let kind = match token.kind {
        reef_api::TokenKind::Cli => "cli    ",
        reef_api::TokenKind::Browser => "browser",
    };
    let expires = token
        .expires_at
        .map(|at| format!("  expires {}", crate::start::when(at)))
        .unwrap_or_default();
    let current = if token.current { "  (this reef)" } else { "" };
    format!(
        "{}  {kind}  issued {}{expires}  {}{current}",
        token.id,
        crate::start::when(token.created_at),
        token.label
    )
}

/// Revokes tokens by their ids, or every token of yours but the one this `reef` sends
///
/// # Arguments
///
/// * `client` - The service's client
/// * `ids` - The tokens' ids
/// * `others` - Whether to revoke every token of yours but this `reef`'s instead
/// * `out` - Where to say what was done
pub async fn token_revoke<W: Write>(
    client: &Client,
    ids: &[String],
    others: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    // the others are every token listed but the one sent
    let ids: Vec<String> = match others {
        true => client
            .tokens(None)
            .await?
            .into_iter()
            .filter(|token| !token.current)
            .map(|token| token.id)
            .collect(),
        false => ids.to_vec(),
    };
    if ids.is_empty() {
        writeln!(out, "no other token signs you in")?;
    }
    // each in turn, a token the service doesn't hold for you said so
    for id in &ids {
        match client.revoke_token(id).await {
            Ok(_) => writeln!(out, "revoked {id}")?,
            Err(error) if is_code(&error, ErrorCode::NotFound) => {
                anyhow::bail!("{error}")
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// How many tokens, said in words
///
/// # Arguments
///
/// * `count` - How many
fn tokens(count: usize) -> String {
    match count {
        1 => "1 token".to_string(),
        count => format!("{count} tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::{token_line, tokens};
    use reef_api::{TokenInfo, TokenKind};

    /// A token's line shows its id, kind, times and label, and marks the one this `reef` sends
    #[test]
    fn a_token_line_marks_the_one_sent() {
        // a terminal's, which never expires, sent by this `reef`
        let mut token = TokenInfo {
            id: "0123456789abcdef".into(),
            user: "ada".into(),
            kind: TokenKind::Cli,
            label: "reef on laptop".into(),
            created_at: 0,
            expires_at: None,
            current: true,
        };
        assert_eq!(
            token_line(&token),
            "0123456789abcdef  cli      issued 1970-01-01 00:00:00 +0000  reef on laptop  (this reef)"
        );
        // a browser's, which expires, and isn't
        token.kind = TokenKind::Browser;
        token.expires_at = Some(1_795_000_000_123);
        token.current = false;
        assert_eq!(
            token_line(&token),
            "0123456789abcdef  browser  issued 1970-01-01 00:00:00 +0000  expires 2026-11-18 \
             11:06:40 +0000  reef on laptop"
        );
        // and counts read as words
        assert_eq!(tokens(1), "1 token");
        assert_eq!(tokens(0), "0 tokens");
    }
}
