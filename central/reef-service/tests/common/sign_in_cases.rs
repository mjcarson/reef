//! The sign-in suite's cases, each generic over the backend so it runs against shoal and the twin
//!
//! Each case serves its fixture's service on a loopback port of its own, asking every caller to
//! sign in unless the case is about sign-in off, and calls it as the CLI and a browser do: with a
//! token as `Authorization: Bearer`, with the sign-in cookie, or with nothing.

use reef_api::user::SIGN_IN_COOKIE;
use reef_api::{
    AddedProject, ApiError, Build, Checked, ErrorCode, IssuedToken, Listing, Me, NewProject,
    NewUser, PasswordChange, PasswordChanged, Project, SignIn, SignedOut, TokenInfo, TokenKind,
    User, UserAdded, UserRemoved, paths,
};
use reef_model::{Description, NodeSpec, PlanName};
use reef_service::service::token_hash;
use reef_service::{Asker, Backend, Recovered, Served, Unkeepable};
use reqwest::header::{
    ACCEPT, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE,
    LOCATION, RETRY_AFTER, SET_COOKIE, WWW_AUTHENTICATE,
};
use reqwest::{Method, StatusCode, Version};
use std::time::Duration;

use super::Fx;
use super::git;
use super::http::{Ax, Client, fake_client, stamped};
use reef_api::{NodePut, NodeWrite, Puts};

/// The password every case's users sign in with
pub const PASSWORD: &str = "correct horse battery";

/// A node that says why and what, for a put to write
///
/// # Arguments
///
/// * `id` - The node's id
fn work(id: &str) -> Puts {
    let node = NodeWrite {
        spec: NodeSpec {
            id: id.to_string(),
            title: Some(format!("{id} Some work")),
            description: Some(Description {
                why: Some(format!("Without {id}, something is missing.")),
                what: Some(format!("With {id}, it is there.")),
                details: None,
            }),
            ..NodeSpec::default()
        },
        body: format!("# {id}\n"),
        order: 0,
    };
    Puts::new(vec![NodePut::new(node, None)])
}

/// A new user's request, with the cases' password
///
/// # Arguments
///
/// * `name` - Their name
/// * `admin` - Whether they are to be an admin
pub fn new_user(name: &str, admin: bool) -> NewUser {
    NewUser {
        name: name.to_string(),
        password: PASSWORD.to_string(),
        admin,
    }
}

/// Adds the first user, an admin, straight to the service, as from its own host
///
/// # Arguments
///
/// * `ax` - The served service
/// * `name` - The admin's name
pub async fn first_admin<B: Backend + 'static>(ax: &Ax<B>, name: &str) -> anyhow::Result<User> {
    // the first user, from the service's own host
    let asker = Asker {
        user: None,
        local: true,
        ..Default::default()
    };
    match ax.service.add_user(new_user(name, false), asker).await? {
        UserAdded::Added(user) => Ok(user),
        UserAdded::NameTaken => anyhow::bail!("`{name}` was taken in a fresh service"),
    }
}

/// Signs a user in through the JSON API, returning a client that sends their token
///
/// # Arguments
///
/// * `ax` - The served service
/// * `name` - The user's name
pub async fn signed_in<B: Backend + 'static>(ax: &Ax<B>, name: &str) -> anyhow::Result<Client> {
    let request = SignIn {
        name: name.to_string(),
        password: PASSWORD.to_string(),
        label: Some("a case".to_string()),
    };
    let (status, issued): (_, IssuedToken) = ax.client.post(paths::TOKENS, &request).await?;
    anyhow::ensure!(
        status == StatusCode::CREATED,
        "signing `{name}` in answered {status}"
    );
    Ok(ax.client.signed(&issued.token))
}

/// Checks an answer is a 401 that says to sign in, and with what
///
/// # Arguments
///
/// * `response` - The answer
/// * `what` - What was asked, for the message
async fn is_refused(response: reqwest::Response, what: &str) -> anyhow::Result<()> {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{what}");
    let says = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .map(|value| value.to_str().unwrap_or_default().to_string());
    assert_eq!(says.as_deref(), Some("Bearer realm=\"reef\""), "{what}");
    let error: ApiError = response.json().await?;
    assert_eq!(error.code, ErrorCode::Unauthorized, "{what}");
    Ok(())
}

/// The sign-in cookie a browser was sent, its value and its attributes
///
/// # Arguments
///
/// * `response` - The answer that set it
fn set_cookie(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{SIGN_IN_COOKIE}=")))
        .map(str::to_string)
}

/// Every route of the JSON API that writes, but the ones that sign a user in or out
///
/// # Arguments
///
/// * `plan` - The plan whose routes they are
fn writes(plan: &PlanName) -> anyhow::Result<Vec<(Method, String)>> {
    let node = reef_model::NodeName::new(plan.clone(), "C1")?;
    Ok(vec![
        (Method::POST, paths::PROJECTS.to_string()),
        (Method::POST, paths::plans(&plan.project)),
        (Method::POST, paths::nodes(plan)),
        (Method::PUT, paths::revision(plan)),
        (Method::POST, paths::pages(plan)),
        (Method::POST, paths::queue(&plan.project)),
        (Method::POST, paths::fetch(&plan.project)),
        (Method::POST, paths::node_work(&node)),
        (Method::POST, paths::captures(&node)),
        (Method::POST, paths::node_comments(&node)),
        (Method::PUT, paths::node_review(&node)),
        (Method::POST, paths::node_decisions(&node)),
        (Method::POST, paths::replies(plan, 1)),
        (Method::PUT, paths::thread_state(plan, 1)),
        (Method::POST, paths::thread_resolution(plan, 1)),
        (Method::POST, paths::LOOPS.to_string()),
        (Method::POST, paths::SESSIONS.to_string()),
        (Method::POST, paths::session("s")),
        (Method::DELETE, paths::session("s")),
        (Method::POST, paths::session_orders("s")),
        (Method::POST, paths::TEST_RUNS.to_string()),
        (Method::POST, paths::test_run("r")),
        (Method::DELETE, paths::CURRENT_TOKEN.to_string()),
    ])
}

/// Unauthenticated writes are refused: every write route answers 401, a made-up token's too,
/// and the plan is as it was
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn unauthenticated_writes_are_refused<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let plan = ax.plan.clone();
    // every route that writes, each refused before its body is read
    let writes = writes(&plan)?;
    let made_up = ax.client.signed(&format!("reef_{}", "0".repeat(64)));
    for (method, path) in &writes {
        // with no token, and with one nobody was issued
        for (client, who) in [(&ax.client, "nobody"), (&made_up, "a made-up token")] {
            let request = client
                .request(method.clone(), path)
                .header("content-type", "application/json")
                .body("{}");
            is_refused(
                client.raw(request).await?,
                &format!("{method} {path} by {who}"),
            )
            .await?;
        }
    }
    // a real put the same, before its body is judged
    let (status, _): (_, ApiError) = ax.client.post(&paths::nodes(&plan), &work("C1")).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // and the plan as it was, read by a user signed in
    let ada = signed_in(&ax, "ada").await?;
    let (status, listing): (_, Listing) = ada.get(&paths::nodes(&plan)).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listing.nodes.is_empty(),
        "a refused write was kept: {listing:?}"
    );
    // which may write it
    let (status, checked): (_, Checked) = ada.post(&paths::nodes(&plan), &work("C1")).await?;
    assert_eq!(status, StatusCode::OK, "{checked:?}");
    Ok(())
}

/// An unauthenticated read is refused, and a page a browser opens sends it to sign in
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn unauthenticated_reads_are_refused_and_pages_ask_for_sign_in<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    let plan = ax.plan.clone();
    // every read of a plan, its project or its stream is refused
    let reads = [
        paths::PROJECTS.to_string(),
        paths::plans(&plan.project),
        paths::plan(&plan),
        paths::nodes(&plan),
        paths::node(&plan, "C1"),
        paths::payload(&plan),
        paths::check(&plan),
        paths::next(&plan),
        paths::pages(&plan),
        paths::plan_comments(&plan),
        paths::work(&plan.project),
        paths::queue(&plan.project),
        paths::LOOPS.to_string(),
        paths::changes(None, None),
        format!("{}README.html", paths::docs(&plan)),
    ];
    for path in &reads {
        is_refused(ax.client.open(path).await?, &format!("GET {path}")).await?;
    }
    // a page a browser opens sends it to sign in, and back here once it has
    for (path, next) in [
        ("/".to_string(), "%2F".to_string()),
        (
            "/plans/reef/central".to_string(),
            "%2Fplans%2Freef%2Fcentral".to_string(),
        ),
        (
            format!("{}README.html", paths::docs(&plan)),
            paths::encode(&format!("{}README.html", paths::docs(&plan))),
        ),
    ] {
        let request = ax
            .client
            .request(Method::GET, &path)
            .header(ACCEPT, "text/html,*/*;q=0.8");
        let response = ax.client.raw(request).await?;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "GET {path}");
        let location = response.headers().get(LOCATION).unwrap().to_str()?;
        assert_eq!(
            location,
            format!("{}?next={next}", paths::SIGN_IN),
            "GET {path}"
        );
    }
    // the sign-in page itself answers, with no script let in
    let request = ax
        .client
        .request(Method::GET, paths::SIGN_IN)
        .header(ACCEPT, "text/html");
    let response = ax.client.raw(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let policy = response
        .headers()
        .get(CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()?
        .to_string();
    assert!(policy.contains("default-src 'none'"), "{policy}");
    assert!(response.text().await?.contains("Sign in to reef"));
    // and anyone may ask who they are taken for
    let (status, me): (_, Me) = ax.client.get(paths::ME).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        me,
        Me {
            user: None,
            required: true
        }
    );
    Ok(())
}

/// The first user is added from the service's host as an admin, and only an admin adds the rest
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn the_first_user_is_an_admin_and_admins_add_the_rest<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    // with no users, anyone lists none
    let (status, users): (_, Vec<User>) = ax.client.get(paths::USERS).await?;
    assert_eq!((status, users.len()), (StatusCode::OK, 0));
    // the first, from this host, is an admin though it didn't ask to be
    let (status, added): (_, UserAdded) = ax
        .client
        .post(paths::USERS, &new_user("ada", false))
        .await?;
    assert_eq!(status, StatusCode::CREATED);
    let UserAdded::Added(ada) = added else {
        anyhow::bail!("the first user wasn't added: {added:?}");
    };
    assert!(ada.admin, "the first user isn't an admin");
    // then nobody adds a user, nor lists them
    let (status, error): (_, ApiError) =
        ax.client.post(paths::USERS, &new_user("eve", true)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
    );
    let (status, _): (_, ApiError) = ax.client.get(paths::USERS).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // an admin adds one, no admin unless asked
    let ada_client = signed_in(&ax, "ada").await?;
    let (status, added): (_, UserAdded) = ada_client
        .post(paths::USERS, &new_user("bob", false))
        .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        matches!(&added, UserAdded::Added(user) if !user.admin),
        "{added:?}"
    );
    // a user who is no admin may do neither
    let bob = signed_in(&ax, "bob").await?;
    let (status, error): (_, ApiError) = bob.post(paths::USERS, &new_user("carol", false)).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    let (status, _): (_, ApiError) = bob.get(paths::USERS).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // a name taken is said so, and a name or password reef can't keep is refused
    let (status, taken): (_, UserAdded) = ada_client
        .post(paths::USERS, &new_user("bob", false))
        .await?;
    assert_eq!(
        (status, taken),
        (StatusCode::CONFLICT, UserAdded::NameTaken)
    );
    let (status, error): (_, ApiError) = ada_client
        .post(paths::USERS, &new_user("-bad", false))
        .await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    let short = NewUser {
        password: "short".to_string(),
        ..new_user("dan", false)
    };
    let (status, error): (_, ApiError) = ada_client.post(paths::USERS, &short).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    // and the admin lists both, by name
    let (status, users): (_, Vec<User>) = ada_client.get(paths::USERS).await?;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<_> = users
        .iter()
        .map(|user| (user.name.as_str(), user.admin))
        .collect();
    assert_eq!(names, [("ada", true), ("bob", false)]);
    Ok(())
}

/// A token signs requests in until it is revoked, and a wrong name and a wrong password are told
/// alike
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn a_token_signs_requests_in_until_revoked<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    // the right password issues a token, shaped as one, that never expires
    let request = SignIn {
        name: "ada".to_string(),
        password: PASSWORD.to_string(),
        label: None,
    };
    let (status, issued): (_, IssuedToken) = ax.client.post(paths::TOKENS, &request).await?;
    assert_eq!(status, StatusCode::CREATED);
    assert!(reef_api::user::is_token_shaped(&issued.token), "{issued:?}");
    assert_eq!((issued.kind, issued.expires_at), (TokenKind::Cli, None));
    assert_eq!(issued.user.name, "ada");
    // a wrong password and a name nobody has are told the same
    let wrong = SignIn {
        password: "not the password".to_string(),
        ..request.clone()
    };
    let nobody = SignIn {
        name: "nobody".to_string(),
        ..request.clone()
    };
    let (status, wrong): (_, ApiError) = ax.client.post(paths::TOKENS, &wrong).await?;
    assert_eq!(
        (status, wrong.code),
        (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
    );
    let (status, nobody): (_, ApiError) = ax.client.post(paths::TOKENS, &nobody).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.message, nobody.message);
    // the token names its user, and reads and writes
    let ada = ax.client.signed(&issued.token);
    let (_, me): (_, Me) = ada.get(paths::ME).await?;
    assert_eq!(me.user.map(|user| user.name).as_deref(), Some("ada"));
    let (status, _): (_, Vec<Project>) = ada.get(paths::PROJECTS).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _): (_, Checked) = ada.post(&paths::nodes(&ax.plan), &work("C1")).await?;
    assert_eq!(status, StatusCode::OK);
    // revoked, it signs nobody in, at once
    let request = ada.request(Method::DELETE, paths::CURRENT_TOKEN);
    let response = ada.raw(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let revoked: SignedOut = response.json().await?;
    assert!(revoked.revoked);
    is_refused(ada.open(paths::PROJECTS).await?, "a revoked token").await?;
    let (_, me): (_, Me) = ada.get(paths::ME).await?;
    assert_eq!(me.user, None);
    Ok(())
}

/// A browser signs in with a cookie it can't read, set for this origin's paths alone, and signs
/// out again
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn the_browser_signs_in_with_a_cookie<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let form = |password: &str, next: &str| {
        ax.client
            .request(Method::POST, paths::SIGN_IN)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(format!(
                "name=ada&password={}&next={}",
                paths::encode(password),
                paths::encode(next)
            ))
    };
    // a wrong password is the form again, saying so, and sets no cookie
    let response = ax
        .client
        .raw(form("wrong password", "/plans/reef/central"))
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(set_cookie(&response), None);
    assert!(response.text().await?.contains("don&#39;t match"));
    // the right one sets the cookie and goes on where it was going
    let response = ax.client.raw(form(PASSWORD, "/plans/reef/central")).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(LOCATION).unwrap(),
        "/plans/reef/central"
    );
    let cookie = set_cookie(&response).expect("signing in set no cookie");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"] {
        assert!(cookie.contains(attribute), "{cookie} lacks {attribute}");
    }
    // Secure, since every port reef has speaks HTTPS
    assert!(
        cookie.contains("Secure"),
        "{cookie} isn't Secure over HTTPS"
    );
    let pair = cookie.split(';').next().unwrap().to_string();
    // but never on to another site
    let response = ax
        .client
        .raw(form(PASSWORD, "//elsewhere.example/"))
        .await?;
    assert_eq!(response.headers().get(LOCATION).unwrap(), "/");
    // with the cookie, the API and the stream answer, and the sign-in page goes straight on
    let with_cookie =
        |method: Method, path: &str| ax.client.request(method, path).header(COOKIE, &pair);
    let response = ax
        .client
        .raw(with_cookie(Method::GET, paths::PROJECTS))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let response = ax
        .client
        .raw(with_cookie(Method::GET, &paths::changes(None, None)))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let request = with_cookie(
        Method::GET,
        &format!("{}?next=%2Fplans%2Fa%2Fb", paths::SIGN_IN),
    );
    let response = ax.client.raw(request).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(LOCATION).unwrap(), "/plans/a/b");
    // signing out clears the cookie, revokes its token, and goes back to sign in
    let response = ax
        .client
        .raw(with_cookie(Method::POST, paths::SIGN_OUT))
        .await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(LOCATION).unwrap(), paths::SIGN_IN);
    let cleared = set_cookie(&response).expect("signing out cleared no cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    // so the old cookie reads nothing, and a page it opens is sent to sign in, the cookie cleared
    let response = ax
        .client
        .raw(with_cookie(Method::GET, paths::PROJECTS))
        .await?;
    is_refused(response, "a cookie signed out").await?;
    let request = with_cookie(Method::GET, "/plans/reef/central").header(ACCEPT, "text/html");
    let response = ax.client.raw(request).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(set_cookie(&response).is_some_and(|cookie| cookie.contains("Max-Age=0")));
    Ok(())
}

/// With sign-in off, every route answers as before: anyone reads and writes, a bogus token is
/// ignored, and only adding users asks who is calling
///
/// # Arguments
///
/// * `fx` - The fixture, served asking no caller to sign in
pub async fn sign_in_off_changes_nothing<B: Backend + 'static>(fx: Fx<B>) -> anyhow::Result<()> {
    let ax = fx.api().await?;
    let plan = ax.plan.clone();
    // anyone reads and writes, as before users
    let (status, _): (_, Vec<Project>) = ax.client.get(paths::PROJECTS).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _): (_, Checked) = ax.client.post(&paths::nodes(&plan), &work("C1")).await?;
    assert_eq!(status, StatusCode::OK);
    let response = ax.client.open(&paths::changes(None, None)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    // a token nobody was issued changes nothing
    let bogus = ax.client.signed(&format!("reef_{}", "0".repeat(64)));
    let (status, _): (_, Checked) = bogus.post(&paths::nodes(&plan), &work("C2")).await?;
    assert_eq!(status, StatusCode::OK);
    // no page sends a browser to sign in
    let request = ax
        .client
        .request(Method::GET, "/")
        .header(ACCEPT, "text/html");
    let response = ax.client.raw(request).await?;
    assert_ne!(response.status(), StatusCode::SEE_OTHER);
    // and the service says nobody need sign in
    let (_, me): (_, Me) = ax.client.get(paths::ME).await?;
    assert_eq!(
        me,
        Me {
            user: None,
            required: false
        }
    );
    // users can be made ready before sign-in is turned on: the first from this host, then by it
    let (status, _): (_, UserAdded) = ax
        .client
        .post(paths::USERS, &new_user("ada", false))
        .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, error): (_, ApiError) = ax
        .client
        .post(paths::USERS, &new_user("eve", false))
        .await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
    );
    Ok(())
}

/// Checks every write route refuses a request a browser says came from a page of another
/// origin, signed in or not, while reads, the CLI's writes and the service's own pages' go on
///
/// # Arguments
///
/// * `ax` - The served service
/// * `client` - The client to call it as, signed in if the service asks callers to
async fn refuses_cross_origin<B: Backend + 'static>(
    ax: &Ax<B>,
    client: &Client,
) -> anyhow::Result<()> {
    let plan = ax.plan.clone();
    // every route that writes, and the ones that sign a user in or out or add one
    let mut writes = writes(&plan)?;
    writes.extend([
        (Method::POST, paths::SIGN_IN.to_string()),
        (Method::POST, paths::SIGN_OUT.to_string()),
        (Method::POST, paths::TOKENS.to_string()),
        (Method::POST, paths::USERS.to_string()),
    ]);
    // a page on another port of this host, the same site, and another site, as each browser
    // says so
    let elsewhere = [
        ("sec-fetch-site", "same-site".to_string()),
        ("sec-fetch-site", "cross-site".to_string()),
        (
            "origin",
            format!("http://127.0.0.1:{}", ax.address.port() + 1),
        ),
        ("origin", "https://evil.example".to_string()),
        ("origin", "null".to_string()),
    ];
    for (method, path) in &writes {
        for (header, value) in &elsewhere {
            let request = client
                .request(method.clone(), path)
                .header("content-type", "application/json")
                .header(*header, value)
                .body("{}");
            let response = client.raw(request).await?;
            let what = format!("{method} {path} with {header}: {value}");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{what}");
            let error: ApiError = response.json().await?;
            assert_eq!(error.code, ErrorCode::Forbidden, "{what}");
            assert!(
                error.message.contains("its own pages"),
                "{what}: {}",
                error.message
            );
        }
    }
    // nothing they sent was kept
    let (status, listing): (_, Listing) = client.get(&paths::nodes(&plan)).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listing.nodes.is_empty(),
        "a refused write was kept: {listing:?}"
    );
    // a read from another origin goes on, since no page can read what it answers
    let request = client
        .request(Method::GET, &paths::nodes(&plan))
        .header("sec-fetch-site", "cross-site")
        .header("origin", "https://evil.example");
    assert_eq!(client.raw(request).await?.status(), StatusCode::OK);
    // the CLI's write, which names no origin, goes on
    let (status, checked): (_, Checked) = client.post(&paths::nodes(&plan), &work("C1")).await?;
    assert_eq!(status, StatusCode::OK, "{checked:?}");
    // and so does a page of the service's own, as each browser says so
    let ours = [
        ("sec-fetch-site", "same-origin".to_string()),
        ("origin", format!("https://{}", ax.address)),
    ];
    for ((header, value), id) in ours.iter().zip(["C2", "C3"]) {
        let request = client
            .request(Method::POST, &paths::nodes(&plan))
            .header(*header, value)
            .json(&work(id));
        let response = client.raw(request).await?;
        assert_eq!(response.status(), StatusCode::OK, "{header}: {value}");
    }
    let (_, listing): (_, Listing) = client.get(&paths::nodes(&plan)).await?;
    assert_eq!(listing.nodes.len(), 3, "{listing:?}");
    Ok(())
}

/// Every write route of the JSON API refuses a request from a page of another origin, with
/// sign-in on, the caller's token and all, while the CLI's writes go on
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn cross_origin_writes_are_refused<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    refuses_cross_origin(&ax, &ada).await
}

/// With sign-in off, every write route still refuses a request from a page of another origin,
/// which any page a browser opens could send to localhost
///
/// # Arguments
///
/// * `fx` - The fixture, served asking nobody to sign in
pub async fn cross_origin_writes_are_refused_with_sign_in_off<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api().await?;
    refuses_cross_origin(&ax, &ax.client).await
}

/// Signing in over TLS sets a Secure cookie, and clears it Secure too, while over plain loopback
/// the cookie isn't, since a browser drops a Secure cookie set without TLS
///
/// # Arguments
///
/// * `fx` - The fixture, served over TLS beside loopback, asking callers to sign in
pub async fn signing_in_over_tls_sets_a_secure_cookie<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_over_tls().await?;
    first_admin(&ax, "ada").await?;
    let tls = &ax.tls.as_ref().expect("the fixture serves over TLS").client;
    // the sign-in form, posted as the service's own page posts it
    let form = |client: &Client| {
        client
            .request(Method::POST, paths::SIGN_IN)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header("origin", &client.base)
            .body(format!(
                "name=ada&password={}&next=%2F",
                paths::encode(PASSWORD)
            ))
    };
    // over TLS, the cookie is Secure, and signs the browser in there
    let response = tls.raw(form(tls)).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = set_cookie(&response).expect("signing in over TLS set no cookie");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/", "Secure"] {
        assert!(cookie.contains(attribute), "{cookie} lacks {attribute}");
    }
    let pair = cookie.split(';').next().unwrap().to_string();
    let request = tls.request(Method::GET, paths::ME).header(COOKIE, &pair);
    let me: Me = tls.raw(request).await?.json().await?;
    assert_eq!(me.user.map(|user| user.name).as_deref(), Some("ada"));
    // signing out over TLS clears it as it was set
    let request = tls
        .request(Method::POST, paths::SIGN_OUT)
        .header(COOKIE, &pair)
        .header("origin", &tls.base);
    let response = tls.raw(request).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cleared = set_cookie(&response).expect("signing out cleared no cookie");
    assert!(
        cleared.contains("Max-Age=0") && cleared.contains("Secure"),
        "{cleared}"
    );
    // on this host's loopback port too, which speaks HTTPS as well, the cookie is Secure
    let response = ax.client.raw(form(&ax.client)).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = set_cookie(&response).expect("signing in over loopback set no cookie");
    assert!(
        cookie.contains("Secure"),
        "{cookie} isn't Secure over HTTPS"
    );
    // and an origin naming plain HTTP is refused over TLS, the same host and port or not
    let request = tls
        .request(Method::POST, paths::SIGN_OUT)
        .header("origin", tls.base.replace("https://", "http://"));
    assert_eq!(tls.raw(request).await?.status(), StatusCode::FORBIDDEN);
    Ok(())
}

/// Over TLS, a client that offers HTTP/2 is answered over it, eight change streams held open
/// and a read beside them, as a browser's tabs share one connection, on this host's port as on
/// other hosts'; a client that offers only HTTP/1.1 is answered over HTTP/1.1
///
/// # Arguments
///
/// * `fx` - The fixture, served over TLS beside loopback, asking callers to sign in
pub async fn tls_serves_http2_to_a_client_that_offers_it<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_over_tls().await?;
    first_admin(&ax, "ada").await?;
    let (_, token) = try_sign_in(&ax, "ada", PASSWORD).await?;
    let token = token.expect("ada signs in");
    let over = ax.tls.as_ref().expect("the fixture serves over TLS");
    // a client that offers HTTP/2 by ALPN, as a browser does over TLS
    let tls = over.client.signed(&token);
    // eight of the project's change streams held open, as a few plan pages hold theirs
    let path = paths::changes(Some(&ax.plan.project), None);
    let mut streams = Vec::new();
    for _ in 0..8 {
        streams.push(tls.open(&path).await?);
    }
    for stream in &streams {
        assert_eq!(
            (stream.status(), stream.version()),
            (StatusCode::OK, Version::HTTP_2)
        );
    }
    // a read beside them is answered at once, over HTTP/2 too
    let read = tls.raw(tls.request(Method::GET, paths::PROJECTS)).await?;
    assert_eq!(
        (read.status(), read.version()),
        (StatusCode::OK, Version::HTTP_2)
    );
    // and a change made now reaches every stream
    let local = ax.client.signed(&token);
    let (status, _): (_, Checked) = local.post(&paths::nodes(&ax.plan), &work("C2")).await?;
    assert_eq!(status, StatusCode::OK);
    for stream in &mut streams {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.chunk()).await??;
        let chunk = chunk.map(|bytes| String::from_utf8_lossy(&bytes).to_string());
        assert!(
            chunk.as_deref().is_some_and(|text| text.contains("C2")),
            "{chunk:?}"
        );
    }
    // a client that offers only HTTP/1.1 over TLS is answered over it
    let pem = std::fs::read(&over.minted.ca)?;
    let http1 = reqwest::Client::builder()
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_pem(&pem)?)
        .http1_only()
        .build()?;
    let request = http1
        .get(format!("{}{}", over.client.base, paths::PROJECTS))
        .bearer_auth(&token);
    let read = tokio::time::timeout(Duration::from_secs(10), request.send()).await??;
    assert_eq!(
        (read.status(), read.version()),
        (StatusCode::OK, Version::HTTP_11)
    );
    // and this host's loopback port, HTTPS as well, speaks HTTP/2 too
    let read = local
        .raw(local.request(Method::GET, paths::PROJECTS))
        .await?;
    assert_eq!(
        (read.status(), read.version()),
        (StatusCode::OK, Version::HTTP_2)
    );
    Ok(())
}

/// Sends a request's head over plain TCP to a TLS port, and reads the whole answer
///
/// # Arguments
///
/// * `address` - The port's address
/// * `head` - The request's head, up to its blank line
async fn plain_answer(address: std::net::SocketAddr, head: &str) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // the head written, then everything read until the port closes
    let mut stream = tokio::net::TcpStream::connect(address).await?;
    stream.write_all(head.as_bytes()).await?;
    let mut answer = String::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut answer)).await??;
    Ok(answer)
}

/// A plain-HTTP request on a TLS port, this host's or other hosts', is answered with a 307 to
/// https at the host, port and path it asked for, its method kept, and one naming no host with
/// a 400; the port then answers over HTTPS as ever
///
/// # Arguments
///
/// * `fx` - The fixture, served over TLS beside loopback, asking callers to sign in
pub async fn plain_http_is_redirected_to_https<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_over_tls().await?;
    let wide = ax
        .tls
        .as_ref()
        .expect("the fixture serves other hosts")
        .address;
    let wide = std::net::SocketAddr::from(([127, 0, 0, 1], wide.port()));
    // a page asked for on either port, by the name it was called at, through a tunnel or not
    for address in [ax.address, wide] {
        let head = "GET /plans/reef/central?node=C1 HTTP/1.1\r\nHost: localhost:11180\r\n\
                    User-Agent: a case\r\n\r\n";
        let answer = plain_answer(address, head).await?;
        assert!(
            answer.starts_with("HTTP/1.1 307 Temporary Redirect\r\n"),
            "{address}: {answer}"
        );
        assert!(
            answer.contains("\r\nLocation: https://localhost:11180/plans/reef/central?node=C1\r\n"),
            "{address}: {answer}"
        );
        assert!(answer.ends_with(
            "reef answers over HTTPS only: https://localhost:11180/plans/reef/central?node=C1\n"
        ));
        // and a request naming no host, which has nowhere to go, is told to call https
        let answer = plain_answer(address, "GET / HTTP/1.0\r\n\r\n").await?;
        assert!(
            answer.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{answer}"
        );
        assert!(answer.contains("HTTPS only"), "{answer}");
    }
    // a write, as an HTTP client sends it, is sent on with its method and body kept
    let plain = Client::new(format!("http://{}", ax.address));
    let request = plain
        .request(Method::POST, &paths::nodes(&ax.plan))
        .json(&work("C1"));
    let response = plain.raw(request).await?;
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|location| location.to_str().ok())
        .map(str::to_string);
    assert_eq!(
        location,
        Some(format!("https://{}{}", ax.address, paths::nodes(&ax.plan)))
    );
    // and the same port answers over HTTPS beside it, as if nothing happened
    let (status, _): (_, Me) = ax.client.get(paths::ME).await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

/// With no certificate configured and sign-in off, as one person's server on localhost runs,
/// the loopback port serves HTTPS over HTTP/2 with reef's own certificate, and the CLI on the
/// host calls it trusting reef's own CA with no `ca` named
///
/// # Arguments
///
/// * `fx` - The fixture, served asking nobody to sign in
pub async fn this_hosts_port_serves_the_cli_over_https_with_sign_in_off<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api().await?;
    // a client trusting reef's own CA, offering HTTP/2, as a browser that trusts it does
    let read = ax
        .client
        .raw(ax.client.request(Method::GET, paths::ME))
        .await?;
    assert_eq!(
        (read.status(), read.version()),
        (StatusCode::OK, Version::HTTP_2)
    );
    // the CLI on the host, its config naming the address and where reef keeps its CA alone
    let conf = ax.dir.join("cli.toml");
    std::fs::write(&conf, ax.cli_config())?;
    let cli = reef::Client::new(&reef::Conf::load(Some(&conf))?)?;
    assert_eq!(cli.base(), format!("https://{}", ax.address));
    let me = cli.me().await?;
    assert!(!me.required, "{me:?}");
    // and with no CA to trust, it says what to trust rather than that nothing is running
    let mut bare = reef::Conf::load(Some(&conf))?;
    bare.api.certs = Some(ax.dir.join("no-certs-here"));
    let refused = reef::Client::new(&bare)?
        .me()
        .await
        .expect_err("an untrusted certificate was trusted")
        .to_string();
    assert!(refused.contains("certificate"), "{refused}");
    assert!(refused.contains("`[api] ca`"), "{refused}");
    assert!(!refused.contains("isn't running"), "{refused}");
    Ok(())
}

/// With sign-in on, only an admin registers a project by a directory on the service's host or
/// a path a remote names there, a user who isn't one refused before the path is looked at
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn only_an_admin_registers_what_is_on_the_host<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    // ada, the first user and an admin, and bob, who isn't
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    let (status, _): (_, UserAdded) = ada.post(paths::USERS, &new_user("bob", false)).await?;
    assert_eq!(status, StatusCode::CREATED);
    let bob = signed_in(&ax, "bob").await?;
    // a repo of its own, and one published to a bare remote on this host
    let repo = git::repo(&ax.dir.join("here"))?;
    let (bare, _) = git::published(&ax.dir.join("up"), &[("README.md", "# Up\n")])?;
    let url = format!("file://{}", bare.display());
    // bob may name none of them, however the path is written, nor one that isn't there
    let on_host = [
        NewProject::dir("here", repo.clone()),
        NewProject::dir("nowhere", ax.dir.join("nowhere")),
        NewProject::remote("up", url.clone(), None),
        NewProject::remote("bare", bare.display().to_string(), None),
    ];
    for new in &on_host {
        let (status, error): (_, ApiError) = bob.post(paths::PROJECTS, new).await?;
        assert_eq!(
            (status, error.code),
            (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            "{}: {}",
            new.name,
            error.message
        );
        assert!(error.message.contains("only an admin"), "{}", error.message);
    }
    // and nothing was registered
    let (_, projects): (_, Vec<Project>) = ada.get(paths::PROJECTS).await?;
    assert_eq!(projects.len(), 1, "{projects:?}");
    // ada may name both
    for new in [
        NewProject::dir("here", repo),
        NewProject::remote("up", url, None),
    ] {
        let (status, added): (_, AddedProject) = ada.post(paths::PROJECTS, &new).await?;
        assert_eq!(status, StatusCode::CREATED, "{}: {added:?}", new.name);
    }
    Ok(())
}

/// Signs a user in through the JSON API with a password, returning the answer's status and the
/// token issued, if one was
///
/// # Arguments
///
/// * `ax` - The served service
/// * `name` - The user's name
/// * `password` - The password tried
async fn try_sign_in<B: Backend + 'static>(
    ax: &Ax<B>,
    name: &str,
    password: &str,
) -> anyhow::Result<(StatusCode, Option<String>)> {
    let request = SignIn {
        name: name.to_string(),
        password: password.to_string(),
        label: Some(format!("{name}'s case")),
    };
    let response = ax
        .client
        .raw(
            ax.client
                .request(Method::POST, paths::TOKENS)
                .json(&request),
        )
        .await?;
    let status = response.status();
    // a token only with a 201
    let token = match status == StatusCode::CREATED {
        true => Some(response.json::<IssuedToken>().await?.token),
        false => None,
    };
    Ok((status, token))
}

/// Adds a user as the admin a client signs in, with the cases' password
///
/// # Arguments
///
/// * `admin` - A client signed in as an admin
/// * `name` - The new user's name
/// * `is_admin` - Whether they are to be an admin
async fn add_by(admin: &Client, name: &str, is_admin: bool) -> anyhow::Result<()> {
    let (status, added): (_, UserAdded) =
        admin.post(paths::USERS, &new_user(name, is_admin)).await?;
    anyhow::ensure!(
        status == StatusCode::CREATED,
        "adding `{name}` answered {status}: {added:?}"
    );
    Ok(())
}

/// The id a listing shows a token by
///
/// # Arguments
///
/// * `token` - The token
fn id_of(token: &str) -> String {
    token_hash(token)[..reef_api::user::TOKEN_ID_LEN].to_string()
}

/// Whether a token still signs anyone in, as `/api/v1/me` says
///
/// # Arguments
///
/// * `ax` - The served service
/// * `token` - The token
async fn signs_in<B: Backend + 'static>(ax: &Ax<B>, token: &str) -> anyhow::Result<bool> {
    let (_, me): (_, Me) = ax.client.signed(token).get(paths::ME).await?;
    Ok(me.user.is_some())
}

/// A token revoked by its id signs nothing in at once, though it was looked up a moment before,
/// whether its user or an admin revokes it, and a browser's cookie as well as a terminal's
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn a_revoked_token_signs_nothing_in<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    // two terminals of bob's, each looked up so the service trusts it for a while
    let (_, first) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let (_, second) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let (first, second) = (first.expect("bob signs in"), second.expect("bob signs in"));
    assert!(signs_in(&ax, &first).await? && signs_in(&ax, &second).await?);
    // bob revokes his second by its id, from his first, and it stops at once
    let bob = ax.client.signed(&first);
    let response = bob
        .raw(bob.request(Method::DELETE, &paths::token(&id_of(&second))))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let revoked: SignedOut = response.json().await?;
    assert!(revoked.revoked);
    is_refused(
        ax.client.signed(&second).open(paths::PROJECTS).await?,
        "a revoked token",
    )
    .await?;
    assert!(!signs_in(&ax, &second).await?);
    // revoked again, it is nothing of his
    let response = bob
        .raw(bob.request(Method::DELETE, &paths::token(&id_of(&second))))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // an admin revokes his first, and a write with it is refused
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::token(&id_of(&first))))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let request = bob
        .request(Method::POST, &paths::nodes(&ax.plan))
        .json(&work("C9"));
    is_refused(bob.raw(request).await?, "a write with a revoked token").await?;
    // a browser's cookie revoked by its id sends the browser back to sign in
    let form = ax
        .client
        .request(Method::POST, paths::SIGN_IN)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(format!(
            "name=bob&password={}&next=%2F",
            PASSWORD.replace(' ', "+")
        ));
    let signed = ax.client.raw(form).await?;
    let cookie = set_cookie(&signed).expect("the form sets a cookie");
    let token = cookie
        .split(';')
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, token)| token.to_string())
        .expect("the cookie holds a token");
    let page = |token: &str| {
        ax.client
            .request(Method::GET, "/")
            .header(ACCEPT, "text/html")
            .header(COOKIE, format!("{SIGN_IN_COOKIE}={token}"))
    };
    let (_, listed): (_, Vec<TokenInfo>) = ada.get(&paths::tokens_of("bob")).await?;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].kind, TokenKind::Browser);
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::token(&listed[0].id)))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let sent_back = ax.client.raw(page(&token)).await?;
    assert_eq!(sent_back.status(), StatusCode::SEE_OTHER);
    let location = sent_back
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok());
    assert!(
        location.is_some_and(|location| location.starts_with(paths::SIGN_IN)),
        "{location:?}"
    );
    Ok(())
}

/// Repeated wrong passwords are slowed: after the free failures the next sign-in waits, its
/// password unchecked, said as a 429 with how long, while another name signs in, and once the
/// wait passes the right password signs in again
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn repeated_wrong_passwords_are_slowed<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    // five wrong passwords are each told so
    for _ in 0..5 {
        let (status, _) = try_sign_in(&ax, "bob", "not the password").await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // then even the right one waits, told how long
    let request = SignIn {
        name: "bob".to_string(),
        password: PASSWORD.to_string(),
        label: None,
    };
    let response = ax
        .client
        .raw(
            ax.client
                .request(Method::POST, paths::TOKENS)
                .json(&request),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry = response
        .headers()
        .get(RETRY_AFTER)
        .map(|value| value.to_str().unwrap_or_default().to_string());
    assert_eq!(retry.as_deref(), Some("1"));
    let error: ApiError = response.json().await?;
    assert_eq!(
        (error.code, error.retry_after),
        (ErrorCode::TooManyTries, Some(1))
    );
    assert!(
        error.message.contains("try again in 1 s"),
        "{}",
        error.message
    );
    // and the browser's form says the same, with the page again
    let form = ax
        .client
        .request(Method::POST, paths::SIGN_IN)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(format!(
            "name=bob&password={}&next=%2F",
            PASSWORD.replace(' ', "+")
        ));
    let page = ax.client.raw(form).await?;
    assert_eq!(page.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(page.headers().contains_key(RETRY_AFTER));
    assert!(set_cookie(&page).is_none());
    assert!(page.text().await?.contains("try again in"));
    // another name signs in meanwhile, from the same address
    let (status, _) = try_sign_in(&ax, "ada", PASSWORD).await?;
    assert_eq!(status, StatusCode::CREATED);
    // once the wait passes, the right password signs bob in, and his failures are forgotten
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (status, token) = try_sign_in(&ax, "bob", PASSWORD).await?;
    assert_eq!(status, StatusCode::CREATED);
    assert!(token.is_some());
    let (status, _) = try_sign_in(&ax, "bob", "not the password").await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}

/// A user changes their own password with their current one, keeping their tokens, and an
/// admin sets another's, revoking theirs; nobody else sets anyone's
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn passwords_are_changed_by_their_user_or_set_by_an_admin<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    let (_, bob_token) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let bob_token = bob_token.expect("bob signs in");
    let bob = ax.client.signed(&bob_token);
    let change = |current: Option<&str>, password: &str| PasswordChange {
        current: current.map(str::to_string),
        password: password.to_string(),
    };
    let new = "a newer horse battery";
    // his own needs his current password, right, and a new one the service keeps
    let (status, error): (_, ApiError) = bob
        .put(&paths::user_password("bob"), &change(None, new))
        .await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
    );
    let wrong = change(Some("not the password"), new);
    let (status, error): (_, ApiError) = bob.put(&paths::user_password("bob"), &wrong).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    let short = change(Some(PASSWORD), "short");
    let (status, _): (_, ApiError) = bob.put(&paths::user_password("bob"), &short).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // with it, the new one signs him in, the old one doesn't, and his token still does
    let (status, changed): (_, PasswordChanged) = bob
        .put(&paths::user_password("bob"), &change(Some(PASSWORD), new))
        .await?;
    assert_eq!((status, changed.revoked), (StatusCode::OK, 0));
    assert_eq!(
        try_sign_in(&ax, "bob", PASSWORD).await?.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(try_sign_in(&ax, "bob", new).await?.0, StatusCode::CREATED);
    assert!(signs_in(&ax, &bob_token).await?);
    // he sets nobody else's, and nobody signed in sets any
    let (status, error): (_, ApiError) = bob
        .put(&paths::user_password("ada"), &change(None, new))
        .await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    let request = ax
        .client
        .request(Method::PUT, &paths::user_password("bob"))
        .json(&change(Some(new), PASSWORD));
    is_refused(
        ax.client.raw(request).await?,
        "an anonymous password change",
    )
    .await?;
    // an admin sets his, needing no current one, and both his tokens stop
    let (status, changed): (_, PasswordChanged) = ada
        .put(&paths::user_password("bob"), &change(None, PASSWORD))
        .await?;
    assert_eq!((status, changed.revoked), (StatusCode::OK, 2));
    assert!(!signs_in(&ax, &bob_token).await?);
    assert_eq!(
        try_sign_in(&ax, "bob", new).await?.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        try_sign_in(&ax, "bob", PASSWORD).await?.0,
        StatusCode::CREATED
    );
    // and a user nobody has is said so, to an admin
    let (status, error): (_, ApiError) = ada
        .put(&paths::user_password("nobody"), &change(None, new))
        .await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    // an admin's own is their own, and needs their current one
    let (status, _): (_, ApiError) = ada
        .put(&paths::user_password("ada"), &change(None, new))
        .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

/// An admin removes a user, whose tokens stop at once; the last admin stays; nobody else removes
/// anyone; and a new user of the same name is signed in by no token of the old one's
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn an_admin_removes_a_user_and_their_tokens<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    add_by(&ada, "carol", false).await?;
    let (_, bob_token) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let bob_token = bob_token.expect("bob signs in");
    assert!(signs_in(&ax, &bob_token).await?);
    // a user who isn't an admin removes nobody, and nobody signed in is told to sign in
    let carol = signed_in(&ax, "carol").await?;
    let response = carol
        .raw(carol.request(Method::DELETE, &paths::user("bob")))
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ApiError = response.json().await?;
    assert_eq!(error.code, ErrorCode::Forbidden);
    assert!(signs_in(&ax, &bob_token).await?);
    let request = ax.client.request(Method::DELETE, &paths::user("bob"));
    is_refused(ax.client.raw(request).await?, "an anonymous removal").await?;
    // an admin removes bob, and his token, trusted a moment ago, stops at once
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::user("bob")))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let removed: UserRemoved = response.json().await?;
    assert_eq!(removed, UserRemoved::Removed { revoked: 1 });
    assert!(!signs_in(&ax, &bob_token).await?);
    assert_eq!(
        try_sign_in(&ax, "bob", PASSWORD).await?.0,
        StatusCode::UNAUTHORIZED
    );
    // gone, he is said so
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::user("bob")))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.json::<UserRemoved>().await?,
        UserRemoved::NoSuchUser
    );
    // the last admin stays, until there is another
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::user("ada")))
        .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.json::<UserRemoved>().await?,
        UserRemoved::LastAdmin
    );
    // a token kept for a bob before the new bob was added signs the new one in no more
    let stale = format!("{}{}", reef_api::user::TOKEN_PREFIX, "5a".repeat(32));
    ax.service
        .store()
        .keep_token(
            &token_hash(&stale),
            "bob",
            TokenKind::Cli,
            "an old bob's",
            None,
        )
        .await?;
    tokio::time::sleep(Duration::from_millis(5)).await;
    add_by(&ada, "bob", false).await?;
    assert!(!signs_in(&ax, &stale).await?);
    is_refused(
        ax.client.signed(&stale).open(paths::PROJECTS).await?,
        "an old bob's token",
    )
    .await?;
    assert_eq!(ax.service.store().token(&token_hash(&stale)).await?, None);
    // and the new bob signs in with his own
    assert_eq!(
        try_sign_in(&ax, "bob", PASSWORD).await?.0,
        StatusCode::CREATED
    );
    Ok(())
}

/// Everyone lists their own tokens, oldest first, the one sent marked and no token shown, and
/// only an admin lists or revokes another user's
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn tokens_are_listed_and_revoked_by_their_user<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    let (_, first) = try_sign_in(&ax, "bob", PASSWORD).await?;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let (_, second) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let (first, second) = (first.expect("bob signs in"), second.expect("bob signs in"));
    // bob's own, from his second, oldest first, the second marked and no token in the answer
    let bob = ax.client.signed(&second);
    let response = bob.raw(bob.request(Method::GET, paths::TOKENS)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await?;
    assert!(!text.contains(&first) && !text.contains(&second), "{text}");
    let listed: Vec<TokenInfo> = serde_json::from_str(&text)?;
    let seen: Vec<_> = listed
        .iter()
        .map(|token| {
            (
                token.id.clone(),
                token.user.as_str(),
                token.kind,
                token.label.as_str(),
                token.current,
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (id_of(&first), "bob", TokenKind::Cli, "bob's case", false),
            (id_of(&second), "bob", TokenKind::Cli, "bob's case", true),
        ]
    );
    // he lists nobody else's, and nobody signed in lists any
    let (status, error): (_, ApiError) = bob.get(&paths::tokens_of("ada")).await?;
    assert_eq!(
        (status, error.code),
        (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
    );
    is_refused(ax.client.open(paths::TOKENS).await?, "an anonymous listing").await?;
    // nor revokes another's: an admin's token is no token of his
    let (_, ada_tokens): (_, Vec<TokenInfo>) = ada.get(paths::TOKENS).await?;
    assert_eq!(ada_tokens.len(), 1);
    assert!(ada_tokens[0].current);
    let response = bob
        .raw(bob.request(Method::DELETE, &paths::token(&ada_tokens[0].id)))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let (_, still): (_, Vec<TokenInfo>) = ada.get(paths::TOKENS).await?;
    assert_eq!(still.len(), 1);
    // an id of no shape names nothing
    let response = bob
        .raw(bob.request(Method::DELETE, &paths::token("zz")))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // an admin lists bob's, marking none, and revokes one
    let (status, bobs): (_, Vec<TokenInfo>) = ada.get(&paths::tokens_of("bob")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bobs.len(), 2);
    assert!(bobs.iter().all(|token| !token.current));
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::token(&id_of(&first))))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let (_, left): (_, Vec<TokenInfo>) = bob.get(paths::TOKENS).await?;
    assert_eq!(
        left.iter()
            .map(|token| token.id.clone())
            .collect::<Vec<_>>(),
        [id_of(&second)]
    );
    Ok(())
}

/// Reads a change stream until it ends, or until a while passes, returning whether it ended
///
/// # Arguments
///
/// * `stream` - The stream's answer
/// * `within` - How long to wait for its end
async fn ends(stream: &mut reqwest::Response, within: Duration) -> anyhow::Result<bool> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        // each chunk read, until none is left or the wait is over
        match tokio::time::timeout_at(deadline, stream.chunk()).await {
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => return Ok(true),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Ok(false),
        }
    }
}

/// An open change stream ends once its token is revoked, while another's stays open and is
/// sent the changes after
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn a_change_stream_ends_when_its_token_is_revoked<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    let (_, bob_token) = try_sign_in(&ax, "bob", PASSWORD).await?;
    let bob_token = bob_token.expect("bob signs in");
    let bob = ax.client.signed(&bob_token);
    // a stream each, open
    let mut bobs = bob.open(paths::CHANGES).await?;
    let mut adas = ada.open(paths::CHANGES).await?;
    assert_eq!(
        (bobs.status(), adas.status()),
        (StatusCode::OK, StatusCode::OK)
    );
    assert!(!ends(&mut bobs, Duration::from_millis(200)).await?);
    // bob's token revoked by an admin ends his stream, at once
    let response = ada
        .raw(ada.request(Method::DELETE, &paths::token(&id_of(&bob_token))))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        ends(&mut bobs, Duration::from_secs(5)).await?,
        "bob's stream outlived his token"
    );
    // and opening another is refused
    is_refused(
        bob.open(paths::CHANGES).await?,
        "a stream with a revoked token",
    )
    .await?;
    // ada's is still open, and is sent the next change
    let (status, _): (_, Checked) = ada.post(&paths::nodes(&ax.plan), &work("C2")).await?;
    assert_eq!(status, StatusCode::OK);
    let chunk = tokio::time::timeout(Duration::from_secs(5), adas.chunk()).await??;
    let chunk = chunk.map(|bytes| String::from_utf8_lossy(&bytes).to_string());
    assert!(
        chunk.as_deref().is_some_and(|text| text.contains("C2")),
        "{chunk:?}"
    );
    Ok(())
}

/// A lost admin is recovered from the store itself: their password set, an admin made of them,
/// their tokens revoked, or a new admin added for a name nobody has
///
/// # Arguments
///
/// * `fx` - The fixture, served asking callers to sign in
pub async fn the_host_recovers_a_lost_admin<B: Backend + 'static>(fx: Fx<B>) -> anyhow::Result<()> {
    let ax = fx.api_signing_in().await?;
    first_admin(&ax, "ada").await?;
    let ada = signed_in(&ax, "ada").await?;
    add_by(&ada, "bob", false).await?;
    let (_, ada_token) = try_sign_in(&ax, "ada", PASSWORD).await?;
    let ada_token = ada_token.expect("ada signs in");
    // ada's password is lost: recovered, her tokens stop and the new one signs her in
    let new = "a recovered horse battery";
    let recovered = ax.service.recover_admin("ada", new.to_string()).await?;
    assert_eq!(
        recovered,
        Recovered::Reset {
            revoked: 2,
            was_admin: true
        }
    );
    assert!(!signs_in(&ax, &ada_token).await?);
    assert_eq!(
        try_sign_in(&ax, "ada", PASSWORD).await?.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, token) = try_sign_in(&ax, "ada", new).await?;
    assert_eq!(status, StatusCode::CREATED);
    let ada = ax.client.signed(&token.expect("ada signs in"));
    // bob, recovered, is an admin from now on
    let recovered = ax.service.recover_admin("bob", new.to_string()).await?;
    assert_eq!(
        recovered,
        Recovered::Reset {
            revoked: 0,
            was_admin: false
        }
    );
    let (_, users): (_, Vec<User>) = ada.get(paths::USERS).await?;
    let admins: Vec<_> = users
        .iter()
        .filter(|user| user.admin)
        .map(|user| user.name.as_str())
        .collect();
    assert_eq!(admins, ["ada", "bob"]);
    // a name nobody has is added, as an admin
    let recovered = ax.service.recover_admin("zed", new.to_string()).await?;
    assert_eq!(recovered, Recovered::Added);
    let (status, token) = try_sign_in(&ax, "zed", new).await?;
    assert_eq!(status, StatusCode::CREATED);
    let (_, me): (_, Me) = ax
        .client
        .signed(&token.expect("zed signs in"))
        .get(paths::ME)
        .await?;
    assert!(me.user.is_some_and(|user| user.admin));
    // a password the service can't keep is refused, and changes nothing
    let refused = ax
        .service
        .recover_admin("ada", "short".to_string())
        .await
        .expect_err("kept a short password");
    assert!(
        refused.downcast_ref::<Unkeepable>().is_some(),
        "{refused:#}"
    );
    assert_eq!(try_sign_in(&ax, "ada", new).await?.0, StatusCode::CREATED);
    Ok(())
}

/// The `reef` a case's service hands its users: a fake client in the case's own directory,
/// stamped with a commit and its time, read as the service reads a release's
///
/// # Arguments
///
/// * `fx` - The fixture, whose directory holds the client
async fn served_client<B: Backend>(fx: &Fx<B>) -> anyhow::Result<Served> {
    let build = stamped("dc5e68cd25c95db18f885357f505e97af24175f9", 1_795_800_000);
    let path = fake_client(&fx.dir.join("release/bin/reef"), &build)?;
    Served::read(&path).await
}

/// A service handing its users a client says its build and sends its bytes, as a release's, and
/// names the build on every answer the API gives, its refusals and errors too
///
/// # Arguments
///
/// * `fx` - The fixture, served asking no caller to sign in
pub async fn the_served_client_is_its_build_its_bytes_and_a_header<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let served = served_client(&fx).await?;
    let ax = fx.api_serving(false, served.clone()).await?;
    let named = "dc5e68cd25c95db18f885357f505e97af24175f9 1795800000";
    // its build, as the client said it, named in the answer's header too
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, paths::CLIENT))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[paths::CLIENT_HEADER], named);
    let build: Build = response.json().await?;
    assert_eq!(build, served.build);
    // its bytes, whole, as a file to save
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, paths::CLIENT_BINARY))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[CONTENT_TYPE], "application/octet-stream");
    assert_eq!(headers[CONTENT_LENGTH], served.size.to_string());
    assert_eq!(
        headers[CONTENT_DISPOSITION],
        "attachment; filename=\"reef\""
    );
    let bytes = response.bytes().await?;
    assert_eq!(bytes.as_ref(), std::fs::read(&served.path)?.as_slice());
    // and every other answer names it, a listing, a write and an error alike
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, paths::PROJECTS))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[paths::CLIENT_HEADER], named);
    let response = ax
        .client
        .raw(
            ax.client
                .request(Method::POST, &paths::nodes(&ax.plan))
                .json(&work("C1")),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[paths::CLIENT_HEADER], named);
    let missing = paths::node(&ax.plan, "C404");
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, &missing))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()[paths::CLIENT_HEADER], named);
    Ok(())
}

/// Both client routes answer a caller with no token on a server that asks every caller to sign
/// in, so a new machine fetches `reef` before it can, while the rest still refuse it
///
/// # Arguments
///
/// * `fx` - The fixture, served asking every caller to sign in
pub async fn the_client_routes_answer_without_a_token<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let served = served_client(&fx).await?;
    let ax = fx.api_serving(true, served.clone()).await?;
    // no token sent, and both answer
    let (status, build): (_, Build) = ax.client.get(paths::CLIENT).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(build, served.build);
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, paths::CLIENT_BINARY))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await?.len() as u64, served.size);
    // a HEAD of either too, as a download's first look
    let response = ax
        .client
        .raw(ax.client.request(Method::HEAD, paths::CLIENT_BINARY))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    // but only those: the plan still asks who is calling
    let (status, error): (_, ApiError) = ax.client.get(paths::PROJECTS).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error.code, ErrorCode::Unauthorized);
    // and a write to either is no read
    let response = ax
        .client
        .raw(ax.client.request(Method::POST, paths::CLIENT_BINARY))
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

/// A service handing its users no client says so on its route, and names no build on any answer
///
/// # Arguments
///
/// * `fx` - The fixture, served asking no caller to sign in
pub async fn with_no_client_the_route_says_not_found<B: Backend + 'static>(
    fx: Fx<B>,
) -> anyhow::Result<()> {
    let ax = fx.api().await?;
    // the build's route, and the bytes', are not found, naming the key that would serve one
    let (status, error): (_, ApiError) = ax.client.get(paths::CLIENT).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(
        error.message.contains("`[client] reef`"),
        "{}",
        error.message
    );
    let (status, _): (_, ApiError) = ax.client.get(paths::CLIENT_BINARY).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // and no answer names a build
    let response = ax
        .client
        .raw(ax.client.request(Method::GET, paths::PROJECTS))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(paths::CLIENT_HEADER).is_none());
    Ok(())
}
