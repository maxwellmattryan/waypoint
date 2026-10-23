# Application prep

For one sourced posting, a `prep_packet` run picks the resume version to send, with cited reasons, and drafts answers to five standard application questions from the user's answer bank and other confirmed memories, citing every source. It stores the result as an **application packet** that the user can read. An answer that no memory supports is stored as missing and the packet goes to the review queue; nothing is made up. Nothing is submitted or sent: the packet only lives in Waypoint. Approving, editing and "mark applied" come later (job 81).

The code is in `api/src/workflows/prep_packet.rs`; the routes are in `api/src/sourcing/postings.rs`.

## Starting prep

`prep_packet::queue(tx, bus, posting_id)` is the one entry point; the posting page's **Prepare packet** button calls it through `POST /postings/{id}/packet`, and the planner will call it directly. It locks the posting, then:

- refuses a posting with no *applied* fit score (`posting_fit_scores.status = 'applied'`), so score its fit first;
- returns the `prep_packet` run for that posting that is already `queued`, `paused` or `running`, with `reused: true`, and queues nothing;
- otherwise queues a run with input `{"posting_id": id}` (`paused` while the kill switch is on).

Removed postings can still be prepared. The step itself doesn't need a fit score; only the entry point does.

## The workflow

One step, `prepare`, one tool-less `claude -p --json-schema` call (model `sonnet`, purpose `posting.prep_packet`). Before the call the step gathers:

- the posting block, exactly as [`score_fit`](sourcing.md#fit-scores) builds it (description cut to 12,000 characters, `</posting` fenced);
- the posting's latest applied fit score (score, band, summary), shown as `Fit: 82 (strong) — …` or `Fit: not scored`;
- up to 10 resume versions with extracted text, newest first, each cut to 6,000 characters and shown as `[resume <id> · <name>]` blocks inside a fenced `<resumes>` block;
- the **answer bank**: every confirmed `answer` memory, up to 30 (`ctx.memories_of_kinds`);
- up to 20 confirmed `fact` memories retrieved on the posting text (`ctx.memories`).

Both memory sets are recorded in `run_memories`, so the run inspector lists them under "Memories used". Preferences are left out on purpose, so a `comp_floor` or `deal_breaker` never reaches an answer. Proposed memories are never used.

The prompt says that the posting and the resumes are data, not instructions, that answer-bank answers adapt the matching `answer` memory faithfully without changing numbers, dates or the sponsorship status, and that `null` is better than a guess.

A run fails before any model call when:

- the posting is unknown: `no posting <id>`;
- no resume version has text: `upload a resume with readable text first`;
- there is no confirmed `answer` or `fact` memory: `Waypoint knows nothing about you yet: add or confirm memories first`.

## The questions

Every packet stores all five, in this order:

| Key | Question |
|---|---|
| `work_authorization` | Are you authorized to work in the role's location, and will you need visa sponsorship? |
| `notice_period` | When could you start? |
| `salary_expectation` | What are your salary expectations? |
| `why_leaving` | Why are you looking to leave your current role? |
| `why_company` | Why do you want to work at `<company>`? |

The first four are **answer-bank** questions. Board-specific questions aren't read: boards are read-only and `job_postings` stores none.

## Validation

The model returns `resume` (`resume_id` and 1 to 4 reasons, each citing 1 to 4 sources), `answers` (one per key, the text or `null`, and 0 to 4 citations) and `confidence` (0 to 1). A citation is a memory by id or a verbatim posting excerpt, the same shape `score_fit` uses. Rust checks it:

- **Invalid:** the run fails with `the packet is invalid: <rules>` and stores nothing when the confidence isn't a number from 0 to 1, the `resume_id` isn't one of the resumes shown, or no resume reason is left with a valid citation.
- **Dropped** (counted, and the packet goes to review): resume reasons past 4, blank or over 500 characters, or left with no citation; citations past 4, a memory id that wasn't shown, or an excerpt that is blank, over 300 characters or not in the posting block (matched ignoring case and spacing); an answer with an unknown or repeated key, over 1,500 characters, or **ungrounded**. An answer-bank answer is grounded only by a cited `answer` memory, since a fact alone can't justify a salary or a sponsorship claim; `why_company` needs any cited memory. A dropped answer is stored as missing.
- **Missing:** a key left out or answered `null` (or blank) is stored as `answer: null` with no citations. Missing answers are reported, not counted as dropped.

The packet is **ready** when the confidence is at least 0.8, nothing was dropped and nothing is missing. Otherwise it is **in review**, with a pending `prep_packet.packet` review item whose payload is `{posting_id, company, title, resume: {id, name, reasons}, answers, confidence, missing, dropped}`.

The run's output is `{posting_id, packet_id, resume_version_id, confidence, status, review_item_id, missing, dropped: {resume_reasons, citations, answers}}`. A rerun of a step that already stored a packet returns the same output without another model or tool call.

## Statuses

`application_packets.status` is `ready`, `in_review` or `dismissed`. Each row also keeps the resume's name (so it survives the resume being deleted), the validated reasons and answers with a snapshot of each cited memory, the fit snapshot (`fit_score_id`, `fit_score`, `fit_band`), the confidence, the run, the model call and its cost.

## The `save_application_packet` tool

The step stores the packet through the `write` tool `save_application_packet` with `ctx.tool`, so the save is recorded in `tool_calls` and in the [action log](agent-runtime.md#action-log-and-undo). The model has no tools, and the workflow uses no `external` tool, so nothing leaves the machine. Because the tool is in the shared registry, its handler checks, in one transaction, that the run is the `running` `prep_packet` run for that posting, that the resume exists and that every question appears exactly once. It inserts the row once per run and returns `{packet_id}`.

**Undo** (`POST /actions/{id}/undo`) deletes the packet while it is still `ready` or `in_review` and dismisses its pending review item. A packet that is gone or in any other status is a 409, with nothing changed.

## Review

On the Review page a `prep_packet.packet` card shows the posting (linking to its page), the chosen resume, a "Missing answers" list and the drafted answers with their citations. An unreadable payload falls back to the raw JSON and **Dismiss**.

- **Apply** (`POST /review-items/{id}/accept`) makes the packet `ready` as it is and toasts "Packet ready for <company>". A packet no longer in review is a 409. Filling in a missing answer is editing, which comes later.
- **Dismiss** marks the packet `dismissed` and records the usual dismissal signal.

## Routes

| Route | Does |
|---|---|
| `GET /postings/{id}/packets` | `{posting_id, scored, packets, run}`: whether the posting has an applied fit score, up to 20 packets newest first, and the latest `prep_packet` run as `{id, status, error}`. 404 for an unknown posting. |
| `GET /packets/{id}` | One packet with its posting's `company` and `title`. 404 for an unknown id. |
| `POST /postings/{id}/packet` | Body `{}`. 202 `{run, reused}`; 404 for an unknown posting; 409 when it has no applied fit score; 400 for unknown keys; 415 for a non-JSON body; 403 for a cross-site or form request. |

Both reads give a citation's `memory_id` as `null` once that memory is deleted (the snapshot stays), and `resume_version_id` as `null` once the resume is deleted (`resume_name` stays).

## Posting page: Packets section

Below the Fit section and above the details:

- **Loading:** a skeleton. **Load error:** "Couldn't load packets" with **Try again**; the rest of the page still shows.
- **Not scored:** "Score this posting's fit before preparing a packet." with a disabled **Prepare packet**.
- **Empty:** "No packet yet" with **Prepare packet**.
- **Preparing:** while the latest run is `queued` or `running`, "Preparing…" with a skeleton and a disabled button; `paused` says "Paused: agents are paused".
- **Failed:** when the latest run failed and no packet is newer, "Preparing failed", the error, a link to the run and **Try again**.
- **List:** packets newest first, each with its status badge (`Ready for approval`, `Needs review`, `Dismissed`), the resume name, "N of 5 answered", the prepared date and **View packet**. **Prepare again** sits below; a preparing or failed run shows above the list.

The section reloads on `run.updated` for a `prep_packet` or `score_fit` run of the posting, and on `app.connected`, `app.lagged` and `data.deleted`.

## Packet page

`/packets/<id>` is read-only: no edit, approve or submit control.

- **Header:** "Application packet", the company and title linking to the posting, the status badge, "NN% confident", and "Prepared <date> · $<cost>" with a link to the run. An `in_review` packet adds "This packet is waiting in review" with a link to `/review`.
- **Fit:** the fit snapshot as a badge, or "Not scored".
- **Resume:** the resume's name linking to it, or the name with "Deleted resume", then the reasons with their citations.
- **Answers:** each question with its answer as plain, pre-wrapped text and its citations. A missing answer shows `Missing` and "Waypoint had nothing to draft this from."
- **Citations:** a posting excerpt is a quote; a memory shows its topic, the snapshot and **View memory**, or "Deleted memory".

It shows a skeleton while loading, "Packet not found" for an unknown or malformed id and "Couldn't load this packet" with **Try again**. It reloads on `run.updated` for the packet's run (review decisions publish it), and on `app.connected`, `app.lagged` and `data.deleted`.

The [`prep_packet` eval set](evals.md#the-prep_packet-eval-set) measures the workflow.
