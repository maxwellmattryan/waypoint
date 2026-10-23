//! Saving one board's fetch into `job_postings`.

use sqlx::{PgPool, Postgres, Transaction};

use super::Source;
use super::boards::Posting;

/// What [`save_board`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Saved {
    pub inserted: u64,
    pub updated: u64,
    pub removed: u64,
}

/// Takes the per-board advisory lock that `sourcing::request` also takes, so a board's
/// enqueue and its save never interleave.
pub(crate) async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    source: Source,
    board: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtext('waypoint.source_board:' || $1 || ':' || $2))",
    )
    .bind(source.as_str())
    .bind(board)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

const UPSERT: &str = "INSERT INTO job_postings (source, board, external_id, title, company, url, \
     apply_url, department, employment_type, location_raw, remote, city, region, country, \
     description, posted_at) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
     ON CONFLICT (source, external_id) DO UPDATE SET board = EXCLUDED.board, \
     title = EXCLUDED.title, company = EXCLUDED.company, url = EXCLUDED.url, \
     apply_url = EXCLUDED.apply_url, department = EXCLUDED.department, \
     employment_type = EXCLUDED.employment_type, location_raw = EXCLUDED.location_raw, \
     remote = EXCLUDED.remote, city = EXCLUDED.city, region = EXCLUDED.region, \
     country = EXCLUDED.country, description = EXCLUDED.description, \
     posted_at = EXCLUDED.posted_at, last_seen_at = now(), removed_at = NULL, \
     updated_at = now() \
     RETURNING (xmax = 0)";

/// In one transaction: upserts every posting on `(source, external_id)`, then marks as
/// removed the live postings on this board that the fetch didn't list. `now()` is the
/// transaction's start, so the rows upserted here all have `last_seen_at = now()` and are
/// left alone. An empty `postings` removes every live posting on the board. Rows on other
/// boards, or from other sources, are never touched.
pub async fn save_board(
    pool: &PgPool,
    source: Source,
    board: &str,
    postings: &[Posting],
) -> Result<Saved, sqlx::Error> {
    let mut tx = pool.begin().await?;
    lock(&mut tx, source, board).await?;
    let mut saved = Saved::default();
    for posting in postings {
        let location = &posting.location;
        let inserted: bool = sqlx::query_scalar(UPSERT)
            .bind(source.as_str())
            .bind(board)
            .bind(&posting.external_id)
            .bind(&posting.title)
            .bind(&posting.company)
            .bind(&posting.url)
            .bind(&posting.apply_url)
            .bind(&posting.department)
            .bind(&posting.employment_type)
            .bind(&posting.location_raw)
            .bind(location.remote)
            .bind(&location.city)
            .bind(&location.region)
            .bind(&location.country)
            .bind(&posting.description)
            .bind(posting.posted_at)
            .fetch_one(&mut *tx)
            .await?;
        if inserted {
            saved.inserted += 1;
        } else {
            saved.updated += 1;
        }
    }
    let removed = sqlx::query(
        "UPDATE job_postings SET removed_at = now(), updated_at = now() \
         WHERE source = $1 AND board = $2 AND removed_at IS NULL AND last_seen_at < now()",
    )
    .bind(source.as_str())
    .bind(board)
    .execute(&mut *tx)
    .await?;
    saved.removed = removed.rows_affected();
    tx.commit().await?;
    Ok(saved)
}
