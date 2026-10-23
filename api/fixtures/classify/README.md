# Synthetic eval-only emails

These `.eml` files are **synthetic**, like the ones in [`../emails/`](../emails/README.md): written by hand, with `.example` domains, and taken from no one's mailbox. Only the `classify_email` eval set (`api/evals/classify_email.json`) reads them, so every class has a case. They are kept out of `../emails/` so the dev fixture import and its manifest don't change. See [docs/evals.md](../../../docs/evals.md).
