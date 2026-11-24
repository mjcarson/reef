//! The account page: your password, your tokens, and, for an admin, every user
//!
//! The bar's name links here once the server asks callers to sign in. Your own password changes
//! with your current one, and leaves your tokens signing in. Each token is listed by its id, kind,
//! label and dates, this browser's marked, and any other is revoked from here. An admin sees every
//! user, sets another's password, which revokes that user's tokens, and removes a user, asked to
//! confirm first, and never the last admin.
//!
//! No password field sits in a `form`, so nothing typed is sent before the page's script runs.

use dioxus::prelude::*;
use reef_api::{PasswordChange, TokenInfo, TokenKind, User, UserRemoved};

use crate::app::Route;
use crate::calls;
use crate::sign_in::{CallFailed, SignedInAs};

/// A time the service stamped, in milliseconds since the epoch, as a date and minute in UTC
///
/// # Arguments
///
/// * `ms` - The milliseconds since the epoch
pub fn utc_minute(ms: u64) -> String {
    // whole days, and the minute within the last
    let minutes = ms / 60_000;
    let (days, minute) = (minutes / (24 * 60), minutes % (24 * 60));
    // the civil date of a count of days since 1970-01-01, as Howard Hinnant reckons it
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02} UTC",
        minute / 60,
        minute % 60
    )
}

/// The account page: who you are, your password and your tokens, and every user to an admin
#[component]
pub fn Account() -> Element {
    // read on the server as the page renders, and again after each change
    let mut account = use_server_future(calls::account)?;
    let changed = move |_| account.restart();
    rsx! {
        main { class: "reef-app-picker reef-app-account",
            header { class: "reef-app-picker-head",
                h1 { "Account" }
                Link { to: Route::Picker {}, "← Plans" }
                SignedInAs {}
            }
            match &*account.read() {
                Some(Ok(account)) => match &account.me.user {
                    Some(user) => rsx! {
                        p { class: "reef-app-muted",
                            "Signed in as "
                            strong { "{user.name}" }
                            if user.admin { ", an admin" }
                            "."
                        }
                        OwnPassword { name: user.name.clone() }
                        Tokens { tokens: account.tokens.clone(), changed }
                        if user.admin {
                            Users { users: account.users.clone(), me: user.name.clone(), changed }
                        }
                    },
                    None if account.me.required => rsx! {
                        p { class: "reef-app-muted", "Sign in to see your account." }
                    },
                    None => rsx! {
                        p { class: "reef-app-muted",
                            "reef asks nobody to sign in, so there is no account to manage here."
                        }
                    },
                },
                Some(Err(error)) => rsx! {
                    p { CallFailed { error: error.clone(), doing: "Reading your account" } }
                },
                None => rsx! {
                    p { class: "reef-app-muted", "Loading…" }
                },
            }
        }
    }
}

/// Changing your own password: your current one, and the new one twice
///
/// # Arguments
///
/// * `name` - Your name
#[component]
fn OwnPassword(name: String) -> Element {
    let mut current = use_signal(String::new);
    let mut new = use_signal(String::new);
    let mut again = use_signal(String::new);
    let mut said = use_signal(|| None::<(bool, String)>);
    let mut busy = use_signal(|| false);
    let change = move |_| {
        // the two new ones the same, before anything is sent
        if *new.peek() != *again.peek() {
            said.set(Some((false, "The two new passwords differ.".to_string())));
            return;
        }
        let change = PasswordChange {
            current: Some(current.peek().clone()),
            password: new.peek().clone(),
        };
        let name = name.clone();
        busy.set(true);
        spawn(async move {
            // what came of it, the fields emptied once it is changed
            let outcome = match calls::change_password(name, change).await {
                Ok(_) => {
                    current.set(String::new());
                    new.set(String::new());
                    again.set(String::new());
                    (
                        true,
                        "Your password is changed. Your tokens still sign you in.".to_string(),
                    )
                }
                Err(error) => (false, format!("Changing your password failed: {error}")),
            };
            said.set(Some(outcome));
            busy.set(false);
        });
    };
    let empty = current.read().is_empty() || new.read().is_empty();
    rsx! {
        section { class: "reef-app-account-part",
            h2 { "Your password" }
            div { class: "reef-app-fields",
                label { r#for: "account-current", "Current password" }
                input {
                    id: "account-current",
                    r#type: "password",
                    autocomplete: "current-password",
                    value: "{current}",
                    oninput: move |event| current.set(event.value()),
                }
                label { r#for: "account-new", "New password" }
                input {
                    id: "account-new",
                    r#type: "password",
                    autocomplete: "new-password",
                    value: "{new}",
                    oninput: move |event| new.set(event.value()),
                }
                label { r#for: "account-again", "The new one again" }
                input {
                    id: "account-again",
                    r#type: "password",
                    autocomplete: "new-password",
                    value: "{again}",
                    oninput: move |event| again.set(event.value()),
                }
            }
            div { class: "reef-app-actions",
                button {
                    class: "reef-button",
                    "data-act": "change-password",
                    disabled: empty || *busy.read(),
                    onclick: change,
                    "Change password"
                }
            }
            Said { said: said() }
        }
    }
}

/// What an action on the page came to, said beside it
///
/// # Arguments
///
/// * `said` - Whether it went through, and what to say, if anything is to be said
#[component]
fn Said(said: Option<(bool, String)>) -> Element {
    match said {
        Some((done, text)) => rsx! {
            p {
                class: if done { "reef-app-account-said" } else { "reef-app-account-said reef-app-error" },
                role: "status",
                {text}
            }
        },
        None => rsx! {},
    }
}

/// Your tokens, oldest first, each revoked from here but this browser's
///
/// # Arguments
///
/// * `tokens` - Your tokens
/// * `changed` - Called once a token is revoked, to read the account again
#[component]
fn Tokens(tokens: Vec<TokenInfo>, changed: EventHandler) -> Element {
    let mut said = use_signal(|| None::<(bool, String)>);
    let revoke = move |id: String| {
        move |_| {
            let id = id.clone();
            spawn(async move {
                // what came of it, and the list read again
                let outcome = match calls::revoke_token(id.clone()).await {
                    Ok(revoked) if revoked.revoked => (true, format!("Revoked {id}.")),
                    Ok(_) => (false, format!("{id} was revoked already.")),
                    Err(error) => (false, format!("Revoking {id} failed: {error}")),
                };
                said.set(Some(outcome));
                changed.call(());
            });
        }
    };
    rsx! {
        section { class: "reef-app-account-part",
            h2 { "Your tokens" }
            p { class: "reef-app-muted",
                "Each signs you in: a terminal's until "
                code { "reef logout" }
                ", a browser's for 30 days. Revoke one you no longer use, or don't know."
            }
            table { class: "reef-app-account-table",
                thead {
                    tr {
                        th { "Id" }
                        th { "Kind" }
                        th { "For" }
                        th { "Issued" }
                        th { "Expires" }
                        th {}
                    }
                }
                tbody {
                    for token in tokens {
                        tr { key: "{token.id}", "data-token": "{token.id}",
                            td { code { "{token.id}" } }
                            td {
                                match token.kind {
                                    TokenKind::Cli => "terminal",
                                    TokenKind::Browser => "browser",
                                }
                            }
                            td { class: "reef-app-account-label", "{token.label}" }
                            td { "{utc_minute(token.created_at)}" }
                            td {
                                match token.expires_at {
                                    Some(at) => utc_minute(at),
                                    None => "never".to_string(),
                                }
                            }
                            td {
                                if token.current {
                                    span { class: "reef-app-muted", "this browser" }
                                } else {
                                    button {
                                        class: "reef-button",
                                        "data-act": "revoke",
                                        onclick: revoke(token.id.clone()),
                                        "Revoke"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Said { said: said() }
        }
    }
}

/// Every user, to an admin: each but you removed or given a new password from here
///
/// # Arguments
///
/// * `users` - Every user
/// * `me` - Your name
/// * `changed` - Called once a user is removed or their password set, to read the account again
#[component]
fn Users(users: Vec<User>, me: String, changed: EventHandler) -> Element {
    let said = use_signal(|| None::<(bool, String)>);
    rsx! {
        section { class: "reef-app-account-part",
            h2 { "Users" }
            p { class: "reef-app-muted",
                "Setting another user's password revokes their tokens, and removing a user revokes them too. Add a user with "
                code { "reef user add NAME" }
                "."
            }
            table { class: "reef-app-account-table",
                thead {
                    tr {
                        th { "Name" }
                        th { "Added" }
                        th {}
                    }
                }
                tbody {
                    for user in users {
                        UserRow {
                            key: "{user.name}",
                            own: user.name == me,
                            user,
                            said,
                            changed,
                        }
                    }
                }
            }
            Said { said: said() }
        }
    }
}

/// One user, to an admin: their name and whether they are an admin, and setting their password
/// or removing them, unless they are you
///
/// # Arguments
///
/// * `user` - The user
/// * `own` - Whether they are you
/// * `said` - Where to say what came of an action
/// * `changed` - Called once they are removed or their password set
#[component]
fn UserRow(
    user: User,
    own: bool,
    said: Signal<Option<(bool, String)>>,
    changed: EventHandler,
) -> Element {
    let mut confirming = use_signal(|| false);
    let mut setting = use_signal(|| false);
    let mut password = use_signal(String::new);
    let name = user.name.clone();
    let remove = {
        let name = name.clone();
        move |_| {
            let name = name.clone();
            confirming.set(false);
            spawn(async move {
                // what came of it, the list read again
                let outcome = match calls::remove_user(name.clone()).await {
                    Ok(UserRemoved::Removed { revoked }) => (
                        true,
                        format!("Removed {name}, revoking {revoked} token(s)."),
                    ),
                    Ok(UserRemoved::NoSuchUser) => (false, format!("{name} was removed already.")),
                    Ok(UserRemoved::LastAdmin) => (
                        false,
                        format!("{name} is the last admin, so stays until another is added."),
                    ),
                    Err(error) => (false, format!("Removing {name} failed: {error}")),
                };
                said.set(Some(outcome));
                changed.call(());
            });
        }
    };
    let set = {
        let name = name.clone();
        move |_| {
            let name = name.clone();
            let change = PasswordChange {
                current: None,
                password: password.peek().clone(),
            };
            spawn(async move {
                // what came of it, the field emptied once it is set
                let outcome = match calls::change_password(name.clone(), change).await {
                    Ok(changed) => {
                        password.set(String::new());
                        setting.set(false);
                        (
                            true,
                            format!(
                                "Set the password of {name}, revoking {} token(s).",
                                changed.revoked
                            ),
                        )
                    }
                    Err(error) => (
                        false,
                        format!("Setting the password of {name} failed: {error}"),
                    ),
                };
                said.set(Some(outcome));
                changed.call(());
            });
        }
    };
    rsx! {
        tr { "data-user": "{user.name}",
            td {
                strong { "{user.name}" }
                if user.admin { span { class: "reef-app-muted", " admin" } }
                if own { span { class: "reef-app-muted", " (you)" } }
            }
            td { "{utc_minute(user.created_at)}" }
            td {
                div { class: "reef-app-account-acts",
                    if !own {
                        if confirming() {
                            span { "Remove {name}? " }
                            button { class: "reef-button", "data-act": "confirm-remove", onclick: remove, "Yes, remove" }
                            button { class: "reef-button", "data-act": "keep", onclick: move |_| confirming.set(false), "Keep" }
                        } else if setting() {
                            input {
                                r#type: "password",
                                autocomplete: "new-password",
                                placeholder: "Their new password",
                                "aria-label": "New password for {name}",
                                value: "{password}",
                                oninput: move |event| password.set(event.value()),
                            }
                            button {
                                class: "reef-button",
                                "data-act": "confirm-password",
                                disabled: password.read().is_empty(),
                                onclick: set,
                                "Set"
                            }
                            button { class: "reef-button", "data-act": "keep", onclick: move |_| setting.set(false), "Cancel" }
                        } else {
                            button { class: "reef-button", "data-act": "set-password", onclick: move |_| setting.set(true), "Set password" }
                            button { class: "reef-button", "data-act": "remove", onclick: move |_| confirming.set(true), "Remove" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utc_minute;

    /// A time the service stamped reads as a date and minute in UTC
    #[test]
    fn times_read_as_utc_minutes() {
        // the epoch, a leap day, and a time this year, as `date -u` writes them
        assert_eq!(utc_minute(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_minute(951_782_400_000), "2000-02-29 00:00 UTC");
        assert_eq!(utc_minute(1_795_000_000_123), "2026-11-18 11:06 UTC");
        assert_eq!(utc_minute(1_798_761_599_000), "2026-12-31 23:59 UTC");
    }
}
