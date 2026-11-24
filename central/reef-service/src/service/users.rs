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
//!   reef-service is shoal's only writer, so a revoke here drops its token at once.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use reef_api::user::{PASSWORD_MAX, check_password, check_user_name, is_token_shaped};
use reef_api::{IssuedToken, NewUser, TokenKind, User, UserAdded};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use tracing::instrument;

use super::Service;
use crate::backend::Backend;
use crate::store::{StoredUser, Unkeepable, UserKept};

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

/// Who asks to add or list users: the user their token signs in, and whether they call from the
/// service's own host
#[derive(Debug, Clone, Copy, Default)]
pub struct Asker<'a> {
    /// The user the caller's token signs in, if it signs one in
    pub user: Option<&'a User>,
    /// Whether the caller reached the service from its own host
    pub local: bool,
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
        let kept = self
            .store
            .add_user(&new.name, &password, |users| {
                judge_add(users, asker, new.admin)
            })
            .await?;
        Ok(match kept {
            UserKept::Added(user) => UserAdded::Added(user),
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
    #[instrument(name = "Service::sign_in", skip_all, err(Debug))]
    pub async fn sign_in(
        &self,
        name: &str,
        password: &str,
        kind: TokenKind,
        label: &str,
    ) -> anyhow::Result<Option<IssuedToken>> {
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
        self.store.drop_token(&stored).await
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
