# Application prep

For one sourced posting, a `prep_packet` run picks the resume version to send, with cited reasons, and drafts answers to five standard application questions from the user's answer bank and other confirmed memories, citing every source. It stores the result as an **application packet**. An answer that no memory supports is stored as missing and the packet goes to the review queue; nothing is made up. The user then edits and approves (or rejects) the packet through the approval gate, applies on the company's site themselves, and marks it applied, which creates or updates the posting's application. **Nothing is submitted or sent automatically:** no step or tool contacts a job board, and "mark applied" only records what the user did.

The code is in `api/src/workflows/prep_packet.rs` (prep, the packet reads) and `api/src/workflows/packet_actions.rs` (approval, mark applied and their undos); the routes are in `api/src/sourcing/postings.rs`.

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

```
in_review --accept review item--> ready --approve (pending action)--> approved --mark applied--> applied
in_review --dismiss review item--> dismissed      ready --reject--> dismissed
approved --undo approval--> ready                 applied --undo--> approved
```

`application_packets.status` is `ready`, `in_review`, `dismissed`, `approved` or `applied`. The draft columns (`resume_version_id`, `answers`) never change after the save; what the user approved is in `approved_resume_version_id`, `approved_resume_name`, `approved_answers` and `approved_at`, and marking applied sets `application_id` and `applied_at`. CHECKs require the `approved_*` columns on an `approved` or `applied` packet, and `applied_at` exactly on an `applied` one. Each row also keeps the resume's name (so it survives the resume being deleted), the validated reasons and answers with a snapshot of each cited memory, the fit snapshot (`fit_score_id`, `fit_score`, `fit_band`), the confidence, the run, the model call and its cost.

## The `save_application_packet` tool

The step stores the packet through the `write` tool `save_application_packet` with `ctx.tool`, so the save is recorded in `tool_calls` and in the [action log](agent-runtime.md#action-log-and-undo). The model has no tools, and the workflow uses no `external` tool, so nothing leaves the machine. Because the tool is in the shared registry, its handler checks, in one transaction, that the run is the `running` `prep_packet` run for that posting, that the resume exists and that every question appears exactly once. It inserts the row once per run and returns `{packet_id}`.

**Undo** (`POST /actions/{id}/undo`) deletes the packet while it is still `ready` or `in_review` and dismisses its pending review item. While the packet's approval action is still `pending`, the undo also rejects that action and ends its run `rejected` (error `the packet was undone`), without a learning signal. While the approval is `approved` or `executing`, the undo is a 409. A packet that is gone or in any other status is a 409, with nothing changed.

## Review

On the Review page a `prep_packet.packet` card shows the posting (linking to its page), the chosen resume, a "Missing answers" list and the drafted answers with their citations. An unreadable payload falls back to the raw JSON and **Dismiss**.

- **Apply** (`POST /review-items/{id}/accept`) makes the packet `ready` as it is, queues its approval request in the same transaction, and toasts "Packet ready for <company>". It records no signal. A packet no longer in review is a 409. A missing answer is filled in by editing the packet when approving it.
- **Dismiss** marks the packet `dismissed` and records the usual dismissal signal. An `in_review` packet has no approval request, so nothing else is recorded.

So every decision records exactly one signal: a dismissal, an approval or a rejection. An `in_review` packet never has an approval request, so a low-confidence packet has to clear review first.

## Approval

A `ready` packet gets one **approval request**: an `approve_packet` run (input `{posting_id, packet_id}`) whose one step, `approve`, calls the tool `approve_application_packet`. That tool is a `write` tool, since it only changes Waypoint's data, but it declares [`NEEDS_APPROVAL`](agent-runtime.md#approval-gate), so the call parks the run in `awaiting_approval` with one `pending` pending action. Its requested input is the draft: `{packet_id, resume_version_id, answers: {work_authorization, notice_period, salary_expectation, why_leaving, why_company}}`, each answer its text or `null`. A rerun of the step asks with the same input, so it finds the same pending action.

`packet_actions::request_approval(tx, bus, packet_id)` queues it, refusing a packet that isn't `ready` and reusing an `approve_packet` run that is `queued`, `paused`, `running` or `awaiting_approval`. It is called:

- by `prepare` when it stored a `ready` packet (also on its stored-output rerun), in its own transaction;
- by accepting the packet's review item, in the same transaction;
- by `POST /packets/{id}/approval`, for packets prepared before approvals existed and after an approval is undone.

**Approve** goes through `POST /pending-actions/{id}/approve`: `{}` approves the draft as it is, `{"input": {…}}` an edited resume choice and edited answers. Inside the decision's transaction, before the action becomes `approved`, `packet_actions::check_approval` locks the packet and checks that it still exists, is `ready` and belongs to this run (409 otherwise), that `packet_id` is unchanged, that `resume_version_id` is set and names an existing resume, and that each answer has at most 1,500 characters after trimming (422 `{"errors":[{"field":"input","message":"<path>: <reason>"}]}` otherwise, never quoting a value). A refusal changes nothing. The approval records exactly one `approval` signal, whose `changes` has one entry per edited field (`/answers/<key>`, `/resume_version_id`).

When the run resumes, the handler makes the packet `approved` with `approved_resume_version_id` and `approved_resume_name`, `approved_answers` (all five, trimmed, a blank answer stored as `null`, an answer keeping its citations only when its text didn't change) and `approved_at`. Its call is logged in the action log.

**Reject** goes through `POST /pending-actions/{id}/reject`: the run ends `rejected`, the one `rejection` signal is recorded and the `ready` packet becomes `dismissed`, in one transaction. A rejected packet has no status of its own.

## Mark applied

Once the user has applied on the company's site, `POST /packets/{id}/applied` queues an `apply_packet` run (input `{posting_id, packet_id}`; it waits while the kill switch is on, like prep). It answers 409 unless the packet is `approved` and its approved resume still exists, and reuses a run that is `queued`, `paused` or `running`. The run's one step, `apply`, calls the `write` tool `mark_packet_applied`, which checks that its run is the `running` `apply_packet` run for the packet and then, in one transaction, finds the **posting's application**: the `application_id` of the posting's most recently applied packet, if that application still exists. Postings and applications have no other link.

- **None:** it inserts an application from the posting with status `applied`, the approved resume and a `created` event: `company`, `title` and `location` (from `location_raw`) cut to 200 characters, `url` (null unless an http(s) URL of at most 2,048 characters) and `source` `Greenhouse`, `Lever` or `Ashby`.
- **Exists:** it locks the application, sets its resume to the approved one and moves `updated_at`. A `sourced` application also becomes `applied` with a `status_changed` event; any other status is left as it is.

The packet becomes `applied` with `application_id` and `applied_at`. The **answers used** live on the packet (`approved_answers`); the application reaches them through `application_packets.application_id`, and the `Application` shape is unchanged. The step then publishes `application.created` or `application.updated`. A rerun after the tool succeeded returns its output, `{packet_id, application_id, created}`.

Because `applications.resume_version_id` is `ON DELETE RESTRICT`, the approved resume can't be deleted while the application links it. That is expected: the application records which resume was sent.

## Undo

Both go through `POST /actions/{id}/undo` and refuse (409, nothing changed) rather than overwrite something that changed since. `undo_action_id` on the packet names the action to undo.

- **Mark applied:** refused unless the packet is still `applied` with the same `application_id` and `applied_at`, and the application still has the `updated_at` the action left. The packet goes back to `approved` with `application_id` and `applied_at` cleared. A created application is deleted; an updated one gets back its previous `status`, `status_changed_at`, `resume_version_id` and `updated_at`, and the `status_changed` event the action recorded (if any) is deleted.
- **Approval:** refused unless the packet is still `approved` with the same `approved_at` (so never once it is `applied`). It goes back to `ready` with every `approved_*` column cleared. No new approval request is queued; the packet page offers **Request approval**.

Undo publishes no event (the runtime's rule); the packet page reloads itself after its own undo.

## Routes

| Route | Does |
|---|---|
| `GET /postings/{id}/packets` | `{posting_id, scored, packets, run}`: whether the posting has an applied fit score, up to 20 packets newest first, and the latest `prep_packet` run as `{id, status, error}`. 404 for an unknown posting. |
| `GET /packets/{id}` | One packet with its posting's `company` and `title`. 404 for an unknown id. |
| `POST /postings/{id}/packet` | Body `{}`. 202 `{run, reused}`; 404 for an unknown posting; 409 when it has no applied fit score; 400 for unknown keys; 415 for a non-JSON body; 403 for a cross-site or form request. |
| `POST /packets/{id}/approval` | Body `{}`. Queues the packet's approval request (`approve_packet`). 202 `{run, reused}`; 404 for an unknown packet; 409 unless it is `ready`; 400, 415 and 403 as above. |
| `POST /packets/{id}/applied` | Body `{}`. Queues marking it applied (`apply_packet`). 202 `{run, reused}`; 404 for an unknown packet; 409 unless it is `approved` with its approved resume present; 400, 415 and 403 as above. |

Both reads give a citation's `memory_id` as `null` once that memory is deleted (the snapshot stays, in `approved_answers` too), and `resume_version_id` as `null` once the resume is deleted (`resume_name` stays). Each packet also has `approved_resume_version_id`, `approved_resume_name`, `approved_answers`, `approved_at`, `application_id` and `applied_at` (all nullable), and:

- `pending_action_id`: the `pending` approval action for the packet, or `null`;
- `undo_action_id`: the latest not-undone `approve_application_packet` action while the packet is `approved`, the latest not-undone `mark_packet_applied` action while it is `applied`, otherwise `null`;
- `latest_run`: the latest `approve_packet` or `apply_packet` run for the packet as `{id, status, error}`, or `null`.

## Posting page: Packets section

Below the Fit section and above the details:

- **Loading:** a skeleton. **Load error:** "Couldn't load packets" with **Try again**; the rest of the page still shows.
- **Not scored:** "Score this posting's fit before preparing a packet." with a disabled **Prepare packet**.
- **Empty:** "No packet yet" with **Prepare packet**.
- **Preparing:** while the latest run is `queued` or `running`, "Preparing…" with a skeleton and a disabled button; `paused` says "Paused: agents are paused".
- **Failed:** when the latest run failed and no packet is newer, "Preparing failed", the error, a link to the run and **Try again**.
- **List:** packets newest first, each with its status badge (`Ready for approval`, `Needs review`, `Dismissed`, `Approved` in the info tone, `Applied` in the success tone), the resume name, "N of 5 answered", the prepared date and **View packet**. **Prepare again** sits below; a preparing or failed run shows above the list.

The section reloads on `run.updated` for a `prep_packet`, `score_fit`, `approve_packet` or `apply_packet` run of the posting, and on `app.connected`, `app.lagged` and `data.deleted`.

## Packet page

`/packets/<id>`:

- **Header:** "Application packet", the company and title linking to the posting, the status badge, "NN% confident", and "Prepared <date> · $<cost>" with a link to the run. An `in_review` packet adds "This packet is waiting in review" with a link to `/review`.
- **Actions:** a card between the header and Fit (see below).
- **Fit:** the fit snapshot as a badge, or "Not scored".
- **Resume:** the resume's name linking to it, or the name with "Deleted resume", then the reasons with their citations.
- **Answers:** each question with its answer as plain, pre-wrapped text and its citations. A missing answer shows `Missing` and "Waypoint had nothing to draft this from."
- **Citations:** a posting excerpt is a quote; a memory shows its topic, the snapshot and **View memory**, or "Deleted memory".

For an `approved` or `applied` packet, Resume and Answers show the approved resume and answers, labelled "Approved version". The draft's reasons show only when the approved resume is the drafted one.

The **Actions card** (`PacketActions.svelte`, its state from `actionState` in `lib/packets/api.ts`):

| State | Shows |
|---|---|
| `in_review` | "Accept or dismiss its review item before approving." and a link to `/review`. |
| `ready`, approval pending | **Approve**, **Edit**, **Reject** (through "Reject this packet?" / "It will be dismissed, and Waypoint learns from the rejection."). |
| `ready`, approval run `queued` or `running` | "Getting the approval ready…" (`paused`: "Paused: agents are paused"). |
| `ready`, otherwise | **Request approval**; after a failed run, "Couldn't request approval", its error and a run link. |
| `approved` | "Waypoint doesn't submit anything. Apply on the company's site, then mark it applied.", **Mark applied** and **Undo approval**. A deleted approved resume says so and disables Mark applied; a `queued` or `running` mark-applied run shows "Marking applied…" (or the paused text); a failed one shows its error, a run link, and Mark applied reads **Try again**. |
| `applied` | "Applied <date>", **View application** (or "Application deleted") and **Undo**. |
| `dismissed` | "This packet was dismissed." |

**Edit** replaces Resume and Answers with a form (`PacketAnswerEditor.svelte`): a resume Select of the user's resume versions, preselected to the draft's resume ("Choose a resume" when that resume was deleted; disabled with "Loading resumes…" while they load; "Couldn't load resumes" with **Try again**), and one Textarea per question prefilled with the draft. **Approve with edits** checks "Choose a resume" and "At most 1,500 characters", sends the trimmed answers (a blank one as `null`) and shows a 422's message above the buttons; **Cancel** closes it.

Every button is disabled while its request is in flight, and the clicked one shows a pending label. The toasts are "Packet approved", "Packet rejected", "Approval requested", "Marking applied…" and "Undone"; a 409 toasts "This packet changed. It has been reloaded." and reloads, any other failure "Couldn't <verb>. Try again." Every successful action reloads the packet. The card's state changes and the edit form fade with the shared motion tokens.

It shows a skeleton while loading, "Packet not found" for an unknown or malformed id and "Couldn't load this packet" with **Try again**. It reloads on `run.updated` for the packet's run (review decisions publish it) and for an `approve_packet` or `apply_packet` run whose input names the packet, and on `app.connected`, `app.lagged` and `data.deleted`.

The [`prep_packet` eval set](evals.md#the-prep_packet-eval-set) measures the workflow.
