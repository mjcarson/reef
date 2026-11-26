//! The app's server asking every caller to sign in: one layer guards its pages, its server
//! functions and the JSON API it mounts, refusing writes from other origins, and the page a
//! browser signs in on is the service's
//!
//! The service is the process's own, so this suite runs in a binary of its own, apart from
//! `app.rs`, whose server asks nobody to sign in.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_api::{Me, NewUser, UserAdded, paths};
use reef_service::{Asker, Conf, Service, api};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN, SET_COOKIE};
use std::net::SocketAddr;
use std::path::Path;

/// The password the suite's user signs in with
const PASSWORD: &str = "correct horse battery";

/// A comment the signed-in browser starts, and a node it saves, name the user its cookie signs in
///
/// # Arguments
///
/// * `service` - The service the app serves
/// * `ada` - The user signed in, an admin
/// * `dir` - The suite's own directory, which a repo is made in
/// * `base` - The app's root
/// * `http` - The browser's client
/// * `cookie` - The sign-in cookie it sends
async fn authored_by_the_browser(
    service: &Service,
    ada: &reef_api::User,
    dir: &Path,
    base: &str,
    http: &reqwest::Client,
    cookie: &str,
) -> anyhow::Result<()> {
    // a project registered by ada for a fresh repo, its plan listed, and a node in it
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo)?;
    let made = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(&repo)
        .status()?;
    anyhow::ensure!(made.success(), "git init failed");
    let admin = Asker {
        user: Some(ada),
        local: true,
        ..Default::default()
    };
    service.add_project("reef", &repo, admin).await?;
    let plan = reef_model::PlanName::new("reef", "central")?;
    service.add_plan(&plan, &reef_api::NOBODY).await?;
    let spec = reef_model::NodeSpec::from_yaml(
        "id: C1\ntitle: C1 Some work\ndescription:\n  why: Missing.\n  what: There.\n",
    )
    .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    let node = reef_api::NodeWrite {
        spec,
        body: String::new(),
        order: 0,
    };
    service
        .put(
            &plan,
            vec![reef_api::NodePut::new(node, None)],
            &reef_api::NOBODY,
        )
        .await?;
    // the browser starts a thread on it, and saves it in progress, as its own pages do
    let post = |path: &str, body: serde_json::Value| {
        http.post(format!("{base}{path}"))
            .header(CONTENT_TYPE, "application/json")
            .header(COOKIE, cookie)
            .header("sec-fetch-site", "same-origin")
            .json(&body)
    };
    let thread = serde_json::json!({
        "project": "reef",
        "plan": "central",
        "id": "C1",
        "target": {"on": "node"},
        "body": "Mine",
        "review": false,
    });
    let answer = post("/app/thread", thread).send().await?;
    anyhow::ensure!(answer.status().is_success(), "{}", answer.text().await?);
    let yaml = "id: C1\ntitle: C1 Some work\nstatus: in-progress\ndescription:\n  why: \
                Missing.\n  what: There.\n";
    let save = serde_json::json!({
        "project": "reef",
        "plan": "central",
        "id": "C1",
        "yaml": yaml,
        "read": 1,
    });
    let answer = post("/app/save", save).send().await?;
    anyhow::ensure!(answer.status().is_success(), "{}", answer.text().await?);
    // each names ada, by her name
    let named = plan.node("C1")?;
    let threads = service
        .node_threads(&named, reef_service::Pending::Leave)
        .await?;
    assert_eq!(
        threads[0].comments[0].author.describe().as_deref(),
        Some("ada")
    );
    let history = service
        .node_history(&named, &reef_api::HistoryQuery::default())
        .await?
        .edits;
    assert_eq!(history[0].author.describe().as_deref(), Some("ada"));
    assert_eq!(history[0].version, Some(2));
    // and nobody, for the put the suite made itself
    assert!(history[1].author.is_unknown());
    Ok(())
}

/// One layer guards the app's pages, its server functions and the JSON API: nobody signed in is
/// sent to sign in or refused, a browser that signs in is shown whom it is signed in as, and a
/// post from a page of another origin is refused, its cookie and all
#[test]
fn one_layer_guards_the_app_and_the_api() -> anyhow::Result<()> {
    // a fresh directory under cargo's target dir, rather than a tmpfs `/tmp`
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reef-app-sign-in");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    // Dioxus serves the client's files from `public` beside the binary, which the suite's has not
    let exe = std::env::current_exe()?;
    let public = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no directory", exe.display()))?
        .join("public");
    std::fs::create_dir_all(public)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // the service over a pool of its own, asking every caller to sign in
        let mut conf = Conf::default();
        conf.shoal.data = Some(dir.join("shoal"));
        conf.shoal.cores = 1;
        conf.auth.required = true;
        let service = Service::shared(&conf).await?;
        // its first user, from this host
        let new = NewUser {
            name: "ada".to_string(),
            password: PASSWORD.to_string(),
            admin: false,
        };
        let local = Asker {
            user: None,
            local: true,
            ..Default::default()
        };
        let UserAdded::Added(ada) = service.add_user(new, local).await? else {
            anyhow::bail!("ada was taken in a fresh service");
        };
        // the app's router on any free local port, each request knowing where it came from
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address: SocketAddr = listener.local_addr()?;
        let app = reef_app::server::router(service);
        tokio::spawn(api::serve_router(
            app,
            listener.into(),
            std::future::pending(),
        ));
        let base = format!("http://{address}");
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let get = |path: &str| http.get(format!("{base}{path}"));
        // nobody signed in opening a page is sent to sign in, and back once they have
        for (path, next) in [
            ("/", "%2F"),
            ("/plans/reef/central", "%2Fplans%2Freef%2Fcentral"),
            ("/account", "%2Faccount"),
        ] {
            let answer = get(path).header(ACCEPT, "text/html").send().await?;
            assert_eq!(answer.status(), StatusCode::SEE_OTHER, "GET {path}");
            let location = answer
                .headers()
                .get(LOCATION)
                .unwrap()
                .to_str()?
                .to_string();
            assert_eq!(location, format!("/sign-in?next={next}"), "GET {path}");
        }
        // and the server functions and the JSON API refuse them, a server function even when
        // a page asks for one as it would a page
        let html = get("/app/projects")
            .header(ACCEPT, "text/html")
            .send()
            .await?;
        assert_eq!(html.status(), StatusCode::UNAUTHORIZED);
        for (method, path) in [
            (reqwest::Method::GET, "/app/projects"),
            (reqwest::Method::GET, "/app/me"),
            (reqwest::Method::POST, "/app/save"),
            (reqwest::Method::GET, "/app/account"),
            (reqwest::Method::POST, "/app/account/password"),
            (reqwest::Method::POST, "/app/account/revoke"),
            (reqwest::Method::POST, "/app/account/remove"),
            (reqwest::Method::GET, paths::PROJECTS),
            (reqwest::Method::POST, paths::PROJECTS),
        ] {
            let answer = http
                .request(method.clone(), format!("{base}{path}"))
                .header(CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await?;
            assert_eq!(answer.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
        }
        // the stylesheet and the static files answer anyone, and the sign-in page is served
        assert_eq!(get("/reef.css").send().await?.status(), StatusCode::OK);
        assert_eq!(
            get("/assets/none.js").send().await?.status(),
            StatusCode::NOT_FOUND
        );
        let page = get(paths::SIGN_IN)
            .header(ACCEPT, "text/html")
            .send()
            .await?;
        assert_eq!(page.status(), StatusCode::OK);
        assert!(page.text().await?.contains("Sign in to reef"));
        // signing in sets the cookie and goes on where the browser was going
        let answer = http
            .post(format!("{base}{}", paths::SIGN_IN))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(format!(
                "name=ada&password={}&next=%2F",
                paths::encode(PASSWORD)
            ))
            .send()
            .await?;
        assert_eq!(answer.status(), StatusCode::SEE_OTHER);
        assert_eq!(answer.headers().get(LOCATION).unwrap(), "/");
        let cookie = answer
            .headers()
            .get(SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .expect("signing in set a cookie")
            .to_string();
        // so the front page renders, saying whom it is signed in as, as the server renders it
        let answer = get("/")
            .header(ACCEPT, "text/html")
            .header(COOKIE, &cookie)
            .send()
            .await?;
        assert_eq!(answer.status(), StatusCode::OK);
        let body = answer.text().await?;
        assert!(body.contains("reef-app-user-name"), "{body}");
        assert!(body.contains(">ada<"), "{body}");
        assert!(body.contains("Sign out"), "{body}");
        // and the server functions and the JSON API answer it
        let me: Me = get("/app/me")
            .header(COOKIE, &cookie)
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(me.user.map(|user| user.name).as_deref(), Some("ada"));
        assert!(me.required);
        let answer = get(paths::PROJECTS).header(COOKIE, &cookie).send().await?;
        assert_eq!(answer.status(), StatusCode::OK);
        // the account page renders her password's form and her token, this browser's
        let answer = get("/account")
            .header(ACCEPT, "text/html")
            .header(COOKIE, &cookie)
            .send()
            .await?;
        assert_eq!(answer.status(), StatusCode::OK);
        let body = answer.text().await?;
        assert!(
            body.contains("Your password") && body.contains("Your tokens"),
            "{body}"
        );
        assert!(body.contains("this browser"), "{body}");
        let account: reef_app::calls::Account = get("/app/account")
            .header(COOKIE, &cookie)
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(account.tokens.len(), 1);
        assert!(account.tokens[0].current);
        assert_eq!(account.users.len(), 1, "an admin sees every user");
        // a comment and a save the signed-in browser makes name her
        authored_by_the_browser(&service, &ada, &dir, &base, &http, &cookie).await?;
        // but a page on another port of this host, the same site, posts with the cookie and is
        // refused, whether its browser says so or only names its origin
        let elsewhere = "http://127.0.0.1:1";
        for (path, body) in [
            ("/app/save", "{}"),
            ("/app/queue", "{}"),
            ("/app/account/password", "{}"),
            (paths::PROJECTS, "{}"),
            (paths::SIGN_OUT, ""),
        ] {
            for (header, value) in [("sec-fetch-site", "same-site"), ("origin", elsewhere)] {
                let answer = http
                    .post(format!("{base}{path}"))
                    .header(CONTENT_TYPE, "text/plain")
                    .header(COOKIE, &cookie)
                    .header(header, value)
                    .body(body)
                    .send()
                    .await?;
                assert_eq!(answer.status(), StatusCode::FORBIDDEN, "{path} {header}");
                let text = answer.text().await?;
                assert!(text.contains("its own pages"), "{path}: {text}");
            }
        }
        // as is a sign-in form another origin posts, which would sign the browser in as another
        let answer = http
            .post(format!("{base}{}", paths::SIGN_IN))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ORIGIN, elsewhere)
            .body(format!("name=ada&password={}", paths::encode(PASSWORD)))
            .send()
            .await?;
        assert_eq!(answer.status(), StatusCode::FORBIDDEN);
        assert!(answer.headers().get(SET_COOKIE).is_none());
        // the refused sign-out left the cookie signing ada in
        let me: Me = get("/app/me")
            .header(COOKIE, &cookie)
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(me.user.map(|user| user.name).as_deref(), Some("ada"));
        // and the app's own page signs out, as its browser says it is the same origin
        let answer = http
            .post(format!("{base}{}", paths::SIGN_OUT))
            .header(COOKIE, &cookie)
            .header("sec-fetch-site", "same-origin")
            .header(ORIGIN, &base)
            .send()
            .await?;
        assert_eq!(answer.status(), StatusCode::SEE_OTHER);
        anyhow::Ok(())
    })
}
