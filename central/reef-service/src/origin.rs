//! Cross-origin writes: what keeps a page of another origin from writing through a browser
//!
//! A browser sends its cookies with a form a page of another origin posts, and SameSite stops
//! only another *site*: a page served from another port of the same host, or another subdomain,
//! is the same site, and its posts carry reef's cookie. So every write
//! is judged by where the browser says it came from, as Go's `CrossOriginProtection` judges it,
//! with no token for a page to carry:
//!
//! - **Reads go on.** GET, HEAD and OPTIONS change nothing, and a page of another origin can't
//!   read what they answer without CORS, which reef never grants.
//! - **`Sec-Fetch-Site` first.** Today's browsers send it to https and to localhost:
//!   `same-origin` and `none` (the user's own navigation) go on, and `same-site` or `cross-site`
//!   is refused.
//! - **Else `Origin`.** A browser too old for `Sec-Fetch-Site`, or a request to plain HTTP on a
//!   host it won't send it to, still sends `Origin` with a write. It must name the scheme the
//!   connection speaks and the host and port the request was sent to.
//! - **Neither is not a browser.** The CLI, the MCP server and the hooks send neither, and go
//!   on; a page can't keep a browser from sending them.

use axum::http::uri::{Authority, Uri};
use axum::http::{HeaderMap, Method, header};

/// The header a browser says how a request's origin stands to its target's in
const SEC_FETCH_SITE: &str = "sec-fetch-site";

/// Judges whether a request may go on, by its method and where its browser says it came from
///
/// Returns why it may not, for the 403 that refuses it.
///
/// # Arguments
///
/// * `method` - The request's method
/// * `headers` - The request's headers
/// * `uri` - The request's target, whose authority, if it has one, wins over `Host`
/// * `tls` - Whether the request came over TLS
///
/// # Examples
///
/// ```
/// use axum::http::{HeaderMap, HeaderValue, Method, Uri};
/// use reef_service::origin::judge;
///
/// let uri = Uri::from_static("/api/v1/projects");
/// let mut headers = HeaderMap::new();
/// headers.insert("host", HeaderValue::from_static("127.0.0.1:12180"));
/// // the CLI sends no origin
/// assert!(judge(&Method::POST, &headers, &uri, false).is_ok());
/// // a page on another port does
/// headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:8000"));
/// assert!(judge(&Method::POST, &headers, &uri, false).is_err());
/// ```
pub fn judge(method: &Method, headers: &HeaderMap, uri: &Uri, tls: bool) -> Result<(), String> {
    // reads change nothing
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    // a browser that says how the origins stand is taken at its word
    if let Some(site) = headers.get(SEC_FETCH_SITE) {
        return match site.to_str() {
            Ok("same-origin" | "none") => Ok(()),
            Ok(site) => Err(refusal(&format!(
                "a page of another origin (`Sec-Fetch-Site: {site}`)"
            ))),
            Err(_) => Err(refusal("a page whose `Sec-Fetch-Site` can't be read")),
        };
    }
    // else its origin, if it names one; with neither, it is no browser
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let Ok(origin) = origin.to_str() else {
        return Err(refusal("a page whose `Origin` can't be read"));
    };
    match same_origin(origin, headers, uri, tls) {
        true => Ok(()),
        false => Err(refusal(origin)),
    }
}

/// Whether an `Origin` names the scheme the connection speaks, and the host and port the request
/// was sent to
///
/// # Arguments
///
/// * `origin` - The request's `Origin`
/// * `headers` - The request's headers, whose `Host` names where it was sent
/// * `uri` - The request's target, whose authority wins over `Host`
/// * `tls` - Whether the request came over TLS
fn same_origin(origin: &str, headers: &HeaderMap, uri: &Uri, tls: bool) -> bool {
    // an opaque origin, `null`, or anything else with no scheme and host, is no origin of ours
    let Ok(origin) = origin.parse::<Uri>() else {
        return false;
    };
    let (Some(scheme), Some(host)) = (origin.scheme_str(), origin.host()) else {
        return false;
    };
    // where the request was sent, as the browser named it
    let sent = match uri.authority() {
        Some(authority) => Some(authority.clone()),
        None => headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| host.parse::<Authority>().ok()),
    };
    let Some(sent) = sent else {
        return false;
    };
    // the scheme this connection speaks, and each side's port with its scheme's default
    let ours = match tls {
        true => "https",
        false => "http",
    };
    let port = |port: Option<u16>, scheme: &str| port.or(default_port(scheme));
    scheme.eq_ignore_ascii_case(ours)
        && host.eq_ignore_ascii_case(sent.host())
        && port(origin.port_u16(), scheme) == port(sent.port_u16(), ours)
}

/// The port a scheme's URLs name when they name none
///
/// # Arguments
///
/// * `scheme` - The scheme
fn default_port(scheme: &str) -> Option<u16> {
    match scheme.to_ascii_lowercase().as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// Why a write is refused, naming where it came from
///
/// # Arguments
///
/// * `from` - Where its browser says it came from
fn refusal(from: &str) -> String {
    format!("reef takes writes only from its own pages, and this one came from {from}")
}

#[cfg(test)]
mod tests {
    use super::judge;
    use axum::http::{HeaderMap, HeaderValue, Method, Uri};

    /// Headers holding a `Host` and whichever of the others are given
    ///
    /// # Arguments
    ///
    /// * `host` - Where the request was sent
    /// * `site` - Its `Sec-Fetch-Site`, if it sent one
    /// * `origin` - Its `Origin`, if it sent one
    fn headers(host: &str, site: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_str(host).unwrap());
        if let Some(site) = site {
            headers.insert("sec-fetch-site", HeaderValue::from_str(site).unwrap());
        }
        if let Some(origin) = origin {
            headers.insert("origin", HeaderValue::from_str(origin).unwrap());
        }
        headers
    }

    /// A write is judged by its `Sec-Fetch-Site`, else its `Origin` against the Host it was sent
    /// to, and a request with neither goes on
    #[test]
    fn writes_are_judged_by_where_they_came_from() {
        let uri = Uri::from_static("/api/v1/projects");
        let post = Method::POST;
        let plain = "127.0.0.1:12180";
        // reads go on from anywhere
        for read in [Method::GET, Method::HEAD, Method::OPTIONS] {
            let cross = headers(plain, Some("cross-site"), Some("https://evil.example"));
            assert!(judge(&read, &cross, &uri, false).is_ok(), "{read}");
        }
        // a write with neither header is no browser's, and goes on
        assert!(judge(&post, &headers(plain, None, None), &uri, false).is_ok());
        assert!(judge(&Method::DELETE, &headers(plain, None, None), &uri, true).is_ok());
        // the browser's word: its own pages and the user's own navigation go on
        for site in ["same-origin", "none"] {
            let sent = headers(plain, Some(site), Some("https://evil.example"));
            assert!(judge(&post, &sent, &uri, false).is_ok(), "{site}");
        }
        // and another site, or another origin of the same site, is refused, naming it
        for site in ["same-site", "cross-site", "anything"] {
            let sent = headers(plain, Some(site), Some("http://127.0.0.1:12180"));
            let refused = judge(&post, &sent, &uri, false).expect_err(site);
            assert!(refused.contains(site), "{refused}");
        }
        // without it, an origin of this scheme, host and port goes on
        let ours = headers(plain, None, Some("http://127.0.0.1:12180"));
        assert!(judge(&post, &ours, &uri, false).is_ok());
        let ours = headers("Reef.Example", None, Some("https://reef.example:443"));
        assert!(judge(&post, &ours, &uri, true).is_ok());
        let ours = headers("[::1]:12180", None, Some("http://[::1]:12180"));
        assert!(judge(&Method::PUT, &ours, &uri, false).is_ok());
        // but another port, host or scheme is refused, naming it
        for (host, origin, tls) in [
            (plain, "http://127.0.0.1:8000", false),
            (plain, "http://localhost:12180", false),
            (plain, "https://127.0.0.1:12180", false),
            ("reef.example:12443", "http://reef.example:12443", true),
            ("reef.example", "https://evil.example", true),
            ("reef.example", "https://reef.example:8443", true),
        ] {
            let sent = headers(host, None, Some(origin));
            let refused = judge(&post, &sent, &uri, tls).expect_err(origin);
            assert!(refused.contains(origin), "{refused}");
        }
        // as is an opaque origin, or one with no Host to match
        assert!(judge(&post, &headers(plain, None, Some("null")), &uri, false).is_err());
        let mut hostless = headers(plain, None, Some("http://127.0.0.1:12180"));
        hostless.remove("host");
        assert!(judge(&post, &hostless, &uri, false).is_err());
        // a target with its own authority is judged by it, over the Host
        let absolute = Uri::from_static("http://127.0.0.1:12180/api/v1/projects");
        let sent = headers("elsewhere.example", None, Some("http://127.0.0.1:12180"));
        assert!(judge(&post, &sent, &absolute, false).is_ok());
    }
}
