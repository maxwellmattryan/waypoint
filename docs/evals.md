# Evals

Every workflow has a labelled eval set: a dataset of cases, each with the run's input and what the run must look like once it settles. The `eval` binary runs a dataset through the real job worker and `claude -p` runner, scores every run against its labels, and reports the pass rate with the tokens, cost and duration of each case.

Evals run locally only. They call `claude` with your subscription login, so CI never runs them.

## Running an eval

With the Compose Postgres running and `DATABASE_URL` in `.env`:

```sh
docker compose up -d --wait db
cargo run --bin eval -- tidy_application
```

```
cargo run --bin eval -- <workflow> [--case <name>]... [--json] [--keep-db] [--dataset <path>] [--case-timeout-secs <n>]
```

| Argument | Meaning |
|---|---|
| `<workflow>` | A registered eval suite: `tidy_application`, `memory_writer`, `classify_email` or `match_email`. |
| `--case <name>` | Runs only the named case. Repeat it for several. A name the dataset doesn't have is a usage error. |
| `--json` | Prints the report as JSON on stdout instead of the text table. |
| `--keep-db` | Keeps the scratch database and prints its name on stderr, so you can inspect the runs. |
| `--dataset <path>` | Reads another dataset file instead of `api/evals/<workflow>.json`. |
| `--case-timeout-secs <n>` | How long one case's run may take to settle (default 3600, long enough for a usage-limit pause). |

With no arguments, `--help` or an unknown workflow, it prints the usage and the suite names on stderr.

Exit codes:

- `0`: every selected case passed.
- `1`: a case failed, errored or was skipped.
- `2`: a usage or setup error: bad arguments, an invalid dataset, `DATABASE_URL` unset or unreachable, the scratch database not created or not migrated, or invalid runner or queue settings in the environment.

Workflows whose model calls use tools need the `waypoint-mcp` relay beside the binary: `cargo build -p waypoint-api --bins` builds it, or set `WAYPOINT_MCP_BIN`.

## Isolation

- The command reads `.env` like the API does and connects to the server in `DATABASE_URL`. It creates a fresh database named `waypoint_eval_<unix_ms>_<pid>`, runs the migrations on it, runs every case there, then drops it with `DROP DATABASE … WITH (FORCE)`. It drops it after failures and errors too, unless you pass `--keep-db`. Your own database is never written to. The database user needs `CREATEDB`, which the Compose superuser has.
- The scratch database starts with the kill switch off.
- Cases run one at a time, in dataset order, so each case's model calls are exactly the `claude_calls` rows logged while it ran.
- The real worker runs the cases with the same settings as the app (`WAYPOINT_CLAUDE_*`, `WAYPOINT_JOB_*`), so retries, backoff, the usage-limit pause and the concurrency limit behave as they do in the app. A run that a usage limit put back in the queue isn't settled, so the harness keeps waiting, and the report counts the limit hit.
- If the eval is killed, its scratch database is left behind. List and drop leftovers by hand:

  ```sh
  docker compose exec db psql -U waypoint -c '\l waypoint_eval_*'
  docker compose exec db psql -U waypoint -c 'DROP DATABASE "waypoint_eval_<unix_ms>_<pid>" WITH (FORCE)'
  ```

## The harness never approves

A run that parks at the approval gate (`awaiting_approval`) is a final state for scoring, and the pending actions it proposed are part of what gets scored. The harness never approves or rejects a pending action, so no `external` tool handler ever runs in an eval. Later workflows' `external` tools send real messages, and nothing is sent without your click.

## Dataset format

A dataset lives at `api/evals/<workflow>.json`. It is read at run time, so editing labels needs no rebuild.

```json
{
  "workflow": "tidy_application",
  "cases": [
    {
      "name": "confident note is applied",
      "setup": {"application": {"company": "Acme", "title": "Engineer"}},
      "input": {"application_id": "${application_id}", "note": "Called the recruiter", "message": "Thanks", "confidence": 0.9},
      "expect": {
        "status": "awaiting_approval",
        "actions": [{"tool": "add_note"}],
        "review_items": [],
        "pending_actions": [{"tool": "draft_follow_up", "status": "pending", "input": {"application_id": "${application_id}", "message": "Thanks"}}]
      }
    }
  ]
}
```

- `workflow` must name the same workflow as the command.
- `cases` is a non-empty array. Each case has a `name` (non-empty and unique in the file), an optional `setup` (any JSON, handed to the suite's setup; `null` when absent), an `input` (the run's input) and `expect`.
- Unknown fields are rejected everywhere.
- `expect.status` is required and is one of `succeeded`, `failed`, `awaiting_approval` or `rejected`. The rest are optional, and an omitted one isn't checked:
  - `output`: subset-matched against the run's `output`.
  - `error_contains`: a substring of the run's `error`.
  - `pending_actions`: one `{tool, status, input}` per pending action.
  - `review_items`: one `{kind, confidence, payload}` per review item.
  - `actions`: one `{tool, undone}` per logged `write` action (`undone` is a bool).
  - `memories`: one `{step, memory_id}` per memory the run's steps [retrieved](agent-runtime.md#retrieval) (`run_memories` rows), so the count and order check recall and rank, e.g. `memories[0].memory_id: expected 3, got 4`.

  For the four arrays, the count must equal the observed count, and the i-th expected entry is subset-matched against the i-th observed entry, ordered by `id`.

An invalid file exits 2 with a message that names the file and, where it can, the case.

### Placeholders

A suite's setup returns variables, such as the id of the application it inserted. A JSON string that is exactly `${name}` is replaced by that variable's JSON value, keeping its type (an integer stays an integer), at any depth in `input` and `expect`. A string that only contains `${…}` somewhere inside is left as it is. A placeholder naming a variable setup didn't return makes the case an `error`.

### Matching rules

- An expected object matches when each of its keys exists in the actual object and its value matches, recursively. Extra keys in the actual object are fine.
- An array matches when it has the same length and every element matches in order.
- Numbers compare by `f64` value, so `1` equals `1.0`.
- Other values compare with `==`.

Each mismatch is reported with its JSON path, for example `pending_actions[0].input.message: expected "Thanks", got "Hi"`.

## Report

A case's result is `pass` (every check passed), `fail` (one or more checks failed, all listed), `error` (setup, a placeholder, the database or the case timeout failed, with a message) or `skipped`. A case that doesn't settle within the case timeout is an `error`, and every later case is `skipped`, because a run still in flight would spend calls that later cases would be charged for.

Per case, the report records:

| Field | Meaning |
|---|---|
| `run_id`, `status` | The run and its final (or last seen) status. |
| `calls` | `claude_calls` rows logged during the case, failed calls included. |
| `usage_limit_hits` | Those rows with `status = 'usage_limit'`. |
| `input_tokens` | Sum of `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. |
| `output_tokens` | Sum of `output_tokens`. |
| `cost_usd` | Sum of the reported `cost_usd`. |
| `call_ms` | Sum of the calls' `duration_ms`. |
| `wall_ms` | From starting the run until it settled, measured by the harness. |
| `failures`, `error` | The failed checks, or the error message. |

Missing values count as 0. The totals are `cases`, `passed`, `failed`, `errors`, `skipped`, `pass_rate` (passed / cases, so skipped cases count against it), the sum of every per-case number, and `input_tokens_per_run`, `output_tokens_per_run` and `cost_per_run_usd` (each sum divided by the number of cases that weren't skipped).

The text report has a header line (workflow, dataset, number of cases), one row per case with its failed checks or error indented under it, and a final line starting `pass rate: <passed>/<cases> (<pct>%)` with the totals and per-run means. `--json` prints `{"workflow", "dataset", "cases": [...], "totals": {...}}` with the field names above, plus `result`, `failures` and `error` per case:

```sh
cargo run --bin eval -- tidy_application --json | jq .totals.pass_rate
```

## The `tidy_application` eval set

`api/evals/tidy_application.json` covers the sample workflow. It makes no model call, so `calls` is 0 in every case. Setup takes `{"application": {"company", "title", "status"?}, "memories": [...]}`, both optional. It first deletes every memory, so each case starts with exactly its own, then inserts the application (returning `{"application_id": <id>}`) and each memory, `{"name", "kind", "topic", "content", "tags"?, "status"?}`, validated like `POST /memories`, `confirmed` unless `status` says `proposed`, returning its id as `memory_<name>`. A blank or repeated `name`, or a memory that fails validation, is a setup error. With `{}` or no setup it inserts nothing.

The guardrail cases check that a confident note (0.9, and exactly 0.8) is applied as an `add_note` action, that a note below the threshold (0.79, 0.2) goes to the review queue instead, that every run parks with its `draft_follow_up` pending, that an unknown application or a missing `confidence` fails the run, and that none of them retrieves a memory. The unknown-application case retries the way the app does (3 attempts with the queue's backoff), so it takes about half a minute with the default settings.

The `recall:` cases check the `draft` step's retrieval: a relevant memory is retrieved and an unrelated one isn't, a proposed memory with matching words isn't, the memory sharing more terms with the query ranks first, a memory whose only match is a tag equal to the company is retrieved, no shared term retrieves nothing, and seven matching memories give exactly five rows.

The `draft:` cases check that the `draft` step appends confirmed `answer` and `preference` memories to the drafted message: a matching confirmed preference is appended to the pending `draft_follow_up` input's message while a matching confirmed fact is retrieved and recorded but left out of it, and a matching proposed preference leaves the message exactly as the run input's with nothing retrieved.

## The `memory_writer` eval set

`api/evals/memory_writer.json` covers the [memory writer](memory-writer.md). Every case but the last calls the real model once, so it costs a few cents per run. Setup takes `{"memories": [...], "signals": [...]}`, both optional. It first deletes every signal, evidence row and memory, then inserts the memories as the `tidy_application` set does (returning `memory_<name>`) and then each signal, `{"name", "kind", "subject", "drafted", "approved"?, "reason"?}`, returning its id as `signal_<name>`. Signals get run id 0 and their 1-based position as their pending action or review item id. An approval's `changes` is the diff from `drafted` to `approved`, and an approval without `approved` is an unedited one (`changes: []`). `approved` on anything but an approval, or `reason` on anything but a dismissal, is a setup error.

The cases check that an edit from "happy to relocate to NYC" to "remote only" becomes one `preference`/`location` proposal citing that signal, that a dismissal reason ("I won't work in crypto") becomes one preference, that an unedited approval teaches nothing, that correcting "two weeks' notice" to "one month's notice" proposes a revision of the confirmed `notice_period` answer instead of a new memory, and that a run with no signals makes no call. A case whose proposal falls below the 0.8 confidence threshold goes to the review queue and so fails its `created` label; the pass rate isn't expected to be 100%.

## The `classify_email` eval set

`api/evals/classify_email.json` covers [classify and extract](email.md#classify-and-extract): 17 cases, one model call each. Setup takes `{"emails": ["<path relative to api/fixtures/>", ...]}`. It deletes every classification, email and thread, then ingests the files in order (`source = fixture`, the file stem as the provider id, no known companies) **without queuing a run**, and returns `email_id`, the last file's id. Each case's input is `{"email_id": "${email_id}"}`, so the case's own run is the only one, and it expects `status: succeeded`. An empty list, a path containing `..` or starting with `/`, or a file that is missing or doesn't parse is a setup error.

There is one case per candidate fixture in `api/fixtures/emails/` (15). A reply is seeded after the message it answers, so the prompt's thread block has it: `recruiter-outreach.eml` comes before `recruiter-outlook-reply.eml` and before `recruiter-reply-subject-only.eml`. Two synthetic eval-only files in `api/fixtures/classify/` cover the classes the corpus lacks: `offer.eml`, and `availability-pt.eml`, a recruiter proposing "Tue Oct 27 at 2pm PT or Tue Nov 3 at 2pm PT", which crosses the US DST change. They sit outside `api/fixtures/emails/`, so the dev import and its manifest don't change.

Every case labels `output.class`:

| Fixture | Class |
|---|---|
| `ashby-rejection` | rejection |
| `greenhouse-application-received`, `greenhouse-company-domain`, `greenhouse-second-company`, `workday-application-update`, `workday-candidate-home`, `recruiter-bulk-tool`, `forwarded-thread`, `recruiter-reply-subject-only` | other |
| `interview-invite-ics`, `lever-interview-invite`, `gmail-reply-quoted` | interview_invite |
| `recruiter-outreach`, `recruiter-outlook-reply`, `rfc2047-latin1`, `availability-pt` | availability_request |
| `offer` | offer |

`forwarded-thread` is a friend passing along a recruiter's note to someone else, which is outreach, not a request for the user's availability. `recruiter-reply-subject-only` is the user's own reply, which asks nothing and confirms no slot. `recruiter-outlook-reply` asks for a phone screen "next Tuesday or Wednesday".

`output.company` is labelled only where the email names the company in exactly one form (not `gmail-reply-quoted` or `rfc2047-latin1`). `output.proposed_times` (its length and each `start_utc`) is labelled for `interview-invite-ics` (`2026-09-22T15:00:00Z`), `availability-pt` (`2026-10-27T21:00:00Z` and `2026-11-03T22:00:00Z`) and `gmail-reply-quoted` (one time with `start_utc: null`, since no zone is stated). Role, stage and interviewer are free-form and unlabelled.

**Classification accuracy** is the share of cases with no failure starting `output.class`:

```sh
cargo run --bin eval -- classify_email --json \
  | jq '([.cases[] | select(all(.failures[]; startswith("output.class") | not))] | length) / .totals.cases'
```

A case whose run fails has a `status` failure but no `output.class` one, so check the pass rate too. Tokens per email are the totals' `input_tokens_per_run` and `output_tokens_per_run`, since each run reads one email.

**Fixture results (synthetic corpus, not your inbox)**, recorded on 2026-10-23 with `haiku`:

| Pass rate | Classification accuracy | Input tokens per email | Output tokens per email | Cost per email |
|---|---|---|---|---|
| 17/17 (100%) | 17/17 (100%) | 5,172 | 447 | $0.0010 |

These files were written to exercise each class, so the numbers say how the prompt does on them, not how often it is right on real mail. Real-inbox numbers need your own labelled emails.

## The `match_email` eval set

`api/evals/match_email.json` covers [match and act](email.md#match-and-act): 15 cases. Matching is deterministic Rust, so the set makes **no model call** and runs the same anywhere. Setup takes:

```json
{"applications": [{"name", "company", "title", "status", "stages"?}],
 "emails": [{"file", "classification": {"class", "confidence", "company"?, "role"?, "stage"?}, "matched_to"?}]}
```

with unknown fields refused. It clears matches, classifications, emails, threads, application events, stages and applications; inserts the applications with their stages and a `created` event; then ingests each file in order like the `classify_email` set (**without queuing a run**), stores a `classified` row for it with the given answer, and, for `matched_to`, an `applied` match row to that application. Variables: `application_<name>` per application, and `email_id` and `classification_id` of the last email. Each case's input is `{"classification_id": "${classification_id}"}`. An empty email list, a bad path, an unknown class, a confidence outside 0..=1, a blank or repeated application name, or a `matched_to` naming no application is a setup error.

Every case labels `status: succeeded`, `output.status` and `output.application_id`, and `actions` and `review_items` where they matter:

| Case | Fixture | Outcome |
|---|---|---|
| `rejection-company-and-role` | `ashby-rejection` | applied |
| `ats-sender-name-and-role` | `lever-interview-invite` (company null) | applied |
| `company-only-is-unsure` | `greenhouse-application-received` | review, `email.match` |
| `role-mismatch-proposes-new` | `greenhouse-second-company` | review, `email.new_application` |
| `ambiguous-same-company` | `lever-interview-invite`, two Hooli applications | review, `ambiguous` |
| `invite-adds-stage` | `lever-interview-invite` | applied, 2 actions |
| `existing-stage-no-action` | `lever-interview-invite`, already interviewing with that stage | no_action |
| `sender-domain-and-company` | `recruiter-outreach` | applied |
| `thread-history` | `recruiter-outreach` matched, then `recruiter-outlook-reply` (company and role null) | applied |
| `offer` | `classify/offer` | applied |
| `invite-on-rejected-is-unexpected` | `interview-invite-ics` | review, `unexpected_status` |
| `low-classification-confidence` | `ashby-rejection` at 0.6 | review, `low_confidence` |
| `new-application-from-ats` | `greenhouse-second-company`, no applications | review, `email.new_application` (`applied`) |
| `no-company-unmatched` | `forwarded-thread` (company null) | unmatched |
| `confident-other-links` | `greenhouse-company-domain` | no_action with the application |

**Match accuracy** is the share of cases with no failure starting `output.status` or `output.application_id`:

```sh
cargo run --bin eval -- match_email --json \
  | jq '([.cases[] | select(all(.failures[]; (startswith("output.status") or startswith("output.application_id")) | not))] | length) / .totals.cases'
```

**Fixture results (synthetic corpus, not your inbox)**, recorded on 2026-10-23:

| Pass rate | Match accuracy | Model calls | Cost |
|---|---|---|---|
| 15/15 (100%) | 15/15 (100%) | 0 | $0.0000 |

The cases and their classifications were written to exercise each signal and outcome, so these numbers check the rules, not how often matching is right on real mail. Real-inbox numbers need your own labelled emails.

## Adding a suite

1. Implement `waypoint_api::evals::Suite` for the workflow, next to it in `api/src/workflows/`: `workflow()` returns its name, and `setup()` seeds the rows a case's `setup` describes and returns the variables its placeholders use.
2. Register it in `workflows::eval_suites()`.
3. Write `api/evals/<workflow>.json` with labelled cases, covering the confident path, the review-queue path, the approval gate and the failures.
4. Run `cargo run --bin eval -- <workflow>` and check the pass rate and the cost per run.

Scoring is label matching only.
