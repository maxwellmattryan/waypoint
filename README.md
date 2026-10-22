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

The runner removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` and the `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`/`_FOUNDRY` switches from the CLI's environment, so nothing in your shell moves billing or traffic away from the subscription. It also withholds Waypoint's own settings (`DATABASE_URL`, `POSTGRES_*`, `WAYPOINT_*`). `WAYPOINT_CLAUDE_BIN` (default `claude` on `PATH`) and `WAYPOINT_CLAUDE_TIMEOUT_SECS` (default 300) are optional; see `.env.example`.

To check the round trip, start the app with `./scripts/dev.sh` and, in another terminal:

```sh
curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/claude/ping
docker compose exec db psql -U waypoint -c 'select status, model, input_tokens, output_tokens, cost_usd from claude_calls order by id desc limit 1'
```

The ping should answer `pong` with the model and token counts. `/dev/claude/ping` exists only in debug builds (`cargo run`); release builds don't serve it. It answers 403 without the `X-Waypoint-Dev: 1` header or when a browser sends it from another site, so a web page you visit can't spend your usage.

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
