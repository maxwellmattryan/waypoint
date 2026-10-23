//! Each board's list API and the mappers from its entries to [`Posting`]s. A fetch reads
//! the whole board into memory before anything is saved, so a failure partway writes
//! nothing. The mappers are pure.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use reqwest::Url;
use serde_json::Value;

use super::Source;
use super::location::{self, Address, Hints, Location};
use super::text::{html_to_text, unescape};
use crate::applications::{MAX_URL, is_web_url};
use crate::capture::fetch::{self, FetchError};
use crate::capture::fields::{clean, humanise};
use crate::capture::{Capturer, Config};

/// Most characters of a posting's description that are stored.
pub const MAX_DESCRIPTION: usize = 100_000;

/// One board entry, mapped: a `job_postings` row's board-derived columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Posting {
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    pub apply_url: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub location_raw: Option<String>,
    pub location: Location,
    pub description: String,
    pub posted_at: Option<DateTime<Utc>>,
}

/// A whole board, read and mapped.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    /// Deduped on `external_id`, the last entry winning.
    pub postings: Vec<Posting>,
    /// Board entries received.
    pub fetched: usize,
    /// Entries that couldn't be mapped (no id, title or http(s) URL, or unlisted).
    pub skipped: usize,
    /// List requests made.
    pub pages: usize,
}

/// The list URL for one board, with Lever's page offset, or `None` when the configured
/// base isn't a URL that can take a path. The slug goes in as one path segment, so it is
/// percent-encoded rather than able to change the path or the query.
pub fn list_url(config: &Config, source: Source, board: &str, skip: usize) -> Option<Url> {
    let (base, segments, query) = match source {
        Source::Greenhouse => (
            &config.greenhouse.api,
            vec!["v1", "boards", board, "jobs"],
            Some("content=true".to_string()),
        ),
        Source::Lever => (
            &config.lever.api,
            vec!["v0", "postings", board],
            Some(format!(
                "mode=json&skip={skip}&limit={}",
                config.board_page_size.max(1)
            )),
        ),
        Source::Ashby => (
            &config.ashby.api,
            vec!["posting-api", "job-board", board],
            None,
        ),
    };
    let mut url = Url::parse(base).ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(segments);
    url.set_query(query.as_deref());
    Some(url)
}

/// One list request; an `Err` is a reason for the log, never with the URL.
async fn get(
    capturer: &Capturer,
    config: &Config,
    source: Source,
    board: &str,
    skip: usize,
) -> Result<Value, String> {
    let url = list_url(config, source, board, skip).ok_or("the API base is not a URL")?;
    fetch::fetch_json(capturer.client(), config, url)
        .await
        .map_err(|err| match err {
            FetchError::Failed(failure) => failure.message,
            FetchError::RedirectRefused { .. } => "refused a redirect to a private address".into(),
        })
}

/// `value[key]` as an array, taken out of it.
fn take_array(value: &mut Value, key: &str) -> Result<Vec<Value>, String> {
    match value.get_mut(key).map(Value::take) {
        Some(Value::Array(entries)) => Ok(entries),
        _ => Err(format!("the answer has no `{key}` array")),
    }
}

/// Reads the whole board and maps it. Every request goes through
/// [`fetch::fetch_json`] with the body cap at `max_board_bytes`. Any failed request, a
/// wrong-shaped answer, a Greenhouse `meta.total` that disagrees with the jobs returned,
/// or a Lever board longer than `max_board_pages` pages is an `Err`.
pub async fn fetch(capturer: &Capturer, source: Source, board: &str) -> Result<Fetched, String> {
    let config = Config {
        max_bytes: capturer.config.max_board_bytes,
        ..capturer.config.clone()
    };
    let (entries, pages) = match source {
        Source::Greenhouse => {
            let mut answer = get(capturer, &config, source, board, 0).await?;
            let total = answer.pointer("/meta/total").and_then(Value::as_u64);
            let jobs = take_array(&mut answer, "jobs")?;
            if let Some(total) = total.filter(|total| usize::try_from(*total) != Ok(jobs.len())) {
                return Err(format!(
                    "meta.total is {total} but {} jobs came back",
                    jobs.len()
                ));
            }
            (jobs, 1)
        }
        Source::Lever => {
            let size = config.board_page_size.max(1);
            let mut entries = Vec::new();
            let mut pages = 0;
            loop {
                if pages == config.max_board_pages {
                    return Err(format!(
                        "the board has more than {} pages",
                        config.max_board_pages
                    ));
                }
                let answer = get(capturer, &config, source, board, pages * size)
                    .await
                    .map_err(|err| format!("page {}: {err}", pages + 1))?;
                let Value::Array(page) = answer else {
                    return Err(format!("page {} is not a JSON array", pages + 1));
                };
                pages += 1;
                let full = page.len() == size;
                entries.extend(page);
                if !full {
                    break;
                }
            }
            (entries, pages)
        }
        Source::Ashby => {
            let mut answer = get(capturer, &config, source, board, 0).await?;
            (take_array(&mut answer, "jobs")?, 1)
        }
    };
    let fetched = entries.len();
    // Parsing every description's HTML is CPU-bound, so it stays off the async workers.
    let owned = board.to_string();
    let (postings, skipped) = tokio::task::spawn_blocking(move || collect(source, &owned, entries))
        .await
        .map_err(|err| format!("the mapping task failed: {err}"))?;
    Ok(Fetched {
        postings,
        fetched,
        skipped,
        pages,
    })
}

/// Removes U+0000 from every string in `value`. Postgres `text` can't hold it, so one
/// posting carrying it would fail the whole board's save on every run.
fn strip_nul(value: &mut Value) {
    match value {
        Value::String(text) if text.contains('\0') => text.retain(|ch| ch != '\0'),
        Value::Array(items) => items.iter_mut().for_each(strip_nul),
        Value::Object(fields) => fields.values_mut().for_each(strip_nul),
        _ => {}
    }
}

/// Strips U+0000 from every entry, maps it, counts the ones that don't map, and dedupes
/// on `external_id` with the last entry winning (in the first one's place).
pub fn collect(source: Source, board: &str, entries: Vec<Value>) -> (Vec<Posting>, usize) {
    let mut postings: Vec<Posting> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut skipped = 0;
    for mut entry in entries {
        strip_nul(&mut entry);
        let Some(posting) = map(source, &entry, board) else {
            skipped += 1;
            continue;
        };
        match seen.get(&posting.external_id) {
            Some(&at) => postings[at] = posting,
            None => {
                seen.insert(posting.external_id.clone(), postings.len());
                postings.push(posting);
            }
        }
    }
    (postings, skipped)
}

/// The mapper for `source`.
pub fn map(source: Source, entry: &Value, board: &str) -> Option<Posting> {
    match source {
        Source::Greenhouse => greenhouse(entry, board),
        Source::Lever => lever(entry, board),
        Source::Ashby => ashby(entry, board),
    }
}

fn text<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

/// A string id, or a number's digits; blank is none.
fn id(value: &Value) -> Option<String> {
    match value.get("id")? {
        Value::String(id) => Some(id.trim()).filter(|id| !id.is_empty()).map(Into::into),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

/// A trimmed absolute http or https URL of at most [`MAX_URL`] characters.
fn web_url(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    (raw.chars().count() <= MAX_URL && is_web_url(raw)).then(|| raw.to_string())
}

/// The board slug as a company name: `example-labs` → `Example Labs`.
fn company(board: &str) -> String {
    clean(Some(&humanise(board))).unwrap_or_else(|| board.to_string())
}

fn description(text: &str) -> String {
    text.trim().chars().take(MAX_DESCRIPTION).collect()
}

fn rfc3339(raw: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw?.trim())
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// A Greenhouse Job Board API job, listed with `content=true`.
pub fn greenhouse(job: &Value, board: &str) -> Option<Posting> {
    let external_id = id(job)?;
    let title = clean(text(job, "title"))?;
    let url = web_url(text(job, "absolute_url"))?;
    let location_raw = job.get("location").and_then(|l| clean(text(l, "name")));
    let department = job
        .get("departments")
        .and_then(Value::as_array)
        .and_then(|departments| departments.iter().find_map(|d| clean(text(d, "name"))));
    let content = text(job, "content").unwrap_or_default();
    Some(Posting {
        external_id,
        title,
        company: clean(text(job, "company_name")).unwrap_or_else(|| company(board)),
        apply_url: Some(url.clone()),
        url,
        department,
        employment_type: None,
        location: location::normalize(location_raw.as_deref(), Hints::default()),
        location_raw,
        description: description(&html_to_text(&unescape(content))),
        posted_at: rfc3339(text(job, "first_published"))
            .or_else(|| rfc3339(text(job, "updated_at"))),
    })
}

/// A Lever Postings API posting, listed with `mode=json`. The description is the
/// opening, each list as its heading line and items, then the closing.
pub fn lever(posting: &Value, board: &str) -> Option<Posting> {
    let external_id = id(posting)?;
    let title = clean(text(posting, "text"))?;
    let url = web_url(text(posting, "hostedUrl"))?;
    let categories = posting.get("categories").unwrap_or(&Value::Null);
    let location_raw = clean(text(categories, "location"));
    let hints = Hints {
        remote: text(posting, "workplaceType").is_some_and(|w| w.eq_ignore_ascii_case("remote")),
        country_code: text(posting, "country"),
        address: None,
    };
    let mut parts = vec![html_to_text(
        text(posting, "description").unwrap_or_default(),
    )];
    for list in posting
        .get("lists")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let heading = clean(text(list, "text"));
        let items = html_to_text(text(list, "content").unwrap_or_default());
        parts.push(match heading {
            Some(heading) if items.is_empty() => heading,
            Some(heading) => format!("{heading}\n{items}"),
            None => items,
        });
    }
    parts.push(html_to_text(
        text(posting, "additional").unwrap_or_default(),
    ));
    parts.retain(|part| !part.is_empty());
    Some(Posting {
        external_id,
        title,
        company: company(board),
        url,
        apply_url: web_url(text(posting, "applyUrl")),
        department: clean(text(categories, "department"))
            .or_else(|| clean(text(categories, "team"))),
        employment_type: clean(text(categories, "commitment")),
        location: location::normalize(location_raw.as_deref(), hints),
        location_raw,
        description: description(&parts.join("\n\n")),
        posted_at: posting
            .get("createdAt")
            .and_then(Value::as_i64)
            .and_then(DateTime::from_timestamp_millis),
    })
}

/// An Ashby Posting API job. Unlisted jobs are skipped.
pub fn ashby(job: &Value, board: &str) -> Option<Posting> {
    if job.get("isListed").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let external_id = id(job)?;
    let title = clean(text(job, "title"))?;
    let url = web_url(text(job, "jobUrl"))?;
    let location_raw = clean(text(job, "location"));
    let address = job.pointer("/address/postalAddress").map(|a| Address {
        locality: text(a, "addressLocality"),
        region: text(a, "addressRegion"),
        country: text(a, "addressCountry"),
    });
    let hints = Hints {
        remote: job.get("isRemote").and_then(Value::as_bool) == Some(true),
        country_code: None,
        address,
    };
    let html = html_to_text(text(job, "descriptionHtml").unwrap_or_default());
    let body = if html.is_empty() {
        text(job, "descriptionPlain")
            .unwrap_or_default()
            .to_string()
    } else {
        html
    };
    Some(Posting {
        external_id,
        title,
        company: company(board),
        url,
        apply_url: web_url(text(job, "applyUrl")),
        department: clean(text(job, "department")),
        employment_type: clean(text(job, "employmentType")),
        location: location::normalize(location_raw.as_deref(), hints),
        location_raw,
        description: description(&body),
        posted_at: rfc3339(text(job, "publishedAt")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Value {
        serde_json::from_str(raw).expect("fixture parses")
    }

    fn at(raw: &str) -> Option<DateTime<Utc>> {
        Some(raw.parse().expect("valid time"))
    }

    fn place(
        remote: bool,
        city: Option<&str>,
        region: Option<&str>,
        country: Option<&str>,
    ) -> Location {
        Location {
            remote,
            city: city.map(Into::into),
            region: region.map(Into::into),
            country: country.map(Into::into),
        }
    }

    fn entries(value: &Value) -> Vec<Value> {
        match value {
            Value::Array(entries) => entries.clone(),
            other => other["jobs"].as_array().expect("a jobs array").clone(),
        }
    }

    #[test]
    fn maps_fixture_boards() {
        let board = "example-labs";
        let greenhouse_board = parse(include_str!(
            "../../tests/fixtures/sourcing/greenhouse_board.json"
        ));
        let lever_board: Vec<Value> = [
            include_str!("../../tests/fixtures/sourcing/lever_page_1.json"),
            include_str!("../../tests/fixtures/sourcing/lever_page_2.json"),
            include_str!("../../tests/fixtures/sourcing/lever_page_3.json"),
        ]
        .iter()
        .flat_map(|page| entries(&parse(page)))
        .collect();
        let ashby_board = parse(include_str!(
            "../../tests/fixtures/sourcing/ashby_board.json"
        ));

        let (greenhouse, skipped) = collect(Source::Greenhouse, board, entries(&greenhouse_board));
        assert_eq!(skipped, 0);
        assert_eq!(
            greenhouse,
            vec![
                Posting {
                    external_id: "4012345".into(),
                    title: "Senior Backend Engineer".into(),
                    company: "Example Labs Inc.".into(),
                    url: "https://job-boards.greenhouse.io/example-labs/jobs/4012345".into(),
                    apply_url: Some(
                        "https://job-boards.greenhouse.io/example-labs/jobs/4012345".into()
                    ),
                    department: Some("Engineering".into()),
                    employment_type: None,
                    location_raw: Some("New York, NY".into()),
                    location: place(false, Some("New York"), Some("NY"), Some("US")),
                    description: "Build & ship the platform.\n\n- Rust\n- Postgres".into(),
                    posted_at: at("2026-09-15T13:00:00Z"),
                },
                Posting {
                    external_id: "4012346".into(),
                    title: "Product Designer".into(),
                    company: "Example Labs".into(),
                    url: "https://job-boards.greenhouse.io/example-labs/jobs/4012346".into(),
                    apply_url: Some(
                        "https://job-boards.greenhouse.io/example-labs/jobs/4012346".into()
                    ),
                    department: None,
                    employment_type: None,
                    location_raw: Some("Remote - US".into()),
                    location: place(true, None, None, Some("US")),
                    description: "Design things.".into(),
                    posted_at: at("2026-10-02T08:00:00Z"),
                },
                Posting {
                    external_id: "4012347".into(),
                    title: "Recruiter".into(),
                    company: "Example Labs Inc.".into(),
                    url: "https://job-boards.greenhouse.io/example-labs/jobs/4012347".into(),
                    apply_url: Some(
                        "https://job-boards.greenhouse.io/example-labs/jobs/4012347".into()
                    ),
                    department: Some("People".into()),
                    employment_type: None,
                    location_raw: Some("London, United Kingdom".into()),
                    location: place(false, Some("London"), None, Some("GB")),
                    description: "About\n\nHire people.".into(),
                    posted_at: at("2026-08-01T09:00:00Z"),
                },
            ]
        );

        let (lever, skipped) = collect(Source::Lever, board, lever_board.clone());
        assert_eq!(skipped, 0);
        assert_eq!(lever.len(), 5);
        assert_eq!(
            lever[0],
            Posting {
                external_id: "aaaaaaaa-0000-4000-8000-000000000001".into(),
                title: "Staff Engineer".into(),
                company: "Example Labs".into(),
                url: "https://jobs.lever.co/example-labs/aaaaaaaa-0000-4000-8000-000000000001".into(),
                apply_url: Some(
                    "https://jobs.lever.co/example-labs/aaaaaaaa-0000-4000-8000-000000000001/apply"
                        .into()
                ),
                department: Some("Engineering".into()),
                employment_type: Some("Full-time".into()),
                location_raw: Some("San Francisco, California".into()),
                location: place(false, Some("San Francisco"), Some("CA"), Some("US")),
                description: "Join the platform team.\n\nWhat you'll do\n- Design APIs\n- Run Postgres\n\nWe sponsor visas.".into(),
                posted_at: at("2026-10-01T10:00:00Z"),
            }
        );
        // No department: the team stands in. A remote workplace and Lever's country.
        assert_eq!(lever[1].department.as_deref(), Some("Design"));
        assert_eq!(lever[1].location, place(true, None, None, Some("CA")));
        assert_eq!(lever[1].description, "Design the product.");
        assert_eq!(lever[2].employment_type.as_deref(), Some("Contract"));
        assert_eq!(
            lever[2].location,
            place(false, Some("London"), None, Some("GB"))
        );
        assert_eq!(
            lever[2].description,
            "Model things.\n\nRequirements\n- Python"
        );
        // A `javascript:` apply link is dropped; the posting stays.
        assert_eq!(lever[3].apply_url, None);
        assert_eq!(
            lever[3].location,
            place(true, Some("Austin"), Some("TX"), Some("US"))
        );
        assert_eq!(
            lever[4].location,
            place(false, Some("Berlin"), None, Some("DE"))
        );
        assert_eq!(lever[4].posted_at, at("2026-10-05T10:00:00Z"));

        let (ashby, skipped) = collect(Source::Ashby, board, entries(&ashby_board));
        assert_eq!(skipped, 1, "the unlisted job");
        assert_eq!(
            ashby,
            vec![
                Posting {
                    external_id: "11111111-1111-4111-8111-111111111111".into(),
                    title: "Backend Engineer".into(),
                    company: "Example Labs".into(),
                    url: "https://jobs.ashbyhq.com/example-labs/11111111-1111-4111-8111-111111111111".into(),
                    apply_url: Some(
                        "https://jobs.ashbyhq.com/example-labs/11111111-1111-4111-8111-111111111111/application"
                            .into()
                    ),
                    department: Some("Engineering".into()),
                    employment_type: Some("FullTime".into()),
                    location_raw: Some("Remote".into()),
                    location: place(true, None, None, Some("US")),
                    description: "Own the API.\n\n- Go".into(),
                    posted_at: at("2026-09-20T16:00:00Z"),
                },
                Posting {
                    external_id: "22222222-2222-4222-8222-222222222222".into(),
                    title: "Recruiter".into(),
                    company: "Example Labs".into(),
                    url: "https://jobs.ashbyhq.com/example-labs/22222222-2222-4222-8222-222222222222".into(),
                    apply_url: Some(
                        "https://jobs.ashbyhq.com/example-labs/22222222-2222-4222-8222-222222222222/application"
                            .into()
                    ),
                    department: Some("People".into()),
                    employment_type: Some("PartTime".into()),
                    location_raw: Some("London".into()),
                    location: place(false, Some("London"), Some("England"), Some("GB")),
                    description: "Hire great people.".into(),
                    posted_at: at("2026-09-21T08:30:00Z"),
                },
            ]
        );

        // Entries with no id, no title or no http(s) URL are skipped; a repeated id keeps
        // the last entry, in the first one's place.
        let gh = entries(&greenhouse_board);
        let edit = |entry: &Value, key: &str, value: Value| {
            let mut entry = entry.clone();
            entry[key] = value;
            entry
        };
        let mut no_id = gh[0].clone();
        no_id.as_object_mut().unwrap().remove("id");
        let edge_cases = vec![
            no_id,
            edit(&gh[0], "id", Value::String(" ".into())),
            edit(&gh[1], "title", Value::String(" &nbsp; ".into())),
            edit(
                &gh[2],
                "absolute_url",
                Value::String("ftp://example.com/jobs/1".into()),
            ),
            edit(&gh[0], "absolute_url", Value::String("/relative".into())),
            gh[1].clone(),
            edit(
                &gh[1],
                "title",
                Value::String("Lead Product Designer".into()),
            ),
            gh[2].clone(),
        ];
        let (deduped, skipped) = collect(Source::Greenhouse, board, edge_cases);
        assert_eq!(skipped, 5);
        let titles: Vec<&str> = deduped.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Lead Product Designer", "Recruiter"]);

        // U+0000 anywhere in an entry is dropped before mapping, so it never reaches a
        // `text` column.
        let nul = parse(
            r#"{"id": "40\u000012345", "title": "Senior\u0000 Backend Engineer",
                "company_name": "Example\u0000 Labs Inc.",
                "absolute_url": "https://job-boards.greenhouse.io/example-labs/jobs/4012345\u0000",
                "location": {"name": "New York,\u0000 NY"},
                "departments": [{"name": "Engi\u0000neering"}],
                "content": "&lt;p&gt;Build &amp;amp; ship\u0000 the platform.&lt;/p&gt;&lt;ul&gt;&lt;li&gt;Rust&lt;/li&gt;&lt;li&gt;Post\u0000gres&lt;/li&gt;&lt;/ul&gt;",
                "first_published": "2026-09-15T13:00:00Z"}"#,
        );
        let (stripped, skipped) = collect(Source::Greenhouse, board, vec![nul]);
        assert_eq!(skipped, 0);
        assert_eq!(stripped, greenhouse[..1]);

        let lever_no_url = edit(&lever_board[0], "hostedUrl", Value::Null);
        assert_eq!(super::lever(&lever_no_url, board), None);
        let ashby_no_title = edit(&entries(&ashby_board)[0], "title", Value::Null);
        assert_eq!(super::ashby(&ashby_no_title, board), None);
        let long = format!("<p>{}</p>", "é".repeat(MAX_DESCRIPTION + 10));
        let ashby_long = edit(
            &entries(&ashby_board)[0],
            "descriptionHtml",
            Value::String(long),
        );
        let mapped = super::ashby(&ashby_long, board).expect("maps");
        assert_eq!(mapped.description.chars().count(), MAX_DESCRIPTION);
        let bad_time = edit(
            &entries(&ashby_board)[0],
            "publishedAt",
            Value::String("soon".into()),
        );
        assert_eq!(
            super::ashby(&bad_time, board).expect("maps").posted_at,
            None
        );
    }

    #[test]
    fn builds_list_urls() {
        let mut config = Config::default();
        config.lever.api = "http://127.0.0.1:9/".into();
        config.board_page_size = 2;
        let cases = [
            (
                Source::Greenhouse,
                "example-labs",
                0,
                "https://boards-api.greenhouse.io/v1/boards/example-labs/jobs?content=true",
            ),
            (
                Source::Lever,
                "Example_Labs",
                4,
                "http://127.0.0.1:9/v0/postings/Example_Labs?mode=json&skip=4&limit=2",
            ),
            (
                Source::Ashby,
                "example-labs",
                0,
                "https://api.ashbyhq.com/posting-api/job-board/example-labs",
            ),
        ];
        for (source, board, skip, want) in cases {
            assert_eq!(
                list_url(&config, source, board, skip)
                    .map(String::from)
                    .as_deref(),
                Some(want)
            );
        }
    }
}
