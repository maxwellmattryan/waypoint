# Waypoint

A personal job-search assistant: tracks applications through any interview process, manages resume versions, reads recruiter emails to keep statuses current, sources matching roles, and prepares applications for one-click approval. It is a personal tool first and a place to practice agentic pipelines.

Waypoint is a standalone project. Don't share code, packages, infrastructure or config with any other project, even one built on the same stack.

The plan lives in [ROADMAP.md](ROADMAP.md) and in the GitHub milestones (`MVP n · …`, then `1.0.0 · …`).

## Stack

- **Frontend:** SvelteKit with Svelte 5 (runes), TypeScript and Tailwind, built as a static SPA that talks to the API.
- **Backend:** Rust API on Rocket, with sqlx and PostgreSQL. Background work goes through a Postgres-backed job queue.
- **API contract:** the frontend's TypeScript types are generated from the API's OpenAPI spec; don't hand-write them.
- **Local only:** there's no hosted deployment. The API binds to `127.0.0.1`, Postgres runs in Docker Compose, and there's no app login.

## Model calls go through `claude -p`

Waypoint uses the user's Claude subscription instead of the Anthropic API, so there's no API key and no Anthropic SDK. All model calls go through one Rust module that spawns the `claude` CLI:

- `claude -p --input-format stream-json --output-format stream-json --verbose`, reading and writing JSON lines.
- Never pass `--bare`. It ignores the subscription's OAuth login and only accepts `ANTHROPIC_API_KEY`, which would bill the API.
- Isolate every call: `--tools ""` (no built-in tools), `--strict-mcp-config`, `--no-session-persistence`, and a clean working directory so no repo's CLAUDE.md loads.
- Use `--json-schema` for structured output, and validate the result in Rust anyway.
- Agent tools are exposed only through Waypoint's own MCP server passed with `--mcp-config`. Approval gates live in those tool handlers, not in Claude Code's permission prompts.
- Waypoint's memory and run state live in Postgres. Don't rely on Claude Code sessions for state.
- Log model, tokens, duration and reported cost for every call. Run work through the job queue with a concurrency limit, and back off when a subscription usage limit is hit.
- Evals call `claude` too, so they run locally rather than in CI.

## UI/UX conventions

- **4px grid.** Padding, margin, gap, width, height and offsets are multiples of 4px. In Tailwind, use whole steps of the default scale (`p-1` = 4px, `p-2` = 8px, …) and avoid half steps (`0.5`, `1.5`, `2.5`, `3.5`). 1px borders and hairlines are the only exception.
- **Cursor.** Everything clickable gets `cursor-pointer`: buttons, links, clickable rows and cards, tabs, toggles, menu items, drag handles (`cursor-grab`). Disabled controls get `cursor-not-allowed`. Nothing non-interactive gets a pointer. Bake this into the primitives so it isn't repeated at call sites.
- **States.** Every interactive element has hover, `focus-visible`, active and disabled styles.
- **Reusable components first.** Before building a component, check for an existing one. If part of the new component is likely to be reused, extract that part into its own component, build it first, then compose the larger component from it.
- **Motion.** Anything that appears, disappears, moves or changes state gets a transition: fade for appear and disappear, slide for drawers, toasts and panels, layout animation for reorders and items moving between columns, color transitions for hover and state changes. Use the shared motion tokens rather than ad hoc durations, and honor `prefers-reduced-motion`.

## Agent guardrails

- Every agent tool declares a permission level: `read`, `write` (reversible, logged with undo) or `external` (leaves Waypoint). `external` tools always pass through an approval gate.
- Nothing is submitted or sent on the user's behalf without their click. Agents draft; the user sends.
- Request the narrowest OAuth scopes that work (Gmail read-only to start; Calendar events only; Gmail compose only for drafts).
- Low-confidence results go to the review queue instead of being applied.
- Every workflow has an eval set and records cost per run.
