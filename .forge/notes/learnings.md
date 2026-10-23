# Forge learnings

<!--
Lessons every Forge agent reads before starting. Written by the retro agent (via a reviewed
PR) and by humans. Keep each entry short, actionable and backed by evidence (job ids). Prune
entries that stop paying for themselves; this file is read on every agent run, so its length
costs tokens on every job.

Format:
- **<rule>** — why, evidence: <job ids>. (added YYYY-MM-DD · experiment: <id> · review-by: YYYY-MM-DD)
`experiment:` names an entry in experiments.md; `review-by:` is when the retro checks that its metric moved.
-->

- **Every API route with a non-GET method takes a cross-site guard besides `LocalHost`, and gets a test that a cross-site and a form-encoded request are refused with 403 and no side effect.** — Use `NotCrossSite` (api/src/google/mod.rs; move it beside `LocalHost` in api/src/host.rs when a second module needs it), or a custom-header guard like `DevOnly` (api/src/dev.rs) for `/dev` and upload routes, since `NotCrossSite` refuses multipart. `LocalHost` checks only `Host`, so it doesn't stop CSRF. Any page the user visits can send a no-preflight form POST to 127.0.0.1, and Rocket 0.5 turns `_method=delete` form POSTs into DELETE, so a spec must never assume a method needs a preflight; evidence: 6-claude-runner, 9-google-oauth-for-gmail-and. (added 2026-10-22 · experiment: csrf-guard-learning · review-by: 2026-11-19)
- **A new migration takes the next version no other branch has claimed, and migration-count assertions match the files present.** — Parallel jobs in one milestone each took "latest on develop + 1" and collided on `20261022000011`, which forced human reworks to rename files and bump counts. Before naming `api/migrations/<version>_<name>.sql`, check `git fetch origin && git ls-tree -r --name-only origin/develop api/migrations` and the migrations in open `forge/*` PRs (`gh pr list --search "head:forge/"` then `gh pr diff`), and take the next unused version. When you add or rename a migration, set the count assertions in api/tests/migrations.rs and api/tests/timeline.rs to the number of files in api/migrations, or derive the count from the directory so the next job doesn't conflict; evidence: 49-retrieval-and-run-inspector-provenance, 50-learning-signals, 56-memory-writer-job. (added 2026-10-23 · experiment: migration-version-learning · review-by: 2026-11-20)
- **A change that adds or alters a user-facing route, page or nav link updates README.md in the same PR.** — Add the route to that section's `| Route | Does |` table, update the nav sentence near the top when a page joins the nav, give a new page its own `##` section (what it shows, its states, where the data comes from), and refresh any list the change extends, such as memory sources. Missing README coverage made up all 3 docs-lens majors and cost an extra round each time; evidence: 47-memory-store-and-what-waypoint, 56-memory-writer-job. (added 2026-10-23 · experiment: readme-coverage-learning · review-by: 2026-11-20)
