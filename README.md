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
