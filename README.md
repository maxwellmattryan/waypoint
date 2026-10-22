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

The web app opens on a dashboard inside the app shell. The top bar links to the Dashboard and [Applications](#applications) and shows whether the live event stream is connected (Connecting, Live, Reconnecting). The dashboard has three cards:

- **System**: whether the API and its database are reachable (`GET /health`), re-checked each time the event stream (re)connects.
- **Google**: the Google connection (`GET /auth/google`). Until the three Google settings are in `.env` it reads "Not configured" and the Connect button is disabled; see [Google account](#google-account).
- **Claude**: a **Test Claude** button that enqueues a test prompt through `claude -p` and follows the job live until it shows the reply.

The browser only talks to its own origin. In development, Vite proxies `/api/*` to the API (`VITE_API_URL`, default <http://127.0.0.1:8000>) with the `/api` prefix stripped, passing `Host`, `Origin` and `Sec-Fetch-Site` through unchanged, so the API's local-host and same-origin guards still apply and the API needs no CORS headers. Vite's own CORS is turned off (`server.cors: false`): by default it answers any `localhost` or `127.0.0.1` origin on any port, which would let a page on another local port read job errors and the event stream through the proxy. The proxy exists only in `vite dev`; the built SPA isn't served or proxied yet.

The end-to-end tests start the real API (with a fake `claude` that answers `pong` after a second) and the dev server, then drive the dashboard and the applications pages in Chromium and check that the proxy sends no CORS headers to other origins. The applications test adds, edits and deletes its own uniquely named application in the Compose database. It runs locally, not in CI. It needs the Compose Postgres, a `.env`, Chromium installed once, and ports 8000 and 5173 free:

```sh
docker compose up -d --wait db
npx --prefix web playwright install chromium   # once
npm --prefix web run test:e2e
```

## Applications

The Applications page (`/applications`) lists every job application you're tracking, the most recently updated first, as cards showing the company, title, location, status and when it was last updated. **Add application** opens a form; clicking a card opens its detail page (`/applications/<id>`), where you change the status, edit the details or delete the application.

An application has a company and a title (both required) and optional free-text fields: job posting URL, location, compensation (e.g. "$180k–$210k + equity"), source (e.g. "Referral") and notes. Its status is one of Sourced (the default), Applied, Interviewing, Offer and Rejected. The API trims every field and stores a blank optional field as `null`. Company, title, location, compensation and source are limited to 200 characters, the URL to 2048 and notes to 20,000, and the URL must be an absolute `http` or `https` URL.

Each application also has its own interview **stages** (e.g. Recruiter screen, Onsite): on the detail page you add them, tick them off, move them up and down and remove them. A stage name is limited to 200 characters and an application to 50 stages. The **timeline** below them is an append-only log of what happened when: the application being added, every status change, every stage added, completed, reopened or removed, and the notes you add there (up to 20,000 characters). Choosing **Rejected** on the detail page asks for optional **rejection feedback** (what the company said) and records it in the timeline; once an application is rejected you can add more feedback later. Every timeline write also moves the application to the top of the list.

| Route | Does |
|---|---|
| `GET /applications` | Lists every application, newest `updated_at` first |
| `POST /applications` | Adds one (`company`, `title`, optional fields and `status`): 201 |
| `GET /applications/<id>` | Reads one |
| `PUT /applications/<id>` | Replaces the text fields; an omitted optional field is cleared and the status is left alone |
| `PUT /applications/<id>/status` | Sets the status (`{"status": "applied"}`); `status_changed_at` moves only when it changes. With `rejected`, an optional `feedback` is also recorded as rejection feedback |
| `DELETE /applications/<id>` | Deletes one permanently, with its stages and timeline: 204 |
| `GET /applications/<id>/stages` | Lists its stages in order |
| `POST /applications/<id>/stages` | Appends a stage (`{"name": "Onsite"}`): 201 with the whole list |
| `PUT /applications/<id>/stages/<stage_id>` | Completes or reopens a stage (`{"completed": true}`); returns the whole list |
| `PUT /applications/<id>/stage-order` | Reorders the stages (`{"stage_ids": [3, 1, 2]}`, every stage once); returns the whole list |
| `DELETE /applications/<id>/stages/<stage_id>` | Removes a stage; later stages move up; returns the remaining list |
| `GET /applications/<id>/events` | Lists its timeline, newest first |
| `POST /applications/<id>/events` | Adds a note (`{"kind": "note", "body": "…"}`) or, for a rejected application, rejection feedback (`"kind": "rejection_feedback"`): 201 |

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

## Capture from URL

On the Applications page, **Capture from URL** reads a job posting's link and opens the add form prefilled for you to check. It uses the page's JSON-LD first, then the Greenhouse, Lever or Ashby posting API, and asks Claude to read the page text only when neither has the details. [docs/capture.md](docs/capture.md) covers the tiers, the limits, the redirect guard and what is sent to Claude.

## Google account

Gmail and Calendar access goes through one Google account that you connect with your own Google Cloud OAuth client. [docs/google-oauth.md](docs/google-oauth.md) walks through creating the client, the three `.env` settings (`WAYPOINT_GOOGLE_CLIENT_ID`, `WAYPOINT_GOOGLE_CLIENT_SECRET`, `WAYPOINT_TOKEN_KEY`) and connecting. Waypoint asks only for read-only Gmail and Calendar events, and stores the tokens encrypted.

Once it's set up, open <http://127.0.0.1:8000/auth/google/connect> to connect, check `curl -s http://127.0.0.1:8000/auth/google`, and disconnect with `curl -s -X DELETE http://127.0.0.1:8000/auth/google`. Without the settings, everything else works and Google reports "not configured".

## API contract

The API's OpenAPI 3.1 document is generated from the `#[utoipa::path]` annotations on the Rocket handlers (listed in `api/src/openapi.rs`) and committed at `api/openapi.json`. The web app's types are generated from that document into `web/src/lib/api/schema.d.ts` with openapi-typescript, and `#lib/api` exports a typed client (`createApiClient()`, built on openapi-fetch) plus the `Health`, `Job`, `Event`, `GoogleStatus`, `Application`, `ApplicationStatus`, `NewApplication`, `ApplicationInput`, `ValidationErrors` and `FieldError` types. `#lib/applications/api.js` wraps the applications routes in helpers that never throw and return `{ kind: 'ok' | 'invalid' | 'not-found' | 'failed' }`. The debug-only `/dev` routes are not part of the contract.

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
