# Synthetic eval-only emails

These `.eml` files are **synthetic**, like the ones in [`../emails/`](../emails/README.md): written by hand, with `.example` domains, and taken from no one's mailbox. The `classify_email` and `match_email` eval sets (`api/evals/`) read them. They are kept out of `../emails/` so the dev fixture import and its manifest don't change. See [docs/evals.md](../../../docs/evals.md), and [Labelling emails](../../../docs/evals.md#labelling-emails) for the format and how to label your own redacted emails in the gitignored `../private/`.
