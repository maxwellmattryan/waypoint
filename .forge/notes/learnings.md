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
