# Memory writer

The memory writer is how Waypoint learns from the user's decisions. Every approval, rejection and dismissal of drafted work is stored as a [learning signal](agent-runtime.md#learning-signals). The writer reads the unprocessed ones together with the memories already stored, asks Claude once per batch which durable facts, preferences or ready answers they show, checks every proposal in Rust and stores the valid ones as `proposed` memories linked to their evidence. Nothing it writes affects a draft until the user confirms it on the "What Waypoint knows about me" page, because retrieval only uses confirmed memories.

The code is `api/src/workflows/memory_writer.rs`. It's a registered workflow, `memory_writer`, with one step, `write`, and its runs go through the job queue as `agent_run` jobs, like any other run. That gives it the kill switch, the run inspector (tokens and cost per run), the review queue and the eval harness.

## Scheduling

- **After a decision.** `record_approval`, `record_rejection` and `record_dismissal` schedule a writer run in the decision's own transaction, with input `{"trigger": "decision"}` and its job's `run_at` 60 s in the future (`SCHEDULE_DELAY`). Decisions made within that window share one model call.
- **At most one waiting run.** When a writer run is already waiting, a decision schedules nothing and leaves that run's `run_at` alone. A waiting run is `paused`, or `queued` with its job still in the queue. A `running` run doesn't count, so a decision made during a run schedules the next one. An advisory transaction lock makes this exact under concurrent decisions.
- **On demand.** `POST /memories/learn` (no body) answers `202 {"run_id", "reused"}`. A waiting run comes back with `reused: true`, and when it's `queued` its job's `run_at` moves to now. Otherwise a new run starts with `{"trigger": "manual"}` and no delay. The worker is woken either way. A run with no unprocessed signals makes no model call. The route takes `LocalHost` and `NotCrossSite`: a foreign `Host`, a cross-site request or a form or multipart body is 403 and creates nothing.
- **The next batch.** A run that leaves signals unprocessed schedules a `{"trigger": "batch"}` run with no delay, with the same reuse rules as the route.
- **Kill switch.** While it's on, a scheduled run is stored `paused` like any run.

## Claims

A run claims up to `MAX_BATCH` (25) unprocessed signals, oldest `id` first, by setting `memory_signals.writer_run_id` to itself. A signal is claimable when `processed_at` is `NULL` and it's unclaimed, claimed by this run (a retried or resumed step takes its batch back), or claimed by a run that is gone or no longer `queued`, `running` or `paused`. The claim is one `UPDATE … WHERE id IN (SELECT … FOR UPDATE OF s SKIP LOCKED)`, so two runs never take the same signal.

Claims replace a transaction held across the model call, which could take minutes and would block `DELETE /data`. Because a claim only counts while its run is live, a run that fails for good releases its signals with no cleanup, and a later run picks them up.

## The `write` step

1. Claim the batch. With none, succeed with empty lists and `remaining: 0`, without a model call.
2. Read up to `MAX_CONTEXT_MEMORIES` (100) memories of any status, newest `updated_at` first, ties by `id` descending: "the shown memories". They aren't recorded in `run_memories`, which is for memories used as context for a draft.
3. Make one structured call (`purpose: "memory.writer"`, model `sonnet`), tool-less and isolated like every call. It's logged in `claude_calls`, and its tokens and cost count against the step and the run. A usage limit defers the job, and any other runner error, a schema mismatch included, retries the step. Nothing is written then, and the signals stay claimed by the run.
4. [Validate](#validation) every proposal. Valid ones with `confidence` below 0.8 go to the review queue as `memory_writer.proposal` items with the normalised proposal `{action, target_memory_id, kind, topic, content, tags, evidence_signal_ids}` as payload, and aren't stored. A rerun gets the same items back.
5. In one transaction, [store](#storage) the rest and set `processed_at` on every claimed signal, including ones no proposal cited. If fewer signals are marked than were claimed, or anything fails, everything rolls back and the step retries.
6. When unprocessed signals remain, schedule the next batch.

The step's output, and so the run's:

```json
{
  "signals": [1, 2, 3],
  "created":   [{"memory_id": 7, "kind": "preference", "topic": "location", "evidence_signal_ids": [1]}],
  "updated":   [],
  "revisions": [{"memory_id": 8, "revises_memory_id": 1, "kind": "answer", "topic": "notice_period", "evidence_signal_ids": [2, 3]}],
  "already_known": 0,
  "reviewed": [4],
  "invalid": [{"index": 3, "reason": "the topic does not belong to the kind"}],
  "remaining": 0
}
```

`signals` are the claimed ids, ascending. `invalid` lists each dropped proposal's position in Claude's list with a fixed reason written in Rust, never model text. `remaining` counts the unprocessed signals left after the commit.

## Prompt and untrusted input

The prompt explains each signal kind, lists the kind/topic table from `MemoryKind::topics()`, and asks for durable facts, preferences or answers that the evidence supports, with an empty list being a fine answer. It asks Claude to prefer updating a shown memory over a near-duplicate create, to cite only listed signals as evidence, and to calibrate `confidence` from 0 to 1.

Signal payloads and memory content are untrusted, and the prompt says to ignore any instructions in them. The call has no tools. Each signal is one compact JSON line, `{"id", "kind", "subject", "drafted", "approved", "changes", "reason"}`, with `drafted`, `approved` and `changes` as their JSON text cut to 2,000 characters (`…` appended when cut). Each shown memory is one line, `{"id", "kind", "topic", "status", "content"}`, with content cut to 500 characters. The blocks sit inside `<signals>…</signals>` and `<memories>…</memories>`, and any `</signals>` or `</memories>` inside them is rewritten to `</ signals>` / `</ memories>`, so the content can't close a block early. Signal reasons are never logged.

The `--json-schema` requires every key and allows no others: `{"proposals": [{"action", "target_memory_id", "kind", "topic", "content", "tags", "evidence_signal_ids", "confidence"}]}`. Rust reads it with `deny_unknown_fields` too, and keeps `action`, `kind` and `topic` as text, so one bad proposal is dropped instead of failing the batch.

## Validation

Applied in proposal order (`validate`, a pure function):

- **V1.** Only the first 20 proposals (`MAX_PROPOSALS`) are considered; later ones are invalid.
- **V2.** `action` is `create` or `update`. A create has `target_memory_id: null`. An update targets one of the shown memories, never just any id in the table. A second update of the same target in one batch is invalid.
- **V3.** `kind` and `topic` parse, the topic belongs to the kind, and content and tags pass `memories::validate`: trimmed, non-blank content of at most 5,000 characters, and at most 20 tags of at most 50 characters after normalisation.
- **V4.** `evidence_signal_ids` is non-empty after de-duplication, and every id is in the claimed batch.
- **V5.** `confidence` is finite and within 0..=1.
- **V6.** A proposal that already holds is counted in `already_known` and writes nothing: for a create, a shown memory with the same kind, topic and normalised content; for an update, a target that already has the same kind, topic, normalised content and tags.

## Storage

Each target is locked with `SELECT … FOR UPDATE` first. Nothing ever stores or changes a memory as `confirmed`.

- **Create.** A new `proposed` memory with `source_kind: signal`, `source_ref` set to the lowest evidence signal id and `source_label` set to `"1 decision"` or `"<n> decisions"`, plus one `memory_evidence (memory_id, signal_id)` row per evidence signal.
- **Update of the writer's own proposal** (`proposed`, `source_kind = 'signal'`). Updated in place: kind, topic, content, tags and `updated_at`. The new evidence rows are added, `source_label` is recomputed from the memory's total evidence, and it stays `proposed`.
- **Update of anything else** (a confirmed memory, or another source's proposal). The target is never modified. A new proposed `signal` memory with the proposal's fields is stored as for a create, with `revises_memory_id` pointing at the target, and listed under `revisions`.
- **A target deleted since the prompt** makes that proposal invalid (`target memory no longer exists`); the rest of the batch still commits.

`memory_evidence` cascades from both sides, and `revises_memory_id` is set to `NULL` when the revised memory is deleted. The memories page shows a `signal` memory as "Learned from <source_label>", or "Learned from your decisions" without a label. Showing evidence and revisions, and confirming a revision in place of the original, come later in the #51 follow-up.

## Evals

`api/evals/memory_writer.json` holds five cases: an edited location becomes a preference, a dismissal reason becomes a preference, an unedited approval teaches nothing, a correction revises a confirmed answer, and no signals makes no call. Run them with `cargo run --bin eval -- memory_writer`; see [evals.md](evals.md#the-memory_writer-eval-set).
