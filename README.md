# Waypoint

Waypoint is a personal job-search assistant that runs on your own machine. It tracks applications through any interview process, manages resume versions, reads recruiter email to keep statuses current, sources matching roles and prepares applications for one-click approval. It is a SvelteKit single-page app backed by a Rust (Rocket) API and PostgreSQL, all bound to localhost. See [ADR 0001](docs/adr/0001-stack.md) for the stack decision, [ROADMAP.md](ROADMAP.md) for the plan and [the design system](docs/design-system.md) for UI tokens and primitives.

## Prerequisites

- Rust stable. `rust-toolchain.toml` pins the channel, so `rustup` installs it on first use.
- Node 22 or newer, with npm.
- Docker with Compose, running.

## Quickstart

```sh
cp .env.example .env
./scripts/dev.sh
```

`scripts/dev.sh` starts Postgres in Docker Compose, installs the web dependencies on first run, then runs the API on <http://127.0.0.1:8000> (`GET /health`) and the web app on <http://127.0.0.1:5173>. Press Ctrl-C to stop the API and web server.

Postgres keeps running after the script exits, and its data persists in the `waypoint-pgdata` volume. Stop it with:

```sh
docker compose down
```

## Dashboard

The web app opens on a dashboard inside the app shell. The top bar links to the Dashboard, [Applications](#applications) and [Resumes](#resumes) and shows whether the live event stream is connected (Connecting, Live, Reconnecting). The dashboard has three cards:

- **System**: whether the API and its database are reachable (`GET /health`), re-checked each time the event stream (re)connects.
- **Google**: the Google connection (`GET /auth/google`). Until the three Google settings are in `.env` it reads "Not configured" and the Connect button is disabled; see [Google account](#google-account).
- **Claude**: a **Test Claude** button that enqueues a test prompt through `claude -p` and follows the job live until it shows the reply.

The browser only talks to its own origin. In development, Vite proxies `/api/*` to the API (`VITE_API_URL`, default <http://127.0.0.1:8000>) with the `/api` prefix stripped, passing `Host`, `Origin` and `Sec-Fetch-Site` through unchanged, so the API's local-host and same-origin guards still apply and the API needs no CORS headers. Vite's own CORS is turned off (`server.cors: false`): by default it answers any `localhost` or `127.0.0.1` origin on any port, which would let a page on another local port read job errors and the event stream through the proxy. The proxy exists only in `vite dev`; the built SPA isn't served or proxied yet.

The end-to-end tests start the real API (with a fake `claude` that answers `pong` after a second) and the dev server, then drive the dashboard, applications and resumes pages in Chromium and check that the proxy sends no CORS headers to other origins. The applications test adds, edits and deletes its own uniquely named application in the Compose database, and the resumes test uploads `api/tests/fixtures/resumes/resume.pdf` under a unique name, links it to its own application, then removes both. It runs locally, not in CI. It needs the Compose Postgres, a `.env`, Chromium installed once, and ports 8000 and 5173 free:

```sh
docker compose up -d --wait db
npx --prefix web playwright install chromium   # once
npm --prefix web run test:e2e
```

## Applications

The Applications page (`/applications`) lists every job application you're tracking, the most recently updated first, as cards showing the company, title, location, status and when it was last updated. **Add application** opens a form; clicking a card opens its detail page (`/applications/<id>`), where you change the status, edit the details or delete the application.

An application has a company and a title (both required) and optional free-text fields: job posting URL, location, compensation (e.g. "$180k–$210k + equity"), source (e.g. "Referral") and notes. Its status is one of Sourced (the default), Applied, Interviewing, Offer and Rejected. The API trims every field and stores a blank optional field as `null`. Company, title, location, compensation and source are limited to 200 characters, the URL to 2048 and notes to 20,000, and the URL must be an absolute `http` or `https` URL.

| Route | Does |
|---|---|
| `GET /applications` | Lists every application, newest `updated_at` first |
| `POST /applications` | Adds one (`company`, `title`, optional fields and `status`): 201 |
| `GET /applications/<id>` | Reads one |
| `PUT /applications/<id>` | Replaces the text fields; an omitted optional field is cleared and the status is left alone |
| `PUT /applications/<id>/status` | Sets the status (`{"status": "applied"}`); `status_changed_at` moves only when it changes |
| `PUT /applications/<id>/resume` | Links the [resume version](#resumes) sent with it (`{"resume_version_id": 3}`), or clears it with `null`; an unknown version is a 422 |
| `DELETE /applications/<id>` | Deletes one permanently: 204 |

Every application carries `resume_version_id` (`null` until set). Only `PUT /applications/<id>/resume` changes it, and `updated_at` moves only when the link changes. The detail page's **Resume** section picks the version from a "Resume sent" menu and links to it.

An invalid field answers 422 with `{"errors": [{"field", "message"}]}`, one entry per failing field; a body that isn't valid JSON for the route (or an unknown status) answers 400 with a single `body` entry. Bodies must be `Content-Type: application/json` (anything else is refused: 403 for an HTML form, 415 otherwise), and every changing route answers 403 to a request a browser sends from another site, so a page you visit can't edit your applications. With the API running:

```sh
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"company": "Acme", "title": "Staff Engineer", "location": "Remote"}' \
  http://127.0.0.1:8000/applications                                        # 201, "status": "sourced"
curl -s http://127.0.0.1:8000/applications
curl -s -X PUT -H 'Content-Type: application/json' -d '{"status": "applied"}' \
  http://127.0.0.1:8000/applications/1/status
curl -s -X PUT -H 'Content-Type: application/json' \
  -d '{"company": "Acme", "title": "Staff Engineer", "location": "Berlin"}' \
  http://127.0.0.1:8000/applications/1
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE http://127.0.0.1:8000/applications/1   # 204
```

## Resumes

The Resumes page (`/resumes`) keeps every resume PDF you send out, newest first, as cards showing the name, file name, size, upload date, how many applications use it, and a "No text" badge when no text could be extracted. **Upload resume** picks a PDF and a name (it defaults to the file name without `.pdf`). Clicking a card opens the viewer (`/resumes/<id>`) with the details, the extracted text, an **Open PDF** link that opens the file in the browser's PDF viewer in a new tab, and **Delete**. A version can't be edited: upload a new one instead.

Uploads are PDF only, at most 10 MiB (10,485,760 bytes), and must start with `%PDF-`. The name is required (trimmed, at most 200 characters); the original file name is optional (at most 255, `resume.pdf` when missing). The PDF bytes are stored in Postgres (`resume_versions.pdf`, `BYTEA`), not on disk, so they're backed up and deleted with the row. Text is extracted when you upload, for later scoring and application prep; when that fails (an encrypted, scanned or malformed PDF) the version is still stored, with `text: null` and `has_text: false`.

| Route | Does |
|---|---|
| `GET /resumes` | Lists every version without its text, newest `created_at` first, with `has_text` and `application_count` |
| `POST /resumes?name=<label>&filename=<file name>` | Uploads the raw PDF as the body (`Content-Type: application/pdf`, not multipart): 201 |
| `GET /resumes/<id>` | Reads one, with its `text` (or `null`) |
| `GET /resumes/<id>/pdf` | The exact stored bytes, `inline`, with `nosniff` and `no-store` |
| `DELETE /resumes/<id>` | Deletes one permanently: 204, or 409 while an application links it (unlink it first) |

An invalid `name`, `filename` or `file` (empty, or not a PDF) answers 422 with every failing field, a body over 10 MiB answers 413, and any type other than `application/pdf` answers 415; nothing is stored on any error. Like the applications routes, the upload and delete answer 403 to an HTML form or multipart body and to a request a browser sends from another site. With the API running:

```sh
curl -s -X POST -H 'Content-Type: application/pdf' --data-binary @resume.pdf \
  'http://127.0.0.1:8000/resumes?name=Backend%20CV&filename=resume.pdf'   # 201, "has_text": true
curl -s http://127.0.0.1:8000/resumes/1                                    # includes "text"
curl -s -o resume-copy.pdf http://127.0.0.1:8000/resumes/1/pdf
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE http://127.0.0.1:8000/resumes/1   # 204, or 409 if linked
```

## Model calls

Waypoint calls Claude through your Claude subscription, not the API. Every call spawns the [Claude Code](https://docs.claude.com/en/docs/claude-code) CLI (`claude -p`) in an empty temp directory with no tools and no saved session, and logs the model, tokens, duration and reported cost to the `claude_calls` table. Install the CLI and log in once with `claude` before using any model-backed feature.

The runner removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` and the `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`/`_FOUNDRY` switches from the CLI's environment, so nothing in your shell moves billing or traffic away from the subscription. It also withholds Waypoint's own settings (`DATABASE_URL`, `POSTGRES_*`, `WAYPOINT_*`). `WAYPOINT_CLAUDE_BIN` (default `claude` on `PATH`) and `WAYPOINT_CLAUDE_TIMEOUT_SECS` (default 300) are optional; see `.env.example`. The CLI runs in its own process group, and when a call ends, times out or is cancelled the runner kills that whole group, so nothing the CLI started outlives the call.

To check the round trip, start the app with `./scripts/dev.sh`, open <http://127.0.0.1:5173> and click **Test Claude** on the dashboard: the card shows Queued, Running and then Succeeded with the reply `pong`. Or, in another terminal:

```sh
curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/claude/ping
docker compose exec db psql -U waypoint -c 'select status, model, input_tokens, output_tokens, cost_usd from claude_calls order by id desc limit 1'
```

The ping should answer `pong` with the model and token counts. `/dev/claude/ping` exists only in debug builds (`cargo run`); release builds don't serve it. It answers 403 without the `X-Waypoint-Dev: 1` header or when a browser sends it from another site, so a web page you visit can't spend your usage.

The API only answers requests whose `Host` header is `127.0.0.1`, `localhost` or `[::1]` (any port), and returns 403 to everything else, so a site that re-resolves its own domain to `127.0.0.1` (DNS rebinding) can't reach it.

## Jobs and live events

Background work, including every model call, runs through a job queue in Postgres (the `jobs` table). An in-process worker runs at most `WAYPOINT_JOB_CONCURRENCY` jobs at once (default 2) and retries failures with exponential backoff (10s, doubling, up to 10 minutes; 3 attempts). When the Claude subscription's usage limit is hit, the job goes back in the queue without using up an attempt and the whole queue pauses for `WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS` (default 900). Jobs left running when the API stopped are requeued on the next start.

`GET /events` streams job and queue events as server-sent events (`job.queued`, `job.started`, `job.retrying`, `job.deferred`, `job.succeeded`, `job.failed`, `queue.paused`, `queue.resumed`), starting with `app.connected`. There is no replay, so a client that reconnects reads the current state from `GET /jobs/<id>`.

To watch a test job, start the app with `./scripts/dev.sh`, then in one terminal:

```sh
curl -N -H 'Host: 127.0.0.1' http://127.0.0.1:8000/events
```

and in another:

```sh
curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/jobs/test-prompt   # {"job_id":1}
curl -s http://127.0.0.1:8000/jobs/1                                                # result.text is "pong"
curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' http://127.0.0.1:8000/health   # 403
```

The stream shows `job.queued`, `job.started` and `job.succeeded` for the job. Like the ping, `/dev/jobs/test-prompt` exists only in debug builds and needs the `X-Waypoint-Dev: 1` header.

## Google account

Gmail and Calendar access goes through one Google account that you connect with your own Google Cloud OAuth client. [docs/google-oauth.md](docs/google-oauth.md) walks through creating the client, the three `.env` settings (`WAYPOINT_GOOGLE_CLIENT_ID`, `WAYPOINT_GOOGLE_CLIENT_SECRET`, `WAYPOINT_TOKEN_KEY`) and connecting. Waypoint asks only for read-only Gmail and Calendar events, and stores the tokens encrypted.

Once it's set up, open <http://127.0.0.1:8000/auth/google/connect> to connect, check `curl -s http://127.0.0.1:8000/auth/google`, and disconnect with `curl -s -X DELETE http://127.0.0.1:8000/auth/google`. Without the settings, everything else works and Google reports "not configured".

## API contract

The API's OpenAPI 3.1 document is generated from the `#[utoipa::path]` annotations on the Rocket handlers (listed in `api/src/openapi.rs`) and committed at `api/openapi.json`. The web app's types are generated from that document into `web/src/lib/api/schema.d.ts` with openapi-typescript, and `#lib/api` exports a typed client (`createApiClient()`, built on openapi-fetch) plus the `Health`, `Job`, `Event`, `GoogleStatus`, `Application`, `ApplicationStatus`, `NewApplication`, `ApplicationInput`, `ValidationErrors`, `FieldError`, `ResumeVersion`, `ResumeVersionDetail` and `ResumeLink` types. `#lib/applications/api.js` and `#lib/resumes/api.js` wrap the applications and resumes routes in helpers that never throw and return `{ kind: 'ok' | 'invalid' | 'not-found' | 'conflict' | 'failed' }`. The debug-only `/dev` routes are not part of the contract.

After changing any route or response type, regenerate both files and commit them:

```sh
scripts/gen-api.sh
```

Never edit `api/openapi.json` or `schema.d.ts` by hand. CI fails when either is stale: a Rust test compares `api/openapi.json` with the code and checks that every mounted route is documented, and the web job regenerates `schema.d.ts` and fails on any diff.

## Checks and tests

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the same format, lint, type and test checks on every pull request and every push to `develop`; Rust tests run against a Postgres service container.

Rust (no database needed):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
```

Rust tests (need the Compose Postgres and `DATABASE_URL` from `.env`):

```sh
docker compose up -d --wait db && set -a && . ./.env && set +a && cargo test --workspace
```

Web:

```sh
npm --prefix web run check   # svelte-check, fails on warnings
npm --prefix web run lint    # ESLint
npm --prefix web test        # Vitest, run once (component tests under jsdom)
npm --prefix web run build   # static SPA in web/build
```

API contract (no database needed):

```sh
scripts/gen-api.sh && git diff --exit-code   # api/openapi.json and schema.d.ts are current
```
