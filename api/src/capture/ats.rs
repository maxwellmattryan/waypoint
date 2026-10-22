//! Tier 2: Greenhouse, Lever and Ashby job URLs, read through each board's public posting
//! API. Detection looks only at the pasted URL; the mappers are pure.

use reqwest::Url;
use serde_json::Value;

use super::fetch::{self, FetchError};
use super::fields::{Extracted, Unit, clean, format_comp, humanise};
use super::jsonld::number;
use super::{Board, Config};

/// A detected posting on one of the supported boards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ats {
    Greenhouse { board: String, id: String },
    Lever { site: String, id: String },
    Ashby { org: String, id: String },
}

impl Ats {
    /// The board's name, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Greenhouse { .. } => "greenhouse",
            Self::Lever { .. } => "lever",
            Self::Ashby { .. } => "ashby",
        }
    }
}

/// Whether `value` is a slug or id this module accepts: `[A-Za-z0-9_-]+`.
fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `url`'s `host` or `host:port` is one of the board's hosts.
fn on_board(board: &Board, url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let with_port = url.port().map(|port| format!("{host}:{port}"));
    board.hosts.iter().any(|configured| {
        configured.eq_ignore_ascii_case(host)
            || with_port
                .as_deref()
                .is_some_and(|with_port| configured.eq_ignore_ascii_case(with_port))
    })
}

/// The board posting a pasted URL points at, if any.
pub fn detect(config: &Config, url: &Url) -> Option<Ats> {
    let mut segments: Vec<&str> = url.path_segments()?.collect();
    if segments.last() == Some(&"") {
        segments.pop();
    }
    if on_board(&config.greenhouse, url) {
        return match segments.as_slice() {
            [board, "jobs", id] if is_slug(board) && is_digits(id) => Some(Ats::Greenhouse {
                board: (*board).into(),
                id: (*id).into(),
            }),
            ["embed", "job_app"] => {
                let query = |key: &str| {
                    url.query_pairs()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.into_owned())
                };
                let (board, id) = (query("for")?, query("token")?);
                (is_slug(&board) && is_digits(&id)).then_some(Ats::Greenhouse { board, id })
            }
            _ => None,
        };
    }
    if on_board(&config.lever, url) {
        return match segments.as_slice() {
            [site, id] | [site, id, "apply"] if is_slug(site) && is_slug(id) => Some(Ats::Lever {
                site: (*site).into(),
                id: (*id).into(),
            }),
            _ => None,
        };
    }
    if on_board(&config.ashby, url) {
        return match segments.as_slice() {
            [org, id] | [org, id, "application"] if is_slug(org) && is_slug(id) => {
                Some(Ats::Ashby {
                    org: (*org).into(),
                    id: (*id).into(),
                })
            }
            _ => None,
        };
    }
    None
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// The posting API URL for a detected posting, or `None` if the configured base isn't a
/// URL that can take a path.
pub fn api_url(config: &Config, ats: &Ats) -> Option<Url> {
    let (base, segments, query): (&str, Vec<&str>, Option<&str>) = match ats {
        Ats::Greenhouse { board, id } => (
            &config.greenhouse.api,
            vec!["v1", "boards", board, "jobs", id],
            Some("pay_transparency=true"),
        ),
        Ats::Lever { site, id } => (&config.lever.api, vec!["v0", "postings", site, id], None),
        Ats::Ashby { org, .. } => (
            &config.ashby.api,
            vec!["posting-api", "job-board", org],
            Some("includeCompensation=true"),
        ),
    };
    let mut url = Url::parse(base).ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(segments);
    url.set_query(query);
    Some(url)
}

/// Calls the board's API and maps the answer. An `Err` is a reason for the log.
pub async fn lookup(
    client: &reqwest::Client,
    config: &Config,
    ats: &Ats,
) -> Result<Extracted, String> {
    let url = api_url(config, ats).ok_or("the API base is not a URL")?;
    let value = fetch::fetch_json(client, config, url)
        .await
        .map_err(|err| match err {
            FetchError::Failed(failure) => failure.message,
            FetchError::RedirectRefused { .. } => "a redirect to a private address".into(),
        })?;
    map(ats, &value).ok_or_else(|| "the API answer has no posting with a title".into())
}

/// The mapper for `ats`'s board.
pub fn map(ats: &Ats, value: &Value) -> Option<Extracted> {
    match ats {
        Ats::Greenhouse { board, .. } => greenhouse(value, board),
        Ats::Lever { site, .. } => lever(value, site),
        Ats::Ashby { org, id } => ashby(value, org, id),
    }
}

fn text<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

/// A Greenhouse Job Board API job: `title`, `company_name` (or the board slug),
/// `location.name` and the first `pay_input_ranges` entry, whose amounts are in cents.
pub fn greenhouse(value: &Value, board: &str) -> Option<Extracted> {
    let title = clean(text(value, "title"))?;
    let company = clean(text(value, "company_name")).or_else(|| clean(Some(&humanise(board))));
    let location = value.get("location").and_then(|l| clean(text(l, "name")));
    let comp = value
        .get("pay_input_ranges")
        .and_then(Value::as_array)
        .and_then(|ranges| ranges.first())
        .and_then(|range| {
            let cents = |key| number(range.get(key)).map(|n| n / 100.0);
            format_comp(
                text(range, "currency_type"),
                cents("min_cents"),
                cents("max_cents"),
                None,
            )
        });
    Some(Extracted {
        company,
        title: Some(title),
        location,
        comp,
    })
}

/// A Lever Postings API posting: `text`, the site slug as the company,
/// `categories.location` (or `Remote` for a remote `workplaceType`) and `salaryRange`.
pub fn lever(value: &Value, site: &str) -> Option<Extracted> {
    let title = clean(text(value, "text"))?;
    let remote = text(value, "workplaceType").is_some_and(|w| w.eq_ignore_ascii_case("remote"));
    let location = value
        .get("categories")
        .and_then(|c| clean(text(c, "location")))
        .or_else(|| remote.then(|| "Remote".into()));
    let comp = value.get("salaryRange").and_then(|range| {
        format_comp(
            text(range, "currency"),
            number(range.get("min")),
            number(range.get("max")),
            text(range, "interval").and_then(Unit::from_lever),
        )
    });
    Some(Extracted {
        company: clean(Some(&humanise(site))),
        title: Some(title),
        location,
        comp,
    })
}

/// The job with `id` in an Ashby Posting API board: `title`, the org slug as the company,
/// `location` (or `Remote` when `isRemote`) and the compensation summary.
pub fn ashby(value: &Value, org: &str, id: &str) -> Option<Extracted> {
    let job = value
        .get("jobs")?
        .as_array()?
        .iter()
        .find(|job| text(job, "id") == Some(id))?;
    let title = clean(text(job, "title"))?;
    let remote = job.get("isRemote").and_then(Value::as_bool) == Some(true);
    let location = clean(text(job, "location")).or_else(|| remote.then(|| "Remote".into()));
    let comp = job.get("compensation").and_then(|c| {
        clean(text(c, "compensationTierSummary"))
            .or_else(|| clean(text(c, "scrapeableCompensationSalarySummary")))
    });
    Some(Extracted {
        company: clean(Some(&humanise(org))),
        title: Some(title),
        location,
        comp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("valid url")
    }

    #[test]
    fn detects_boards() {
        let mut config = Config::default();
        config.lever.hosts.push("127.0.0.1:4100".into());
        let gh = |board: &str, id: &str| {
            Some(Ats::Greenhouse {
                board: board.into(),
                id: id.into(),
            })
        };
        let lever = |site: &str, id: &str| {
            Some(Ats::Lever {
                site: site.into(),
                id: id.into(),
            })
        };
        let ashby = |org: &str, id: &str| {
            Some(Ats::Ashby {
                org: org.into(),
                id: id.into(),
            })
        };
        let cases = [
            (
                "https://boards.greenhouse.io/acme/jobs/123",
                gh("acme", "123"),
            ),
            (
                "https://job-boards.greenhouse.io/acme-co/jobs/4567/",
                gh("acme-co", "4567"),
            ),
            (
                "https://boards.greenhouse.io/embed/job_app?for=acme&token=789",
                gh("acme", "789"),
            ),
            ("https://boards.greenhouse.io/acme/jobs/12a", None),
            ("https://boards.greenhouse.io/embed/job_app?for=acme", None),
            (
                "https://boards.greenhouse.io/embed/job_app?for=ac.me&token=1",
                None,
            ),
            ("https://boards.greenhouse.io/acme", None),
            (
                "https://jobs.lever.co/acme/abc-123",
                lever("acme", "abc-123"),
            ),
            (
                "https://jobs.lever.co/acme/abc-123/apply",
                lever("acme", "abc-123"),
            ),
            ("https://jobs.lever.co/acme", None),
            ("https://jobs.lever.co/acme/abc/other", None),
            ("https://jobs.lever.co/ac%20me/abc", None),
            ("http://127.0.0.1:4100/acme/x1", lever("acme", "x1")),
            ("http://127.0.0.1:4101/acme/x1", None),
            (
                "https://jobs.ashbyhq.com/org/uuid-1",
                ashby("org", "uuid-1"),
            ),
            (
                "https://jobs.ashbyhq.com/org/uuid-1/application",
                ashby("org", "uuid-1"),
            ),
            ("https://jobs.ashbyhq.com/org/uuid.1", None),
            ("https://jobs.ashbyhq.com/org", None),
            ("https://careers.example.com/acme/jobs/123", None),
            ("https://evil.example/jobs.lever.co/acme/abc", None),
        ];
        for (raw, want) in cases {
            assert_eq!(detect(&config, &url(raw)), want, "{raw}");
        }
    }

    #[test]
    fn builds_api_urls() {
        let mut config = Config::default();
        config.lever.api = "http://127.0.0.1:9/".into();
        let cases = [
            (
                Ats::Greenhouse {
                    board: "acme".into(),
                    id: "1".into(),
                },
                "https://boards-api.greenhouse.io/v1/boards/acme/jobs/1?pay_transparency=true",
            ),
            (
                Ats::Lever {
                    site: "acme".into(),
                    id: "a-b".into(),
                },
                "http://127.0.0.1:9/v0/postings/acme/a-b",
            ),
            (
                Ats::Ashby {
                    org: "acme".into(),
                    id: "x".into(),
                },
                "https://api.ashbyhq.com/posting-api/job-board/acme?includeCompensation=true",
            ),
        ];
        for (ats, want) in cases {
            assert_eq!(
                api_url(&config, &ats).map(String::from).as_deref(),
                Some(want)
            );
        }
    }

    #[test]
    fn maps_board_responses() {
        let parse = |raw: &str| serde_json::from_str::<Value>(raw).expect("fixture parses");
        let greenhouse_job = parse(include_str!(
            "../../tests/fixtures/capture/greenhouse_job.json"
        ));
        let lever_posting = parse(include_str!(
            "../../tests/fixtures/capture/lever_posting.json"
        ));
        let ashby_board = parse(include_str!(
            "../../tests/fixtures/capture/ashby_board.json"
        ));
        let mut no_company = greenhouse_job.clone();
        no_company.as_object_mut().unwrap().remove("company_name");

        let fields = |company: &str, title: &str, location: Option<&str>, comp: Option<&str>| {
            Some(Extracted {
                company: Some(company.into()),
                title: Some(title.into()),
                location: location.map(Into::into),
                comp: comp.map(Into::into),
            })
        };
        let cases = [
            (
                "greenhouse",
                greenhouse(&greenhouse_job, "examplelabs"),
                fields(
                    "Example Labs",
                    "Staff Software Engineer, Platform",
                    Some("New York, NY"),
                    Some("USD 180,000–210,000"),
                ),
            ),
            (
                "greenhouse without company_name",
                greenhouse(&no_company, "example-labs"),
                fields(
                    "Example Labs",
                    "Staff Software Engineer, Platform",
                    Some("New York, NY"),
                    Some("USD 180,000–210,000"),
                ),
            ),
            (
                "lever",
                lever(&lever_posting, "example-design-co"),
                fields(
                    "Example Design Co",
                    "Product Designer",
                    Some("Remote"),
                    Some("USD 140,000–165,000 per year"),
                ),
            ),
            (
                "ashby remote with compensation",
                ashby(
                    &ashby_board,
                    "example-org",
                    "22222222-2222-4222-8222-222222222222",
                ),
                fields(
                    "Example Org",
                    "Backend Engineer",
                    Some("Remote"),
                    Some("$160K – $190K • Offers Equity"),
                ),
            ),
            (
                "ashby on-site without compensation",
                ashby(
                    &ashby_board,
                    "example-org",
                    "11111111-1111-4111-8111-111111111111",
                ),
                fields("Example Org", "Recruiter", Some("London"), None),
            ),
            (
                "ashby board without the id",
                ashby(&ashby_board, "example-org", "missing"),
                None,
            ),
            (
                "greenhouse without a title",
                greenhouse(&parse("{}"), "acme"),
                None,
            ),
        ];
        for (name, got, want) in cases {
            assert_eq!(got, want, "{name}");
        }
    }
}
