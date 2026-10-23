# Goals and plans

A **goal** is what you're looking for, in your own words: "senior frontend roles at climate companies in NYC, apply to 5 this week". The `plan_goal` workflow turns it into a **plan** you can read and edit: target companies, the filters that pick postings, how many to score and shortlist, and notes for application prep. Running the plan sources the companies' boards, scores the matching postings for fit, shortlists the best ones and prepares an application packet for each, which then waits for your approval. When a step fails, or you ask, Waypoint replans. The code lives in `api/src/goals/` and `api/src/workflows/plan_goal.rs`; the pages are `/goals` and `/goals/<id>`.

Nothing is applied for or sent anywhere: sourcing reads public boards, scoring and prep call the model, and each packet waits for you on its own page ([application prep](application-prep.md)).

## Statuses

| Status | Meaning |
|---|---|
| `planning` | A `plan_goal` run is queued, running or paused. |
| `needs_answers` | The planner asked clarifying questions and waits for your answers. |
| `planned` | A current plan exists and isn't running. |
| `running` | The plan is executing. |
| `completed` | Every step of the current plan `succeeded` (or, for a legacy prep placeholder, was `skipped`). |
| `failed` | The planner run failed (`Planning failed: <run error>`), or a step failed after the automatic replans ran out (`<step> failed: <error>`). `error` says why. |

## Routes

Every route takes `LocalHost`. The non-GET routes also refuse a `Sec-Fetch-Site: cross-site` request and any HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`) with 403, then require `application/json` (415 otherwise). Unknown keys are 400. Each write locks the goal row while it checks the status, and publishes `goal.updated` after its commit. A 409 answers `{"message": "…"}` and changes nothing.

| Route | Does |
|---|---|
| `GET /goals` | Up to 200 goals, newest first. |
| `GET /goals/{id}` | The goal with its questions (by ordinal), its current plan (steps, shortlist, `packets`, `verified` per company), its versions (newest first) and its latest `plan_goal` run. 404 for an unknown id. |
| `POST /goals` | `{"text"}`: 1–1000 characters after trimming (`Required`, `At most 1000 characters`). Stores the goal `planning`, queues `plan_goal` with `{"goal_id", "trigger": "goal"}` and answers 201 with the detail. |
| `POST /goals/{id}/answers` | `{"answers": [{"question_id", "answer"?}]}`. Only in `needs_answers` (409 otherwise). Each answer is trimmed, at most 1000 characters; blank or missing means skipped. A question id that isn't one of the goal's open questions, or appears twice, is 422. Stores the answers, marks every open question answered, sets `planning`, queues `plan_goal` with `trigger: goal` and answers 202. |
| `PUT /goals/{id}/plan` | The whole plan (see [Editing](#editing)). 200 with the detail. |
| `POST /goals/{id}/run` | `{}`. Only when the goal is `planned` or `failed` and its current plan is `ready` (409 otherwise, e.g. "This plan needs review before it can run"). Resets the current plan's `failed` steps to `pending` (dropping their recorded jobs), sets `auto_replans = 0`, sets `running`, requests a tick and answers 202. |
| `POST /goals/{id}/replan` | `{}` or `{"instruction"}` (trimmed, at most 1000 characters, blank means none). Only in `planned`, `completed` or `failed`. With a current plan it queues `plan_goal` with `{"goal_id", "trigger": "user_replan", "plan_id", "instruction"}`; without one (planning failed) `{"goal_id", "trigger": "goal"}`. Sets `planning` and answers 202. |

## The planner

`plan_goal` has one step, `plan`, and makes one tool-less `claude -p --json-schema` call (model `sonnet`, purpose `goal.plan`). The prompt holds:

- the goal, fenced in `<goal>…</goal>`;
- the answered questions as `Q:`/`A:` pairs, fenced in `<answers>`;
- the user model: every confirmed `preference` memory (up to 30) and up to 20 confirmed `fact` or `answer` memories [retrieved](agent-runtime.md#retrieval) on the goal and answers, rendered with `retrieval::context` and recorded in `run_memories`;
- the **known boards**: up to 200 distinct `(source, board, company)` from `job_postings`, by company, as `- <source>/<board> · <company> (<n> live postings)`, fenced in `<boards>` because company names come from public boards;
- for a replan, the current plan as JSON in `<plan>`, then the failed step's kind, output and error in `<failure>`, or your instruction in `<instruction>`.

The prompt says the fenced text is data, not instructions; each fenced text has any closing tag of these blocks broken up.

The answer is `{"kind": "questions" | "plan", "questions", "plan"}`. **Questions** (1–5, each at most 300 characters; blank, long and extra ones are dropped) are allowed only for the `goal` trigger when the goal has none yet, so there is at most one round. They are stored and the goal becomes `needs_answers`. Questions when not allowed fail the run.

### Validation

| Field | Rule | Planner answer | `PUT` |
|---|---|---|---|
| `summary` | 1–300 characters trimmed | invalid | kept from the base plan |
| `confidence` | finite, 0–1 | invalid | — (null) |
| `companies` | 1–20; `name` 1–100; `source` greenhouse, lever or ashby; `board` a slug (trimmed, `[A-Za-z0-9_-]{1,100}`); unique `(source, board)`; `reason` cut to 300 | bad ones dropped and listed; none left is invalid | 422 per field: `Required`, `At most 100 characters`, `Must be greenhouse, lever or ashby`, `Must be 1–100 letters, digits, hyphens or underscores`, `Duplicate board`, `At most 20 companies` |
| `title_keywords` | 1–10, each 1–50 characters, unique ignoring case | bad ones dropped; none left is invalid | 422 (`title_keywords[i]`, `title_keywords`) |
| `locations` | 0–10, each 1–100 characters, unique ignoring case | bad ones dropped | 422 |
| `max_to_score` | 1–50 | invalid | 422 `Must be 1 to 50` |
| `shortlist_size` | 1–20 | invalid | 422 `Must be 1 to 20` |
| `min_score` | 0–100 | invalid | 422 `Must be 0 to 100` |
| `prep_notes` | at most 1000 characters | cut | 422 `At most 1000 characters` |

An **invalid** answer fails the run with every broken rule named and stores nothing; the worker hook then fails the goal with `Planning failed: <run error>`. Dropped items are never dropped silently: they are stored on the plan (`dropped`), counted in the run output and listed in the review item.

### Ready or in review

With confidence at least 0.8 and nothing dropped, the plan is stored `ready`. Otherwise it is stored `in_review` with a pending `plan_goal.plan` [review item](agent-runtime.md#review-items) whose payload is `{goal_id, plan_id, version, goal, summary, companies, dropped}`; the plan and its item are written in one transaction. Either way the goal becomes `planned`, except after a `step_failed` replan whose plan is `ready`: the goal goes straight back to `running` and a tick is requested.

On the Review page the item shows the goal, the summary, the companies (with an Unverified badge where it applies) and the dropped items. **Apply** makes the plan `ready` (409 when it is no longer `in_review` or no longer the goal's current plan); it doesn't start it, you click Run. **Dismiss** marks the plan `dismissed` and records the usual dismissal learning signal.

A company is **verified** when `job_postings` has any row for its `(source, board)`. It is computed when the plan is read, so a board becomes verified once it has been sourced. Unverified boards are kept: your Run click approves them, and a wrong slug fails the source step, which replans.

The run output is `{"goal_id", "kind": "questions", "question_ids"}` or `{"goal_id", "kind": "plan", "plan_id", "version", "status", "review_item_id", "companies", "unverified", "dropped": {"companies", "keywords", "locations"}}`. Reruns are idempotent: a run that already stored a plan or questions returns them without a second call.

## Versions and steps

Every plan write is a new **version** (1, 2, …); the newest is the goal's current plan. A version records its `author` (`planner` or `user`), `trigger` (`goal`, `step_failed`, `user_replan` or `user_edit`), `status` (`ready`, `in_review` or `dismissed`), and its run, review item, model call and cost (null and 0 for user edits). A `step_failed` version also shows `failed_step`.

Every version has the same five steps, in order: `target_companies`, `source`, `score`, `shortlist`, `prep`. Steps can't be reordered: each consumes the previous one's output; you change what a step does by editing the plan. Each step has a `status` (`pending`, `running`, `succeeded`, `failed`, or the legacy `skipped` that the old prep placeholder wrote), a `config` (its inputs), an `output`, a `last_error`, its times and `kept_from`.

### Keep rule

Each step's config is cumulative:

- `target_companies`: the companies' names, sources and boards, sorted by `(source, board)`;
- `source`: the `source/board` list, sorted;
- `score`: the boards plus the lower-cased, sorted keywords and locations, `remote_only` and `max_to_score`;
- `shortlist`: the score config plus `shortlist_size` and `min_score`;
- `prep`: the shortlist config plus `prep_notes`.

When a version is written, step *i* is copied from the previous version (status, output, times and shortlist rows, with `kept_from` set) when the previous step *i* `succeeded`, its config equals the new one, and every earlier step was kept. Every other step starts `pending`. So changing only `min_score` keeps `target_companies`, `source` and `score`; changing the companies keeps nothing; a company's reason isn't part of any config. A failed step and everything after it are never kept. Only `succeeded` steps are kept, so a legacy `skipped` prep step runs for real in the next version.

## Editing

`PUT /goals/{id}/plan` takes `{base_plan_id, companies[{name, source, board, reason?}], title_keywords, locations, remote_only, max_to_score, shortlist_size, min_score, prep_notes}` and writes a new version with `author: user`, `trigger: user_edit` and status `ready`, keeping the summary of the base plan and applying the keep rule. No model call is made: your edit is authoritative.

- Allowed in `planned`, `running`, `completed` and `failed`; 409 in `planning` and `needs_answers`.
- A `base_plan_id` that isn't the current plan is 409 ("This plan changed since you opened it") and writes nothing.
- An edit equal to the current plan returns 200 with nothing written.
- A `running` goal stays `running` and a tick is requested, so it goes on from the first changed step. A `completed` or `failed` goal becomes `planned`.
- Editing an `in_review` or `dismissed` plan is allowed; the new version is `ready`. The old review item stays, and applying it later is 409.

## Execution

A **tick** is one `plan_step` job with payload `{"goal_id"}`. Requesting one (under the goal's row lock) reuses a `queued` `plan_step` job for the goal, pulled forward to now, or enqueues one. Each tick locks the goal row for its whole run, so ticks for one goal never interleave; a tick on a goal that is gone or not `running` succeeds as a no-op. A database error retries the job.

A tick walks the current plan from its first step that isn't `succeeded`/`skipped`:

- **`target_companies`** succeeds at once with `{"companies", "unverified"}`; an empty list fails with `No target companies`.
- **`source`** requests a fetch of each board through `sourcing::request` (which reuses a queued or running `source_board` job) and records the jobs in `plan_step_tasks`, then waits until each has `succeeded` or `failed`. It succeeds with `{"boards", "inserted", "updated"}`; a failed board fails the step with `Couldn't source <source>/<board>: <job error>` per board, joined with `; `.
- **`score`** picks the live postings on the plan's boards whose title contains a keyword, whose location (raw, city, region or country) contains a location when any are given, and that are remote when `remote_only` is set (all case-insensitive, `%` and `_` literal), newest first, at most `max_to_score`. A posting with an applied fit score is reused; one whose latest score waits in review is left there; every other gets `score_fit::request` (which reuses a waiting or running run), recorded in `plan_step_tasks`. It waits until each queued run is `succeeded`, `failed` or `rejected`, then succeeds with `{"posting_ids", "queued", "reused", "in_review", "failed"}`. It fails with `No live posting on the target boards matches the filters`, or `Every fit score failed: <first error>` when every queued run failed and nothing was reused or in review.
- **`shortlist`** takes the score step's postings whose current applied score is at least `min_score`, best score first (then newest posting), at most `shortlist_size`, and stores them with their rank. It succeeds with `{"shortlisted", "in_review"}` or fails with `No posting scored <min_score> or more (<m> waiting in review)`. A score you apply later doesn't rebuild the shortlist: edit or Run again.
- **`prep`** prepares a packet for each shortlisted posting. On its first tick it reads the shortlist by rank. A posting whose newest packet is `ready`, `in_review`, `approved` or `applied` is **reused** and gets no run. Every other one (no packet, or only `dismissed` ones) gets `prep_packet::queue` with the plan's `prep_notes`, inside the tick's transaction, which reuses a waiting or running run; the run is recorded in `plan_step_tasks`. A posting that is gone or has no applied fit score is `failed` at once (`The posting is gone`, `The posting has no applied fit score`). That progress is stored as the step's output in the same transaction, so a later tick never queues twice. The step waits until each queued run is `succeeded`, `failed` or `rejected` (a `paused` run keeps it waiting; a deleted one counts as settled), then resolves each one: a succeeded run with a saved packet is `prepared`; anything else is `failed` with the run's error, `No packet was saved` or `The run is gone`. The output is `{"packets": [{"posting_id", "packet_id", "run_id", "outcome", "error"}], "prepared", "reused", "failed"}`. The step succeeds when at least one packet was prepared or reused; a partial failure shows only in `failed` and in its entry, and costs no replan. When every posting failed it fails with `Every packet failed: <first error>`. An empty shortlist fails with `No shortlisted posting to prepare`. A ready packet gets its approval request from the prep run itself, so it lands on `/packets/<id>` waiting for your approval.

When every step is done, the goal is `completed`.

### Wake-ups

After the worker records a `succeeded` or `failed` `source_board` or `agent_run` job, it calls `goals::exec::job_settled`. That requests a tick for every `running` goal whose current plan has a task naming the job, or naming a run (`score_fit` or `prep_packet`) whose `job_id` is the job. It first locks every running goal, so it waits for a tick in flight and sees the tasks it recorded. When the job's run is a failed `plan_goal` run and is the goal's latest planner run while the goal is `planning`, the goal becomes `failed` with `Planning failed: <run error>`. When the worker starts it requests a tick for every `running` goal, so a crash never strands a plan.

### Replanning after a failure

When a step fails, a goal with `auto_replans` below 2 counts one, becomes `planning` and queues `plan_goal` with `{"goal_id", "trigger": "step_failed", "plan_id", "step"}`. The new version applies the keep rule: if it is `ready` the goal returns to `running` with no click; if it is `in_review` the goal is `planned` until you apply it and click Run. At 2 the goal fails with the step's error. Run resets the count, so each Run click allows at most two automatic replans.

The kill switch pauses `plan_goal`, `score_fit` and `prep_packet` runs as usual; `plan_step` ticks still run and find nothing ready yet.

## Events

`{"type": "goal.updated", "goal": Goal}` is published after every commit that changes a goal, its questions, its versions, its steps or its shortlist, never from inside a transaction. Refetch `GET /goals/{id}` for the detail.

## Pages

- **Goals** (`/goals`, in the nav before Postings): a composer (**Plan it**, disabled while empty or sending, a 422 message under the field) that opens the new goal's page, and the list with each goal's text, status and update time. It shows a skeleton while loading, "No goals yet" when empty and "Couldn't load your goals" with **Try again**.
- **A goal** (`/goals/<id>`): the text and status, then the block for its state: "Planning…" (or "Paused: agents are paused") with a link to the run; the clarifying questions with **Submit answers** and **Skip and plan anyway**; the error of a failed goal with **Run again** when a ready plan exists, else **Replan**. With a current plan it shows the summary, the version line ("Version 3 · edited by you · 2 min ago"), confidence and cost, the target companies (Unverified badges), filters, shortlist settings, prep notes, the steps with their status, a one-line summary and "kept from version N", and the shortlist linking to each posting with its fit badge. The prep step's summary reads "Preparing packets…" while it runs and "1 prepared · 1 already prepared · 1 failed" when done (zero parts left out). A **Packets** card follows the shortlist once the prep step has entries (`plan.packets`: each entry of its output with the posting's company and title and the packet's live `status`; an entry whose posting was deleted is left out). Each row links the posting, then shows the packet's status badge with **Review and approve** (when `ready`) or **View packet**, linking to `/packets/<id>`; "Preparing…" with a run link while queued; "Couldn't prepare" with the error and a run link when it failed; or "Packet removed" when the packet was deleted. Approval itself stays on the packet's page. **Run**, **Edit plan** (a drawer: add, edit and remove companies, keywords, locations, numbers, remote only, prep notes; checked like the route; a 409 shows "This plan changed since you opened it" and reloads) and **Replan** (a dialog with an optional instruction). An `in_review` plan links to the review queue; a dismissed one says "You dismissed this plan. Edit it or replan." The page reloads on `goal.updated` for its goal, on `run.updated` for a listed packet's run (review decisions publish it) or for an `approve_packet` or `apply_packet` run whose input names a listed packet, and after the event stream reconnects or lags.

The [`plan_goal` eval set](evals.md#the-plan_goal-eval-set) measures the planner.
