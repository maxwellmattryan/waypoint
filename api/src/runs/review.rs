//! Review items: results a step wasn't confident enough to apply (see
//! [`super::StepContext::review_if_unsure`] and [`super::StepContext::review`]). They are
//! stored, listed and dismissed (each dismissal records a learning signal,
//! [`crate::memory_signals`]). Accepting one applies its result, per workflow: today the
//! `email.match` and `email.new_application` kinds ([`crate::workflows::match_email`]) and
//! the `score_fit.score` kind ([`crate::workflows::score_fit`]), whose score becomes the
//! posting's current one, the `plan_goal.plan` kind ([`crate::workflows::plan_goal`]),
//! whose plan becomes `ready`, and the `prep_packet.packet` kind
//! ([`crate::workflows::prep_packet`]), whose packet becomes `ready` as it is. Dismissing a
//! `score_fit.score` item also marks its score `dismissed`, a `plan_goal.plan` item its plan,
//! and a `prep_packet.packet` item its packet.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use super::approvals::DecisionError;
use crate::applications::{ApiError, JsonContent, ValidationErrors, internal, parsed, text};
use crate::events::{Event, EventBus};
use crate::host::{LocalHost, NotCrossSite};
use crate::workflows::{match_email, plan_goal, prep_packet, score_fit};
use crate::{goals, memory_signals};

/// Most review items one `GET /review-items` returns.
pub const MAX_ITEMS: i64 = 200;
/// Most characters in a dismissal's reason.
pub const MAX_REASON: usize = 1_000;

/// The `POST /review-items/{id}/dismiss` body: `{}`, or `{"reason": "…"}` to say why.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissReviewItem {
    /// Why the result was wrong; trimmed, with a blank one stored as `null`. At most 1000
    /// characters. Never logged.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The `POST /review-items/{id}/accept` body: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptReviewItem {}

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

/// Dismisses `pending` item `id`: marks it `dismissed` with `resolved_at` set, applies
/// nothing, and records a `dismissal` signal with `reason` (trimmed, never blank) in the
/// same transaction, which schedules the memory writer.
pub async fn dismiss(
    pool: &PgPool,
    bus: &EventBus,
    id: i64,
    reason: Option<String>,
) -> Result<ReviewItem, DecisionError> {
    let mut tx = pool.begin().await?;
    let dismissed: Option<ReviewItem> = sqlx::query_as(&format!(
        "UPDATE review_items SET status = 'dismissed', resolved_at = now() \
         WHERE id = $1 AND status = 'pending' RETURNING {ITEM_COLUMNS}"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(item) = dismissed {
        let mut goal = None;
        if item.kind == score_fit::REVIEW_KIND {
            score_fit::dismissed(&mut tx, &item).await?;
        } else if item.kind == plan_goal::REVIEW_KIND {
            goal = plan_goal::dismissed(&mut tx, &item).await?;
        } else if item.kind == prep_packet::REVIEW_KIND {
            prep_packet::dismissed(&mut tx, &item).await?;
        }
        let writer =
            memory_signals::record_dismissal(&mut tx, bus, &item, reason.as_deref()).await?;
        tx.commit().await?;
        if let Some(run) = writer {
            bus.publish(Event::RunUpdated { run });
        }
        if let Some(goal) = goal {
            goals::publish(pool, bus, goal).await;
        }
        return Ok(item);
    }
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM review_items WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    match exists {
        Some(_) => Err(DecisionError::Conflict(format!(
            "review item {id} is not pending"
        ))),
        None => Err(DecisionError::NotFound),
    }
}

/// Accepts `pending` item `id`: applies its result as the user's own change, in one
/// transaction that locks the item, then its application, and marks it `accepted` with
/// `resolved_at` set. Only the `email.match`, `email.new_application`, `score_fit.score`
/// and `prep_packet.packet` kinds can be accepted; a refusal changes nothing. For the email
/// kinds the email's interview times are proposed in the same transaction, and
/// `application.created` or `application.updated` and the queued `propose_interview` runs
/// are published after the commit. A `score_fit.score` item's score becomes `applied`, a
/// `prep_packet.packet` item's packet becomes `ready`, and the item's run is published
/// after the commit.
pub async fn accept(pool: &PgPool, bus: &EventBus, id: i64) -> Result<ReviewItem, DecisionError> {
    let mut tx = pool.begin().await?;
    let item: ReviewItem = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM review_items WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DecisionError::NotFound)?;
    if item.status != ReviewItemStatus::Pending {
        return Err(DecisionError::Conflict(format!(
            "review item {id} is not pending"
        )));
    }
    if item.kind == score_fit::REVIEW_KIND || item.kind == prep_packet::REVIEW_KIND {
        if item.kind == score_fit::REVIEW_KIND {
            score_fit::accept(&mut tx, &item).await?;
        } else {
            prep_packet::accept(&mut tx, &item).await?;
        }
        let item = mark_accepted(&mut tx, id).await?;
        tx.commit().await?;
        super::publish(pool, bus, item.run_id).await;
        return Ok(item);
    }
    if item.kind == plan_goal::REVIEW_KIND {
        let goal = plan_goal::accept(&mut tx, &item).await?;
        let item = mark_accepted(&mut tx, id).await?;
        tx.commit().await?;
        goals::publish(pool, bus, goal).await;
        return Ok(item);
    }
    if ![
        match_email::MATCH_REVIEW_KIND,
        match_email::NEW_APPLICATION_REVIEW_KIND,
    ]
    .contains(&item.kind.as_str())
    {
        return Err(DecisionError::Conflict(format!(
            "review items of kind {:?} can't be accepted",
            item.kind
        )));
    }
    let accepted = match_email::accept(&mut tx, bus, &item).await?;
    let item = mark_accepted(&mut tx, id).await?;
    tx.commit().await?;
    let application = accepted.application;
    bus.publish(if accepted.created {
        Event::ApplicationCreated { application }
    } else {
        Event::ApplicationUpdated { application }
    });
    for run in accepted.runs {
        bus.publish(Event::RunUpdated { run });
    }
    Ok(item)
}

/// Marks item `id` `accepted` with `resolved_at` set, inside `tx`.
async fn mark_accepted(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<ReviewItem, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE review_items SET status = 'accepted', resolved_at = now() WHERE id = $1 \
         RETURNING {ITEM_COLUMNS}"
    ))
    .bind(id)
    .fetch_one(&mut **tx)
    .await
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
    request_body = DismissReviewItem,
    responses(
        (status = 200, description = "The dismissed item, with `resolved_at` set", body = ReviewItem),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No review item has this id"),
        (status = 409, description = "The item is not `pending`"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`reason` is longer than 1000 characters", body = ValidationErrors),
        (status = 500, description = "The dismissal could not be stored"),
    )
)]
#[rocket::post("/review-items/<id>/dismiss", data = "<body>")]
pub async fn dismiss_route(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    body: Result<Json<DismissReviewItem>, json::Error<'_>>,
) -> Result<Json<ReviewItem>, ApiError> {
    let DismissReviewItem { reason } = parsed(body)?;
    let mut errors = Vec::new();
    let reason = text(&mut errors, "reason", reason.as_deref(), MAX_REASON);
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }
    dismiss(pool, bus, id, reason)
        .await
        .map(Json)
        .map_err(|err| err.into_api("dismiss a review item"))
}

/// `POST /review-items/{id}/accept`: applies an email, fit-score or packet review item's
/// result.
#[utoipa::path(
    post,
    path = "/review-items/{id}/accept",
    tag = "runs",
    operation_id = "accept_review_item",
    params(("id" = i64, Path, description = "The review item's id")),
    request_body = AcceptReviewItem,
    responses(
        (status = 200, description = "The accepted item, with `resolved_at` set. An `email.match` item's changes were made to its application (each event naming the email); an `email.new_application` item's application was created; a `score_fit.score` item's score became its posting's current score; a `prep_packet.packet` item's packet became `ready` as it is", body = ReviewItem),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No review item has this id"),
        (status = 409, description = "Nothing changed: the item is not `pending`, its kind can't be accepted, its application is gone or its status changed since, its score or packet is no longer waiting for review, or its payload doesn't parse"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The change could not be stored"),
    )
)]
#[rocket::post("/review-items/<id>/accept", data = "<body>")]
pub async fn accept_route(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    body: Result<Json<AcceptReviewItem>, json::Error<'_>>,
) -> Result<Json<ReviewItem>, ApiError> {
    let AcceptReviewItem {} = parsed(body)?;
    accept(pool, bus, id)
        .await
        .map(Json)
        .map_err(|err| err.into_api("accept a review item"))
}
