# Board sourcing

Waypoint reads target companies' public Greenhouse, Lever and Ashby job boards into one table, `job_postings`. One `source_board` job reads one board, identified by its source and its slug, and keeps that board's rows in step with what it lists. There is no UI and no schedule yet: you ask for a board with `POST /sourcing/boards` each time. The code lives in `api/src/sourcing/`.

## Route

`POST /sourcing/boards` with a JSON body:

```sh
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"source": "greenhouse", "board": "example-labs"}' \
  http://127.0.0.1:8000/sourcing/boards
```

- `source` is `greenhouse`, `lever` or `ashby`, exactly. A missing one is 422 `Required`, and anything else is 422 `Must be greenhouse, lever or ashby`.
- `board` is the slug from the board's URL (`boards.greenhouse.io/<board>`, `jobs.lever.co/<board>`, `jobs.ashbyhq.com/<board>`). It's trimmed, and its case is kept. A missing or blank one is 422 `Required`. Otherwise it must be 1–100 letters, digits, hyphens or underscores, so nothing in it can change the path or the query of the board API URL.
- Both fields are checked at once, and the 422 body lists every failing field. A body that isn't valid JSON is 400, and a body that isn't `application/json` is 415.
- Like every route that changes something, it accepts only a local `Host` and refuses cross-site and form-encoded requests with 403.

It answers **202** `{"job": Job, "reused": bool}`. If a `source_board` job for the same source and board is already running, that job comes back with `reused: true`. If one is queued, its `run_at` is pulled forward to now and it comes back with `reused: true`. Otherwise a new job is queued with `reused: false`. The check and the insert run in one transaction under an advisory lock on the source and board, so two requests can't both queue a fetch. Follow the job with `GET /jobs/<id>` or the event stream. A database error is 500.

## The job

A `source_board` job's payload is `{"source": …, "board": …}`. Each attempt:

1. **Fetches the whole board into memory** (see below). If any request fails (timeout, connection error, a non-2xx status, a non-JSON or over-size body, a refused redirect), or the answer has the wrong shape, nothing is written and the attempt is retried. The queue's backoff and its three attempts apply, so a wrong slug fails after three tries. An invalid payload fails at once.
2. **Maps** each entry to a posting (see [Field mapping](#field-mapping)). Entries with no id, no title or no http(s) posting URL, and unlisted Ashby jobs, are skipped and counted. An id that appears twice in one fetch keeps the last entry.
3. **Saves** in one transaction, under the same advisory lock as the route: each posting is upserted on `(source, external_id)`, then the board's postings that weren't listed are marked removed (see [Removal](#removal)). A database error writes nothing and retries.

The job's `result` is:

```json
{"source": "lever", "board": "example-labs", "pages": 3, "fetched": 5, "skipped": 0,
 "inserted": 5, "updated": 0, "removed": 0, "duration_ms": 412}
```

`fetched` counts the entries the board returned, `inserted` and `updated` split the upserts, `removed` counts the postings newly marked removed, and `duration_ms` covers the fetch and the save. The API log gets one line per success, such as `sourcing: lever/example-labs: 5 fetched, 5 new, 0 updated, 0 removed, 0 skipped in 3 page(s), 412ms`, and a warning with the source, the board and the reason per failure. Descriptions and URLs are never logged.

The job runs on the shared queue, so `WAYPOINT_JOB_CONCURRENCY` caps it along with every other job. Because the route reuses a queued or running job, only one fetch per board is queued or running at a time. The kill switch doesn't stop it, since it isn't an agent run.

## Boards and pagination

Each request goes through the same fetcher as [Capture from URL](capture.md): the 15-second timeout, the redirect guard, and a body cap, which is 25 MiB here instead of capture's 5 MiB.

| Board | Request | Pages |
|---|---|---|
| Greenhouse | `GET {api}/v1/boards/{board}/jobs?content=true` | One request. The body is `{"jobs": […], "meta": {"total": n}}`. If `meta.total` is present and differs from the number of jobs returned, the fetch counts as partial and fails. |
| Lever | `GET {api}/v0/postings/{board}?mode=json&skip={skip}&limit=100` | Starts at `skip=0` and adds 100 while a page comes back with exactly 100 postings. The first shorter page, including an empty one, ends the board. Needing more than 50 pages fails the fetch. |
| Ashby | `GET {api}/posting-api/job-board/{board}` | One request. The body is `{"jobs": […]}`. |

The API bases are `https://boards-api.greenhouse.io`, `https://api.lever.co` and `https://api.ashbyhq.com`. Lever's EU host isn't supported.

## Field mapping

| Column | Greenhouse | Lever | Ashby |
|---|---|---|---|
| `external_id` | `id` (a number, stored as text) | `id` | `id` |
| `title` | `title` | `text` | `title` |
| `company` | `company_name`, else the slug humanised (`example-labs` → `Example Labs`) | the slug humanised | the slug humanised |
| `url` | `absolute_url` | `hostedUrl` | `jobUrl` |
| `apply_url` | `absolute_url` | `applyUrl` | `applyUrl` |
| `department` | the first `departments[].name` | `categories.department`, else `categories.team` | `department` |
| `employment_type` | none | `categories.commitment` | `employmentType` |
| `location_raw` | `location.name` | `categories.location` | `location` |
| `description` | `content`, which is HTML escaped once, decoded and then turned into text | `description`, then each of `lists[]` as its `text` heading line followed by its items, then `additional`, separated by blank lines | `descriptionHtml` as text, else `descriptionPlain` |
| `posted_at` | `first_published`, else `updated_at` | `createdAt` (epoch milliseconds) | `publishedAt` |

Text fields have their entities decoded and whitespace collapsed, are at most 200 characters, and are `null` when blank. `url` must be an absolute http or https URL of at most 2048 characters, or the entry is skipped. An `apply_url` that fails the same check is stored as `null`. A `posted_at` that doesn't parse is stored as `null`.

HTML descriptions become plain text: `script`, `style`, `noscript`, `template` and `svg` are dropped, paragraphs, headings, lists, tables and block quotes are separated by a blank line, `div`, `li`, `tr`, `pre` and `br` start a new line, each list item starts with `- `, whitespace inside a line collapses to one space, and entities are decoded. A description is at most 100,000 characters.

## Locations

Each posting keeps the board's raw text in `location_raw` and a normalized `remote`, `city`, `region` and `country`:

- `remote` is true when the board flags the posting as remote (Lever `workplaceType: "remote"`, Ashby `isRemote: true`) or the raw text contains the word `remote`, in any case.
- An Ashby `address.postalAddress` with any part wins: `addressLocality` is the city, `addressRegion` the region and `addressCountry` the country.
- Otherwise the raw text is parsed. Only the part before the first `;` or `|` counts, the word `remote` and the separators around it are removed, and the rest is split on commas:
  - one part is the country when the country table knows it, and the city otherwise;
  - two parts are the city and then a US state (by code or name, stored as its code with country `US`), a known country, or else the region;
  - three or more parts are the city, the region and the last part as the country.
- A Lever `country` code overrides the parsed country.

Countries are stored as ISO 3166-1 alpha-2 codes when the table knows them: the US, the UK, Canada, Germany, France, Ireland, the Netherlands, Spain, Portugal, Poland, Sweden, Switzerland, Israel, India, Singapore, Japan, Australia, Brazil and Mexico, by name and, except where the code is also a US state code (CA, DE, IN, IL), by code. Any other country is kept as written.

| Raw | remote | city | region | country |
|---|---|---|---|---|
| `Remote - US` | yes | | | US |
| `New York, NY` | no | New York | NY | US |
| `San Francisco, California` | no | San Francisco | CA | US |
| `London, United Kingdom` | no | London | | GB |
| `Toronto, ON, Canada` | no | Toronto | ON | CA |
| `Austin, TX; Remote` | yes | Austin | TX | US |

## Removal

`first_seen_at` is when Waypoint first saw a posting, and `last_seen_at` is the last successful fetch that listed it. After the upserts, every live posting on the same source and board whose `last_seen_at` is older than the save is marked with `removed_at`. A posting that comes back later gets `removed_at = NULL` again and keeps its `first_seen_at`. A successful fetch of an empty board marks every live posting on that board removed. A failed or partial fetch changes nothing, and postings on other boards or from other sources are never touched.

`job_postings` is user data: `DELETE /data` empties it.

## Configuration for tests

The board settings sit in `capture::Config` beside the board API bases, and `main.rs` uses the defaults: `board_page_size` (100), `max_board_pages` (50) and `max_board_bytes` (25 MiB). The integration tests in `api/tests/sourcing.rs` point `greenhouse.api`, `lever.api` and `ashby.api` at a wiremock server and set `board_page_size` to 2, so the recorded fixtures in `api/tests/fixtures/sourcing/` cover Lever's paging. No test reaches a real board.
