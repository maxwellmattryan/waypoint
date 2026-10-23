//! Review items: results a step wasn't confident enough to apply (see
//! [`super::StepContext::review_if_unsure`]). They are stored, listed and dismissed;
//! accepting one (applying its result) comes later, per workflow.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use super::approvals::DecisionError;
use crate::applications::{ApiError, internal};
use crate::host::{LocalHost, NotCrossSite};

/// Most review items one `GET /review-items` returns.
pub const MAX_ITEMS: i64 = 200;

/// Where a review item is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ReviewItemStatus {
    /// Waiting for the user.
    Pending,
    Accepted,
    Dismissed,
}

/// One `review_items` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ReviewItem {
    pub id: i64,
    pub run_id: i64,
    pub run_step_id: i64,
    /// What the step was deciding, e.g. `tidy.note`.
    pub kind: String,
    /// The step's confidence, 0 to 1.
    pub confidence: f64,
    /// The result the step didn't apply.
    pub payload: Value,
    pub status: ReviewItemStatus,
    pub created_at: DateTime<Utc>,
    #[schema(required = true)]
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Stores a `pending` item for step `step_id`, or finds the one a previous attempt of the
/// step stored for the same kind and payload, and returns its id. `confidence` is clamped
/// to 0..=1, with NaN stored as 0.
pub(crate) async fn insert(
    pool: &PgPool,
    step_id: i64,
    kind: &str,
    confidence: f64,
    payload: &Value,
) -> Result<i64, sqlx::Error> {
    let confidence = if confidence.is_nan() {
        0.0
    } else {
        confidence.clamp(0.0, 1.0)
    };
    let inserted: Option<i64> = sqlx::query_scalar(
        "INSERT INTO review_items (run_id, run_step_id, kind, confidence, payload) \
         SELECT run_id, id, $2, $3, $4 FROM run_steps WHERE id = $1 \
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(step_id)
    .bind(kind)
    .bind(confidence)
    .bind(payload)
    .fetch_optional(pool)
    .await?;
    match inserted {
        Some(id) => Ok(id),
        None => {
            sqlx::query_scalar(
                "SELECT id FROM review_items WHERE run_step_id = $1 AND kind = $2 \
                 AND md5(payload::text) = md5($3::jsonb::text)",
            )
            .bind(step_id)
            .bind(kind)
            .bind(payload)
            .fetch_one(pool)
            .await
        }
    }
}

const ITEM_COLUMNS: &str =
    "id, run_id, run_step_id, kind, confidence, payload, status, created_at, resolved_at";

/// Up to [`MAX_ITEMS`] `pending` items, oldest first.
pub async fn list_pending(pool: &PgPool) -> Result<Vec<ReviewItem>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM review_items WHERE status = 'pending' ORDER BY id LIMIT $1"
    ))
    .bind(MAX_ITEMS)
    .fetch_all(pool)
    .await
}

/// Dismisses `pending` item `id`: marks it `dismissed` with `resolved_at` set and applies
/// nothing.
pub async fn dismiss(pool: &PgPool, id: i64) -> Result<ReviewItem, DecisionError> {
    let dismissed: Option<ReviewItem> = sqlx::query_as(&format!(
        "UPDATE review_items SET status = 'dismissed', resolved_at = now() \
         WHERE id = $1 AND status = 'pending' RETURNING {ITEM_COLUMNS}"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if let Some(item) = dismissed {
        return Ok(item);
    }
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM review_items WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    match exists {
        Some(_) => Err(DecisionError::Conflict(format!(
            "review item {id} is not pending"
        ))),
        None => Err(DecisionError::NotFound),
    }
}

/// `GET /review-items`: results waiting for the user's review.
#[utoipa::path(
    get,
    path = "/review-items",
    tag = "runs",
    operation_id = "list_review_items",
    responses(
        (status = 200, description = "Up to 200 `pending` items, `id` ascending (oldest first)", body = [ReviewItem]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The review items could not be read"),
    )
)]
#[rocket::get("/review-items")]
pub async fn list(
    _host: LocalHost,
    pool: &State<PgPool>,
) -> Result<Json<Vec<ReviewItem>>, ApiError> {
    list_pending(pool)
        .await
        .map(Json)
        .map_err(internal("list review items"))
}

/// `POST /review-items/{id}/dismiss`: clears an item from the queue without applying it.
#[utoipa::path(
    post,
    path = "/review-items/{id}/dismiss",
    tag = "runs",
    operation_id = "dismiss_review_item",
    params(("id" = i64, Path, description = "The review item's id")),
    responses(
        (status = 200, description = "The dismissed item, with `resolved_at` set", body = ReviewItem),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No review item has this id"),
        (status = 409, description = "The item is not `pending`"),
        (status = 500, description = "The dismissal could not be stored"),
    )
)]
#[rocket::post("/review-items/<id>/dismiss")]
pub async fn dismiss_route(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<ReviewItem>, ApiError> {
    dismiss(pool, id)
        .await
        .map(Json)
        .map_err(|err| err.into_api("dismiss a review item"))
}
