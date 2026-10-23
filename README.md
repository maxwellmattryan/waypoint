# Waypoint

Waypoint is a personal job-search assistant that runs on your own machine. It tracks applications through any interview process, manages resume versions, reads recruiter email to keep statuses current, sources matching roles and prepares applications for one-click approval. It is a SvelteKit single-page app backed by a Rust (Rocket) API and PostgreSQL, all bound to localhost. See [ADR 0001](docs/adr/0001-stack.md) for the stack decision, [ROADMAP.md](ROADMAP.md) for the plan and [the design system](docs/design-system.md) for UI tokens and primitives.

## Prerequisites

- Rust nightly, pinned to a dated build by `rust-toolchain.toml`. `rustup` installs it on first use, and it overrides your default toolchain in this repo. To bump it, set `channel` to a newer `nightly-YYYY-MM-DD` whose rustfmt and clippy are available (see the [rustup components history](https://rust-lang.github.io/rustup-components-history/)), then fix what fmt, clippy and the tests report.
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

The web app opens on a dashboard inside the app shell. The top bar links to the Dashboard, [Applications](#applications), [Resumes](#resumes), [Memory](#memory), [Review](#agent-runs-and-tools) and [Runs](#agent-runs-and-tools) and shows whether the live event stream is connected (Connecting, Live, Reconnecting). The dashboard has three cards:

- **System**: whether the API and its database are reachable (`GET /health`), re-checked each time the event stream (re)connects.
- **Google**: the Google connection (`GET /auth/google`). Until the three Google settings are in `.env` it reads "Not configured" and the Connect button is disabled; see [Google account](#google-account).
- **Claude**: a **Test Claude** button that enqueues a test prompt through `claude -p` and follows the job live until it shows the reply.

The browser only talks to its own origin. In development, Vite proxies `/api/*` to the API (`VITE_API_URL`, default <http://127.0.0.1:8000>) with the `/api` prefix stripped, passing `Host`, `Origin` and `Sec-Fetch-Site` through unchanged, so the API's local-host and same-origin guards still apply and the API needs no CORS headers. Vite's own CORS is turned off (`server.cors: false`): by default it answers any `localhost` or `127.0.0.1` origin on any port, which would let a page on another local port read job errors and the event stream through the proxy. The proxy exists only in `vite dev`; the built SPA isn't served or proxied yet.

The end-to-end tests start the real API (with a fake `claude` that answers `pong` after a second) and the dev server, then drive the dashboard, applications, resumes and runs pages in Chromium and check that the proxy sends no CORS headers to other origins. The applications test adds, edits and deletes its own uniquely named application in the Compose database, the board test adds one, drags it and moves it from its Move menu while a second page follows along live, then deletes it, and the resumes test uploads `api/tests/fixtures/resumes/resume.pdf` under a unique name, links it to its own application, then removes both. The runs test mocks the `/runs` routes and saves screenshots of the list and a run's page (light and dark) to `$WAYPOINT_SCREENSHOTS`, or `web/test-results/screenshots`. They run locally, not in CI. It needs the Compose Postgres, a `.env`, Chromium installed once, and ports 8000 and 5173 free:

```sh
docker compose up -d --wait db
npx --prefix web playwright install chromium   # once
npm --prefix web run test:e2e
```

## Applications

The Applications page (`/applications`) lists every job application you're tracking, the most recently updated first, as cards showing the company, title, location, status and when it was last updated. **Add application** opens a form; clicking a card opens its detail page (`/applications/<id>`), where you change the status, edit the details or delete the application.

The **View** tabs switch between List and Board, and the URL follows along (`/applications?view=board`), so a reload or a link keeps the Board view. The board has one column per status (Sourced, Applied, Interviewing, Offer, Rejected). Move a card by dragging it to another column, or by picking a status from its **Move** menu; on a touch device, use the menu. The card moves at once and goes back if the API refuses the change. Both views update live: an application added, edited, moved or deleted in another tab shows up without a reload, through the `application.*` events on the [event stream](#jobs-and-live-events).

An application has a company and a title (both required) and optional free-text fields: job posting URL, location, compensation (e.g. "$180k–$210k + equity"), source (e.g. "Referral") and notes. Its status is one of Sourced (the default), Applied, Interviewing, Offer and Rejected. The API trims every field and stores a blank optional field as `null`. Company, title, location, compensation and source are limited to 200 characters, the URL to 2048 and notes to 20,000, and the URL must be an absolute `http` or `https` URL.

Each application also has its own interview **stages** (e.g. Recruiter screen, Onsite): on the detail page you add them, tick them off, move them up and down and remove them. A stage name is limited to 200 characters and an application to 50 stages. The **timeline** below them is an append-only log of what happened when: the application being added, every status change, every stage added, completed, reopened or removed, and the notes you add there (up to 20,000 characters). Choosing **Rejected** on the detail page asks for optional **rejection feedback** (what the company said) and records it in the timeline; once an application is rejected you can add more feedback later. Every timeline write also moves the application to the top of the list.

| Route | Does |
|---|---|
| `GET /applications` | Lists every application, newest `updated_at` first |
| `POST /applications` | Adds one (`company`, `title`, optional fields and `status`): 201 |
| `GET /applications/<id>` | Reads one |
| `PUT /applications/<id>` | Replaces the text fields; an omitted optional field is cleared and the status is left alone |
| `PUT /applications/<id>/status` | Sets the status (`{"status": "applied"}`); `status_changed_at` moves only when it changes. With `rejected`, an optional `feedback` is also recorded as rejection feedback |
| `PUT /applications/<id>/resume` | Links the [resume version](#resumes) sent with it (`{"resume_version_id": 3}`), or clears it with `null`; an unknown version is a 422 |
| `DELETE /applications/<id>` | Deletes one permanently, with its stages and timeline: 204 |
| `GET /applications/<id>/stages` | Lists its stages in order |
| `POST /applications/<id>/stages` | Appends a stage (`{"name": "Onsite"}`): 201 with the whole list |
| `PUT /applications/<id>/stages/<stage_id>` | Completes or reopens a stage (`{"completed": true}`); returns the whole list |
| `PUT /applications/<id>/stage-order` | Reorders the stages (`{"stage_ids": [3, 1, 2]}`, every stage once); returns the whole list |
| `DELETE /applications/<id>/stages/<stage_id>` | Removes a stage; later stages move up; returns the remaining list |
| `GET /applications/<id>/events` | Lists its timeline, newest first |
| `POST /applications/<id>/events` | Adds a note (`{"kind": "note", "body": "…"}`) or, for a rejected application, rejection feedback (`"kind": "rejection_feedback"`): 201 |

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

## Memory

The Memory page (`/memories`, "What Waypoint knows about me") lists everything Waypoint knows about you, one section per kind: **facts** (positions, skills, education), **preferences** (target role, seniority, location and remote, comp floor, industry, deal-breakers) and **answers** for application forms (work authorization, notice period, salary expectation, why you're leaving). Every kind also allows the topic Other. Each memory shows its content, its tags, where it came from (added by you, a resume version, a LinkedIn export, an agent run, an email or your decisions) and when it was added and last updated. **Add memory** opens a form for the kind, topic, content and tags; **Edit** changes them, and **Delete** removes the memory after you confirm. A delete is permanent.

A memory is either **confirmed** or **proposed**. Proposed memories are ones Waypoint learned on its own, waiting for you: they carry a Proposed badge and a **Confirm** button. A proposed memory learned from your decisions also has **Show evidence**, which lists the decisions behind it so you can confirm on evidence. Everything you add yourself is confirmed. The search box, the Status filter and clicking a tag (which filters by that tag until you remove it) narrow the list; **Clear filters** brings everything back.

Content is required (trimmed, at most 5,000 characters) and the topic must belong to the kind. Tags are trimmed, lowercased and single-spaced, with blanks and duplicates dropped; a memory has at most 20 tags of at most 50 characters each.

| Route | Does |
|---|---|
| `GET /memories?kind=&status=&tag=&q=` | Lists memories, newest `updated_at` first. Every filter is optional: `kind` (`fact`, `preference`, `answer`), `status` (`proposed`, `confirmed`), `tag` (one tag, normalised like stored tags) and `q`, a full-text search over content, topic and tags that puts the best matches first |
| `POST /memories` | Adds one (`kind`, `topic`, `content`, optional `tags`): 201, always `confirmed` with source `manual` |
| `GET /memories/<id>` | Reads one |
| `PUT /memories/<id>` | Replaces the kind, topic, content and tags; status and source are kept |
| `GET /memories/<id>/evidence` | Lists the learning signals (decisions) linked to a memory as evidence, newest first; `[]` when it has none, 404 for an unknown id |
| `POST /memories/<id>/confirm` | Confirms a proposed memory; confirming one already confirmed changes nothing |
| `DELETE /memories/<id>` | Deletes one permanently: 204 |
| `POST /memories/learn` | Asks the memory writer to learn from your unprocessed decisions now (no body): 202 `{run_id, reused}`, reusing a writer run that is already waiting |

The HTTP routes never set the status or the source, so a client can't add a proposed memory or claim one came from somewhere else; only Waypoint's own workflows store those. Errors follow the applications routes: 422 with every failing field, 400 for a body that isn't valid JSON or names an unknown kind or topic, and 403 to an HTML form or a request a browser sends from another site. [Delete all data](docs/google-oauth.md#delete-all-data) (`DELETE /data`) empties the memories along with everything else. With the API running:

```sh
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"kind": "preference", "topic": "location", "content": "Remote only, US time zones", "tags": ["Remote"]}' \
  http://127.0.0.1:8000/memories                                       # 201, "tags": ["remote"]
curl -s 'http://127.0.0.1:8000/memories?kind=preference&q=remote'
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE http://127.0.0.1:8000/memories/1   # 204
```

**Import LinkedIn** on the Memory page reads the positions, skills and education in a LinkedIn data export into confirmed facts, and **Add to profile** on a resume's page has Claude read that version into proposed ones. Re-importing adds no duplicates and never changes a memory you edited. [docs/profile-import.md](docs/profile-import.md) covers both routes, the limits, the content format and the dedupe rules.

Waypoint also learns from your decisions. About a minute after you approve, edit, reject or dismiss something an agent drafted, the memory writer has Claude read those decisions and stores what they show as proposed memories ("Learned from your decisions"). It never changes a confirmed memory: a correction to one becomes a new proposal. Nothing it learns affects a draft until you confirm it. [docs/memory-writer.md](docs/memory-writer.md) covers the scheduling, the validation rules and the eval set.

## Model calls

Waypoint calls Claude through your Claude subscription, not the API. Every call spawns the [Claude Code](https://docs.claude.com/en/docs/claude-code) CLI (`claude -p`) in an empty temp directory with no tools and no saved session, and logs the model, tokens, duration and reported cost to the `claude_calls` table. Install the CLI and log in once with `claude` before using any model-backed feature.

The runner removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` and the `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`/`_FOUNDRY` switches from the CLI's environment, so nothing in your shell moves billing or traffic away from the subscription. It also withholds Waypoint's own settings (`DATABASE_URL`, `POSTGRES_*`, `WAYPOINT_*`). `WAYPOINT_CLAUDE_BIN` (default `claude` on `PATH`) and `WAYPOINT_CLAUDE_TIMEOUT_SECS` (default 300) are optional; see `.env.example`. The CLI runs in its own process group, and when a call ends, times out or is cancelled the runner kills that whole group, so nothing the CLI started outlives the call.

Agent steps can give the model Waypoint's own tools, and no others, through a per-call MCP server: the CLI starts the `waypoint-mcp` relay bin, which forwards to a private socket the API serves. `cargo build -p waypoint-api --bins` builds the relay beside the API binary (`./scripts/dev.sh` does this for you); set `WAYPOINT_MCP_BIN` to use one elsewhere. [docs/agent-runtime.md](docs/agent-runtime.md#mcp-server) covers the protocol and how tool calls are recorded.

To check the round trip, start the app with `./scripts/dev.sh`, open <http://127.0.0.1:5173> and click **Test Claude** on the dashboard: the card shows Queued, Running and then Succeeded with the reply `pong`. Or, in another terminal:

```sh
curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/claude/ping
docker compose exec db psql -U waypoint -c 'select status, model, input_tokens, output_tokens, cost_usd from claude_calls order by id desc limit 1'
```

The ping should answer `pong` with the model and token counts. `/dev/claude/ping` exists only in debug builds (`cargo run`); release builds don't serve it. It answers 403 without the `X-Waypoint-Dev: 1` header or when a browser sends it from another site, so a web page you visit can't spend your usage.

The API only answers requests whose `Host` header is `127.0.0.1`, `localhost` or `[::1]` (any port), and returns 403 to everything else, so a site that re-resolves its own domain to `127.0.0.1` (DNS rebinding) can't reach it.

## Jobs and live events

Background work, including every model call, runs through a job queue in Postgres (the `jobs` table). An in-process worker runs at most `WAYPOINT_JOB_CONCURRENCY` jobs at once (default 2) and retries failures with exponential backoff (10s, doubling, up to 10 minutes; 3 attempts). When the Claude subscription's usage limit is hit, the job goes back in the queue without using up an attempt and the whole queue pauses for `WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS` (default 900). Jobs left running when the API stopped are requeued on the next start.

`GET /events` streams job, queue, application and run events as server-sent events (`job.queued`, `job.started`, `job.retrying`, `job.deferred`, `job.succeeded`, `job.failed`, `queue.paused`, `queue.resumed`, `application.created`, `application.updated`, `application.deleted`, `run.updated`), starting with `app.connected`, plus `app.lagged` when a slow client fell behind. Every successful application create, edit, status change, resume link and delete publishes one application event: `application.created` and `application.updated` carry `{ application }` (the route's response body), `application.deleted` carries `{ id }`, and a refused or failed request publishes nothing. There is no replay, so a client that reconnects reads the current state again (`GET /jobs/<id>`, `GET /applications`).

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

## Agent runs and tools

Agent workflows run as runs: each step's input, output, status, tokens and cost is stored, along with every tool call it makes, and a failed or interrupted run picks up at its first unfinished step. Tools are typed Rust handlers with JSON schemas and a permission level (`read`, `write` or `external`). `GET /runs` and `GET /runs/<id>` show what a run did, and the **Runs** page shows the same live: each run's status, duration, tokens and cost, and on a run's page every step and tool call with its permission level, input and output. [docs/agent-runtime.md](docs/agent-runtime.md) covers the tables, the resume rules, how tokens and cost add up, the tool trait, the supported schema subset and the `run.updated` event.

Permission levels are enforced. An `external` call (one that would leave Waypoint) never runs until you approve it: its step parks and the run shows **Awaiting approval** until you approve (optionally with an edited input) or reject it. Every `write` call is logged with what it takes to undo it, and undo refuses rather than overwrite a record that changed since. A result a step isn't confident about goes to the review queue instead of being applied. [docs/agent-runtime.md](docs/agent-runtime.md) covers the approval gate, undo, review items and their routes.

### Try the sample workflow

The app ships one workflow, `tidy_application`, which needs no model call: it reads an application (`read`), adds a note to its timeline (`write`, undoable), then drafts a follow-up message (`external`), which waits for your approval. The draft is only shown, never sent.

In the app, open an application and click **Run sample workflow**. The draft waits on the **Review** page: approve it there (or edit or reject it), undo the note in its **Action log** tab, and follow the run link to **Runs** to inspect the run.

The same flow with curl, with the API running (`./scripts/dev.sh`):

```sh
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"company": "Acme", "title": "Staff Engineer"}' \
  http://127.0.0.1:8000/applications                                     # 201, note its "id" (1 here)
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"note": "Ask about the team", "message": "Thanks for your time", "confidence": 0.9}' \
  http://127.0.0.1:8000/applications/1/tidy                              # 201, a queued run; note its "id"
curl -s http://127.0.0.1:8000/runs/1                                     # "status": "awaiting_approval"
curl -s http://127.0.0.1:8000/pending-actions                            # the draft_follow_up action; note its "id"
curl -s -X POST -H 'Content-Type: application/json' -d '{}' \
  http://127.0.0.1:8000/pending-actions/1/approve                        # or send {"input": {...}} to edit it first
curl -s http://127.0.0.1:8000/runs/1                                     # "succeeded", "output": {"draft": "Thanks for your time (to Acme)"}
curl -s 'http://127.0.0.1:8000/actions?run_id=1'                         # the add_note action; note its "id"
curl -s -X POST http://127.0.0.1:8000/actions/1/undo                     # "undone_at" is set
curl -s http://127.0.0.1:8000/applications/1/events                      # the note is gone from the timeline
```

The **Runs** page shows the run live, including while it awaits approval. `POST /pending-actions/<id>/reject` instead ends the run `rejected`. A `confidence` below 0.8 sends the note to the review queue (`GET /review-items`) instead of the timeline, and the run still stops at the draft.

## Capture from URL

On the Applications page, **Capture from URL** reads a job posting's link and opens the add form prefilled for you to check. It uses the page's JSON-LD first, then the Greenhouse, Lever or Ashby posting API, and asks Claude to read the page text only when neither has the details. [docs/capture.md](docs/capture.md) covers the tiers, the limits, the redirect guard and what is sent to Claude.

## Google account

Gmail and Calendar access goes through one Google account that you connect with your own Google Cloud OAuth client. [docs/google-oauth.md](docs/google-oauth.md) walks through creating the client, the three `.env` settings (`WAYPOINT_GOOGLE_CLIENT_ID`, `WAYPOINT_GOOGLE_CLIENT_SECRET`, `WAYPOINT_TOKEN_KEY`) and connecting. Waypoint asks only for read-only Gmail and Calendar events, and stores the tokens encrypted.

Once it's set up, open <http://127.0.0.1:8000/auth/google/connect> to connect, check `curl -s http://127.0.0.1:8000/auth/google`, and disconnect with `curl -s -X DELETE http://127.0.0.1:8000/auth/google`. Without the settings, everything else works and Google reports "not configured".

## Recruiter email

Waypoint stores email in two tables, `emails` and `email_threads`. Each raw message is parsed, its body is reduced to clean text (quoted replies removed, forwards kept, the signature split off), it's grouped into a thread, and cheap sender rules decide whether it's worth a model call (`candidate`) or not (`skip`). The parsing, threading and sender rules don't call Claude or the network. Once Google is connected, Gmail sync reads the inbox read-only into these tables: the first run backfills the last 30 days, and later runs read only what's new. It runs when the API starts after more than 10 minutes away and when you press **Sync now** on the dashboard's Gmail card. [docs/gmail-sync.md](docs/gmail-sync.md) covers the modes, pacing, the size cap, errors and the routes. Without a Google account, a synthetic corpus in `api/fixtures/emails/` stands in for the inbox, and in debug builds `curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/emails/import-fixtures` imports it. Each candidate is then classified by Claude and matched to an application by thread, sender, company and role, without another model call: confident status and stage changes are applied as undoable actions, unsure matches and proposed new applications wait on the Review page for **Apply** or **Dismiss**, and the application timeline names the email behind each change. [docs/email.md](docs/email.md) covers the schema, the normalizer rules, the thread order, every heuristic rule, the dev route, classification and matching.

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

### Evals

Each workflow has a labelled eval set in `api/evals/`. The `eval` binary runs one through the real worker and `claude -p` runner in a throwaway database, scores each run against its labels and prints the pass rate with each case's tokens, cost and duration. It never approves a pending action. Evals call `claude` with your login, so they run locally and CI doesn't run them. [docs/evals.md](docs/evals.md) covers the dataset format, the matching rules and how to add a suite.

```sh
docker compose up -d --wait db && cargo run --bin eval -- tidy_application   # pass rate: 6/6 (100.0%)
```
