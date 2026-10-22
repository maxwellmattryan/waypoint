//! `GET /health`: reports whether the API can reach Postgres.

use std::time::Duration;

use rocket::State;
use rocket::http::Status;
use rocket::serde::json::Json;
use serde::Serialize;
use sqlx::PgPool;

use crate::host::LocalHost;

const DB_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// The `GET /health` body.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Health {
    /// `ok` or `degraded`.
    pub status: &'static str,
    /// `ok` or `unavailable`.
    pub db: &'static str,
}

/// Reports whether the API can reach Postgres.
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "The API and its database are reachable", body = Health),
        (status = 503, description = "The database check failed or timed out", body = Health),
        (status = 403, description = "The `Host` header is not a local address"),
    )
)]
#[rocket::get("/health")]
pub async fn health(_host: LocalHost, pool: &State<PgPool>) -> (Status, Json<Health>) {
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
