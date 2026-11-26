//! The service's users: adding them, signing them in for a token, and knowing a token again
//!
//! - **Sign-in is off unless reef's config requires it.** Off, every caller is answered as
//!   today, and only adding or listing users asks who is calling.
//! - **The first user is added from the service's own host,** and is an admin whatever the
//!   request says. After that only an admin adds or lists users, judged against the users as
//!   they stand under the store's lock, so the first can't be added twice.
//! - **A password is kept only as its argon2id hash,** hashed and checked off the async threads,
//!   two at a time, since each takes 19 MiB and a core.
//! - **A wrong name and a wrong password answer alike.** A name no user has is checked against a
//!   hash made once for the purpose, so it takes as long, and both are told the same.
//! - **A token is 32 random bytes,** shown once and kept only as its SHA-256. A browser's
//!   expires after 30 days, and a terminal's at its logout. Text that isn't shaped as a token is
//!   never looked up.
//! - **Tokens looked up are trusted for a minute,** so a request doesn't cost a read of shoal.
//!   reef-service is shoal's only writer, so a revoke here drops its token at once. A token
//!   issued before its user was added signs nobody in, since it was another user's of that name.
//! - **Repeated failures are slowed** ([`throttle`](super::throttle)), by the name tried and the
//!   address tried from, before any hash is checked.
//! - **Everyone lists and revokes their own tokens,** each named by an id cut from its hash, and
//!   an admin anyone's. A token that isn't the caller's to see is answered as no token at all.
//! - **A password is changed by its user, with their current one,** which leaves their tokens,
//!   or set by an admin for another user, which revokes theirs. Only an admin removes a user,
//!   their tokens with them, and never the last admin.
//! - **A lost admin is recovered from the store itself** ([`Service::recover_admin`]), by
//!   whoever can open it, as `reef-serve recover` does, never over the API.
//! - **Every act on a user names who made it** (C89), in a log only admins read: adding and
//!   removing users, setting a password, revoking a token and a recovery, which names nobody.
//!   Signing in and out, and an expired token let go, keep nothing. A user also says who added
//!   them, which a password set later leaves as it was.
//! - **A revoke is told to the change streams.** Each revoke, removal or reset bumps a counter
//!   [`Service::signed_out`] watches, so a stream whose token stops signing anyone in ends.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use reef_api::user::{
    PASSWORD_MAX, TOKEN_ID_LEN, check_password, check_user_name, is_token_id, is_token_shaped,
};
use reef_api::{
    Author, HistoryPage, HistoryQuery, IssuedToken, NOBODY, NewUser, PasswordChange,
    PasswordChanged, TokenInfo, TokenKind, User, UserAct, UserAdded, UserRemoved, Writer,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{StoredToken, StoredUser, Unkeepable, UserDropped, UserKept};

/// How long a browser's token signs it in: 30 days
pub const BROWSER_TOKEN_LIFE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// How long a token looked up is trusted before it is looked up again
const TOKEN_TRUST: Duration = Duration::from_secs(60);

/// How many tokens the lookups remember before forgetting those no longer trusted
const TOKENS_REMEMBERED: usize = 1024;

/// How many passwords are hashed or checked at once
pub(super) const HASHING_AT_ONCE: usize = 2;

/// A caller that must sign in to do what it asked
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSignedIn(pub String);

impl std::fmt::Display for NotSignedIn {
    /// Says what the caller must sign in to do
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotSignedIn {}

/// A caller that may not do what it asked, signed in or not
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAllowed(pub String);

impl std::fmt::Display for NotAllowed {
    /// Says what the caller may not do
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotAllowed {}

/// A user that isn't there, named
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoUser(pub String);

impl std::fmt::Display for NoUser {
    /// Says no user has the name
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no user is named `{}`", self.0)
    }
}

impl std::error::Error for NoUser {}

/// Who asks of reef's users or tokens: the user their token signs in, the token itself, and
/// where they call from
#[derive(Clone, Copy, Default)]
pub struct Asker<'a> {
    /// The user the caller's token signs in, if it signs one in
    pub user: Option<&'a User>,
    /// Who wrote for that user: the user, or Claude with their token
    pub writer: Writer,
    /// Whether the caller reached the service from its own host
    pub local: bool,
    /// The token the caller sent, if it sent one
    pub token: Option<&'a str>,
    /// The address the caller called from, if it is known
    pub address: Option<IpAddr>,
}

impl Asker<'_> {
    /// Who the asker is as an author: the user their token signs in, and whether Claude wrote
    /// for them, or nobody known
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_service::Asker;
    ///
    /// assert!(Asker::default().author().is_unknown());
    /// ```
    pub fn author(&self) -> Author {
        Author::signed(self.user, self.writer)
    }
}

impl std::fmt::Debug for Asker<'_> {
    /// Shows the asker with its token left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asker")
            .field("user", &self.user)
            .field("writer", &self.writer)
            .field("local", &self.local)
            .field("token", &self.token.map(|_| "…"))
            .field("address", &self.address)
            .finish()
    }
}

/// What came of recovering a lost admin
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovered {
    /// The user's password was set, and they are an admin, every token of theirs revoked
    Reset {
        /// How many tokens were revoked
        revoked: usize,
        /// Whether they were an admin already
        was_admin: bool,
    },
    /// No user had the name, so one was added as an admin
    Added,
}

/// A token looked up lately, and when
#[derive(Debug, Clone)]
struct Seen {
    /// The user it signs in
    user: User,
    /// When it stops signing anyone in, if it ever does
    expires_at: Option<u64>,
    /// When it was looked up
    read_at: Instant,
}

/// The tokens looked up lately, by hash
#[derive(Debug, Default)]
pub(super) struct Tokens {
    /// Each token's hash and what its lookup found
    seen: Mutex<HashMap<String, Seen>>,
}

impl Tokens {
    /// The user a token looked up lately signs in, if it is still trusted
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's hash
    /// * `now` - The time, in milliseconds since the Unix epoch
    fn trusted(&self, hash: &str, now: u64) -> Option<User> {
        // a lock poisoned by a panic still holds a map, which is all this reads
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let found = seen.get(hash)?;
        // looked up too long ago, or expired since, it is looked up again
        let fresh = found.read_at.elapsed() < TOKEN_TRUST;
        let live = found.expires_at.is_none_or(|expires_at| now < expires_at);
        (fresh && live).then(|| found.user.clone())
    }

    /// Remembers what a token's lookup found
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's hash
    /// * `user` - The user it signs in
    /// * `expires_at` - When it stops signing anyone in, if it ever does
    fn remember(&self, hash: &str, user: &User, expires_at: Option<u64>) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        // forget those no longer trusted before the map grows past its bound
        if seen.len() >= TOKENS_REMEMBERED {
            seen.retain(|_, found| found.read_at.elapsed() < TOKEN_TRUST);
        }
        seen.insert(
            hash.to_string(),
            Seen {
                user: user.clone(),
                expires_at,
                read_at: Instant::now(),
            },
        );
    }

    /// Forgets a token, so its next lookup reads the store
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's hash
    fn forget(&self, hash: &str) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(hash);
    }

    /// Forgets every token of a user's, so their next lookups read the store
    ///
    /// # Arguments
    ///
    /// * `user` - The user's name
    fn forget_user(&self, user: &str) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, found| found.user.name != user);
    }
}

/// Makes a new token: the prefix, then 32 random bytes as lowercase hex
fn new_token() -> anyhow::Result<String> {
    // the operating system's randomness, which never runs short once seeded
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("no randomness for a token: {error}"))?;
    Ok(format!("{}{}", reef_api::user::TOKEN_PREFIX, hex(&bytes)))
}

/// A token's SHA-256, as 64 lowercase hex digits, the key it is kept under
///
/// # Arguments
///
/// * `token` - The token
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Bytes as lowercase hex
///
/// # Arguments
///
/// * `bytes` - The bytes
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A token's id: the first [`TOKEN_ID_LEN`] hex digits of its hash
///
/// # Arguments
///
/// * `hash` - The token's SHA-256, as 64 lowercase hex digits
fn token_id(hash: &str) -> String {
    hash.chars().take(TOKEN_ID_LEN).collect()
}

/// A token as a listing shows it
///
/// # Arguments
///
/// * `stored` - The token, as the store keeps it
/// * `sent` - The hash of the token the listing's request sent, if it sent one
fn token_info(stored: StoredToken, sent: Option<&str>) -> TokenInfo {
    TokenInfo {
        id: token_id(&stored.hash),
        current: sent == Some(stored.hash.as_str()),
        user: stored.user,
        kind: stored.kind,
        label: stored.label,
        created_at: stored.created_at,
        expires_at: stored.expires_at,
    }
}

/// The user an asker's token signs in, or a refusal saying to sign in first
///
/// # Arguments
///
/// * `asker` - Who asks
/// * `doing` - What they ask to do, for the refusal
fn signed_in_user<'a>(asker: Asker<'a>, doing: &str) -> anyhow::Result<&'a User> {
    asker
        .user
        .ok_or_else(|| NotSignedIn(format!("sign in to reef to {doing}")).into())
}

/// The milliseconds since the Unix epoch
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// A hash no password matches, checked in place of a user's when no user has the name, so a
/// name no one has takes as long to refuse as a wrong password
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        // a password nobody is told, hashed with the same parameters as every user's
        Argon2::default()
            .hash_password(b"reef keeps no password like this one")
            .map(|hash| hash.to_string())
            .unwrap_or_default()
    })
}

/// Judges an add of a user against every user, returning whether the new user is an admin
///
/// # Arguments
///
/// * `users` - Every user, as they stand
/// * `asker` - Who asks
/// * `admin` - Whether the request asks for an admin
fn judge_add(users: &[StoredUser], asker: Asker<'_>, admin: bool) -> anyhow::Result<bool> {
    // the first user comes from the service's own host, and is an admin
    if users.is_empty() {
        return match asker.local {
            true => Ok(true),
            false => Err(NotAllowed(
                "the first user of reef is added from the service's own host".to_string(),
            )
            .into()),
        };
    }
    // after that, only a user whose stored row says they are an admin
    is_admin(users, asker, "add a user")?;
    Ok(admin)
}

/// Checks the asker is a user the stored rows say is an admin
///
/// # Arguments
///
/// * `users` - Every user, as they stand
/// * `asker` - Who asks
/// * `doing` - What they ask to do, for the refusal
fn is_admin(users: &[StoredUser], asker: Asker<'_>, doing: &str) -> anyhow::Result<()> {
    // nobody signed in must sign in first
    let Some(user) = asker.user else {
        return Err(NotSignedIn(format!("sign in as an admin of reef to {doing}")).into());
    };
    // the rows as they stand decide, not what the token's lookup remembered
    let stored = users.iter().find(|stored| stored.user.name == user.name);
    match stored.is_some_and(|stored| stored.user.admin) {
        true => Ok(()),
        false => Err(NotAllowed(format!("only an admin of reef may {doing}")).into()),
    }
}

impl<B: Backend> Service<B> {
    /// Asks every caller to sign in, or none
    ///
    /// # Arguments
    ///
    /// * `required` - Whether every caller must sign in
    pub fn requiring_sign_in(mut self, required: bool) -> Self {
        self.requires_sign_in = required;
        self
    }

    /// Whether every caller must sign in
    pub fn sign_in_required(&self) -> bool {
        self.requires_sign_in
    }

    /// Checks the asker may register what is on the service's host: a directory, or a path a
    /// remote names
    ///
    /// With sign-in off anyone may, as anyone may do anything, and the service serves this host
    /// alone; on, only an admin, since the service reads or clones what they name.
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `doing` - What they ask to do, for the refusal
    #[instrument(name = "Service::judge_on_host", skip_all, err(Debug))]
    pub async fn judge_on_host(&self, asker: Asker<'_>, doing: &str) -> anyhow::Result<()> {
        // anyone, with sign-in off
        if !self.requires_sign_in {
            return Ok(());
        }
        // else an admin, as the stored rows say
        is_admin(&self.store.users().await?, asker, doing)
    }

    /// What a server says of signing in as it starts: that callers must, and how to add the
    /// first user while there is none, or nothing with sign-in off
    #[instrument(name = "Service::sign_in_note", skip_all, err(Debug))]
    pub async fn sign_in_note(&self) -> anyhow::Result<Option<String>> {
        // nothing to say with sign-in off
        if !self.requires_sign_in {
            return Ok(None);
        }
        // and how to add the first user while there is none
        let note = match self.store.users().await?.is_empty() {
            true => "callers must sign in, and reef has no user yet: add the first with \
                     `reef user add NAME` from this host"
                .to_string(),
            false => "callers must sign in".to_string(),
        };
        Ok(Some(note))
    }

    /// Reads a page of the log of acts on reef's users, newest first, to an admin, or nothing
    /// while there are no users
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `query` - Which page to read
    #[instrument(name = "Service::user_history", skip_all, err(Debug))]
    pub async fn user_history(
        &self,
        asker: Asker<'_>,
        query: &HistoryQuery,
    ) -> anyhow::Result<HistoryPage> {
        // with no users yet, there is nobody to hide it from, and nothing to show
        let users = self.store.users().await?;
        if users.is_empty() {
            return Ok(HistoryPage::default());
        }
        is_admin(&users, asker, "read the users' log")?;
        self.store.user_history(query).await
    }

    /// Lists every user, by name, to an admin, or nobody while there are none
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    #[instrument(name = "Service::users", skip_all, err(Debug))]
    pub async fn users(&self, asker: Asker<'_>) -> anyhow::Result<Vec<User>> {
        // with no users yet, there is nobody to hide
        let users = self.store.users().await?;
        if users.is_empty() {
            return Ok(Vec::new());
        }
        is_admin(&users, asker, "list its users")?;
        Ok(users.into_iter().map(|stored| stored.user).collect())
    }

    /// Adds a user: the first from the service's own host, as an admin, and the rest by an admin
    ///
    /// # Arguments
    ///
    /// * `new` - The new user, with their password
    /// * `asker` - Who asks
    #[instrument(name = "Service::add_user", skip_all, err(Debug))]
    pub async fn add_user(&self, new: NewUser, asker: Asker<'_>) -> anyhow::Result<UserAdded> {
        // a name and a password the service can keep
        check_user_name(&new.name).map_err(Unkeepable)?;
        check_password(&new.password).map_err(Unkeepable)?;
        // judged once before the slow hash, so a refusal costs no hashing
        judge_add(&self.store.users().await?, asker, new.admin)?;
        let password = self.hash_password(new.password).await?;
        // and again under the lock, against the users as they stand then
        let author = asker.author();
        let kept = self
            .store
            .add_user(&new.name, &password, &author, |users| {
                judge_add(users, asker, new.admin)
            })
            .await?;
        Ok(match kept {
            UserKept::Added(user) => {
                // kept in the users' log once they are
                let added = UserAct::Added { admin: user.admin };
                self.store.keep_user_act(&author, &user.name, added).await;
                UserAdded::Added(user)
            }
            UserKept::NameTaken => UserAdded::NameTaken,
        })
    }

    /// Signs a user in by name and password, issuing a token, or `None` if no user has that
    /// name and password
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    /// * `password` - The user's password
    /// * `kind` - What the token is issued to
    /// * `label` - What the token is for
    /// * `from` - The address the sign-in came from, if it is known
    #[instrument(name = "Service::sign_in", skip_all, err(Debug))]
    pub async fn sign_in(
        &self,
        name: &str,
        password: &str,
        kind: TokenKind,
        label: &str,
        from: Option<IpAddr>,
    ) -> anyhow::Result<Option<IssuedToken>> {
        // one that must wait is refused before any hashing, and the rest counted until they sign in
        self.throttle.attempt(name, from, Instant::now())?;
        // a password too long to hash quickly matches nobody's
        if password.len() > PASSWORD_MAX {
            return Ok(None);
        }
        // the user's hash, else one no password matches, so both take as long
        let stored = self.store.user(name).await?;
        let hash = stored
            .as_ref()
            .map(|stored| stored.password.clone())
            .unwrap_or_default();
        let matched = self.verify_password(password.to_string(), hash).await?;
        let Some(stored) = stored.filter(|_| matched) else {
            return Ok(None);
        };
        self.throttle.succeeded(name, from);
        // a token for them, kept only by its hash
        let token = new_token()?;
        let expires_at = match kind {
            TokenKind::Browser => Some(now_ms() + BROWSER_TOKEN_LIFE.as_millis() as u64),
            TokenKind::Cli => None,
        };
        self.store
            .keep_token(&token_hash(&token), name, kind, label, expires_at)
            .await?;
        Ok(Some(IssuedToken {
            token,
            user: stored.user,
            kind,
            expires_at,
        }))
    }

    /// The user a token signs in, if it signs one in
    ///
    /// # Arguments
    ///
    /// * `token` - The token a request sent
    #[instrument(name = "Service::signed_in", skip_all, err(Debug))]
    pub async fn signed_in(&self, token: &str) -> anyhow::Result<Option<User>> {
        // text not shaped as a token is never looked up
        if !is_token_shaped(token) {
            return Ok(None);
        }
        // one looked up lately is trusted
        let hash = token_hash(token);
        let now = now_ms();
        if let Some(user) = self.tokens.trusted(&hash, now) {
            return Ok(Some(user));
        }
        let Some(stored) = self.store.token(&hash).await? else {
            return Ok(None);
        };
        // an expired token is dropped, and signs nobody in
        if stored.expired(now) {
            self.store.drop_token(&stored).await?;
            return Ok(None);
        }
        // nor does one whose user is gone
        let Some(user) = self.store.user(&stored.user).await? else {
            return Ok(None);
        };
        // nor one issued before its user was added, which was an earlier user's of the name
        if stored.created_at < user.user.created_at {
            self.store.drop_token(&stored).await?;
            return Ok(None);
        }
        self.tokens.remember(&hash, &user.user, stored.expires_at);
        Ok(Some(user.user))
    }

    /// Revokes a token, returning whether it was one the service held
    ///
    /// # Arguments
    ///
    /// * `token` - The token to revoke
    #[instrument(name = "Service::sign_out", skip_all, err(Debug))]
    pub async fn sign_out(&self, token: &str) -> anyhow::Result<bool> {
        // text not shaped as a token was never held
        if !is_token_shaped(token) {
            return Ok(false);
        }
        // forgotten here first, so no lookup trusts it while it is dropped
        let hash = token_hash(token);
        self.tokens.forget(&hash);
        let Some(stored) = self.store.token(&hash).await? else {
            return Ok(false);
        };
        let dropped = self.store.drop_token(&stored).await?;
        self.revoked();
        Ok(dropped)
    }

    /// Lists the asker's tokens, or another user's to an admin, the one the asker sent marked,
    /// oldest first
    ///
    /// An expired token is dropped as it is found, and left out.
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `of` - The user whose tokens to list, if not the asker's own
    #[instrument(name = "Service::tokens", skip_all, err(Debug))]
    pub async fn tokens(
        &self,
        asker: Asker<'_>,
        of: Option<&str>,
    ) -> anyhow::Result<Vec<TokenInfo>> {
        // the asker's own, or another's if the stored rows say the asker is an admin
        let me = signed_in_user(asker, "list tokens")?;
        let whose = of.unwrap_or(&me.name);
        if whose != me.name {
            is_admin(
                &self.store.users().await?,
                asker,
                "list another user's tokens",
            )?;
        }
        // the token sent, by its hash, to mark it
        let sent = asker
            .token
            .filter(|token| is_token_shaped(token))
            .map(token_hash);
        let now = now_ms();
        let mut listed = Vec::new();
        for stored in self.store.tokens(Some(whose)).await? {
            // an expired one signs nobody in, so goes
            if stored.expired(now) {
                self.store.drop_token(&stored).await?;
                continue;
            }
            listed.push(token_info(stored, sent.as_deref()));
        }
        listed.sort_by_key(|token| token.created_at);
        Ok(listed)
    }

    /// Revokes a token by its id: one of the asker's own, or anyone's by an admin, returning
    /// whether there was one to revoke
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `id` - The token's id, as a listing shows it
    #[instrument(name = "Service::revoke_token", skip_all, err(Debug))]
    pub async fn revoke_token(&self, asker: Asker<'_>, id: &str) -> anyhow::Result<bool> {
        let me = signed_in_user(asker, "revoke a token")?;
        // text not shaped as an id names no token
        if !is_token_id(id) {
            return Ok(false);
        }
        // the tokens it names that the asker may see: their own, or any to an admin
        let named: Vec<StoredToken> = self
            .store
            .tokens(None)
            .await?
            .into_iter()
            .filter(|token| token.hash.starts_with(id))
            .collect();
        let admin = match named.iter().any(|token| token.user != me.name) {
            true => is_admin(&self.store.users().await?, asker, "").is_ok(),
            false => false,
        };
        let mut visible: Vec<StoredToken> = named
            .into_iter()
            .filter(|token| admin || token.user == me.name)
            .collect();
        // 64 bits of a hash name one token; if they ever named two, neither is guessed at
        anyhow::ensure!(
            visible.len() <= 1,
            "token id `{id}` names {} tokens, so none was revoked",
            visible.len()
        );
        let Some(token) = visible.pop() else {
            return Ok(false);
        };
        // forgotten here first, so no lookup trusts it while it is dropped
        self.tokens.forget(&token.hash);
        let dropped = self.store.drop_token(&token).await?;
        self.tokens.forget(&token.hash);
        self.revoked();
        // kept in the users' log, if there was one to drop
        if dropped {
            let revoked = UserAct::TokenRevoked {
                id: token_id(&token.hash),
                kind: token.kind,
                label: token.label.clone(),
            };
            self.store
                .keep_user_act(&asker.author(), &token.user, revoked)
                .await;
        }
        Ok(dropped)
    }

    /// Changes a user's password: the asker's own with their current one, which leaves their
    /// tokens, or another user's by an admin, which revokes every token of theirs
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `name` - The user whose password it is
    /// * `change` - The current password, for the asker's own, and the new one
    #[instrument(name = "Service::change_password", skip_all, err(Debug))]
    pub async fn change_password(
        &self,
        asker: Asker<'_>,
        name: &str,
        change: PasswordChange,
    ) -> anyhow::Result<PasswordChanged> {
        let me = signed_in_user(asker, "change a password")?;
        // a new password the service can keep
        check_password(&change.password).map_err(Unkeepable)?;
        match me.name == name {
            true => self.change_own_password(asker, name, change).await,
            false => self.set_password(asker, name, change.password).await,
        }
    }

    /// Changes the asker's own password, checked against their current one as a sign-in is
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `name` - The asker's name
    /// * `change` - The current password and the new one
    async fn change_own_password(
        &self,
        asker: Asker<'_>,
        name: &str,
        change: PasswordChange,
    ) -> anyhow::Result<PasswordChanged> {
        // one's own needs the current password, slowed as a sign-in is
        let Some(current) = change.current else {
            return Err(Unkeepable(
                "your own password changes with your current one: send it as `current`".into(),
            )
            .into());
        };
        self.throttle.attempt(name, asker.address, Instant::now())?;
        let Some(stored) = self.store.user(name).await? else {
            return Err(NoUser(name.to_string()).into());
        };
        let matched = self
            .verify_password(current, stored.password.clone())
            .await?;
        if !matched {
            return Err(NotAllowed(
                "your current password doesn't match, so the password is as it was".to_string(),
            )
            .into());
        }
        self.throttle.succeeded(name, asker.address);
        // over the row read, so a change made since isn't written over
        let password = self.hash_password(change.password).await?;
        let written = self
            .store
            .rewrite_user(&stored, &password, stored.user.admin)
            .await?;
        anyhow::ensure!(
            written,
            "the password of `{name}` changed while this change was checked, so it wasn't \
             written: try again"
        );
        // kept in the users' log
        let changed = UserAct::Password {
            own: true,
            revoked: 0,
        };
        self.store
            .keep_user_act(&asker.author(), name, changed)
            .await;
        Ok(PasswordChanged { revoked: 0 })
    }

    /// Sets another user's password, as an admin, revoking every token of theirs
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `name` - The user whose password it is
    /// * `password` - The new password
    async fn set_password(
        &self,
        asker: Asker<'_>,
        name: &str,
        password: String,
    ) -> anyhow::Result<PasswordChanged> {
        // an admin, as the stored rows say, before a word of who exists
        let users = self.store.users().await?;
        is_admin(&users, asker, "set another user's password")?;
        let Some(stored) = users.into_iter().find(|stored| stored.user.name == name) else {
            return Err(NoUser(name.to_string()).into());
        };
        let password = self.hash_password(password).await?;
        let written = self
            .store
            .rewrite_user(&stored, &password, stored.user.admin)
            .await?;
        anyhow::ensure!(
            written,
            "`{name}` changed while their password was set, so it wasn't: try again"
        );
        // set by someone else, so whoever held the old one signs in no more
        let revoked = self.revoke_all(name).await?;
        // kept in the users' log
        let set = UserAct::Password {
            own: false,
            revoked,
        };
        self.store.keep_user_act(&asker.author(), name, set).await;
        Ok(PasswordChanged { revoked })
    }

    /// Removes a user and every token of theirs, which only an admin may, and never the last
    /// admin
    ///
    /// # Arguments
    ///
    /// * `asker` - Who asks
    /// * `name` - The user's name
    #[instrument(name = "Service::remove_user", skip_all, err(Debug))]
    pub async fn remove_user(&self, asker: Asker<'_>, name: &str) -> anyhow::Result<UserRemoved> {
        // judged against every user under the store's lock
        let dropped = self
            .store
            .remove_user(name, |users| is_admin(users, asker, "remove a user"))
            .await?;
        Ok(match dropped {
            UserDropped::Removed { revoked } => {
                // no lookup trusts a token of theirs from now on
                self.tokens.forget_user(name);
                self.revoked();
                // kept in the users' log, which outlives their row
                let removed = UserAct::Removed { revoked };
                self.store
                    .keep_user_act(&asker.author(), name, removed)
                    .await;
                UserRemoved::Removed { revoked }
            }
            UserDropped::NoSuchUser => UserRemoved::NoSuchUser,
            UserDropped::LastAdmin => UserRemoved::LastAdmin,
        })
    }

    /// Recovers a lost admin: sets their password, makes them an admin and revokes their tokens,
    /// or adds them as an admin if no user has the name
    ///
    /// Nobody is asked who they are: whoever can open the store holds the service's data, as
    /// `reef-serve recover` does on the service's host. It is never reached over the API.
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    /// * `password` - Their new password
    #[instrument(name = "Service::recover_admin", skip_all, err(Debug))]
    pub async fn recover_admin(&self, name: &str, password: String) -> anyhow::Result<Recovered> {
        // a name and a password the service can keep
        check_user_name(name).map_err(Unkeepable)?;
        check_password(&password).map_err(Unkeepable)?;
        let password = self.hash_password(password).await?;
        let Some(stored) = self.store.user(name).await? else {
            // nobody of the name, so a new admin, judged and added by nobody
            return match self
                .store
                .add_user(name, &password, &NOBODY, |_| Ok(true))
                .await?
            {
                UserKept::Added(_) => {
                    let recovered = UserAct::Recovered {
                        added: true,
                        revoked: 0,
                    };
                    self.store.keep_user_act(&NOBODY, name, recovered).await;
                    Ok(Recovered::Added)
                }
                UserKept::NameTaken => {
                    anyhow::bail!("`{name}` was added while it was recovered: run it again")
                }
            };
        };
        // the new password, an admin from now on, over the row read
        let written = self.store.rewrite_user(&stored, &password, true).await?;
        anyhow::ensure!(
            written,
            "`{name}` changed while it was recovered, so it wasn't: run it again"
        );
        let revoked = self.revoke_all(name).await?;
        // kept in the users' log, naming nobody, since nobody signed in did it
        let recovered = UserAct::Recovered {
            added: false,
            revoked,
        };
        self.store.keep_user_act(&NOBODY, name, recovered).await;
        Ok(Recovered::Reset {
            revoked,
            was_admin: stored.user.admin,
        })
    }

    /// Resolves once a token signs nobody in: revoked, expired, or its user removed
    ///
    /// It looks again after each revoke this service makes, and every minute, for an expiry.
    /// A lookup that fails is tried again at the next look, rather than taken for a revoke.
    ///
    /// # Arguments
    ///
    /// * `token` - The token
    pub async fn signed_out(&self, token: String) {
        let mut revocations = self.revocations.subscribe();
        loop {
            // after a revoke, or a minute on
            tokio::select! {
                changed = revocations.changed() => {
                    // a service that is gone signs nobody in
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(TOKEN_TRUST) => {}
            }
            match self.signed_in(&token).await {
                Ok(Some(_)) => {}
                Ok(None) => return,
                Err(error) => tracing::warn!("a change stream's token wasn't looked up: {error:#}"),
            }
        }
    }

    /// Revokes every token of a user's, returning how many were revoked
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    async fn revoke_all(&self, name: &str) -> anyhow::Result<usize> {
        // forgotten before and after the drop, so no lookup between trusts one
        self.tokens.forget_user(name);
        let revoked = self.store.drop_tokens_of(name).await?;
        self.tokens.forget_user(name);
        self.revoked();
        Ok(revoked)
    }

    /// Tells every change stream that a token may have stopped signing anyone in
    fn revoked(&self) {
        self.revocations
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Hashes a password off the async threads, as an argon2id PHC string
    ///
    /// # Arguments
    ///
    /// * `password` - The password
    async fn hash_password(&self, password: String) -> anyhow::Result<String> {
        // two at a time, each on a blocking thread
        let _permit = self.hashing.acquire().await?;
        tokio::task::spawn_blocking(move || {
            Argon2::default()
                .hash_password(password.as_bytes())
                .map(|hash| hash.to_string())
                .map_err(|error| anyhow::anyhow!("hashing a password failed: {error}"))
        })
        .await?
    }

    /// Checks a password against a PHC string off the async threads, or against a hash no
    /// password matches when there is none
    ///
    /// # Arguments
    ///
    /// * `password` - The password
    /// * `hash` - The PHC string to check it against, or empty for none
    async fn verify_password(&self, password: String, hash: String) -> anyhow::Result<bool> {
        // two at a time, each on a blocking thread
        let _permit = self.hashing.acquire().await?;
        tokio::task::spawn_blocking(move || {
            // no user's hash is checked against one no password matches, taking as long
            let real = !hash.is_empty();
            let hash = match real {
                true => hash.as_str(),
                false => dummy_hash(),
            };
            let parsed = PasswordHash::new(hash)
                .map_err(|error| anyhow::anyhow!("a kept password hash doesn't parse: {error}"))?;
            let matched = Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok();
            Ok(real && matched)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::{hex, new_token, token_hash};
    use reef_api::user::is_token_shaped;

    /// A token is prefixed, random, and kept by a SHA-256 that names it alone
    #[test]
    fn tokens_are_prefixed_random_and_kept_by_sha256() {
        // two tokens, each shaped as one, and never the same
        let first = new_token().unwrap();
        let second = new_token().unwrap();
        assert!(is_token_shaped(&first), "{first}");
        assert_ne!(first, second);
        // a hash is 64 hex digits, the same each time, and not the token's
        let hash = token_hash(&first);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, token_hash(&first));
        assert_ne!(hash, token_hash(&second));
        assert!(!first.contains(&hash));
        // SHA-256 of nothing, as everyone writes it
        assert_eq!(
            token_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
