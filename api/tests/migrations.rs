use sqlx::PgPool;

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
    assert_eq!(count, 3);
}
