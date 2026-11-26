//! Signing in: the one layer that guards every route of both servers, and the browser's sign-in
//! page
//!
//! reef-serve wraps the JSON API in [`guard`], and reef-app wraps its whole router, its pages,
//! its server functions and the JSON API it mounts, so one layer decides for both front doors.
//!
//! - **Where a write comes from comes first.** A write a browser sends from a page of another
//!   origin is refused 403, with sign-in on or off ([`origin::judge`](crate::origin::judge)), so
//!   a page on another port of this host, or another site, can't write through a visitor.
//! - **Then who is calling.** The layer reads a token from `Authorization: Bearer`, else from
//!   the sign-in cookie, asks the service whose it is, and hands every route a [`Caller`]: the
//!   user, the token, whether the request came from the service's own host, and whether it came
//!   over TLS. Each connection's [`Peer`] says the last two.
//! - **Who wrote it, for every write.** [`Caller::author`] names the user the token signs in, and
//!   Claude if the request's `reef-writer` header says it writes as Claude, as reef's MCP server
//!   does. The header only ever adds "Claude, for" to the token's user, so no request names
//!   another.
//! - **Off unless reef's config requires it.** With sign-in off every request goes through, as it
//!   did before users, and only adding or listing users asks who is calling.
//! - **On, everything but signing in is guarded** ([`judge`]): the JSON API's reads and writes,
//!   the change stream, docs, the app's pages and server functions. A page a browser opens is
//!   sent to the sign-in page, and anything else is answered 401.
//! - **The sign-in page is plain HTML the service serves,** so reef-serve has one too, and it
//!   runs no script at all. Signing in sets an HttpOnly, SameSite=Strict cookie, `Secure` when
//!   the request came over TLS, and goes on only to a path of this origin ([`safe_next`]).
//! - **Repeated failures are slowed.** A sign-in that must wait, after too many failed for its
//!   name or from its address, is answered 429 with the page again, saying how long.

use axum::Router;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Form, FromRequestParts, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::serve::IncomingStream;
use reef_api::user::SIGN_IN_COOKIE;
use reef_api::{ApiError, Author, ErrorCode, TokenKind, User, WRITER_HEADER, Writer, paths};
use reef_render::paths::escape;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpListener;
use tracing::instrument;

use crate::api::{Failure, Serves};
use crate::origin;
use crate::service::{Asker, BROWSER_TOKEN_LIFE, TooManyTries};

/// Where a connection comes from, and whether it came over TLS, as each request learns it
///
/// Both servers serve with `into_make_service_with_connect_info::<Peer>()`, which
/// [`serve_router`](crate::api::serve_router) does for them, so the guard finds it on every
/// request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// The address the connection came from
    pub address: SocketAddr,
    /// Whether it came over TLS
    pub tls: bool,
}

impl Peer {
    /// Whether the connection came from the service's own host, over loopback
    pub fn is_local(&self) -> bool {
        self.address.ip().to_canonical().is_loopback()
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for Peer {
    /// A connection over plain HTTP, and the address it came from
    ///
    /// # Arguments
    ///
    /// * `stream` - The connection axum accepted
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Peer {
        Peer {
            address: *stream.remote_addr(),
            tls: false,
        }
    }
}

/// Who a request comes from, as the guard found: every route can take one
#[derive(Clone, Default)]
pub struct Caller {
    /// The user the request's token signs in, if it signs one in
    pub user: Option<User>,
    /// The token the request sent, if it sent one
    pub token: Option<String>,
    /// Whether the request came from the service's own host
    pub local: bool,
    /// Whether the request came over TLS, so a cookie set for it is `Secure`
    pub secure: bool,
    /// The address the request came from, if the connection said
    pub address: Option<IpAddr>,
    /// Whether it says it writes as Claude, for its user, or as the user themselves
    pub writer: Writer,
}

impl Caller {
    /// Who a write this caller makes is by: the user its token signs in, if any, and Claude if
    /// it says it writes as Claude
    pub fn author(&self) -> Author {
        Author::signed(self.user.as_ref(), self.writer)
    }

    /// The caller as the service judges one asking of its users or tokens
    pub fn asker(&self) -> Asker<'_> {
        Asker {
            user: self.user.as_ref(),
            writer: self.writer,
            local: self.local,
            token: self.token.as_deref(),
            address: self.address,
        }
    }
}

impl std::fmt::Debug for Caller {
    /// Shows the caller with its token left out
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Caller")
            .field("user", &self.user)
            .field("token", &self.token.as_ref().map(|_| "…"))
            .field("local", &self.local)
            .field("secure", &self.secure)
            .field("address", &self.address)
            .field("writer", &self.writer)
            .finish()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    /// A route outside the guard takes an anonymous caller from elsewhere
    type Rejection = Infallible;

    /// Takes the caller the guard found, or an anonymous one from elsewhere if none did
    ///
    /// # Arguments
    ///
    /// * `parts` - The request's head
    /// * `_state` - The router's state
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Caller, Infallible> {
        Ok(parts
            .extensions
            .get::<Caller>()
            .cloned()
            .unwrap_or_default())
    }
}

/// Whom a request's token names, as far as the guard judges it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    /// It sent no token
    Nobody,
    /// Its token signs a user in
    User,
    /// It sent a token that signs nobody in: made up, revoked or expired
    Unknown,
}

/// What the guard does with a request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It goes on to its route
    Pass,
    /// It is answered 401
    Refuse,
    /// The browser is sent to sign in, and back here after
    SignIn,
}

/// Judges a request by whether sign-in is required, whom its token names, and where it goes
///
/// # Arguments
///
/// * `required` - Whether every caller must sign in
/// * `named` - Whom the request's token names
/// * `method` - The request's method
/// * `path` - The request's path
/// * `wants_html` - Whether it asks for a page, as a browser opening one does
pub fn judge(
    required: bool,
    named: Named,
    method: &Method,
    path: &str,
    wants_html: bool,
) -> Verdict {
    // sign-in off, or a user signed in, or a route that signs one in, goes on
    if !required || named == Named::User || is_open(method, path) {
        return Verdict::Pass;
    }
    // a page a browser opens sends it to sign in; the app's calls and every other are refused
    let reads = method == Method::GET || method == Method::HEAD;
    let app_call = path == "/app" || path.starts_with("/app/");
    match reads && wants_html && !app_call {
        true => Verdict::SignIn,
        false => Verdict::Refuse,
    }
}

/// Whether a route answers anyone while sign-in is required: the routes that sign a user in or
/// out, the one that says who is calling, the users' routes, which judge their callers
/// themselves, and the app's static files
///
/// # Arguments
///
/// * `method` - The request's method
/// * `path` - The request's path
fn is_open(method: &Method, path: &str) -> bool {
    // reads, as a browser fetches static files
    let reads = method == Method::GET || method == Method::HEAD;
    match path {
        // the sign-in page and its form, and signing out
        paths::SIGN_IN => reads || method == Method::POST,
        paths::SIGN_OUT => method == Method::POST,
        // a token for a name and password, and who the caller is
        paths::TOKENS => method == Method::POST,
        paths::ME => reads,
        // the users' routes, which judge their callers themselves
        paths::USERS => reads || method == Method::POST,
        // the app's stylesheet, its bundle and its assets
        "/reef.css" => reads,
        _ => reads && (path.starts_with("/assets/") || path.starts_with("/wasm/")),
    }
}

/// The path to go on to after signing in: the one asked for if it is a path of this origin, else
/// the front page
///
/// A path starts with one `/` and no other, holds only printable ASCII and no `\`, and isn't the
/// sign-in page itself, so a link can't send a browser that signs in to another site.
///
/// # Arguments
///
/// * `next` - The path asked for, if one was
///
/// # Examples
///
/// ```
/// use reef_service::auth::safe_next;
///
/// assert_eq!(safe_next(Some("/plans/reef/central")), "/plans/reef/central");
/// assert_eq!(safe_next(Some("//elsewhere.example")), "/");
/// ```
pub fn safe_next(next: Option<&str>) -> String {
    // nothing asked for is the front page
    let Some(next) = next else {
        return "/".to_string();
    };
    // one `/`, then anything but a second `/` or a `\`, which browsers read as one
    let rooted = next.starts_with('/') && !next.starts_with("//") && !next.starts_with("/\\");
    // printable ASCII alone, so no space, control or escape hides a scheme or a host
    let plain = next.bytes().all(|b| b.is_ascii_graphic() && b != b'\\');
    // and not back to the sign-in page, which would only ask again
    let ours =
        next != paths::SIGN_IN && !next.starts_with("/sign-in?") && !next.starts_with("/sign-in/");
    match rooted && plain && ours {
        true => next.to_string(),
        false => "/".to_string(),
    }
}

/// The token a request sent, and whether it came in the sign-in cookie rather than a header
///
/// # Arguments
///
/// * `headers` - The request's headers
fn presented(headers: &HeaderMap) -> Option<(String, bool)> {
    // a bearer token first, which terminal clients send
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let (scheme, token) = value.trim().split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| token.trim().to_string())
        });
    if let Some(token) = bearer {
        return Some((token, false));
    }
    // else the sign-in cookie, in any of the request's cookie headers
    cookie(headers).map(|token| (token, true))
}

/// Who a request says writes it: Claude if its `reef-writer` header says `claude`, else the user
///
/// # Arguments
///
/// * `headers` - The request's headers
fn writer(headers: &HeaderMap) -> Writer {
    // only Claude's word is read; anything else, or none, is the user's own
    let claude = headers
        .get(WRITER_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(Writer::Claude.as_str()));
    match claude {
        true => Writer::Claude,
        false => Writer::User,
    }
}

/// The sign-in cookie's value, if a request sent one
///
/// # Arguments
///
/// * `headers` - The request's headers
fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SIGN_IN_COOKIE)
        .map(|(_, value)| value.trim().to_string())
}

/// Whether a request asks for a page, as a browser opening one does
///
/// # Arguments
///
/// * `headers` - The request's headers
fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// The cookie that signs a browser in with a token, for as long as a browser's token lasts
///
/// # Arguments
///
/// * `token` - The token
/// * `secure` - Whether it is set over TLS, so the browser sends it over TLS alone
fn sign_in_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SIGN_IN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        BROWSER_TOKEN_LIFE.as_secs(),
        secure_attribute(secure)
    )
}

/// The cookie that clears the sign-in cookie
///
/// # Arguments
///
/// * `secure` - Whether it is set over TLS, as the cookie it clears was
fn cleared_cookie(secure: bool) -> String {
    format!(
        "{SIGN_IN_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        secure_attribute(secure)
    )
}

/// The `Secure` attribute a cookie set over TLS carries, or nothing over plain HTTP
///
/// # Arguments
///
/// * `secure` - Whether the cookie is set over TLS
fn secure_attribute(secure: bool) -> &'static str {
    match secure {
        true => "; Secure",
        false => "",
    }
}

/// The answer to a request that must sign in first: 401, saying how
fn refused() -> Response {
    // the message a person reads, whichever client they called through
    Failure(ApiError::new(
        ErrorCode::Unauthorized,
        "reef asks every caller to sign in: run `reef login`, or sign in at /sign-in",
    ))
    .into_response()
}

/// Wraps a router in the one layer that guards it, beside the browser's sign-in routes
///
/// Call it last, on the whole router, so every route and fallback under it is guarded.
///
/// # Arguments
///
/// * `router` - Every route the server serves
/// * `serves` - The service that knows its users
///
/// # Examples
///
/// ```
/// use reef_service::{Service, api, auth};
/// use std::sync::Arc;
///
/// let service = Arc::new(Service::memory());
/// let router = auth::guard(api::router(service.clone()), service);
/// ```
pub fn guard<S: Serves>(router: Router, serves: S) -> Router {
    // the sign-in page, its form and signing out, guarded too so they see their caller
    let browser = Router::new()
        .route(paths::SIGN_IN, get(sign_in_page::<S>).post(sign_in::<S>))
        .route(paths::SIGN_OUT, axum::routing::post(sign_out::<S>))
        .with_state(serves.clone());
    router
        .merge(browser)
        .layer(middleware::from_fn_with_state(serves, check::<S>))
}

/// The layer itself: refuses a write from another origin, finds who is calling, then lets the
/// request go on, refuses it, or sends its browser to sign in
///
/// # Arguments
///
/// * `serves` - The service that knows its users
/// * `request` - The request
/// * `next` - The route the request goes on to
async fn check<S: Serves>(State(serves): State<S>, mut request: Request, next: Next) -> Response {
    let service = serves.service();
    // from the service's own host, and over TLS, as the connection says
    let peer = request
        .extensions()
        .get::<ConnectInfo<Peer>>()
        .map(|ConnectInfo(peer)| *peer);
    let local = peer.is_some_and(|peer| peer.is_local());
    let secure = peer.is_some_and(|peer| peer.tls);
    let address = peer.map(|peer| peer.address.ip());
    // a write from a page of another origin goes no further, signed in or not
    if let Err(message) = origin::judge(request.method(), request.headers(), request.uri(), secure)
    {
        return Failure(ApiError::new(ErrorCode::Forbidden, message)).into_response();
    }
    // the token sent, and whom it signs in
    let presented = presented(request.headers());
    let user = match &presented {
        Some((token, _)) => match service.signed_in(token).await {
            Ok(user) => user,
            Err(error) => return Failure::from(error).into_response(),
        },
        None => None,
    };
    let named = match (&presented, &user) {
        (_, Some(_)) => Named::User,
        (Some(_), None) => Named::Unknown,
        (None, None) => Named::Nobody,
    };
    let from_cookie = presented.as_ref().is_some_and(|(_, cookie)| *cookie);
    // judged by where it goes and what it asks for
    let verdict = judge(
        service.sign_in_required(),
        named,
        request.method(),
        request.uri().path(),
        wants_html(request.headers()),
    );
    match verdict {
        Verdict::Pass => {
            // every route under the layer reads its caller from here
            let token = presented.map(|(token, _)| token);
            let writer = writer(request.headers());
            request.extensions_mut().insert(Caller {
                user,
                token,
                local,
                secure,
                address,
                writer,
            });
            next.run(request).await
        }
        Verdict::Refuse => refused(),
        Verdict::SignIn => {
            // back here once signed in
            let here = request
                .uri()
                .path_and_query()
                .map(|path| path.as_str())
                .unwrap_or("/");
            let location = format!("{}?next={}", paths::SIGN_IN, paths::encode(here));
            let mut response =
                (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response();
            // a cookie that signs nobody in is cleared, so the browser stops sending it
            if from_cookie
                && named == Named::Unknown
                && let Ok(value) = HeaderValue::from_str(&cleared_cookie(secure))
            {
                response.headers_mut().insert(header::SET_COOKIE, value);
            }
            response
        }
    }
}

/// The sign-in page's query: where to go once signed in
#[derive(Debug, Default, Deserialize)]
struct NextQuery {
    /// The path to go on to
    #[serde(default)]
    next: Option<String>,
}

/// The sign-in form, as a browser sends it
#[derive(Deserialize)]
struct SignInForm {
    /// The user's name
    #[serde(default)]
    name: String,
    /// The user's password
    #[serde(default)]
    password: String,
    /// The path to go on to
    #[serde(default)]
    next: Option<String>,
}

/// The sign-in page's HTML: a form for a name and password, and what went wrong, if anything did
///
/// # Arguments
///
/// * `next` - The path to go on to once signed in, already made safe
/// * `name` - The name to fill in
/// * `problem` - What went wrong last time, if anything did
fn sign_in_html(next: &str, name: &str, problem: Option<&str>) -> String {
    // every value escaped, so nothing sent can write the page
    let next = escape(next);
    let name = escape(name);
    let problem = problem
        .map(|problem| {
            format!(
                "<p class=\"problem\" role=\"alert\">{}</p>",
                escape(problem)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sign in · reef</title>
<style>
:root {{ color-scheme: light dark; font-family: system-ui, sans-serif; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; }}
main {{ width: min(22rem, 100% - 2rem); }}
h1 {{ font-size: 1.5rem; margin: 0 0 1rem; }}
label {{ display: block; margin: 0.75rem 0 0.25rem; }}
input {{ width: 100%; box-sizing: border-box; padding: 0.5rem; font: inherit; }}
button {{ margin-top: 1rem; padding: 0.5rem 1rem; font: inherit; }}
.problem {{ color: #b42318; }}
.already {{ margin-top: 1.5rem; font-size: 0.875rem; opacity: 0.8; }}
</style>
</head>
<body>
<main>
<h1>Sign in to reef</h1>
{problem}<form method="post" action="{action}">
<input type="hidden" name="next" value="{next}">
<label for="name">Name</label>
<input id="name" name="name" value="{name}" autocomplete="username" autocapitalize="none" required autofocus>
<label for="password">Password</label>
<input id="password" name="password" type="password" autocomplete="current-password" required>
<button type="submit">Sign in</button>
</form>
<p class="already">Signed in already? <a href="{next}">Go on</a></p>
</main>
</body>
</html>
"#,
        action = paths::SIGN_IN,
    )
}

/// A sign-in page's answer: no caching, and no script, frame or form but its own
///
/// # Arguments
///
/// * `status` - The status to answer with
/// * `html` - The page
fn page(status: StatusCode, html: String) -> Response {
    // nothing but its own inline style, and its form posts only here
    let policy = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
                  frame-ancestors 'none'; base-uri 'none'";
    (
        status,
        [
            (header::CONTENT_SECURITY_POLICY, policy),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(html),
    )
        .into_response()
}

/// Serves the sign-in page, or goes on at once for a browser signed in already
///
/// # Arguments
///
/// * `caller` - Who is calling
/// * `query` - Where to go once signed in
#[instrument(name = "auth::sign_in_page", skip_all)]
async fn sign_in_page<S: Serves>(
    State(_serves): State<S>,
    caller: Caller,
    Query(query): Query<NextQuery>,
) -> Response {
    let next = safe_next(query.next.as_deref());
    // signed in already, there is nothing to ask
    if caller.user.is_some() {
        return (StatusCode::SEE_OTHER, [(header::LOCATION, next)]).into_response();
    }
    page(StatusCode::OK, sign_in_html(&next, "", None))
}

/// Signs a browser in from the sign-in form: the cookie set and on to where it was going, or the
/// form again, saying the name and password don't match
///
/// # Arguments
///
/// * `serves` - The service that knows its users
/// * `caller` - Who is calling, and whether over TLS
/// * `headers` - The request's headers, whose user agent labels the token
/// * `form` - The name, the password and where to go
#[instrument(name = "auth::sign_in", skip_all)]
async fn sign_in<S: Serves>(
    State(serves): State<S>,
    caller: Caller,
    headers: HeaderMap,
    Form(form): Form<SignInForm>,
) -> Response {
    let next = safe_next(form.next.as_deref());
    // the browser, as it names itself, labels its token
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("a browser");
    let label: String = format!("browser: {agent}").chars().take(120).collect();
    let name = form.name.trim();
    let issued = serves
        .service()
        .sign_in(
            name,
            &form.password,
            TokenKind::Browser,
            &label,
            caller.address,
        )
        .await;
    match issued {
        // the cookie, then on to where it was going
        Ok(Some(issued)) => {
            let cookie = sign_in_cookie(&issued.token, caller.secure);
            (
                StatusCode::SEE_OTHER,
                [(header::LOCATION, next), (header::SET_COOKIE, cookie)],
            )
                .into_response()
        }
        // the same words for a name no user has and a wrong password
        Ok(None) => page(
            StatusCode::UNAUTHORIZED,
            sign_in_html(
                &next,
                &form.name,
                Some("That name and password don't match a user of reef."),
            ),
        ),
        // too many failed lately: the form again, saying how long to wait
        Err(error) if error.downcast_ref::<TooManyTries>().is_some() => {
            let mut response = page(
                StatusCode::TOO_MANY_REQUESTS,
                sign_in_html(&next, &form.name, Some(&error.to_string())),
            );
            if let Some(wait) = error.downcast_ref::<TooManyTries>()
                && let Ok(value) = HeaderValue::from_str(&wait.seconds().to_string())
            {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            response
        }
        Err(error) => Failure::from(error).into_response(),
    }
}

/// Signs a browser out: its cookie's token revoked, the cookie cleared, and back to sign in
///
/// # Arguments
///
/// * `serves` - The service that knows its users
/// * `caller` - Who is calling
#[instrument(name = "auth::sign_out", skip_all)]
async fn sign_out<S: Serves>(State(serves): State<S>, caller: Caller) -> Response {
    // the token revoked, whether or not it still signed anyone in
    if let Some(token) = &caller.token
        && let Err(error) = serves.service().sign_out(token).await
    {
        return Failure::from(error).into_response();
    }
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, paths::SIGN_IN.to_string()),
            (header::SET_COOKIE, cleared_cookie(caller.secure)),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{
        Caller, Named, Verdict, cleared_cookie, cookie, judge, presented, safe_next,
        sign_in_cookie, writer,
    };
    use axum::http::{HeaderMap, HeaderValue, Method, header};
    use reef_api::{User, WRITER_HEADER, Writer};

    /// A request says it writes as Claude only by its header's `claude`, and a caller's author is
    /// its token's user, Claude's for them where it says so
    #[test]
    fn the_writer_header_marks_claude_for_its_user() {
        // no header, or any other word, is the user's own
        let mut headers = HeaderMap::new();
        assert_eq!(writer(&headers), Writer::User);
        headers.insert(WRITER_HEADER, HeaderValue::from_static("ada"));
        assert_eq!(writer(&headers), Writer::User);
        // Claude's, whatever its case
        headers.insert(WRITER_HEADER, HeaderValue::from_static(" Claude "));
        assert_eq!(writer(&headers), Writer::Claude);
        // and a caller's author is its user, Claude's for them, and nobody with no user
        let ada = User {
            added_by: reef_api::Author::default(),
            name: "ada".to_string(),
            admin: false,
            created_at: 3,
        };
        let caller = Caller {
            user: Some(ada),
            writer: Writer::Claude,
            ..Caller::default()
        };
        assert_eq!(
            caller.author().describe().as_deref(),
            Some("Claude, for ada")
        );
        assert_eq!(caller.author().since, Some(3));
        assert!(Caller::default().author().is_unknown());
    }

    /// Only a path of this origin is followed after sign-in
    #[test]
    fn next_is_a_path_of_this_origin() {
        // paths of this origin, queries and all
        assert_eq!(safe_next(Some("/")), "/");
        assert_eq!(
            safe_next(Some("/plans/reef/central")),
            "/plans/reef/central"
        );
        assert_eq!(safe_next(Some("/plans/a/b?x=1#y")), "/plans/a/b?x=1#y");
        // nothing asked for is the front page
        assert_eq!(safe_next(None), "/");
        assert_eq!(safe_next(Some("")), "/");
        // another host, however it is written, is the front page
        for elsewhere in [
            "https://elsewhere.example/",
            "//elsewhere.example",
            "/\\elsewhere.example",
            "\\\\elsewhere.example",
            "javascript:alert(1)",
            "/plans/a b",
            "/plans/\u{7f}",
            "/plans/é",
            "elsewhere.example",
        ] {
            assert_eq!(
                safe_next(Some(elsewhere)),
                "/",
                "{elsewhere:?} was followed"
            );
        }
        // and the sign-in page itself
        assert_eq!(safe_next(Some("/sign-in")), "/");
        assert_eq!(safe_next(Some("/sign-in?next=/x")), "/");
    }

    /// The guard judges by sign-in, the token, the path and whether a page is wanted
    #[test]
    fn the_guard_judges_by_path_and_accept() {
        let get = Method::GET;
        let post = Method::POST;
        // off, everything goes on, whatever the token
        assert_eq!(
            judge(false, Named::Nobody, &post, "/api/v1/projects", false),
            Verdict::Pass
        );
        assert_eq!(judge(false, Named::Unknown, &get, "/", true), Verdict::Pass);
        // on, a user goes anywhere
        assert_eq!(
            judge(true, Named::User, &post, "/api/v1/projects", false),
            Verdict::Pass
        );
        assert_eq!(
            judge(true, Named::User, &post, "/app/save", false),
            Verdict::Pass
        );
        // and anyone reaches the routes that sign in, and the static files
        for (method, path) in [
            (&get, "/sign-in"),
            (&post, "/sign-in"),
            (&post, "/sign-out"),
            (&post, "/api/v1/tokens"),
            (&get, "/api/v1/me"),
            (&get, "/api/v1/users"),
            (&post, "/api/v1/users"),
            (&get, "/reef.css"),
            (&get, "/assets/app-1a2b.wasm"),
            (&get, "/wasm/reef-app.js"),
        ] {
            assert_eq!(
                judge(true, Named::Nobody, method, path, false),
                Verdict::Pass,
                "{path}"
            );
        }
        // but not by another method
        assert_eq!(
            judge(true, Named::Nobody, &Method::DELETE, "/api/v1/users", false),
            Verdict::Refuse
        );
        assert_eq!(
            judge(true, Named::Nobody, &post, "/reef.css", false),
            Verdict::Refuse
        );
        // a browser opening a page is sent to sign in, from a page or a doc
        assert_eq!(judge(true, Named::Nobody, &get, "/", true), Verdict::SignIn);
        assert_eq!(
            judge(true, Named::Unknown, &get, "/plans/a/b", true),
            Verdict::SignIn
        );
        assert_eq!(
            judge(
                true,
                Named::Nobody,
                &get,
                "/api/v1/projects/a/plans/b/docs/x.html",
                true
            ),
            Verdict::SignIn
        );
        // the app's calls, the JSON API, the stream and every write are refused
        assert_eq!(
            judge(true, Named::Nobody, &get, "/app/projects", true),
            Verdict::Refuse
        );
        assert_eq!(
            judge(true, Named::Nobody, &post, "/app/save", false),
            Verdict::Refuse
        );
        assert_eq!(
            judge(true, Named::Nobody, &get, "/api/v1/projects", false),
            Verdict::Refuse
        );
        assert_eq!(
            judge(true, Named::Nobody, &get, "/api/v1/changes", false),
            Verdict::Refuse
        );
        assert_eq!(
            judge(true, Named::Unknown, &post, "/", true),
            Verdict::Refuse
        );
        assert_eq!(
            judge(
                true,
                Named::Nobody,
                &Method::DELETE,
                "/api/v1/tokens/current",
                false
            ),
            Verdict::Refuse
        );
    }

    /// A cookie set over TLS is `Secure`, and one over plain HTTP isn't
    #[test]
    fn cookies_over_tls_are_secure() {
        // set and cleared over TLS, both carry it
        assert!(sign_in_cookie("tok", true).ends_with("; Secure"));
        assert!(cleared_cookie(true).ends_with("; Secure"));
        // over plain HTTP neither does, since a browser drops a Secure cookie set without TLS
        assert!(!sign_in_cookie("tok", false).contains("Secure"));
        assert!(!cleared_cookie(false).contains("Secure"));
    }

    /// A token is read from a bearer header first, else the sign-in cookie
    #[test]
    fn a_token_is_read_from_bearer_or_cookie() {
        // nothing sent is no token
        let mut headers = HeaderMap::new();
        assert_eq!(presented(&headers), None);
        // the cookie among others
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; reef_sign_in=tok; b=2"),
        );
        assert_eq!(cookie(&headers).as_deref(), Some("tok"));
        assert_eq!(presented(&headers), Some(("tok".to_string(), true)));
        // a bearer header wins, its scheme in any case
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer  other "),
        );
        assert_eq!(presented(&headers), Some(("other".to_string(), false)));
        // another scheme is no token, and the cookie is read instead
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(presented(&headers), Some(("tok".to_string(), true)));
    }
}
