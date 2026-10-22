# ADR 0001: Stack

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

Waypoint is a personal job-search assistant that runs on one machine for one user. It tracks applications, manages resume versions, reads recruiter email and prepares applications, and it is also a place to practice agentic pipelines. It starts from an empty repository, so every later job needs a settled stack and a skeleton that runs with one command.

The constraints that shape the choice:

- It is a personal tool, so there is no hosted deployment, no multi-tenant concerns and no login.
- Data is relational (applications, stages, contacts, resume versions, email threads) and background work needs a durable queue.
- Model calls must use the user's Claude subscription rather than an API key (see [AGENTS.md](../../AGENTS.md)).
- Waypoint is standalone. It shares no code, packages, infrastructure or config with any other project, even one built on the same stack.

## Decision

- **Frontend:** SvelteKit with Svelte 5 runes, TypeScript and Tailwind CSS, built with `@sveltejs/adapter-static` as a static single-page app (SSR off, `index.html` fallback) that talks to the API over HTTP.
- **Web package manager:** npm, with `web/package-lock.json` committed.
- **Backend:** a Rust API on Rocket, in a Cargo workspace (`api/`, package `waypoint-api`) pinned to stable Rust by `rust-toolchain.toml`.
- **Database:** PostgreSQL, accessed with sqlx using runtime-checked queries, and schema changes as sqlx migrations in `api/migrations/` that the API applies on startup. Background work will use a Postgres-backed job queue.
- **Local infrastructure:** Postgres runs in Docker Compose (service `db`, `postgres:17-alpine`), published only on `127.0.0.1`, with data in a named volume.
- **Local only:** the API always binds `127.0.0.1`, the web dev server binds `127.0.0.1`, there is no hosted deployment and no app login.
- **Single dev entry point:** `scripts/dev.sh` loads `.env`, starts Postgres, then runs the API and the web dev server together.
- **Model calls:** all model calls go through one Rust module that spawns `claude -p` with the isolation flags listed in AGENTS.md. No Anthropic API key and no Anthropic SDK.
- **Standalone:** Waypoint shares no code, packages, infrastructure or config with any other project.

### Alternatives considered

- **SvelteKit with SSR instead of a static SPA:** a Node server adds a second backend for no gain on a single-user local app; the Rust API already serves data.
- **Axum instead of Rocket:** both fit; Rocket's typed request guards, managed state and built-in local test client keep handlers and tests short.
- **SQLite instead of PostgreSQL:** simpler to run, but a Postgres-backed job queue (`SKIP LOCKED`, `LISTEN/NOTIFY`) and richer types matter more than avoiding Docker.
- **Compile-time-checked `sqlx::query!`:** would require a live database or `.sqlx` offline data for every build; runtime queries keep builds database-free.
- **pnpm or Bun instead of npm:** npm ships with Node, so there is nothing extra to install.
- **`just` or a root `package.json` for task running:** a bash script needs no extra tool.

## Consequences

- Everything binds localhost: the API forces its address to `127.0.0.1` in code regardless of Rocket config, and Compose publishes Postgres on `127.0.0.1:5433` (5433 avoids a locally installed Postgres on 5432).
- `scripts/dev.sh` needs Docker running and a `.env` copied from `.env.example`. Postgres keeps running after the script exits; `docker compose down` stops it.
- Builds and lints never need a database; tests that touch Postgres need the Compose database and `DATABASE_URL`.
- The frontend's TypeScript API types are generated from the API's OpenAPI spec rather than written by hand: utoipa builds `api/openapi.json` from the Rocket handlers, and openapi-typescript and openapi-fetch turn it into types and a typed client. See "API contract" in the README.
- Model calls depend on the `claude` CLI being installed and logged in with the user's subscription. `--bare` is never used because it bills the API, and evals that call `claude` run locally rather than in CI.
- There is no deployment pipeline to maintain; anything that would expose Waypoint beyond localhost needs a new ADR.
