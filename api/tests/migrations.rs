use sqlx::PgPool;
use sqlx::migrate::MigrateError;

#[sqlx::test(migrations = false)]
async fn baseline_applies(pool: PgPool) {
    waypoint_api::MIGRATOR
        .run(&pool)
        .await
        .expect("migrations apply");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("count migrations");
    assert_eq!(count, 25);
}

#[sqlx::test(migrations = false)]
async fn mismatch_is_explained(pool: PgPool) {
    let migrator = &waypoint_api::MIGRATOR;
    migrator.run(&pool).await.expect("migrations apply");

    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = '\\x00'::bytea, description = 'resume versions' \
         WHERE version = 20261022000005",
    )
    .execute(&pool)
    .await
    .expect("tamper with the recorded migration");

    let err = migrator.run(&pool).await.expect_err("mismatch is detected");
    assert!(
        matches!(err, MigrateError::VersionMismatch(20261022000005)),
        "unexpected error: {err:?}"
    );

    let message = waypoint_api::migrate::explain(&pool, migrator, &err).await;
    for expected in [
        "20261022000005",
        "resume versions",
        "stages and events",
        "scripts/db-reset.sh",
    ] {
        assert!(
            message.contains(expected),
            "missing {expected:?} in {message}"
        );
    }
}
