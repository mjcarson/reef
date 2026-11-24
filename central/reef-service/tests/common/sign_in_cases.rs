//! The sign-in suite's cases, each generic over the backend so it runs against shoal and the twin
//!
//! Each case serves its fixture's service on a loopback port of its own, asking every caller to
//! sign in unless the case is about sign-in off, and calls it as the CLI and a browser do: with a
//! token as `Authorization: Bearer`, with the sign-in cookie, or with nothing.

use reef_api::user::SIGN_IN_COOKIE;
use reef_api::{
    AddedProject, ApiError, Checked, ErrorCode, IssuedToken, Listing, Me, NewProject, NewUser,
    Project, SignIn, SignedOut, TokenKind, User, UserAdded, paths,
};
use reef_model::{Description, NodeSpec, PlanName};
use reef_service::{Asker, Backend};
use reqwest::header::{
    ACCEPT, CONTENT_SECURITY_POLICY, COOKIE, LOCATION, SET_COOKIE, WWW_AUTHENTICATE,
};
use reqwest::{Method, StatusCode};

use super::Fx;
use super::git;
use super::http::{Ax, Client};
use reef_api::{NodePut, NodeWrite, Puts};

/// The password every case's users sign in with
const PASSWORD: &str = "correct horse battery";

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
fn new_user(name: &str, admin: bool) -> NewUser {
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
async fn first_admin<B: Backend + 'static>(ax: &Ax<B>, name: &str) -> anyhow::Result<User> {
    // the first user, from the service's own host
    let asker = Asker {
        user: None,
        local: true,
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
async fn signed_in<B: Backend + 'static>(ax: &Ax<B>, name: &str) -> anyhow::Result<Client> {
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
    assert!(
        !cookie.contains("Secure"),
        "{cookie} is Secure with no TLS to send it over"
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
        ("origin", format!("http://{}", ax.address)),
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
    // over plain loopback, the cookie isn't Secure
    let response = ax.client.raw(form(&ax.client)).await?;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = set_cookie(&response).expect("signing in over loopback set no cookie");
    assert!(!cookie.contains("Secure"), "{cookie} is Secure with no TLS");
    // and an origin naming plain HTTP is refused over TLS, the same host and port or not
    let request = tls
        .request(Method::POST, paths::SIGN_OUT)
        .header("origin", tls.base.replace("https://", "http://"));
    assert_eq!(tls.raw(request).await?.status(), StatusCode::FORBIDDEN);
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
