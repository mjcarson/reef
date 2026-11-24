//! reef's users and the tokens they sign in with: a row each, in two constant partitions
//!
//! - **A user is a row under their name,** their password's argon2id hash and whether they are
//!   an admin kept as JSON beside it. The store never sees a password, only its hash.
//! - **Added under one lock, judged against every user.** Adding a user reads them all and
//!   hands them to the caller's judge, which refuses the add or says whether the new user is an
//!   admin, so the first user can't be added twice by two callers at once. The write is
//!   `if_absent`, so a name taken is refused rather than written over.
//! - **A token is a row under its hash.** The token itself is never stored, so the rows can't be
//!   read back into tokens. A token is kept `if_absent`, and revoked by a delete over the version
//!   read. A user's tokens are found by reading the whole partition, which only listing,
//!   revoking by id and removing a user do.
//! - **Removed under the same lock, never the last admin.** The judge sees every user first, so
//!   whoever may not remove learns nothing of who exists. Then the user's row is deleted over its
//!   version, and every token of theirs after it. A token the delete missed signs nobody in,
//!   since a token issued before its user was added was another user's.
//! - **A password rewritten over the version read,** so a change checked against the current
//!   password can't write over another made since.
//! - **Every row checked against its full key on read**, so a hash collision is caught rather
//!   than served as another user's or another token's.

use reef_api::{TokenKind, User};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::Store;
use super::work::now_ms;
use crate::backend::{Backend, Expect, Refusal, Written};
use crate::schema::{TOKENS, TokenRow, USERS, UserRow};

/// A user as the store keeps one: who they are, their password's hash, and the row's version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUser {
    /// The user, as the service shows one
    pub user: User,
    /// The user's password as an argon2id PHC string
    pub password: String,
    /// The row's version, which a later write names
    pub version: u64,
}

/// A token as the store keeps one: who it signs in, and the row's version
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToken {
    /// The token's SHA-256, as 64 lowercase hex digits
    pub hash: String,
    /// The name of the user it signs in
    pub user: String,
    /// What it was issued to
    pub kind: TokenKind,
    /// What it is for, as the caller that asked for it said
    pub label: String,
    /// When it was issued, in milliseconds since the Unix epoch
    pub created_at: u64,
    /// When it stops signing anyone in, in milliseconds since the Unix epoch, if it ever does
    pub expires_at: Option<u64>,
    /// The row's version, which a delete names
    pub version: u64,
}

impl StoredToken {
    /// Whether the token has stopped signing anyone in
    ///
    /// # Arguments
    ///
    /// * `now` - The time to judge it at, in milliseconds since the Unix epoch
    pub fn expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }
}

/// What came of adding a user to the store
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserKept {
    /// The user was added
    Added(User),
    /// A user of that name exists already
    NameTaken,
}

/// What came of removing a user from the store
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDropped {
    /// The user was removed, and this many of their tokens with them
    Removed {
        /// How many tokens were dropped
        revoked: usize,
    },
    /// No user has the name
    NoSuchUser,
    /// The user is the last admin, who stays
    LastAdmin,
}

/// A user's record, as its row keeps it
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeptUser {
    /// The user's password as an argon2id PHC string
    password: String,
    /// Whether the user may add other users
    admin: bool,
    /// When the user was added, in milliseconds since the Unix epoch
    created_at: u64,
}

/// A token's record, as its row keeps it
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeptToken {
    /// The name of the user it signs in
    user: String,
    /// What it was issued to
    kind: TokenKind,
    /// What it is for
    label: String,
    /// When it was issued, in milliseconds since the Unix epoch
    created_at: u64,
    /// When it stops signing anyone in, if it ever does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
}

/// A user row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `row` - The row
fn read_user(row: UserRow) -> anyhow::Result<StoredUser> {
    // a row of another partition whose key hashed the same is a collision, not a user
    anyhow::ensure!(
        row.list == USERS,
        "user row `{}` of partition `{}` was stored where the users are: their partition keys' \
         hashes collide",
        row.name,
        row.list
    );
    // its name is its key, and the rest its record
    let kept: KeptUser = serde_json::from_str(&row.record)
        .map_err(|error| anyhow::anyhow!("user `{}` doesn't read as one: {error}", row.name))?;
    Ok(StoredUser {
        user: User {
            name: row.name,
            admin: kept.admin,
            created_at: kept.created_at,
        },
        password: kept.password,
        version: row.version,
    })
}

/// A user's row, as the store writes one
///
/// # Arguments
///
/// * `name` - The user's name
/// * `kept` - Their record
/// * `version` - The row's version
fn user_row(name: &str, kept: &KeptUser, version: u64) -> anyhow::Result<UserRow> {
    Ok(UserRow {
        list: USERS.to_string(),
        name: name.to_string(),
        version,
        record: serde_json::to_string(kept)?,
    })
}

/// A token row read back, checked against the partition it was read from
///
/// # Arguments
///
/// * `row` - The row
fn read_token(row: TokenRow) -> anyhow::Result<StoredToken> {
    // a row of another partition whose key hashed the same is a collision, not a token
    anyhow::ensure!(
        row.list == TOKENS,
        "token row of partition `{}` was stored where the tokens are: their partition keys' \
         hashes collide",
        row.list
    );
    // its hash is its key, and the rest its record
    let kept: KeptToken = serde_json::from_str(&row.record)
        .map_err(|error| anyhow::anyhow!("a token doesn't read as one: {error}"))?;
    Ok(StoredToken {
        hash: row.hash,
        user: kept.user,
        kind: kept.kind,
        label: kept.label,
        created_at: kept.created_at,
        expires_at: kept.expires_at,
        version: row.version,
    })
}

impl<B: Backend> Store<B> {
    /// Lists every user, by name
    #[instrument(name = "Store::users", skip_all, err(Debug))]
    pub async fn users(&self) -> anyhow::Result<Vec<StoredUser>> {
        // the one partition, each row checked against its full key
        let rows = self.backend.users(None).await?;
        rows.into_iter().map(read_user).collect()
    }

    /// Gets a user by name, if there is one
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    #[instrument(name = "Store::user", skip_all, err(Debug))]
    pub async fn user(&self, name: &str) -> anyhow::Result<Option<StoredUser>> {
        // the one row under the name's key
        let rows = self.backend.users(Some(name)).await?;
        for row in rows {
            let read = read_user(row)?;
            // a row read under another key is not this user's
            if read.user.name == name {
                return Ok(Some(read));
            }
        }
        Ok(None)
    }

    /// Adds a user under the users' lock, judged against every user as they stand
    ///
    /// The judge sees every user and refuses the add with an error, or says whether the new
    /// user is an admin.
    ///
    /// # Arguments
    ///
    /// * `name` - The new user's name
    /// * `password` - Their password as an argon2id PHC string
    /// * `judge` - Refuses the add, or says whether the new user is an admin
    #[instrument(name = "Store::add_user", skip_all, err(Debug))]
    pub async fn add_user<J>(
        &self,
        name: &str,
        password: &str,
        judge: J,
    ) -> anyhow::Result<UserKept>
    where
        J: FnOnce(&[StoredUser]) -> anyhow::Result<bool> + Send,
    {
        // under the lock, against every user as they stand
        let _held = self.user_lock.lock().await;
        let users = self.users().await?;
        let admin = judge(&users)?;
        // a name taken is said so without a write
        if users.iter().any(|stored| stored.user.name == name) {
            return Ok(UserKept::NameTaken);
        }
        let user = User {
            name: name.to_string(),
            admin,
            created_at: now_ms(),
        };
        let kept = KeptUser {
            password: password.to_string(),
            admin,
            created_at: user.created_at,
        };
        let row = user_row(name, &kept, 1)?;
        // where no user of the name is, so a user is never written over
        match self.backend.write_user(row, Expect::Absent).await? {
            Written::Applied => Ok(UserKept::Added(user)),
            Written::Refused(Refusal::Exists) => Ok(UserKept::NameTaken),
            Written::Refused(refusal) => {
                anyhow::bail!("user `{name}` wasn't added: {refusal:?}")
            }
        }
    }

    /// Removes a user under the users' lock, and every token of theirs, judged against every
    /// user as they stand
    ///
    /// The judge sees every user and refuses the removal with an error before anything else is
    /// said, so a caller who may not remove learns nothing of who exists. The last admin is never
    /// removed.
    ///
    /// # Arguments
    ///
    /// * `name` - The user's name
    /// * `judge` - Refuses the removal, or lets it go on
    #[instrument(name = "Store::remove_user", skip_all, err(Debug))]
    pub async fn remove_user<J>(&self, name: &str, judge: J) -> anyhow::Result<UserDropped>
    where
        J: FnOnce(&[StoredUser]) -> anyhow::Result<()> + Send,
    {
        // under the lock, against every user as they stand
        let _held = self.user_lock.lock().await;
        let users = self.users().await?;
        judge(&users)?;
        let Some(stored) = users.iter().find(|stored| stored.user.name == name) else {
            return Ok(UserDropped::NoSuchUser);
        };
        // an admin stays while no other admin would
        let admins = users.iter().filter(|stored| stored.user.admin).count();
        if stored.user.admin && admins <= 1 {
            return Ok(UserDropped::LastAdmin);
        }
        // the row as it was read, deleted only at its version
        let row = UserRow {
            list: USERS.to_string(),
            name: name.to_string(),
            version: stored.version,
            record: String::new(),
        };
        match self.backend.delete_user(&row).await? {
            Written::Applied => {}
            // nothing else writes a user while the lock is held, so this is no race
            Written::Refused(refusal) => {
                anyhow::bail!("user `{name}` wasn't removed: {refusal:?}")
            }
        }
        // then every token of theirs
        let revoked = self.drop_tokens_of(name).await?;
        Ok(UserDropped::Removed { revoked })
    }

    /// Rewrites a user's password, and whether they are an admin, over the version read,
    /// returning whether it was written
    ///
    /// # Arguments
    ///
    /// * `stored` - The user, as read
    /// * `password` - Their new password as an argon2id PHC string
    /// * `admin` - Whether they are an admin from now on
    #[instrument(name = "Store::rewrite_user", skip_all, err(Debug))]
    pub async fn rewrite_user(
        &self,
        stored: &StoredUser,
        password: &str,
        admin: bool,
    ) -> anyhow::Result<bool> {
        // under the lock, so a removal reads the user as written
        let _held = self.user_lock.lock().await;
        let kept = KeptUser {
            password: password.to_string(),
            admin,
            created_at: stored.user.created_at,
        };
        let row = user_row(&stored.user.name, &kept, stored.version + 1)?;
        // only over the version read, so a change since is never written over
        match self
            .backend
            .write_user(row, Expect::Version(stored.version))
            .await?
        {
            Written::Applied => Ok(true),
            Written::Refused(_) => Ok(false),
        }
    }

    /// Lists every token, or one user's, in hash order
    ///
    /// # Arguments
    ///
    /// * `user` - The one user whose tokens to list, or `None` for every token
    #[instrument(name = "Store::tokens", skip_all, err(Debug))]
    pub async fn tokens(&self, user: Option<&str>) -> anyhow::Result<Vec<StoredToken>> {
        // the whole partition, each row checked against its full key
        let rows = self.backend.tokens().await?;
        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            let read = read_token(row)?;
            // only the user's, if one is named
            if user.is_none_or(|user| read.user == user) {
                tokens.push(read);
            }
        }
        Ok(tokens)
    }

    /// Drops every token of a user's, returning how many were dropped
    ///
    /// # Arguments
    ///
    /// * `user` - The user's name
    #[instrument(name = "Store::drop_tokens_of", skip_all, err(Debug))]
    pub async fn drop_tokens_of(&self, user: &str) -> anyhow::Result<usize> {
        // each as read, a token gone already not counted
        let mut dropped = 0;
        for token in self.tokens(Some(user)).await? {
            if self.drop_token(&token).await? {
                dropped += 1;
            }
        }
        Ok(dropped)
    }

    /// Gets the token kept under a hash, if one is
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's SHA-256, as 64 lowercase hex digits
    #[instrument(name = "Store::token", skip_all, err(Debug))]
    pub async fn token(&self, hash: &str) -> anyhow::Result<Option<StoredToken>> {
        // the one row under the hash's key
        let Some(row) = self.backend.token(hash).await? else {
            return Ok(None);
        };
        let read = read_token(row)?;
        // a row read under another key is not this token's
        Ok((read.hash == hash).then_some(read))
    }

    /// Keeps a token for a user, returning it as kept
    ///
    /// # Arguments
    ///
    /// * `hash` - The token's SHA-256, as 64 lowercase hex digits
    /// * `user` - The name of the user it signs in
    /// * `kind` - What it was issued to
    /// * `label` - What it is for
    /// * `expires_at` - When it stops signing anyone in, if it ever does
    #[instrument(name = "Store::keep_token", skip_all, err(Debug))]
    pub async fn keep_token(
        &self,
        hash: &str,
        user: &str,
        kind: TokenKind,
        label: &str,
        expires_at: Option<u64>,
    ) -> anyhow::Result<StoredToken> {
        // the record beside its hash
        let kept = KeptToken {
            user: user.to_string(),
            kind,
            label: label.to_string(),
            created_at: now_ms(),
            expires_at,
        };
        let row = TokenRow {
            list: TOKENS.to_string(),
            hash: hash.to_string(),
            version: 1,
            record: serde_json::to_string(&kept)?,
        };
        // where no token is, since 32 random bytes never meet another's
        match self.backend.write_token(row, Expect::Absent).await? {
            Written::Applied => Ok(StoredToken {
                hash: hash.to_string(),
                user: kept.user,
                kind,
                label: kept.label,
                created_at: kept.created_at,
                expires_at,
                version: 1,
            }),
            Written::Refused(refusal) => anyhow::bail!("a token wasn't kept: {refusal:?}"),
        }
    }

    /// Drops a token, if it is still the one read, returning whether it was dropped
    ///
    /// # Arguments
    ///
    /// * `token` - The token, as read
    #[instrument(name = "Store::drop_token", skip_all, err(Debug))]
    pub async fn drop_token(&self, token: &StoredToken) -> anyhow::Result<bool> {
        // the row as it was read, deleted only at its version
        let row = TokenRow {
            list: TOKENS.to_string(),
            hash: token.hash.clone(),
            version: token.version,
            record: String::new(),
        };
        match self.backend.delete_token(&row).await? {
            Written::Applied => Ok(true),
            // one gone already, or written since, is not this read's to drop
            Written::Refused(_) => Ok(false),
        }
    }
}
