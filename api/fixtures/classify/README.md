# Synthetic eval-only emails

These `.eml` files are **synthetic**, like the ones in [`../emails/`](../emails/README.md): written by hand, with `.example` domains, and taken from no one's mailbox. The `classify_email` and `match_email` eval sets (`api/evals/`) read them. They are kept out of `../emails/` so the dev fixture import and its manifest don't change. See [docs/evals.md](../../../docs/evals.md), and [Labelling emails](../../../docs/evals.md#labelling-emails) for the format and how to label your own redacted emails in the gitignored `../private/`.

People and companies use `.example` domains, and the user is `Alex Doe <me@home.example>`. The real ATS and job-board domains (`greenhouse-mail.io`, `greenhouse.io`, `lever.co`, `ashbyhq.com`, `myworkday.com`, `linkedin.com`) appear only as senders and link hosts, and `mail.gmail.com` only in Gmail-style `Message-ID`s. Every file scores `candidate` under the sender heuristics with no known companies, as a real email reaching the classifier would, and `classify_email_dataset_parses` checks it.

## Coverage

The class is the file's label in `api/evals/classify_email.json`. The categories are the messy-input shapes each file exercises:

| Category | Meaning |
|---|---|
| `multipart-alt` | `multipart/alternative` with text and HTML parts |
| `html-only` | only a `text/html` part |
| `qp-base64` | a quoted-printable or base64 body with non-ASCII characters |
| `rfc2047` | an encoded-word Subject or From (`B` or `Q`) |
| `quoted-reply-gmail` | an `On … wrote:` quoted tail |
| `quoted-reply-outlook` | a `-----Original Message-----` or `From:`/`Sent:` block |
| `thread` | a reply whose case seeds its predecessor first |
| `forward` | a forwarded recruiter message |
| `ats-greenhouse`, `ats-lever`, `ats-ashby`, `ats-workday` | sent from that ATS |
| `tz-us-dst` | US zone abbreviations on both sides of the 1 Nov 2026 change |
| `tz-eu` | a European zone across the 25 Oct 2026 change, or BST/CET |
| `tz-offset` | an explicit numeric offset |
| `tz-none` | a time with no zone stated |
| `ambiguous-company` | an agency recruiter, a company only in an ATS email's body, or a company that differs from the sender domain |
| `own-reply` | the user's own message |
| `ics` | a `text/calendar` invite part |

| File | Class | Categories |
|---|---|---|
| `offer.eml` | offer | — |
| `availability-pt.eml` | availability_request | `tz-us-dst` |
| `ashby-rejection-html-only.eml` | rejection | `ats-ashby`, `html-only` |
| `greenhouse-rejection-position-filled.eml` | rejection | `ats-greenhouse`, `multipart-alt` |
| `lever-rejection-qp.eml` | rejection | `ats-lever`, `qp-base64` |
| `workday-rejection-base64.eml` | rejection | `ats-workday`, `qp-base64` |
| `rejection-after-onsite-thread.eml` | rejection | `thread`, `quoted-reply-gmail` |
| `rejection-rfc2047-q.eml` | rejection | `rfc2047` (Q From and Subject) |
| `rejection-outlook-reply.eml` | rejection | `quoted-reply-outlook` |
| `agency-rejection-client.eml` | rejection | `ambiguous-company` |
| `rejection-rfc2047-b.eml` | rejection | `rfc2047` (B From and Subject) |
| `gringotts-onsite-invite.eml` | interview_invite | `ics` |
| `lever-invite-bst.eml` | interview_invite | `ats-lever`, `tz-eu` |
| `greenhouse-invite-ics.eml` | interview_invite | `ats-greenhouse`, `ics` |
| `invite-offset-ist.eml` | interview_invite | `tz-offset` |
| `invite-html-only-cet.eml` | interview_invite | `html-only`, `tz-eu` |
| `ashby-invite-multipart.eml` | interview_invite | `ats-ashby`, `multipart-alt` |
| `forward-invite-from-hiring-manager.eml` | interview_invite | `forward`, `tz-none` |
| `workday-invite-offset.eml` | interview_invite | `ats-workday`, `tz-offset` |
| `availability-et-dst.eml` | availability_request | `tz-us-dst` |
| `availability-ct-dst-thread.eml` | availability_request | `thread`, `quoted-reply-gmail`, `tz-us-dst` |
| `availability-eu-change.eml` | availability_request | `tz-eu` |
| `availability-uk-time.eml` | availability_request | `multipart-alt`, `tz-eu` |
| `availability-no-zone-outlook.eml` | availability_request | `quoted-reply-outlook`, `tz-none` |
| `availability-offset-base64.eml` | availability_request | `qp-base64`, `rfc2047` (B From), `tz-offset` |
| `lever-availability-request.eml` | availability_request | `ats-lever` |
| `greenhouse-availability-company-in-body.eml` | availability_request | `ats-greenhouse`, `ambiguous-company` |
| `offer-verbal-follow-up.eml` | offer | `multipart-alt` |
| `offer-written-thread.eml` | offer | `thread`, `quoted-reply-outlook` |
| `offer-letter-html-only.eml` | offer | `html-only` |
| `greenhouse-offer-deadline.eml` | offer | `ats-greenhouse` |
| `offer-agency.eml` | offer | `ambiguous-company` |
| `ashby-offer-deadline.eml` | offer | `ats-ashby` |
| `own-reply-interested.eml` | other | `own-reply`, `quoted-reply-gmail` |
| `own-reply-decline.eml` | other | `own-reply`, `quoted-reply-outlook` |
| `own-reply-confirm-time.eml` | other | `own-reply`, `thread`, `quoted-reply-gmail` |
| `availability-acknowledged.eml` | other | `thread` |
| `lever-application-received.eml` | other | `ats-lever` |
| `workday-application-received-multipart.eml` | other | `ats-workday`, `multipart-alt` |
| `position-on-hold-qp.eml` | other | `qp-base64` |
| `linkedin-cold-outreach.eml` | other | `ambiguous-company` |
| `take-home-assignment-q.eml` | other | `rfc2047` (Q Subject) |
| `forward-friend-referral.eml` | other | `forward` |
| `agency-outreach-unnamed-client.eml` | other | `ambiguous-company` |
| `greenhouse-application-received-html.eml` | other | `ats-greenhouse`, `html-only` |
