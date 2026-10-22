//! Host-header allowlist. The API binds to `127.0.0.1`, but a web page on a domain that
//! re-resolves to `127.0.0.1` (DNS rebinding) could still reach it with its own name in
//! `Host`. Every route takes [`LocalHost`] so such requests never reach a handler.

use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};

/// The only hosts the API answers to, compared case-insensitively. The port is ignored.
const ALLOWED_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// Whether a `Host` header value names this machine: `127.0.0.1`, `localhost` or `[::1]`,
/// with or without a port.
pub fn is_allowed_host(header: &str) -> bool {
    let (host, rest) = if header.starts_with('[') {
        match header.find(']') {
            Some(end) => header.split_at(end + 1),
            None => return false,
        }
    } else {
        match header.find(':') {
            Some(colon) => header.split_at(colon),
            None => (header, ""),
        }
    };
    let port_ok = rest.is_empty()
        || rest
            .strip_prefix(':')
            .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()));
    port_ok
        && ALLOWED_HOSTS
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

/// Admits a request only if it carries exactly one `Host` header and that host is
/// allowed. Add it as the first parameter of every handler.
pub struct LocalHost;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LocalHost {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let mut hosts = req.headers().get("Host");
        match (hosts.next(), hosts.next()) {
            (Some(host), None) if is_allowed_host(host) => Outcome::Success(Self),
            _ => Outcome::Error((Status::Forbidden, ())),
        }
    }
}

/// Refuses a request a browser sent from another site (`Sec-Fetch-Site: cross-site`),
/// so a page you visit can't change anything. `same-origin`, `same-site` (the web app on
/// another localhost port), `none` and a missing header (curl) pass. Every handler that
/// changes state takes it after [`LocalHost`].
///
/// It also refuses an HTML form body. Rocket rewrites a form `POST` whose first field is
/// `_method=delete` into a `DELETE`, and any page can submit such a form without a CORS
/// preflight, so the method alone doesn't prove the request came from Waypoint.
pub struct NotCrossSite;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for NotCrossSite {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let cross_site = req
            .headers()
            .get("Sec-Fetch-Site")
            .any(|site| site.eq_ignore_ascii_case("cross-site"));
        let form = req
            .content_type()
            .is_some_and(|ct| ct.is_form() || ct.is_form_data());
        if cross_site || form {
            Outcome::Error((Status::Forbidden, ()))
        } else {
            Outcome::Success(Self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowlist() {
        let cases = [
            ("127.0.0.1", true),
            ("127.0.0.1:8000", true),
            ("localhost", true),
            ("LOCALHOST:5173", true),
            ("[::1]", true),
            ("[::1]:8000", true),
            ("", false),
            ("evil.example", false),
            ("localhost.", false),
            ("foo.localhost", false),
            ("127.0.0.1.nip.io", false),
            ("[::2]", false),
            ("0.0.0.0", false),
            ("::1", false),
            ("localhost:", false),
            ("localhost:80x", false),
            ("[::1", false),
            ("[::1]x", false),
        ];
        for (header, allowed) in cases {
            assert_eq!(is_allowed_host(header), allowed, "{header:?}");
        }
    }
}
