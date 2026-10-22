//! The `claude_calls` call log: one row per call, successful or not.

use sqlx::PgPool;

use super::Usage;

/// One call's log row.
pub(crate) struct Entry<'a> {
    pub purpose: &'a str,
    pub model: Option<&'a str>,
    pub usage: &'a Usage,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
    /// `ok` or a [`super::ClaudeError::kind`].
    pub status: &'a str,
    pub error_message: Option<&'a str>,
}

/// Inserts the row and returns its id.
pub(crate) async fn record(pool: &PgPool, entry: &Entry<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO claude_calls (purpose, model, status, input_tokens, output_tokens, \
         cache_creation_input_tokens, cache_read_input_tokens, duration_ms, cost_usd, \
         error_message) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(entry.purpose)
    .bind(entry.model)
    .bind(entry.status)
    .bind(entry.usage.input_tokens)
    .bind(entry.usage.output_tokens)
    .bind(entry.usage.cache_creation_input_tokens)
    .bind(entry.usage.cache_read_input_tokens)
    .bind(i64::try_from(entry.duration_ms).unwrap_or(i64::MAX))
    .bind(entry.cost_usd)
    .bind(entry.error_message)
    .fetch_one(pool)
    .await
}
