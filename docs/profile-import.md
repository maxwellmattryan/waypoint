# Profile import

Waypoint fills the candidate profile (the `position`, `skill` and `education` facts on the **Memory** page) from two sources you already have. Each import is an explicit click. The code lives in `api/src/profile/`.

| Source | Where | Route | Stored as |
|---|---|---|---|
| LinkedIn data export | **Import LinkedIn** on the Memory page | `POST /profile/linkedin?filename=<name>`, body the raw `.zip` (`Content-Type: application/zip`) | **confirmed**: it is your own structured data |
| Resume version | **Add to profile** on a resume's page | `POST /resumes/<id>/profile` | **proposed**: Claude read it, so you confirm each one on the Memory page |

## LinkedIn export

Get the archive from LinkedIn under Settings → Data privacy → Get a copy of your data. Waypoint reads three files from it and ignores every other entry:

| File | Required columns | Optional columns |
|---|---|---|
| `Positions.csv` | Company Name, Title | Location, Started On, Finished On, Description |
| `Skills.csv` | Name | |
| `Education.csv` | School Name | Degree Name, Start Date, End Date, Notes |

The files are found by name at any depth and in any case; the first match of each wins. The header row is the first row that has every required column (compared case-insensitively), so preamble lines before it, a UTF-8 BOM, extra columns and any column order are fine. An archive with only some of the files imports those. A row missing a required value counts as `skipped`.

The archive is read in memory and never extracted to disk. Limits: the archive is at most 20 MiB (larger is a 413), and each CSV at most 5 MiB once inflated, whatever its header claims (larger is a 422 naming the file). A body that isn't a zip, or a zip with none of the three files, is a 422 on `file`. Everything in one archive is stored in one transaction, so an import applies every row or none.

Each memory's `source_kind` is `linkedin`, `source_ref` is the archive's SHA-256 and `source_label` is the uploaded file name. The route answers `{added, already_known, skipped}`.

## Resume version

The route queues a `resume_profile` job (payload `{resume_version_id, sha256}`) and answers 202 `{job_id, reused}`. The job sends the version's text, cut to 30,000 characters and wrapped in a `<resume>` delimiter the text can't close, to one structured `claude -p` call (`purpose: profile.resume`, model sonnet) with no tools. The answer is checked against the schema in Rust; one that doesn't match fails or retries like any other Claude error, and stores nothing. At most 100 positions, skills and education entries each are kept; the rest count as `skipped`. The job's `result` is the same summary. The job runs on even if you leave the page; only the toast is lost.

Each memory's `source_kind` is `resume`, `source_ref` is the version id and `source_label` is the version name, so the Memory page links it back to the resume.

A version with no extracted text is a 422 on `text`. An unknown id is a 404.

**Reuse.** When an earlier `resume_profile` job for the same PDF bytes (the same `sha256`, so also the same PDF uploaded as another version) is queued, running or succeeded, the route returns that job with `reused: true` and makes no model call. A failed one doesn't block a new import.

## Content

`–` is an en dash. Values are trimmed and dates kept as written.

| Topic | Content | Tags |
|---|---|---|
| position | `{title} at {company}` plus ` · {location}`; then `{started} – {finished or "present"}`, or `Until {finished}` when only that is known; then a blank line and the description | the company |
| skill | `{name}` | none |
| education | `{degree} at {school}`, or `{school}`; then `{started} – {finished}`, or whichever is known; then a blank line and the notes | the school |

Content over 5,000 characters is cut to 4,999 plus `…`. A tag over 50 characters is dropped.

## Duplicates

Every imported memory has a source key built from what it is rather than how it is worded, with each part trimmed, lowercased and whitespace collapsed:

- position: `position:<company>|<title>`
- skill: `skill:<name>`
- education: `education:<school>|<degree>`

A memory whose key is already stored is not inserted and counts as `already_known`. That holds within one import, across re-imports and across sources, so a LinkedIn skill "Rust" and a resume skill "rust" are one memory, and whichever came first wins. Re-importing never changes an existing memory: your edits, confirmations and tags survive. Re-importing the same archive adds nothing, and a newer one adds only rows with new keys.

Memories you add by hand have no key and never conflict. Deleting an imported memory removes its key, so a later import that contains it adds it again. Near-duplicates ("Google" and "Google LLC") are separate memories, and a newer import doesn't update an older memory (a changed end date, say).
