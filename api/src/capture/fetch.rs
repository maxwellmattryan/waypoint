//! Fetching pages and board APIs: a body cap, one timeout over every hop, and a redirect
//! loop that never lets a public hop lead into a private network.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::{Response, StatusCode, Url};
use serde_json::Value;

use super::{CaptureFailure, CaptureFailureReason, Config};

/// Most redirects followed before the fetch counts as `unreachable`.
pub const MAX_REDIRECTS: usize = 5;
const HTML_ACCEPT: &str = "text/html,application/xhtml+xml";
const JSON_ACCEPT: &str = "application/json";

/// A fetched HTML page.
#[derive(Debug, Clone)]
pub struct Page {
    pub final_url: Url,
    pub body: String,
}

/// Why a fetch stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    Failed(CaptureFailure),
    /// A redirect hop went from a public address to a non-public one.
    RedirectRefused {
        host: String,
    },
}

fn failed(reason: CaptureFailureReason) -> FetchError {
    FetchError::Failed(CaptureFailure::new(reason))
}

/// Whether `addr` counts as public for the redirect guard. Loopback, private, link-local,
/// unique-local and unspecified addresses are not; an IPv4-mapped IPv6 address is judged
/// by its IPv4 part. Addresses in `config.treat_as_public` are always public.
pub fn is_public(addr: SocketAddr, config: &Config) -> bool {
    if config.treat_as_public.contains(&addr) {
        return true;
    }
    match addr.ip() {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(ip),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
}

/// Whether a hop from `prev` to a host resolving to `next` may be taken: refused only when
/// `prev` is public and any of `next` is not.
pub fn hop_allowed(prev: SocketAddr, next: &[SocketAddr], config: &Config) -> bool {
    !is_public(prev, config) || next.iter().all(|addr| is_public(*addr, config))
}

/// GETs an HTML page within the cap and the timeout.
pub async fn fetch_page(
    client: &reqwest::Client,
    config: &Config,
    url: Url,
) -> Result<Page, FetchError> {
    timed(config, async {
        let response = send_following(client, config, url, HTML_ACCEPT).await?;
        let final_url = response.url().clone();
        let not_html = content_type(&response)
            .is_some_and(|ty| !(ty == "text/html" || ty == "application/xhtml+xml"));
        if not_html {
            return Err(failed(CaptureFailureReason::NotHtml));
        }
        let body = read_capped(response, config.max_bytes).await?;
        Ok(Page {
            final_url,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    })
    .await
}

/// GETs a JSON document with the same cap, timeout and redirect guard.
pub async fn fetch_json(
    client: &reqwest::Client,
    config: &Config,
    url: Url,
) -> Result<Value, FetchError> {
    timed(config, async {
        let response = send_following(client, config, url, JSON_ACCEPT).await?;
        let is_json = content_type(&response)
            .is_some_and(|ty| ty == "application/json" || ty.ends_with("+json"));
        if !is_json {
            return Err(failed(CaptureFailureReason::NotHtml));
        }
        let body = read_capped(response, config.max_bytes).await?;
        serde_json::from_slice(&body).map_err(|_| failed(CaptureFailureReason::Unreachable))
    })
    .await
}

/// Runs a whole fetch, every hop and the body read, under `config.timeout`.
async fn timed<T>(
    config: &Config,
    fetch: impl Future<Output = Result<T, FetchError>>,
) -> Result<T, FetchError> {
    tokio::time::timeout(config.timeout, fetch)
        .await
        .unwrap_or_else(|_| Err(failed(CaptureFailureReason::Timeout)))
}

/// The media type, lowercased, without parameters.
fn content_type(response: &Response) -> Option<String> {
    let value = response.headers().get(CONTENT_TYPE)?.to_str().ok()?;
    let ty = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(ty)
}

fn transport(err: &reqwest::Error) -> FetchError {
    if err.is_timeout() {
        failed(CaptureFailureReason::Timeout)
    } else {
        failed(CaptureFailureReason::Unreachable)
    }
}

/// Sends GETs, following up to [`MAX_REDIRECTS`] redirects by hand. Before each hop the
/// next host is resolved and checked with [`hop_allowed`] against the address the current
/// response came from; after each response its own address is checked the same way, which
/// catches a DNS answer that changed between the lookup and the connect. The final
/// non-redirect response is returned with its body unread; so is a refused one's.
async fn send_following(
    client: &reqwest::Client,
    config: &Config,
    mut url: Url,
    accept: &str,
) -> Result<Response, FetchError> {
    let mut prev: Option<SocketAddr> = None;
    let mut looked_up: Option<SocketAddr> = None;
    let mut redirects = 0;
    loop {
        let response = client
            .get(url.clone())
            .header(ACCEPT, accept)
            .send()
            .await
            .map_err(|err| transport(&err))?;
        let here = response.remote_addr();
        let rebound = prev
            .zip(here)
            .is_some_and(|(prev, here)| !hop_allowed(prev, &[here], config));
        if rebound {
            return Err(refused(&url));
        }
        let status = response.status();
        let location = response.headers().get(LOCATION).cloned();
        let (true, Some(location)) = (is_redirect(status), location) else {
            if !status.is_success() {
                return Err(FetchError::Failed(CaptureFailure::http_status(
                    status.as_u16(),
                )));
            }
            return Ok(response);
        };
        if redirects == MAX_REDIRECTS {
            return Err(failed(CaptureFailureReason::Unreachable));
        }
        redirects += 1;

        let next = location
            .to_str()
            .ok()
            .and_then(|location| url.join(location).ok())
            .filter(|next| {
                matches!(next.scheme(), "http" | "https")
                    && next.host_str().is_some_and(|host| !host.is_empty())
                    && next.username().is_empty()
                    && next.password().is_none()
            })
            .ok_or_else(|| failed(CaptureFailureReason::Unreachable))?;
        let next_addrs = resolve(&next).await?;
        let from = match here.or(looked_up) {
            Some(from) => Some(from),
            None => resolve(&url).await?.first().copied(),
        };
        if from.is_some_and(|from| !hop_allowed(from, &next_addrs, config)) {
            return Err(refused(&next));
        }
        prev = from;
        looked_up = next_addrs.first().copied();
        url = next;
    }
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn refused(url: &Url) -> FetchError {
    FetchError::RedirectRefused {
        host: url.host_str().unwrap_or_default().to_string(),
    }
}

/// Every address `url`'s host resolves to, with its port. An IP literal skips DNS.
async fn resolve(url: &Url) -> Result<Vec<SocketAddr>, FetchError> {
    let port = url
        .port_or_known_default()
        .ok_or_else(|| failed(CaptureFailureReason::Unreachable))?;
    let host = url
        .host_str()
        .ok_or_else(|| failed(CaptureFailureReason::Unreachable))?;
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match literal.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| failed(CaptureFailureReason::Unreachable))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(failed(CaptureFailureReason::Unreachable));
    }
    Ok(addrs)
}

/// Reads the body, refusing more than `max` bytes by `Content-Length` and then while
/// streaming.
async fn read_capped(mut response: Response, max: u64) -> Result<Vec<u8>, FetchError> {
    if response.content_length().is_some_and(|len| len > max) {
        return Err(failed(CaptureFailureReason::TooLarge));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| transport(&err))? {
        body.extend_from_slice(&chunk);
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > max {
            return Err(failed(CaptureFailureReason::TooLarge));
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(raw: &str) -> SocketAddr {
        let ip: IpAddr = raw.parse().expect("valid ip");
        SocketAddr::new(ip, 443)
    }

    #[test]
    fn classifies_addresses() {
        let config = Config::default();
        let non_public = [
            "127.0.0.0",
            "127.255.255.255",
            "10.0.0.0",
            "10.255.255.255",
            "172.16.0.0",
            "172.31.255.255",
            "192.168.0.0",
            "192.168.255.255",
            "169.254.0.0",
            "169.254.255.255",
            "0.0.0.0",
            "::1",
            "::",
            "fc00::",
            "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "fe80::",
            "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ];
        for raw in non_public {
            assert!(!is_public(addr(raw), &config), "{raw} is not public");
        }
        let public = [
            "8.8.8.8",
            "172.32.0.1",
            "172.15.255.255",
            "192.169.0.1",
            "169.255.0.1",
            "11.0.0.1",
            "2606:4700::1",
            "fec0::1",
            "::ffff:8.8.8.8",
        ];
        for raw in public {
            assert!(is_public(addr(raw), &config), "{raw} is public");
        }

        let listed = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4100);
        let config = Config {
            treat_as_public: vec![listed],
            ..Config::default()
        };
        assert!(is_public(listed, &config), "a listed address is public");
        let other_port = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4101);
        assert!(!is_public(other_port, &config), "only the listed port");
    }

    #[test]
    fn hop_allowed() {
        let config = Config::default();
        let public = addr("8.8.8.8");
        let public2 = addr("1.1.1.1");
        let loopback = addr("127.0.0.1");
        let private = addr("10.0.0.1");
        let cases = [
            (public, vec![loopback], false),
            (public, vec![private], false),
            (public, vec![public2, loopback], false),
            (public, vec![public2], true),
            (loopback, vec![private], true),
            (loopback, vec![loopback], true),
            (private, vec![public], true),
        ];
        for (prev, next, want) in cases {
            assert_eq!(
                super::hop_allowed(prev, &next, &config),
                want,
                "{prev} -> {next:?}"
            );
        }
    }
}
