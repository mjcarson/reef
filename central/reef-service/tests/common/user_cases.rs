//! The users' and tokens' cases, each generic over the backend so it runs against shoal and the
//! twin

use reef_api::TokenKind;
use reef_service::UserKept;

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
        .add_user("ada", "$argon2id$ada", |users| Ok(users.is_empty()))
        .await?;
    let UserKept::Added(ada) = added else {
        anyhow::bail!("ada wasn't added: {added:?}");
    };
    assert!(ada.admin);
    let added = fx
        .store
        .add_user("bob", "$argon2id$bob", |users| Ok(users.is_empty()))
        .await?;
    assert!(
        matches!(&added, UserKept::Added(user) if !user.admin),
        "{added:?}"
    );
    // a name taken is said so, and a judge's refusal writes nothing
    let taken = fx
        .store
        .add_user("bob", "$argon2id$x", |_| Ok(false))
        .await?;
    assert_eq!(taken, UserKept::NameTaken);
    let refused = fx
        .store
        .add_user("carol", "$argon2id$c", |_| anyhow::bail!("not allowed"))
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
