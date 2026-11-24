//! Users, the tokens they sign in with, and the requests that add them
//!
//! Sign-in is off unless reef's config turns it on. With it on, every route but the ones that
//! sign a user in asks who is calling: the CLI, the MCP server and the hooks send a token they
//! were given for a user's name and password, and a browser sends the cookie the sign-in page set.
//! A token is shown once, when it is issued, and the service keeps only its hash. A password is
//! kept only as its argon2id hash.

use serde::{Deserialize, Serialize};
use std::fmt;

/// What every token starts with, so one is known for what it is wherever it is pasted
pub const TOKEN_PREFIX: &str = "reef_";

/// How many hex digits follow a token's prefix: 32 random bytes
pub const TOKEN_HEX_LEN: usize = 64;

/// The cookie a browser that signed in sends its token in
pub const SIGN_IN_COOKIE: &str = "reef_sign_in";

/// The fewest characters a password may hold
pub const PASSWORD_MIN: usize = 8;

/// The most bytes a password may hold, so hashing one can't be made to take long
pub const PASSWORD_MAX: usize = 1024;

/// The most characters a user's name may hold
pub const USER_NAME_MAX: usize = 64;

/// A user of reef, as the service shows one
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// The user's name, which they sign in with
    pub name: String,
    /// Whether the user may add other users
    pub admin: bool,
    /// When the user was added, in milliseconds since the Unix epoch
    pub created_at: u64,
}

/// A request to add a user
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewUser {
    /// The new user's name
    pub name: String,
    /// The password they will sign in with
    pub password: String,
    /// Whether they may add other users; the first user is an admin whatever this says
    #[serde(default)]
    pub admin: bool,
}

impl fmt::Debug for NewUser {
    /// Shows the request with its password left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewUser")
            .field("name", &self.name)
            .field("password", &"…")
            .field("admin", &self.admin)
            .finish()
    }
}

/// What came of adding a user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserAdded {
    /// The user was added
    Added(User),
    /// A user of that name exists already
    NameTaken,
}

/// A request to sign in, trading a user's name and password for a token
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignIn {
    /// The user's name
    pub name: String,
    /// The user's password
    pub password: String,
    /// What the token is for, such as `reef on laptop`, so a list of tokens can tell them apart
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl fmt::Debug for SignIn {
    /// Shows the request with its password left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignIn")
            .field("name", &self.name)
            .field("password", &"…")
            .field("label", &self.label)
            .finish()
    }
}

/// What a token was issued to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TokenKind {
    /// A terminal client: the CLI, the MCP server and the hooks, until `reef logout`
    Cli,
    /// A browser, in the sign-in cookie, until it expires or the browser signs out
    Browser,
}

/// A token the service issued, shown this once
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedToken {
    /// The token, sent as `Authorization: Bearer` from now on
    pub token: String,
    /// The user it signs in
    pub user: User,
    /// What it was issued to
    pub kind: TokenKind,
    /// When it stops signing anyone in, in milliseconds since the Unix epoch, if it ever does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl fmt::Debug for IssuedToken {
    /// Shows the issued token with the token itself left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedToken")
            .field("token", &"…")
            .field("user", &self.user)
            .field("kind", &self.kind)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// What came of signing out
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedOut {
    /// Whether the token was one the service held, now revoked
    pub revoked: bool,
}

/// Who the service takes the caller for
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Me {
    /// The user the caller's token signs in, if it signs one in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    /// Whether the service asks every caller to sign in
    pub required: bool,
}

/// Checks a user's name keeps to the rule project and plan names keep to
///
/// # Arguments
///
/// * `name` - The name to check
///
/// # Examples
///
/// ```
/// use reef_api::user::check_user_name;
///
/// assert!(check_user_name("ada.l").is_ok());
/// assert!(check_user_name("-ada").is_err());
/// ```
pub fn check_user_name(name: &str) -> Result<(), String> {
    // a name starts with a letter or digit
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    // and holds only letters, digits, `.`, `_` and `-`
    let chars_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    match starts_well && chars_ok && name.len() <= USER_NAME_MAX {
        true => Ok(()),
        false => Err(format!(
            "user name `{name}` is not 1 to {USER_NAME_MAX} ASCII letters, digits, `.`, `_` or `-`, starting with a letter or digit"
        )),
    }
}

/// Checks a password is long enough to keep, and short enough to hash quickly
///
/// # Arguments
///
/// * `password` - The password to check
///
/// # Examples
///
/// ```
/// use reef_api::user::check_password;
///
/// assert!(check_password("correct horse").is_ok());
/// assert!(check_password("short").is_err());
/// ```
pub fn check_password(password: &str) -> Result<(), String> {
    // count characters, not bytes, for the floor a person reads
    if password.chars().count() < PASSWORD_MIN {
        return Err(format!(
            "a password holds at least {PASSWORD_MIN} characters"
        ));
    }
    // and bytes for the ceiling, which bounds the hash's work
    if password.len() > PASSWORD_MAX {
        return Err(format!("a password holds at most {PASSWORD_MAX} bytes"));
    }
    Ok(())
}

/// Whether text has a token's shape: the prefix, then 64 lowercase hex digits
///
/// A token of any other shape is never looked up, so a guess can't reach the store.
///
/// # Arguments
///
/// * `token` - The text to check
///
/// # Examples
///
/// ```
/// use reef_api::user::is_token_shaped;
///
/// assert!(is_token_shaped(&format!("reef_{}", "0a".repeat(32))));
/// assert!(!is_token_shaped("reef_nope"));
/// ```
pub fn is_token_shaped(token: &str) -> bool {
    // the prefix comes first
    let Some(hex) = token.strip_prefix(TOKEN_PREFIX) else {
        return false;
    };
    // then exactly the digits the random bytes make
    hex.len() == TOKEN_HEX_LEN && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::{
        IssuedToken, Me, NewUser, SignIn, TokenKind, User, UserAdded, check_password,
        check_user_name, is_token_shaped,
    };
    use serde_json::json;

    /// A user named in a request
    fn ada() -> User {
        User {
            name: "ada".into(),
            admin: true,
            created_at: 7,
        }
    }

    /// The requests and answers keep the shapes the routes speak
    #[test]
    fn users_and_tokens_keep_their_shapes() {
        // an added user is tagged as the other outcomes are
        assert_eq!(
            serde_json::to_value(UserAdded::Added(ada())).unwrap(),
            json!({"added": {"name": "ada", "admin": true, "created_at": 7}})
        );
        assert_eq!(
            serde_json::to_value(UserAdded::NameTaken).unwrap(),
            json!("name-taken")
        );
        // a new user is no admin unless it says so
        let new: NewUser = serde_json::from_value(json!({"name": "bob", "password": "p"})).unwrap();
        assert!(!new.admin);
        // a sign-in's label is optional both ways
        let sign_in: SignIn =
            serde_json::from_value(json!({"name": "a", "password": "p"})).unwrap();
        assert_eq!(sign_in.label, None);
        assert_eq!(
            serde_json::to_value(&sign_in).unwrap(),
            json!({"name": "a", "password": "p"})
        );
        // a CLI token never expires, so says nothing of it
        let issued = IssuedToken {
            token: "reef_x".into(),
            user: ada(),
            kind: TokenKind::Cli,
            expires_at: None,
        };
        assert_eq!(serde_json::to_value(&issued).unwrap()["kind"], json!("cli"));
        assert!(
            serde_json::to_value(&issued)
                .unwrap()
                .get("expires_at")
                .is_none()
        );
        // and nobody signed in reads as no user
        let me: Me = serde_json::from_value(json!({"required": true})).unwrap();
        assert_eq!(
            me,
            Me {
                user: None,
                required: true
            }
        );
    }

    /// Debug never shows a password or a token
    #[test]
    fn debug_leaves_secrets_out() {
        // a new user's password
        let new = NewUser {
            name: "bob".into(),
            password: "hunter22".into(),
            admin: false,
        };
        assert!(!format!("{new:?}").contains("hunter22"));
        // a sign-in's password
        let sign_in = SignIn {
            name: "bob".into(),
            password: "hunter22".into(),
            label: None,
        };
        assert!(!format!("{sign_in:?}").contains("hunter22"));
        // an issued token
        let issued = IssuedToken {
            token: "reef_secret".into(),
            user: ada(),
            kind: TokenKind::Browser,
            expires_at: Some(9),
        };
        assert!(!format!("{issued:?}").contains("reef_secret"));
    }

    /// Names keep the project rule, passwords their bounds, and tokens their shape
    #[test]
    fn names_passwords_and_tokens_are_checked() {
        // names
        assert!(check_user_name("ada").is_ok());
        assert!(check_user_name("a.b_c-9").is_ok());
        assert!(check_user_name("").is_err());
        assert!(check_user_name("a b").is_err());
        assert!(check_user_name(&"a".repeat(65)).is_err());
        // passwords
        assert!(check_password("12345678").is_ok());
        assert!(check_password("1234567").is_err());
        assert!(check_password(&"x".repeat(1025)).is_err());
        // tokens
        assert!(is_token_shaped(&format!("reef_{}", "f".repeat(64))));
        assert!(!is_token_shaped(&format!("reef_{}", "F".repeat(64))));
        assert!(!is_token_shaped(&format!("reef_{}", "f".repeat(63))));
        assert!(!is_token_shaped(&format!("beef_{}", "f".repeat(64))));
    }
}
