# Capture from URL

On the Applications page, **Capture from URL** takes a job posting's link, reads the page and opens **Add application** with the company, title, URL, location and compensation filled in. Nothing is saved until you check the fields and click **Add application**. The API route is `POST /captures` (body `{"url": "…"}`); the code lives in `api/src/capture/`.

## Tiers

The first tier that finds a title wins. Fields are never merged across tiers.

1. **JSON-LD.** Every `<script type="application/ld+json">` on the page is parsed (scripts that don't parse are skipped) and searched for a schema.org `JobPosting`, including inside top-level arrays and `@graph`. The first one with a title and a hiring organization is used: `hiringOrganization` for the company, `jobLocation` (plus `Remote` for `jobLocationType: TELECOMMUTE`) for the location and `baseSalary` for the pay. The route answers 200 with `method: "json_ld"`.
2. **Job boards.** When the pasted URL is a Greenhouse (`/{board}/jobs/{id}` or `/embed/job_app?for={board}&token={id}`), Lever (`/{site}/{id}[/apply]`) or Ashby (`/{org}/{id}[/application]`) posting, the board's public posting API is called:
   - Greenhouse: `GET {api}/v1/boards/{board}/jobs/{id}?pay_transparency=true`
   - Lever: `GET {api}/v0/postings/{site}/{id}`
   - Ashby: `GET {api}/posting-api/job-board/{org}?includeCompensation=true`, then the job with that id

   Detection uses only the pasted URL, and slugs and ids must match `[A-Za-z0-9_-]+`. This tier also runs when the page itself couldn't be fetched. The route answers 200 with `method` set to the board's name. A failed board call is logged and the capture moves on.
3. **Claude.** If the page was read but neither tier found anything, its visible text is extracted and a `capture_extract` job is queued. The route answers 202 `{"job_id": n}`, and the web app follows the job over the event stream. When it succeeds, the job's `result` is the same `Capture` shape with `method: "llm"`.

Every value is normalised the same way: HTML entities decoded, whitespace collapsed, blank values as `null`, and at most 200 characters, so the add form never opens invalid. Pay is formatted like `USD 180,000–210,000 per year`.

## Limits and failures

| Limit | Value |
|---|---|
| URL length | 2048 characters, absolute http or https, no username or password |
| Whole fetch (every redirect hop and the body) | 15 seconds |
| Body size (`Content-Length`, then while streaming) | 5 MiB |
| Redirects | 5 |
| Page text sent to Claude | 30,000 characters |

A page that can't be read answers 502 `{reason, message}`, where `reason` is `timeout`, `unreachable` (DNS, connect, TLS, a bad redirect or too many redirects), `http_status`, `not_html`, `too_large` or `empty_page`. The body never includes page content. An invalid URL answers 422 on the `url` field, and nothing is fetched.

Like every route that changes something, `POST /captures` only accepts requests to a local `Host`, refuses cross-site and form-encoded requests with 403, and takes only JSON. That's what stops another website you visit from making Waypoint fetch URLs.

## The redirect guard

The URL you paste is fetched wherever it points, local and private addresses included: you typed it, and Waypoint only runs on your machine. Redirects are followed by Waypoint itself, and each hop is checked before it is requested:

- the next host is resolved, and the hop is **refused** if the previous hop's address was public and any address of the next host is not;
- after each response, the address it actually came from is checked the same way, which catches a DNS answer that changed between the lookup and the connect.

Non-public means `127.0.0.0/8`, `::1`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `fc00::/7`, `fe80::/10`, `0.0.0.0` and `::`. An IPv4-mapped IPv6 address is judged by its IPv4 part. So a public page can't bounce Waypoint into your local network, while a local page can still redirect to another local page. A refused hop answers 422 on `url` with "The link redirects to a private network address, so Waypoint didn't follow it.", and the private host is never requested.

Captures ignore proxy settings (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`): through a proxy every response would come from the proxy's address, and the per-hop checks would mean nothing.

What's left: if a DNS answer changes between the lookup and the connect, one GET can reach the private host before the check after it fires. Its response is discarded unread.

## What is sent to Claude

Only for the third tier, and only the page's visible text: the `<title>`, the meta description and the body text, without scripts, styles, `noscript`, templates and SVG, capped at 30,000 characters, plus the URL. The call goes through the usual `claude -p` runner (`--tools ""`, `--strict-mcp-config`, no saved session, an empty working directory) with model `haiku` and a `--json-schema` for the four fields. The prompt tells the model the page text is untrusted and to ignore instructions in it, and the output is validated again in Rust. The page text is stored only in the job's payload, which `GET /jobs/{id}` never returns.

A usage limit defers the job until the queue resumes; any other failure retries up to the job's attempts and then fails, and the dialog offers **Add by hand**.

## Configuration for tests

There are no environment variables. `capture::Config` is the sixth argument of `waypoint_api::rocket()`; `main.rs` always passes `Config::default()`. Tests change it:

- `greenhouse`, `lever` and `ashby` are each a `Board { hosts, api }`. A host matches a URL's `host` or `host:port`, so a test can set `hosts: vec!["127.0.0.1:{port}"]` and `api` to a wiremock server's URI to exercise the board tier locally (see `api/tests/capture.rs`).
- `timeout` and `max_bytes` can be shortened to test the timeout and the size cap.
- `treat_as_public` lists socket addresses (IP and port) that the redirect guard treats as public, so a loopback wiremock server can play a "public" origin that redirects to loopback. Leave it empty anywhere else.
