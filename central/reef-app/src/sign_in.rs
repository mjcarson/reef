//! Who the app is signed in as, and signing out, in the bar of every page
//!
//! The sign-in page itself is reef-service's, plain HTML with no script, so a browser that isn't
//! signed in never loads the app. Once it is, the bar says whom the server takes it for, linking
//! to their account page, and offers to sign out, when the server asks callers to sign in at all. A browser whose sign-in lapses,
//! its token revoked or expired, finds the change stream refused and its calls answered 401, and
//! is offered the sign-in page again, which brings it back where it was.

use dioxus::prelude::*;
use reef_api::{Me, paths};

use crate::app::Route;
use crate::calls;

/// Whether the change stream was refused for good, as it is once a browser's sign-in lapses
pub static SIGN_IN_LOST: GlobalSignal<bool> = Signal::global(|| false);

/// The sign-in page's address, coming back to a route once signed in
///
/// # Arguments
///
/// * `route` - Where to come back to
pub fn sign_in_href(route: &Route) -> String {
    format!(
        "{}?next={}",
        paths::SIGN_IN,
        paths::encode(&route.to_string())
    )
}

/// Whom the server takes this browser for, a sign-out beside it, or a way back to signing in
///
/// It shows nothing when the server asks no caller to sign in.
#[component]
pub fn SignedInAs() -> Element {
    // read as the page renders, so the bar says it from the first paint
    let me = use_server_future(calls::me)?;
    let route = use_route::<Route>();
    let lost = SIGN_IN_LOST();
    let me: Option<Me> = me.read().as_ref().and_then(|me| me.as_ref().ok().cloned());
    match me {
        // signed in, and asked to be
        Some(Me {
            user: Some(user),
            required: true,
        }) => rsx! {
            span { class: "reef-app-user",
                if lost {
                    a { class: "reef-app-error", href: sign_in_href(&route),
                        "Live updates stopped: sign in again"
                    }
                }
                span {
                    "signed in as "
                    Link { to: Route::Account {}, title: "Your password and tokens",
                        strong { class: "reef-app-user-name", "{user.name}" }
                    }
                }
                form { method: "post", action: paths::SIGN_OUT,
                    button { class: "reef-button", r#type: "submit", "Sign out" }
                }
            }
        },
        // asked to sign in, and not
        Some(Me {
            user: None,
            required: true,
        }) => rsx! {
            span { class: "reef-app-user",
                a { href: sign_in_href(&route), "Sign in" }
            }
        },
        // nobody need sign in, or it can't be told
        _ => rsx! {},
    }
}

/// What a call that failed says, with a way back to signing in if it was refused for want of it
///
/// # Arguments
///
/// * `error` - What the call failed with
/// * `doing` - What it was doing, such as `Reading the projects`
#[component]
pub fn CallFailed(error: ServerFnError, doing: String) -> Element {
    let route = use_route::<Route>();
    rsx! {
        span { class: "reef-app-error",
            if calls::signed_out(&error) {
                "{doing} needs you signed in: "
                a { href: sign_in_href(&route), "sign in again" }
            } else {
                "{doing} failed: {error}"
            }
        }
    }
}
