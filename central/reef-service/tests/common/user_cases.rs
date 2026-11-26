//! The users' and tokens' cases, each generic over the backend so it runs against shoal and the
//! twin

use reef_api::TokenKind;
use reef_service::{UserDropped, UserKept};

use super::{Fx, Reopen};

/// Users and tokens are kept by their keys: a user once under their name, judged under the lock,
/// a token under its hash and dropped by the version read, and all of it read back once the
/// store is reopened
///
/// # Arguments
///
/// * `fx` - The store
pub async fn users_and_tokens_are_kept_by_key<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // a fresh store has no users
    assert!(fx.store.users().await?.is_empty());
    // the judge sees every user, and says the first is an admin
    let added = fx
        .store
        .add_user("ada", "$argon2id$ada", &reef_api::NOBODY, |users| {
            Ok(users.is_empty())
        })
        .await?;
    let UserKept::Added(ada) = added else {
        anyhow::bail!("ada wasn't added: {added:?}");
    };
    assert!(ada.admin);
    let added = fx
        .store
        .add_user("bob", "$argon2id$bob", &reef_api::NOBODY, |users| {
            Ok(users.is_empty())
        })
        .await?;
    assert!(
        matches!(&added, UserKept::Added(user) if !user.admin),
        "{added:?}"
    );
    // a name taken is said so, and a judge's refusal writes nothing
    let taken = fx
        .store
        .add_user("bob", "$argon2id$x", &reef_api::NOBODY, |_| Ok(false))
        .await?;
    assert_eq!(taken, UserKept::NameTaken);
    let refused = fx
        .store
        .add_user("carol", "$argon2id$c", &reef_api::NOBODY, |_| {
            anyhow::bail!("not allowed")
        })
        .await;
    assert!(refused.is_err());
    // each read by name, with its password's hash
    let stored = fx.store.user("bob").await?.expect("bob reads back");
    assert_eq!(stored.password, "$argon2id$bob");
    assert_eq!(fx.store.user("carol").await?, None);
    // a token kept by its hash, and read back by it alone
    let hash = "a".repeat(64);
    let kept = fx
        .store
        .keep_token(&hash, "ada", TokenKind::Browser, "a browser", Some(9))
        .await?;
    assert_eq!(fx.store.token(&hash).await?, Some(kept.clone()));
    assert_eq!(fx.store.token(&"b".repeat(64)).await?, None);
    assert!(kept.expired(9) && !kept.expired(8));
    // all of it read back once the store is reopened
    let fx = fx.reopen().await?;
    let names: Vec<_> = fx
        .store
        .users()
        .await?
        .into_iter()
        .map(|stored| (stored.user.name, stored.user.admin))
        .collect();
    assert_eq!(
        names,
        [("ada".to_string(), true), ("bob".to_string(), false)]
    );
    let read = fx.store.token(&hash).await?.expect("the token reads back");
    assert_eq!(read, kept);
    // dropped over the version read, once
    assert!(fx.store.drop_token(&read).await?);
    assert!(!fx.store.drop_token(&read).await?);
    assert_eq!(fx.store.token(&hash).await?, None);
    Ok(())
}

/// A user is removed under the lock with every token of theirs, never the last admin, and a
/// password is rewritten only over the version read
///
/// # Arguments
///
/// * `fx` - The store
pub async fn users_are_removed_with_their_tokens<B: Reopen>(fx: Fx<B>) -> anyhow::Result<()> {
    // an admin and a user, with two tokens of bob's and one of ada's
    fx.store
        .add_user("ada", "$argon2id$ada", &reef_api::NOBODY, |_| Ok(true))
        .await?;
    fx.store
        .add_user("bob", "$argon2id$bob", &reef_api::NOBODY, |_| Ok(false))
        .await?;
    for (hash, user) in [("c", "bob"), ("a", "bob"), ("b", "ada")] {
        let hash = hash.repeat(64);
        fx.store
            .keep_token(&hash, user, TokenKind::Cli, "a case", None)
            .await?;
    }
    // every token in hash order, or one user's
    let hashes = |tokens: Vec<reef_service::StoredToken>| -> Vec<String> {
        tokens
            .into_iter()
            .map(|token| token.hash[..1].to_string())
            .collect()
    };
    assert_eq!(hashes(fx.store.tokens(None).await?), ["a", "b", "c"]);
    assert_eq!(hashes(fx.store.tokens(Some("bob")).await?), ["a", "c"]);
    assert!(fx.store.tokens(Some("carol")).await?.is_empty());
    // a password rewritten over the version read, and a stale read written nowhere
    let bob = fx.store.user("bob").await?.expect("bob reads back");
    assert!(fx.store.rewrite_user(&bob, "$argon2id$new", false).await?);
    assert!(!fx.store.rewrite_user(&bob, "$argon2id$stale", true).await?);
    let rewritten = fx.store.user("bob").await?.expect("bob reads back");
    assert_eq!(rewritten.password, "$argon2id$new");
    assert_eq!(rewritten.user, bob.user);
    // the judge is heard first, even for a user nobody has
    let refused = fx
        .store
        .remove_user("nobody", |_| anyhow::bail!("not allowed"))
        .await;
    assert!(refused.is_err());
    let missing = fx.store.remove_user("nobody", |_| Ok(())).await?;
    assert_eq!(missing, UserDropped::NoSuchUser);
    // the last admin stays, and their tokens with them
    let last = fx.store.remove_user("ada", |_| Ok(())).await?;
    assert_eq!(last, UserDropped::LastAdmin);
    assert!(fx.store.user("ada").await?.is_some());
    // bob goes, his tokens with him, and ada's stay
    let removed = fx.store.remove_user("bob", |_| Ok(())).await?;
    assert_eq!(removed, UserDropped::Removed { revoked: 2 });
    assert_eq!(fx.store.user("bob").await?, None);
    assert_eq!(hashes(fx.store.tokens(None).await?), ["b"]);
    // and so it reads once the store is reopened
    let fx = fx.reopen().await?;
    let names: Vec<_> = fx
        .store
        .users()
        .await?
        .into_iter()
        .map(|stored| stored.user.name)
        .collect();
    assert_eq!(names, ["ada"]);
    assert_eq!(hashes(fx.store.tokens(None).await?), ["b"]);
    // a second admin lets the first go
    fx.store
        .add_user("dan", "$argon2id$dan", &reef_api::NOBODY, |_| Ok(true))
        .await?;
    let removed = fx.store.remove_user("ada", |_| Ok(())).await?;
    assert_eq!(removed, UserDropped::Removed { revoked: 1 });
    Ok(())
}
