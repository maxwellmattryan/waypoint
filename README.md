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

## Model calls

Waypoint calls Claude through your Claude subscription, not the API. Every call spawns the [Claude Code](https://docs.claude.com/en/docs/claude-code) CLI (`claude -p`) in an empty temp directory with no tools and no saved session, and logs the model, tokens, duration and reported cost to the `claude_calls` table. Install the CLI and log in once with `claude` before using any model-backed feature.

The runner removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` and the `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`/`_FOUNDRY` switches from the CLI's environment, so nothing in your shell moves billing or traffic away from the subscription. It also withholds Waypoint's own settings (`DATABASE_URL`, `POSTGRES_*`, `WAYPOINT_*`). `WAYPOINT_CLAUDE_BIN` (default `claude` on `PATH`) and `WAYPOINT_CLAUDE_TIMEOUT_SECS` (default 300) are optional; see `.env.example`. The CLI runs in its own process group, and when a call ends, times out or is cancelled the runner kills that whole group, so nothing the CLI started outlives the call.

To check the round trip, start the app with `./scripts/dev.sh` and, in another terminal:

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

The API's OpenAPI 3.1 document is generated from the `#[utoipa::path]` annotations on the Rocket handlers (listed in `api/src/openapi.rs`) and committed at `api/openapi.json`. The web app's types are generated from that document into `web/src/lib/api/schema.d.ts` with openapi-typescript, and `#lib/api` exports a typed client (`createApiClient()`, built on openapi-fetch) plus the `Health`, `Job`, `Event` and `GoogleStatus` types. The debug-only `/dev` routes are not part of the contract.

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
