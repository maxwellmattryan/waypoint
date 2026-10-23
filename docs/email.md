# Email store, normalizer and sender heuristics

Recruiter email becomes application updates in three steps: Gmail sync fetches raw messages (#65), this offline part stores them, and classify-and-extract has Claude read the ones worth reading (#66). This part makes no model calls and no network calls. It turns one raw RFC 822 message into a row with clean body text, groups rows into threads, and decides with cheap rules whether a message is worth a model call.

The code is `api/src/email/`:

| Module | Does |
|---|---|
| `mime.rs` | Parses raw bytes into `ParsedEmail` with [`mail-parser`](https://crates.io/crates/mail-parser) |
| `html.rs` | Converts an HTML part to text |
| `quote.rs` | Removes quoted replies and splits off the signature |
| `normalize.rs` | Picks the body part and runs the two above |
| `thread.rs` | Normalizes subjects and assigns threads |
| `heuristics.rs` | Scores a message: `candidate` or `skip`, with reasons |
| `store.rs` | `ingest`, `list` and `known_companies` |

## Schema

Migration `20261022000015_emails.sql` adds two tables. `DELETE /data` empties both.

**`email_threads`**: `provider_thread_id` (the Gmail thread id when known, unique), `subject` (the first message's subject with prefixes removed), `subject_key` (the same, lowercased, for the reply-subject fallback), `last_message_at` (the latest `sent_at` among its emails), `created_at`, `updated_at`.

**`emails`**: one row per message, unique on `(source, provider_message_id)`.

| Column | Holds |
|---|---|
| `source` | `fixture` or `gmail` |
| `provider_message_id` | The Gmail message id, or a fixture's file stem |
| `thread_id` | Its thread; deleting the thread deletes its emails |
| `rfc_message_id`, `in_reply_to`, `references_ids` | `Message-ID`, `In-Reply-To` and `References`, without angle brackets |
| `labels` | Gmail label ids |
| `from_address`, `from_name`, `to_addresses`, `cc_addresses` | Addresses lowercased, the display name decoded |
| `subject`, `sent_at` | The decoded subject, and `Date` (`NULL` when missing or unparseable) |
| `headers` | Every header as `[{"name", "value"}]`, in order, RFC 2047-decoded |
| `raw` | The original bytes, so a later version can normalize the message again |
| `body_text`, `body_source`, `signature` | The normalized body, which part it came from (`plain`, `html` or `none`) and the split-off signature |
| `verdict`, `verdict_reasons`, `ats` | `candidate` or `skip`, the JSON array of rule codes that fired, and the detected ATS (`greenhouse`, `lever`, `ashby`, `workday`) |
| `normalizer_version`, `heuristics_version` | `NORMALIZER_VERSION` and `HEURISTICS_VERSION` when the row was written, so rows from an older version can be found and re-scored |

CHECK constraints refuse an unknown `source`, `body_source`, `verdict` or `ats`, and a `verdict_reasons` that isn't an array.

## Ingest

`email::ingest(pool, NewEmail { source, provider_message_id, provider_thread_id, labels, raw }, &known)` returns `Ingested { id, thread_id, inserted, verdict }`. It parses, normalizes and scores outside the transaction. Then, in one transaction, it:

1. takes `pg_advisory_xact_lock(hashtext('waypoint.email_threads'))`, so concurrent ingests can't split a thread,
2. keeps the row's thread when the row already exists, otherwise assigns one (below),
3. upserts on `(source, provider_message_id)`: a second ingest updates every derived column, `raw`, `labels` and `updated_at` in place, keeping `id`, `thread_id` and `created_at`,
4. moves the thread's `last_message_at` to the greatest `sent_at`.

Bytes the parser can't read at all (no header) are `IngestError::Parse`, and nothing is written. `known_companies(pool)` loads the company keys for the `known_company` rule; load it once per import. `list(pool, verdict, limit)` returns the newest emails first, optionally only one verdict.

## Normalizer

`mail-parser` handles MIME structure, base64 and quoted-printable, charsets (its `full_encoding` feature, through `encoding_rs`, covers ISO-8859-x, Windows-125x and the CJK encodings), RFC 2047 encoded words, addresses and dates. Waypoint picks the parts itself rather than use `mail-parser`'s HTML-to-text fallback:

- **Part choice.** The first inline `text/plain` body part when it isn't blank (`body_source = plain`), else the first `text/html` part converted to text (`html`), else nothing (`none`, `body_text = ''`). Attachments and other parts (PDF, `text/calendar`, images) are never read.
- **HTML to text.** `script`, `style` and `head` are dropped. `p`, `h1`–`h6` are separated by a blank line; `div`, `br`, `li`, `tr` and other blocks start a new line. List items get a `- ` prefix. A link to an `http(s)` URL whose text differs from the URL becomes `text <url>`. `&nbsp;` is a space, whitespace collapses, and at most one blank line is kept in a row. Before converting, Gmail's `div.gmail_quote` (unless it holds a forwarded message), `blockquote`, Outlook's `#divRplyFwdMsg`, and `#appendonsend` with everything after it are removed.
- **Line cleanup.** Line endings become `\n` and trailing whitespace is trimmed on each line.
- **Quoted replies.** The text is cut at the first of: a Gmail "On … wrote:" line (also wrapped over two lines); an Outlook block (a `From:` line followed within 4 lines by `Sent:` or `Date:` and then `To:` or `Subject:`, with the line of underscores above it if there is one); "-----Original Message-----". Remaining lines starting with `>` are removed.
- **Forwards are kept.** "---------- Forwarded message ---------" and "Begin forwarded message:" aren't cut, and no reply marker after them counts: the forwarded text is often the recruiter's only message.
- **Signature.** A `-- ` line (or `--` once trailing whitespace is trimmed) and everything after it, up to a forward marker, move to `signature`. Without one, trailing "Sent from my …" and "Get Outlook for …" lines do.
- **Fallback.** If stripping leaves the body blank, the unstripped text is stored.

## Threads

A message that isn't stored yet joins a thread by, in order:

1. **Gmail thread id.** The thread whose `provider_thread_id` is the message's.
2. **Header links.** The thread of an existing email whose `rfc_message_id` is in this message's `In-Reply-To` or `References`, or whose `references_ids` contains this message's `Message-ID`. The second case groups a parent that arrives after its reply.
3. **Reply subject.** Only for a message that looks like a reply (its subject had a prefix, or it has `In-Reply-To` or `References`) and has a `sent_at`: the most recent thread with the same `subject_key` that shares a participant address (from, to or cc) with it and whose `last_message_at` is within 90 days of its `sent_at`.

Otherwise it starts a new thread, with the Gmail thread id if it has one. A thread found by headers or subject that has no Gmail thread id yet takes the message's. Two "Thank you for applying" emails from the same ATS address aren't replies, so they stay in separate threads.

`normalize_subject` removes repeated leading `Re`, `Fw`, `Fwd`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RI` and `TR` prefixes in any case (with an optional `[n]` or `(n)` count) and `[External]`/`[EXT]` tags, collapses whitespace, and reports whether a reply prefix was removed. The key is the result lowercased.

## Heuristics

The rules read the sender fields and headers, and the quote-stripped body **with** its signature, because recruiter titles live in signatures. A domain matches when it equals the listed domain or ends with `.` and it. Every rule that fires is recorded, in this order:

| Code | Fires when |
|---|---|
| `ats_sender:<ats>` | The domain of From, Sender, Reply-To or Return-Path is greenhouse (`greenhouse.io`, `greenhouse-mail.io`), lever (`lever.co`), ashby (`ashbyhq.com`) or workday (`myworkday.com`, `workday.com`, `myworkdayjobs.com`). Sets `ats`. |
| `ats_link:<ats>` | The body contains `boards.greenhouse.io`, `job-boards.greenhouse.io`, `jobs.lever.co`, `jobs.ashbyhq.com` or `myworkdayjobs.com`. Sets `ats` if `ats_sender` didn't. |
| `known_company` | A label of the From domain (TLD excluded) or the From display name equals a known company key: an `applications.company` lowercased with non-alphanumerics removed, at least 3 characters long. |
| `recruiter_title` | The From display name or the body contains `recruiter`, `recruiting`, `talent acquisition`, `talent partner`, `sourcer` or `hiring manager` (any case). |
| `recruiting_keywords` | The subject contains, or the body contains at least 2 distinct items of: `your application`, `application for`, `thank you for applying`, `thanks for applying`, `interview`, `phone screen`, `availability`, `next steps`, `offer`, `unfortunately`, `move forward`, `moving forward`, `the role`, `the position`. |
| `job_alert_digest` | The From domain is `linkedin.com`, `indeed.com`, `glassdoor.com`, `ziprecruiter.com`, `monster.com` or `wellfound.com` and its local part contains `alert`, `jobs-noreply` or `digest`; or the subject contains `job alert`, `jobs for you`, `new jobs`, `recommended jobs` or `jobs you may be interested in`. |
| `newsletter` | The subject or From display name contains `newsletter`, `digest` or `weekly roundup`. |
| `list_unsubscribe` | A `List-Unsubscribe` header is present. |
| `list_id` | A `List-Id` header is present. |
| `precedence_bulk` | `Precedence` is `bulk`, `list` or `junk`. |
| `auto_reply` | `Auto-Submitted` is present and not `no`, or `X-Autoreply` or `X-Autorespond` is present. |

The decision takes the first match:

1. `job_alert_digest` → skip.
2. `ats_sender` or `known_company` → candidate.
3. `auto_reply` → skip.
4. Any of `newsletter`, `list_unsubscribe`, `list_id`, `precedence_bulk` → candidate only if `recruiter_title` and `recruiting_keywords` both fired, else skip.
5. `ats_link`, `recruiter_title` or `recruiting_keywords` → candidate.
6. Otherwise skip, with `no_signal` appended to the reasons.

A skip only means "no model call now". Rows keep their `heuristics_version`, so #66 can re-score them.

## Fixture corpus and dev route

`api/fixtures/emails/` holds 20 synthetic `.eml` files written for Waypoint, with no real person's mail: companies and people use `.example` domains, and the real ATS and job-board domains appear only as senders and links. `manifest.json` lists what each file must produce (verdict, `ats`, reason codes, strings the body must and must not contain, and a thread label); its [README](../api/fixtures/emails/README.md) explains the fields.

In debug builds, `POST /dev/emails/import-fixtures` ingests every `*.eml` there in file-name order, with `source = 'fixture'` and the file stem as `provider_message_id`. Like every `/dev` route it needs a local `Host` and `X-Waypoint-Dev: 1`, and refuses a request a browser sends from another site, so an HTML form can't trigger it. It answers:

```json
{"files": 20, "inserted": 20, "updated": 0, "candidates": 15, "skipped": 5, "threads": 18, "failed": []}
```

`threads` is the number of distinct threads the imported emails belong to. A file that fails to parse is listed in `failed` as `{"file", "error"}` and the rest still import; a database error answers 500. Running it again gives `inserted: 0`, `updated: 20` and no new rows.

```sh
curl -s -X POST -H 'X-Waypoint-Dev: 1' http://127.0.0.1:8000/dev/emails/import-fixtures
```

`api/tests/emails.rs` imports the corpus and checks every manifest expectation. **A pass rate on this corpus is a fixture result, not a real-inbox result:** the files were written to exercise each rule, so they say nothing about how often the heuristics are right on real mail.
