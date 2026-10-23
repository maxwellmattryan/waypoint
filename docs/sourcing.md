# Board sourcing

Waypoint reads target companies' public Greenhouse, Lever and Ashby job boards into one table, `job_postings`. One `source_board` job reads one board, identified by its source and its slug, and keeps that board's rows in step with what it lists. You ask for a board with the **Source company** button on the [Postings page](#postings-page), which calls `POST /sourcing/boards`. There's no schedule yet, so you ask again each time you want a board refreshed. The code lives in `api/src/sourcing/`.

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

## Reading postings

Two read-only routes serve `job_postings`. Both accept only a local `Host` (403 otherwise), and a database error is 500.

`GET /postings` lists postings, newest first by `id` (the order Waypoint first saw them). Every query parameter is optional, a blank or whitespace-only value counts as absent, and the filters combine with AND:

| Parameter | Matches |
|---|---|
| `company` | Postings whose `company` contains the text, ignoring case. `%` and `_` are plain characters. |
| `source` | `greenhouse`, `lever` or `ashby`, exactly. Anything else is 422 `Unknown source`. |
| `location` | Postings whose `location_raw`, `city`, `region` or `country` contains the text, ignoring case. |
| `remote` | `true` for only remote postings, `false` for only non-remote ones. Anything else is 422 `Must be true or false`. |
| `include_removed` | `true` also lists postings the board stopped listing (`removed_at` set). Absent or `false` hides them. Anything else is 422 `Must be true or false`. |
| `limit` | The most postings to return: 50 by default, clamped to 1–200. |
| `before` | Only postings with an `id` below this one. Pass the smallest `id` of a page to get the next one. |

Every failing parameter is listed in one 422 body. Each item is a `JobPosting`: every `job_postings` column except `description`, so a page stays small, plus the posting's [fit](#fit-scores): `fit_score` (its latest applied score, `null` when it was never scored), `fit_band` (`strong`, `partial` or `weak`, `null` with the score) and `fit_in_review` (`true` when a newer score waits in the review queue).

`GET /postings/{id}` returns one posting with every column, `description` included (plain text, possibly `""`), and the same `fit_*` fields. A removed posting is still returned. An unknown id is 404.

## Postings page

The web app's **Postings** page (`/postings`) lists the postings with filters for company, location, source, remote and removed postings, 50 at a time with **Load more**. The text filters reload 300 ms after the last keystroke. Each posting links to its own page (`/postings/<id>`), which shows its details, links to the posting and its apply page when they're http(s) URLs, and the description as plain text.

**Source company** opens a dialog that takes a board and a slug, checks them the way the route does, and queues the fetch. When a fetch the page started finishes, it shows a toast with the counts (`example-labs: 3 new, 1 updated, 0 removed`) or the failure. The list refreshes after any `source_board` job succeeds.

Each posting with an applied fit score shows a fit badge beside its source badges, for example `82 · Strong fit`, and a `Needs review` badge when a newer score waits in review. A posting that was never scored shows neither. The list refreshes silently when a `score_fit` run succeeds.

## Fit scores

The `score_fit` workflow (`api/src/workflows/score_fit.rs`) scores how well one posting fits you, from 0 to 100, against what Waypoint knows about you, and cites its evidence. There's no separate profile: the [profile import](profile-import.md) stores positions, skills and education as `fact` memories, so the user model is your **confirmed memories**. A run sees every confirmed `preference` memory (up to 30, newest first, whatever words the posting uses, so a deal-breaker always reaches the prompt) and up to 20 confirmed `fact` or `answer` memories [retrieved](agent-runtime.md#retrieval) on the posting's title, department, employment type, location and description. Both sets are recorded in `run_memories`, so the run inspector shows them under "Memories used". Proposed memories are never used. With no confirmed memories at all the run fails before any model call: `Waypoint knows nothing about you yet: add or confirm memories first`.

The run has one step, `score`, and makes one tool-less `claude -p --json-schema` call (model `sonnet`, purpose `posting.score_fit`). The prompt says the posting is data to assess, not instructions. The posting is shown as a block (`Title:`, `Company:`, `Location:`, `Remote:`, `Department:`, `Employment type:`, then the description cut to 12,000 characters), fenced so its text can't close the block. The answer is a score, a confidence from 0 to 1, a one-line summary and 1 to 8 reasons, each `for` or `against` the fit and citing 1 to 4 sources: a memory shown in the prompt, by id, or a verbatim excerpt of the posting block (at most 300 characters).

Rust checks the answer:

- **Invalid** (the run fails, nothing is stored): a score outside 0–100, a confidence that isn't a number from 0 to 1, a summary that is blank or over 300 characters, or no reason left with a valid citation. The error names each broken rule.
- **Dropped:** reasons past 8, a reason with an unknown stance or a blank or over-500-character text, citations past 4, a memory citation whose id wasn't shown, a posting excerpt that is blank, too long or not in the posting block (matched ignoring case and runs of whitespace), and a reason left with no citation.
- **Applied:** confidence of at least 0.8 and nothing dropped. The score becomes the posting's current score.
- **In review:** confidence below 0.8, or anything dropped. A `score_fit.score` [review item](agent-runtime.md#review-items) is stored and the score isn't applied.

The band comes from the score in Rust: `strong` from 70, `partial` from 40, `weak` below. Each valid score is a `posting_fit_scores` row with its `status` (`applied`, `in_review` or `dismissed`), score, band, confidence, summary, reasons, run, model call and the call's cost. A stored reason's citations keep a snapshot: a memory citation has the memory's topic and its content cut to 300 characters, and a posting citation the trimmed excerpt. A rerun of the step returns the row its run already stored without a second call. The run's output is `{posting_id, fit_score_id, score, band, confidence, status, review_item_id, dropped: {reasons, citations}}`.

On the Review page a `score_fit.score` item shows the posting (company and title, linking to its page), the score and band, the summary and the reasons, with **Apply** and **Dismiss**. **Apply** makes it the posting's current score. **Dismiss** marks it `dismissed` and records the usual dismissal [learning signal](agent-runtime.md#learning-signals); the posting keeps its previous applied score.

### Fit routes

Both accept only a local `Host` (403 otherwise), and a database error is 500.

`GET /postings/{id}/fit` returns `{posting_id, current, in_review, run}`. `current` is the latest applied score (the most recently applied one, so an accepted older score replaces a newer one), `in_review` the latest `in_review` score newer than `current` whose review item is still pending, and `run` the latest `score_fit` run for the posting as `{id, status, error}`. Each is `null` when there is none. A score is `{id, run_id, status, score, band, confidence, summary, reasons, cost_usd, created_at}`, each reason `{stance, text, citations}` and each citation `{source, memory_id, topic, text}`. A memory citation's `memory_id` is `null` once that memory has been deleted; its snapshot stays. An unknown posting is 404.

`POST /postings/{id}/fit` with `{}` queues a `score_fit` run with input `{"posting_id": id}` and answers **202** `{"run": Run, "reused": false}`. If a `score_fit` run for the posting is already waiting (`queued` or `paused`) or `running`, it comes back with `reused: true` and nothing is queued. The posting row is locked while the check and the insert run, so two requests can't both queue a run. While the kill switch is on the run is `paused`, like any run. Removed postings can be scored. An unknown posting is 404, a body that isn't `application/json` 415, and unknown keys 400. It refuses cross-site and form-encoded requests with 403.

### Posting page

A **Fit** section sits above the posting's details:

- **Never scored:** "Not scored yet" and **Score fit**.
- **Scoring:** while the latest run is `queued`, `running` or `paused`, "Scoring…" with a skeleton and a disabled button; for `paused`, "Paused: agents are paused".
- **Failed:** when the latest run failed and no score is newer, "Scoring failed" with the run's error, a link to the run and **Try again**.
- **Applied:** the score, its band, "NN% confident", the summary and the reasons grouped "For" and "Against". Each citation is a quoted posting excerpt, or the memory's topic and snapshot with **View memory** (or "Deleted memory" once it's gone). Below: "Scored <date> · $<cost>" with a link to the run, and **Rescore**.
- **In review, nothing applied:** the provisional score in the same layout, muted, with a `Needs review` badge and a link to the Review page.
- **In review with an applied score:** the applied score, plus "A newer score is waiting in review" with a link to the Review page.

The section reloads when a `run.updated` event arrives for a `score_fit` run of this posting, and after the event stream reconnects or lags. If it can't load, it says "Couldn't load the fit score" with **Try again**, and the rest of the page still shows.

The [`score_fit` eval set](evals.md#the-score_fit-eval-set) measures the scores.

[Goals](goals.md#execution) reuse both halves of this page: a plan's `source` step queues `source_board` jobs through `sourcing::request` (so it shares a fetch already queued or running), and its `score` step queues `score_fit` runs through `score_fit::request`, which `POST /postings/{id}/fit` also uses, reusing a posting's applied score instead of rescoring it.

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

Text fields have their entities decoded and whitespace collapsed, are at most 200 characters, and are `null` when blank. `url` must be an absolute http or https URL of at most 2048 characters, or the entry is skipped. An `apply_url` that fails the same check is stored as `null`. A `posted_at` that doesn't parse is stored as `null`. Every NUL character (U+0000) in an entry is dropped before mapping, since Postgres text can't store it.

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
