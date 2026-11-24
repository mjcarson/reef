//! The app's server asking every caller to sign in: one layer guards its pages, its server
//! functions and the JSON API it mounts, and the page a browser signs in on is the service's
//!
//! The service is the process's own, so this suite runs in a binary of its own, apart from
//! `app.rs`, whose server asks nobody to sign in.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_api::{Me, NewUser, UserAdded, paths};
use reef_service::{Asker, Conf, Service};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use std::net::SocketAddr;
use std::path::Path;

/// The password the suite's user signs in with
const PASSWORD: &str = "correct horse battery";

/// One layer guards the app's pages, its server functions and the JSON API: nobody signed in is
/// sent to sign in or refused, and a browser that signs in is shown whom it is signed in as
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
        };
        anyhow::ensure!(matches!(
            service.add_user(new, local).await?,
            UserAdded::Added(_)
        ));
        // the app's router on any free local port, each request knowing where it came from
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address: SocketAddr = listener.local_addr()?;
        let app =
            reef_app::server::router(service).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
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
        anyhow::Ok(())
    })
}
