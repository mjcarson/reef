//! Users, the tokens they sign in with, and the requests that add them
//!
//! Sign-in is off unless reef's config turns it on. With it on, every route but the ones that
//! sign a user in asks who is calling: the CLI, the MCP server and the hooks send a token they
//! were given for a user's name and password, and a browser sends the cookie the sign-in page set.
//! A token is shown once, when it is issued, and the service keeps only its hash. A password is
//! kept only as its argon2id hash.
//!
//! Each user lists their own tokens, each by an id cut from its hash, and revokes any of them; an
//! admin lists and revokes anyone's. A user changes their own password with their current one,
//! and an admin sets anyone else's, which revokes that user's tokens. Only an admin removes a
//! user, and never the last admin.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::comment::Writer;

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

/// How many hex digits of a token's hash make its id: 64 bits, which no two tokens share
pub const TOKEN_ID_LEN: usize = 16;

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

/// A token as a listing shows it: never the token, only an id cut from its hash
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    /// The first [`TOKEN_ID_LEN`] hex digits of the token's SHA-256, which revoke it
    pub id: String,
    /// The name of the user it signs in
    pub user: String,
    /// What it was issued to
    pub kind: TokenKind,
    /// What it is for, as the caller that asked for it said
    pub label: String,
    /// When it was issued, in milliseconds since the Unix epoch
    pub created_at: u64,
    /// When it stops signing anyone in, in milliseconds since the Unix epoch, if it ever does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Whether it is the token the listing's request sent
    #[serde(default)]
    pub current: bool,
}

/// A request to change a user's password
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordChange {
    /// The user's current password, which their own change needs and an admin's for another
    /// user doesn't
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// The new password
    pub password: String,
}

impl fmt::Debug for PasswordChange {
    /// Shows the request with both passwords left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordChange")
            .field("current", &self.current.as_ref().map(|_| "…"))
            .field("password", &"…")
            .finish()
    }
}

/// What came of changing a password
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordChanged {
    /// How many of the user's tokens were revoked with it: none for their own change, all of
    /// them for an admin's
    pub revoked: usize,
}

/// What came of removing a user
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserRemoved {
    /// The user was removed, and every token of theirs revoked
    Removed {
        /// How many tokens were revoked
        revoked: usize,
    },
    /// No user has that name
    NoSuchUser,
    /// The user is reef's last admin, who stays until another is added
    LastAdmin,
}

/// The header a client sends to say it writes as Claude, for the user its token signs in
///
/// reef's MCP server sends `claude` in it on every request. Anything else, or none, is the user's
/// own. It can only add "Claude, for" to the name the token signs in, never name another user.
pub const WRITER_HEADER: &str = "reef-writer";

/// The author of a write sent with no token, as every write kept before reef knew its users was:
/// nobody
///
/// It lives as long as the program, so a write can name it however long it waits.
pub static NOBODY: Author = Author {
    user: None,
    since: None,
    by: Writer::User,
};

/// Who made a write: the user its token or cookie signed in, and whether Claude made it for them
///
/// The service names the user from the request's token or cookie alone, never from its body, so
/// nobody writes as another. A write sent with no token names nobody, as every write kept before
/// reef knew its users does. Claude writes with the user's own token, so [`WRITER_HEADER`] says a
/// write is Claude's, for that user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    /// The name of the user signed in, if one was
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// When that user was added, in milliseconds since the Unix epoch, which tells a name reused
    /// after its user was removed from the one it named before
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// Whether Claude made it through reef's tools, the user unless it says so
    #[serde(default, skip_serializing_if = "Writer::is_user")]
    pub by: Writer,
}

impl Author {
    /// The author of a write a user's token or cookie signed in, made by them or by Claude
    ///
    /// # Arguments
    ///
    /// * `user` - The user signed in, if one was
    /// * `by` - Whether the user or Claude made it
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{Author, User, Writer};
    ///
    /// let ada = User { name: "ada".to_string(), admin: false, created_at: 7 };
    /// let author = Author::signed(Some(&ada), Writer::Claude);
    /// assert_eq!(author.describe().as_deref(), Some("Claude, for ada"));
    /// assert_eq!(author.since, Some(7));
    /// ```
    pub fn signed(user: Option<&User>, by: Writer) -> Author {
        // the name and when its user was added both come from the user signed in
        Author {
            user: user.map(|user| user.name.clone()),
            since: user.map(|user| user.created_at),
            by,
        }
    }

    /// Whether it names nobody, as a write sent with no token, or kept before authors were,
    /// does
    pub fn is_unknown(&self) -> bool {
        self.user.is_none() && self.by.is_user()
    }

    /// The same author, Claude's if either it or a request's own word says so
    ///
    /// A comment's body has said whether Claude wrote it since before the header did, so an older
    /// client's word is still taken.
    ///
    /// # Arguments
    ///
    /// * `by` - Who the request's body says wrote it
    pub fn claiming(&self, by: Writer) -> Author {
        // Claude's word from either side makes it Claude's, for the same user
        let by = match (self.by, by) {
            (Writer::User, Writer::User) => Writer::User,
            _ => Writer::Claude,
        };
        Author { by, ..self.clone() }
    }

    /// How a reader is told who made it: "ada", "Claude, for ada", "Claude", or `None` for
    /// nobody
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::{Author, Writer};
    ///
    /// let ada = Author { user: Some("ada".to_string()), ..Author::default() };
    /// assert_eq!(ada.describe().as_deref(), Some("ada"));
    /// let claude = Author { by: Writer::Claude, ..Author::default() };
    /// assert_eq!(claude.describe().as_deref(), Some("Claude"));
    /// assert_eq!(Author::default().describe(), None);
    /// ```
    pub fn describe(&self) -> Option<String> {
        // the user's name, said to be Claude's for them where Claude made it
        match (&self.user, self.by) {
            (Some(name), Writer::User) => Some(name.clone()),
            (Some(name), Writer::Claude) => Some(format!("Claude, for {name}")),
            (None, Writer::Claude) => Some("Claude".to_string()),
            (None, Writer::User) => None,
        }
    }
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

/// Whether text has a token id's shape: [`TOKEN_ID_LEN`] lowercase hex digits
///
/// # Arguments
///
/// * `id` - The text to check
///
/// # Examples
///
/// ```
/// use reef_api::user::is_token_id;
///
/// assert!(is_token_id("0123456789abcdef"));
/// assert!(!is_token_id("current"));
/// ```
pub fn is_token_id(id: &str) -> bool {
    id.len() == TOKEN_ID_LEN && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
        Author, IssuedToken, Me, NewUser, PasswordChange, SignIn, TokenInfo, TokenKind, User,
        UserAdded, UserRemoved, check_password, check_user_name, is_token_id, is_token_shaped,
    };
    use crate::comment::Writer;
    use serde_json::json;

    /// An author names the user signed in and when they were added, says whether Claude wrote
    /// for them, and leaves out whatever it doesn't know
    #[test]
    fn authors_say_only_what_they_know() {
        let ada = User {
            name: "ada".to_string(),
            admin: false,
            created_at: 7,
        };
        // a user's own write names them and when they were added
        let author = Author::signed(Some(&ada), Writer::User);
        assert_eq!(
            serde_json::to_value(&author).unwrap(),
            json!({"user": "ada", "since": 7})
        );
        assert!(!author.is_unknown());
        assert_eq!(author.describe().as_deref(), Some("ada"));
        // nobody signed in is nobody, and reads back from nothing at all
        assert_eq!(serde_json::to_value(Author::default()).unwrap(), json!({}));
        assert!(Author::default().is_unknown());
        assert_eq!(
            serde_json::from_value::<Author>(json!({})).unwrap(),
            Author::default()
        );
        // Claude's word from either side makes it Claude's, for the same user
        let claimed = author.claiming(Writer::Claude);
        assert_eq!(claimed.describe().as_deref(), Some("Claude, for ada"));
        assert_eq!(
            serde_json::to_value(&claimed).unwrap(),
            json!({"user": "ada", "since": 7, "by": "claude"})
        );
        let header = Author::signed(None, Writer::Claude);
        assert_eq!(header.claiming(Writer::User).by, Writer::Claude);
        assert!(!header.is_unknown());
    }

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
        // a token's listing never says the token, and a CLI token's no expiry
        let info = TokenInfo {
            id: "0123456789abcdef".into(),
            user: "ada".into(),
            kind: TokenKind::Cli,
            label: "reef on laptop".into(),
            created_at: 7,
            expires_at: None,
            current: true,
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({"id": "0123456789abcdef", "user": "ada", "kind": "cli",
                   "label": "reef on laptop", "created_at": 7, "current": true})
        );
        // an admin's change names no current password, and says nothing of one
        let change: PasswordChange = serde_json::from_value(json!({"password": "p"})).unwrap();
        assert_eq!(change.current, None);
        assert!(
            serde_json::to_value(&change)
                .unwrap()
                .get("current")
                .is_none()
        );
        // a removal is tagged as the other outcomes are
        assert_eq!(
            serde_json::to_value(UserRemoved::Removed { revoked: 2 }).unwrap(),
            json!({"removed": {"revoked": 2}})
        );
        assert_eq!(
            serde_json::to_value(UserRemoved::LastAdmin).unwrap(),
            json!("last-admin")
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
        // a password change's passwords, both of them
        let change = PasswordChange {
            current: Some("hunter22".into()),
            password: "hunter33".into(),
        };
        let shown = format!("{change:?}");
        assert!(
            !shown.contains("hunter22") && !shown.contains("hunter33"),
            "{shown}"
        );
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
        // token ids
        assert!(is_token_id("0123456789abcdef"));
        assert!(!is_token_id("0123456789ABCDEF"));
        assert!(!is_token_id("0123456789abcde"));
        assert!(!is_token_id("current"));
    }
}
