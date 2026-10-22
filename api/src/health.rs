//! `GET /health`: reports whether the API can reach Postgres.

use std::time::Duration;

use rocket::State;
use rocket::http::Status;
use rocket::serde::json::Json;
use serde::Serialize;
use sqlx::PgPool;

const DB_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub db: &'static str,
}

#[rocket::get("/health")]
pub async fn health(pool: &State<PgPool>) -> (Status, Json<Health>) {
    let check = tokio::time::timeout(
        DB_CHECK_TIMEOUT,
        sqlx::query("SELECT 1").execute(pool.inner()),
    )
    .await;

    match check {
        Ok(Ok(_)) => (
            Status::Ok,
            Json(Health {
                status: "ok",
                db: "ok",
            }),
        ),
        Ok(Err(err)) => {
            rocket::warn!("health: database check failed: {err}");
            degraded()
        }
        Err(_) => {
            rocket::warn!("health: database check timed out after {DB_CHECK_TIMEOUT:?}");
            degraded()
        }
    }
}

fn degraded() -> (Status, Json<Health>) {
    (
        Status::ServiceUnavailable,
        Json(Health {
            status: "degraded",
            db: "unavailable",
        }),
    )
}
