# Forge experiments

<!--
One line per retro change: its date, `id <slug>` (what a learning's `experiment:` names), what
changed, the metric it should move (a dotted stats.json path such as `quality.avgRounds`), its
baseline, the evidence, and its status. A new entry always starts open; only
`forge experiment evaluate` settles one (kept | reverted | inconclusive), and `forge experiment
check` and verify refuse a status written by hand or any other edit to an existing entry.
-->

- 2026-10-22 · id csrf-guard-learning · learning: every non-GET API route takes a cross-site guard plus a 403 test (.forge/notes/learnings.md) · metric reviewers.security.blockerMajorShare (down) · baseline 0.091 (2/22) · evidence 6-claude-runner, 9-google-oauth-for-gmail-and · status open
- 2026-10-22 · id cross-site-guard-verifier · verifier (warn): non-GET Rocket routes take NotCrossSite, DevOnly or another cross-site guard (.forge/verifiers/cross-site-guard.md) · metric verifiers.cross-site-guard.hitRate (up) · baseline 0 · evidence 6-claude-runner, 9-google-oauth-for-gmail-and · status open
- 2026-10-23 · id migration-version-learning · learning: new migrations take the next version unclaimed by develop or open forge PRs, and count assertions match the files present (.forge/notes/learnings.md) · metric quality.humanReworks (down) · baseline 2 (2 reworks / 5 merged) · evidence 49-retrieval-and-run-inspector-provenance, 50-learning-signals, 56-memory-writer-job · status open
- 2026-10-23 · id readme-coverage-learning · learning: user-facing route, page or nav changes update README.md in the same PR (.forge/notes/learnings.md) · metric reviewers.docs.blockerMajorShare (down) · baseline 0.167 (3/18) · evidence 47-memory-store-and-what-waypoint, 56-memory-writer-job · status open
