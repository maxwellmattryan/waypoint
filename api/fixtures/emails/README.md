# Synthetic email fixtures

Every `.eml` file here is **synthetic**: written by hand for Waypoint's tests, not taken from anyone's mailbox. They contain no real person's mail. Companies and people use `.example` domains; the real applicant-tracking domains (`greenhouse-mail.io`, `hire.lever.co`, `ashbyhq.com`, `myworkday.com`, `workday.com`) and job boards (`linkedin.com`, `indeed.com`) appear only as sender and link domains, so the sender heuristics can recognise them.

`manifest.json` (`"synthetic": true`) lists one entry per file with what the importer must produce:

- `verdict`: `candidate` or `skip`.
- `ats`: the detected applicant tracking system, or `null`.
- `reasons_include`: rule codes that must be among `verdict_reasons`.
- `body_contains` / `body_excludes`: strings the normalized `body_text` must or must not contain.
- `thread`: a label. Emails with the same label must land in the same thread, and different labels in different threads.

`POST /dev/emails/import-fixtures` imports them in file-name order, and `api/tests/emails.rs` checks every expectation. The pass rate on this corpus says how the normalizer and heuristics do on these files, not on a real inbox. See [docs/email.md](../../../docs/email.md).
