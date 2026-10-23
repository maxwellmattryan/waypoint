//! The email store: schema constraints, threading through `email::ingest`, and the
//! `POST /dev/emails/import-fixtures` route over `api/fixtures/emails/`.

mod common;

use sqlx::PgPool;

/// Inserts one `emails` row with `column` set to `value` and every other column valid.
async fn insert_email_with(pool: &PgPool, column: &str, value: &str) -> Result<(), sqlx::Error> {
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ('s', 's') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert thread");
    let mut values = [
        ("source", "fixture"),
        ("verdict", "skip"),
        ("ats", "greenhouse"),
        ("body_source", "plain"),
    ];
    for (name, v) in values.iter_mut() {
        if *name == column {
            *v = value;
        }
    }
    sqlx::query(
        "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, body_text, \
         body_source, verdict, verdict_reasons, ats, normalizer_version, heuristics_version) \
         VALUES ($1, gen_random_uuid()::text, $2, '[]', '\\x00', '', $3, $4, '[]', $5, 1, 1)",
    )
    .bind(values[0].1)
    .bind(thread)
    .bind(values[3].1)
    .bind(values[1].1)
    .bind(values[2].1)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test]
async fn check_constraints_refuse_unknown_values(pool: PgPool) {
    insert_email_with(&pool, "source", "gmail")
        .await
        .expect("a valid row is accepted");

    for column in ["source", "verdict", "ats", "body_source"] {
        let err = insert_email_with(&pool, column, "unknown")
            .await
            .expect_err(column);
        let code = err
            .as_database_error()
            .and_then(|db| db.code())
            .map(|c| c.to_string());
        assert_eq!(code.as_deref(), Some("23514"), "{column}: {err}");
    }

    let reasons = sqlx::query(
        "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, body_text, \
         body_source, verdict, verdict_reasons, normalizer_version, heuristics_version) \
         SELECT 'fixture', 'obj', id, '[]', '\\x00', '', 'none', 'skip', '{}', 1, 1 \
         FROM email_threads LIMIT 1",
    )
    .execute(&pool)
    .await
    .expect_err("verdict_reasons must be an array");
    assert_eq!(
        reasons
            .as_database_error()
            .and_then(|db| db.code())
            .as_deref(),
        Some("23514")
    );

    let duplicate = "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, \
                     body_text, body_source, verdict, verdict_reasons, normalizer_version, \
                     heuristics_version) \
                     SELECT 'fixture', 'same', id, '[]', '\\x00', '', 'none', 'skip', '[]', 1, 1 \
                     FROM email_threads LIMIT 1";
    sqlx::query(duplicate)
        .execute(&pool)
        .await
        .expect("first copy");
    let err = sqlx::query(duplicate)
        .execute(&pool)
        .await
        .expect_err("(source, provider_message_id) is unique");
    assert_eq!(
        err.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23505")
    );

    let thread = "INSERT INTO email_threads (provider_thread_id, subject, subject_key) \
                  VALUES ('t1', 's', 's')";
    sqlx::query(thread)
        .execute(&pool)
        .await
        .expect("first thread");
    let err = sqlx::query(thread)
        .execute(&pool)
        .await
        .expect_err("provider_thread_id is unique");
    assert_eq!(
        err.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23505")
    );
}
