# Gmail sync

Gmail sync reads the connected Google account's inbox, read-only, and stores each message with [`email::ingest`](email.md). It stops at stored `emails` rows with their heuristic verdicts. It makes no model calls: classifying the messages comes later (#66).

The code is `api/src/gmail/`: `client.rs` is the Gmail REST client, `sync.rs` the sync engine on the job queue, and `mod.rs` the settings and the two routes.

Any numbers so far are fixture results. The sync has been tested against a fake Gmail (wiremock) serving the synthetic messages in `api/fixtures/emails/`, not against a real inbox.

## The client

`GmailClient` sends only `GET` requests with the stored access token as a `Bearer` token: `users.getProfile`, `messages.list`, `messages.get?format=raw` and `history.list`, all under `/gmail/v1/users/me/`. It needs nothing beyond the `gmail.readonly` scope Waypoint already asks for. Responses are parsed into structs trimmed to the fields the sync uses, and Gmail ids and `historyId`s stay strings.

## Runs and modes

A sync **run** is a chain of `gmail_sync` jobs. Each job processes exactly one page and, if more pages remain, enqueues the next page's job. A run has one mode:

| Mode | When | What it reads | Bound |
|---|---|---|---|
| `backfill` | No stored `history_id`, or the connected account's address differs from the stored one | `messages.list` with `newer_than:30d -in:drafts`, newest first | `max_messages` (1000) listed messages |
| `incremental` | A stored `history_id` for the same account | `history.list` from that id, `historyTypes=messageAdded` | None. It reads until Gmail's history ends, one page per job |
| `resync` | `history.list` answered 404: the stored history expired | Same as backfill | `max_messages` |

The first job of a run (payload `{"trigger": "manual" | "startup"}`) reads the profile, picks the mode, resets the run's counts in the state row and processes the first page. Later jobs carry the cursor in their payload: `{"mode", "start_history_id", "page_token", "page", "listed"}`.

**Which `historyId` is stored.** A backfill or resync keeps the profile's `historyId` from the start of the run in its payload and stores it only when the run's last page finishes, never an id from a page. Mail that arrives during the run is newer than that id, so the next incremental run picks it up. An incremental run stores the `historyId` of its last `history.list` page. A run that fails part-way leaves `history_id` unchanged, so the next run repeats the work. That's safe because ingest upserts.

A different account (connecting as someone else after `reconnect_required`, say) drops the old `history_id` and starts a backfill. Existing rows are kept.

Incremental runs have no message cap, because capping would silently skip mail between two ids. They stay paced instead.

## Per page

1. Get an access token. If Google isn't configured, no account is connected or it must be reconnected, the run ends: no next page, `last_error` is set and the job **succeeds** with `{"stopped": "<reason>"}`. A token-endpoint or database error retries.
2. List one page of at most `page_size` (100) ids. Duplicate ids are dropped. History entries labelled `DRAFT`, `SPAM` or `TRASH` are dropped before fetching.
3. Fetch each id with `messages.get?format=raw`, one at a time:
   - Over the [size cap](#size-cap): counted `oversized`, never ingested.
   - 404: deleted since it was listed, counted `vanished`.
   - A response with another id: counted `failed`.
   - Otherwise `email::ingest` with `source = gmail`, the Gmail id, its thread id and its label ids. A message that doesn't parse counts as `failed` and is logged with its Gmail id only. A database error retries the page.
4. In one transaction, lock the `gmail_sync_state` row. If it's gone (`DELETE /data` ran), the run stops. Otherwise add the page's counts to the run's counts, then either enqueue the next page or finish the run: store `history_id`, set `last_finished_at` and clear `last_error`.

The job result is `{"mode", "page", "seen", "inserted", "updated", "oversized", "failed", "vanished", "next_job_id"}`, where `seen` is the sum of the other five counts.

## Pacing

Each page is its own job, and the next one may start `page_delay` (2 s) after the page that queued it, so a large backfill never monopolises the queue or Gmail's quota. Pages run through the normal worker with its concurrency limit.

## Single-flight

`gmail::sync::request` is the only way to start a run. In one transaction it takes the advisory lock `hashtext('waypoint.gmail_sync')`, then:

- Google not configured, or no account, or one that isn't `connected`: nothing is enqueued (the route answers 409).
- A `gmail_sync` job is `running`: it's returned with `reused: true`.
- One is `queued`: its `run_at` is pulled forward to now (so "Sync now" skips a rate-limit wait) and it's returned with `reused: true`.
- Otherwise a new start job is enqueued, `reused: false`.

Next pages are enqueued only by the running page, inside its state transaction, so at most one run exists at a time.

**Catch-up after downtime.** When the worker starts it requeues orphaned jobs, then requests a run (`trigger: startup`) when an account is connected and the last run finished more than `stale_after` (10 min) ago or never. An orphaned page that was just requeued already is a run, so it's reused. With Google disconnected or not configured, nothing is enqueued.

There's no periodic sync while the API stays up: runs happen at startup catch-up and on **Sync now**.

## Size cap

`max_message_bytes` is 5 MiB. The `messages.get` body is read in chunks and abandoned once it passes `max_message_bytes * 4 / 3 + 64 KiB` (the base64 size plus room for the JSON around it). A decoded `raw` longer than `max_message_bytes` is refused too. Either way the message counts as `oversized` and never reaches `email::ingest`.

## Errors and retries

| Gmail answers | What happens |
|---|---|
| 401 | The stored access token is marked expired, so the retry refreshes it |
| 429, or 403 with reason `rateLimitExceeded` / `userRateLimitExceeded` | Retry after `max(backoff, Retry-After)`, capped at the queue's `retry_cap`. Only this job waits: the queue isn't paused, since this isn't a Claude usage limit |
| 404 on `history.list` | The run becomes a resync |
| 404 on `messages.get` | Counted `vanished` |
| Any other non-2xx, a network error or a body that won't parse | Retry with the normal backoff |

Each `gmail_sync` job gets 5 attempts. Every page outcome that isn't a success writes `last_error` and `last_error_at`. After the last attempt the job fails, `last_error` keeps the reason and the run ends. The next run starts over from the stored `history_id`, or with a backfill.

Errors and logs never contain the access token, a URL with a token, a message body or a subject. JSON parse errors report only the error class and position.

## Settings

Code defaults on `gmail::Config`, held as `google::Config.gmail`. There are no env vars; tests override them.

| Setting | Default |
|---|---|
| `base_url` | `https://gmail.googleapis.com` |
| `backfill_days` | 30 |
| `max_messages` | 1000 |
| `page_size` | 100 |
| `page_delay` | 2 s |
| `max_message_bytes` | 5 MiB |
| `stale_after` | 10 min |

## Routes

`GET /gmail/sync` reports the status without calling Google or decrypting a token:

```sh
curl -s http://127.0.0.1:8000/gmail/sync
# {"activity":"idle","next_run_at":null,"mode":"backfill","run_started_at":"…","last_finished_at":"…",
#  "last_error":null,"last_error_at":null,
#  "run":{"seen":120,"inserted":118,"updated":0,"oversized":1,"failed":1,"vanished":0},"emails_stored":118}
```

- `activity`: `idle`, `queued` or `running`, from the `gmail_sync` jobs.
- `next_run_at`: the queued job's `run_at` while `activity` is `queued` (a rate-limit wait shows here), else `null`.
- `mode`: the current or most recent run's mode, `null` before the first run.
- `run`: that run's counts, all 0 before the first run.
- `emails_stored`: how many `emails` rows have `source = 'gmail'`.

`POST /gmail/sync` is **Sync now**. It answers 202 with `{"job": Job, "reused": bool}`, or 409 with the text "Connect Google before syncing Gmail." when Google isn't configured, connected, or needs reconnecting. A cross-site request or an HTML form post gets 403 and enqueues nothing.

The dashboard's Gmail card shows the same status, follows the `job.*` events of `gmail_sync` jobs, and has the **Sync now** button.

## The state table

`gmail_sync_state` holds at most one row:

| Column | Meaning |
|---|---|
| `email_address` | The account `history_id` belongs to |
| `history_id` | Where the next incremental run reads from. Set only when a run's last page finishes |
| `run_mode`, `run_started_at` | The current or most recent run |
| `run_seen`, `run_inserted`, `run_updated`, `run_oversized`, `run_failed`, `run_vanished` | That run's counts so far |
| `last_finished_at` | When a run last finished |
| `last_error`, `last_error_at` | Why the last page didn't succeed. Cleared when a run finishes |

The cursor (page token, page number, listed count) lives in the job payload, so a crash mid-run resumes from the requeued page. `last_error` is only ever written with an `UPDATE`, so a wiped state stays wiped.

## Delete all data

`DELETE /data` empties `gmail_sync_state` along with `emails`, `email_threads`, `jobs` and the Google account (see [google-oauth.md](google-oauth.md#delete-all-data)). Without an account no run continues: the next page stops at its token check, or at the state lock when the row is gone.

**Known narrow window.** If `DELETE /data` lands while a page is mid-fetch, the messages that page already ingested can outlive the wipe. The page's state lock then finds no row and ends the run. Deleting all data again clears them.
