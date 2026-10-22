//! An application's interview stages (recruiter screen, onsite, ...): an ordered list the
//! user adds to, completes, reorders and removes from. Every route returns the whole list
//! in order, so the client never reconciles positions. Positions are always 0..n-1.
//!
//! Each write locks the application first ([`timeline::lock_application`]) and records
//! its event in the same transaction; a reorder records no event but still moves
//! `updated_at`. A stage is always looked up together with its application, so a stage of
//! another application is a 404.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};

use super::timeline::{self, EventKind};
use super::{
    ApiError, FieldError, JsonContent, MAX_SHORT, REQUIRED, ValidationErrors, internal, parsed,
    text,
};
use crate::host::{LocalHost, NotCrossSite};

/// Most stages one application can have.
pub const MAX_STAGES: i64 = 50;

const TOO_MANY: &str = "At most 50 stages";
const BAD_ORDER: &str = "Must list every stage of this application once";

/// One `application_stages` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Stage {
    pub id: i64,
    pub application_id: i64,
    pub name: String,
    /// 0 for the first stage, counting up without gaps.
    pub position: i32,
    /// When the stage was completed; null while it is open. Always serialized.
    #[schema(required = true)]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, application_id, name, position, completed_at, created_at";

/// The `POST /applications/{id}/stages` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct NewStage {
    /// Required; trimmed, at most 200 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub name: String,
}

/// The `PUT /applications/{id}/stages/{stage_id}` body.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct StageChange {
    /// `true` completes the stage, `false` reopens it.
    pub completed: bool,
}

/// The `PUT /applications/{id}/stage-order` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct StageOrder {
    /// Every stage id of the application, each once, in the new order.
    pub stage_ids: Vec<i64>,
}

fn invalid(field: &str, message: &str) -> ApiError {
    ApiError::Invalid(ValidationErrors {
        errors: vec![FieldError {
            field: field.into(),
            message: message.into(),
        }],
    })
}

async fn stages_of(conn: &mut PgConnection, id: i64) -> Result<Vec<Stage>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM application_stages WHERE application_id = $1 ORDER BY position"
    ))
    .bind(id)
    .fetch_all(conn)
    .await
    .map_err(internal("read stages"))
}

/// An application's stages, in order.
#[utoipa::path(
    get,
    path = "/applications/{id}/stages",
    tag = "applications",
    operation_id = "list_application_stages",
    params(("id" = i64, Path, description = "The application's id")),
    responses(
        (status = 200, description = "Every stage of the application, by `position`", body = [Stage]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No application has this id"),
        (status = 500, description = "The stages could not be read"),
    )
)]
#[rocket::get("/applications/<id>/stages")]
pub async fn list(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Vec<Stage>>, ApiError> {
    timeline::ensure_exists(pool, id).await?;
    let mut conn = pool.acquire().await.map_err(internal("read stages"))?;
    stages_of(&mut conn, id).await.map(Json)
}

/// Appends a stage and records `stage_added`.
#[utoipa::path(
    post,
    path = "/applications/{id}/stages",
    tag = "applications",
    operation_id = "add_application_stage",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = NewStage,
    responses(
        (status = 201, description = "Every stage of the application, by `position`, the new one last", body = [Stage]),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`name` is blank or too long, or the application already has 50 stages", body = ValidationErrors),
        (status = 500, description = "The stage could not be stored"),
    )
)]
#[rocket::post("/applications/<id>/stages", data = "<body>")]
pub async fn add(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<NewStage>, json::Error<'_>>,
) -> Result<(Status, Json<Vec<Stage>>), ApiError> {
    let NewStage { name } = parsed(body)?;
    let mut errors = Vec::new();
    let name = text(&mut errors, "name", Some(&name), MAX_SHORT);
    if name.is_none() {
        errors.push(FieldError {
            field: "name".into(),
            message: REQUIRED.into(),
        });
    }
    let Some(name) = name.filter(|_| errors.is_empty()) else {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    timeline::lock_application(&mut tx, id).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_stages WHERE application_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal("count stages"))?;
    if count >= MAX_STAGES {
        return Err(invalid("name", TOO_MANY));
    }
    sqlx::query(
        "INSERT INTO application_stages (application_id, name, position) VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(&name)
    .bind(count as i32)
    .execute(&mut *tx)
    .await
    .map_err(internal("store a stage"))?;
    timeline::record(
        &mut tx,
        id,
        EventKind::StageAdded,
        None,
        None,
        Some(&name),
        None,
    )
    .await
    .map_err(internal("record a stage"))?;
    timeline::touch(&mut tx, id).await?;
    let stages = stages_of(&mut tx, id).await?;
    tx.commit().await.map_err(internal("store a stage"))?;
    Ok((Status::Created, Json(stages)))
}

/// Completes or reopens a stage, recording `stage_completed` or `stage_reopened` when that
/// changes anything.
#[utoipa::path(
    put,
    path = "/applications/{id}/stages/{stage_id}",
    tag = "applications",
    operation_id = "update_application_stage",
    params(
        ("id" = i64, Path, description = "The application's id"),
        ("stage_id" = i64, Path, description = "The stage's id"),
    ),
    request_body = StageChange,
    responses(
        (status = 200, description = "Every stage of the application, by `position`", body = [Stage]),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id, or it has no stage with this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The stage could not be stored"),
    )
)]
#[rocket::put("/applications/<id>/stages/<stage_id>", data = "<body>")]
pub async fn update(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    stage_id: i64,
    pool: &State<PgPool>,
    body: Result<Json<StageChange>, json::Error<'_>>,
) -> Result<Json<Vec<Stage>>, ApiError> {
    let StageChange { completed } = parsed(body)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    timeline::lock_application(&mut tx, id).await?;
    let (name, was_completed): (String, bool) = sqlx::query_as(
        "SELECT name, completed_at IS NOT NULL FROM application_stages \
         WHERE id = $1 AND application_id = $2 FOR UPDATE",
    )
    .bind(stage_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal("read a stage"))?
    .ok_or(ApiError::NotFound)?;
    if completed != was_completed {
        sqlx::query(
            "UPDATE application_stages \
             SET completed_at = CASE WHEN $2 THEN now() ELSE NULL END WHERE id = $1",
        )
        .bind(stage_id)
        .bind(completed)
        .execute(&mut *tx)
        .await
        .map_err(internal("update a stage"))?;
        let kind = if completed {
            EventKind::StageCompleted
        } else {
            EventKind::StageReopened
        };
        timeline::record(&mut tx, id, kind, None, None, Some(&name), None)
            .await
            .map_err(internal("record a stage change"))?;
        timeline::touch(&mut tx, id).await?;
    }
    let stages = stages_of(&mut tx, id).await?;
    tx.commit().await.map_err(internal("update a stage"))?;
    Ok(Json(stages))
}

/// Puts an application's stages in a new order. Records no event.
#[utoipa::path(
    put,
    path = "/applications/{id}/stage-order",
    tag = "applications",
    operation_id = "reorder_application_stages",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = StageOrder,
    responses(
        (status = 200, description = "Every stage of the application, in the new order", body = [Stage]),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`stage_ids` is not exactly the application's stage ids, each once", body = ValidationErrors),
        (status = 500, description = "The order could not be stored"),
    )
)]
#[rocket::put("/applications/<id>/stage-order", data = "<body>")]
pub async fn reorder(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<StageOrder>, json::Error<'_>>,
) -> Result<Json<Vec<Stage>>, ApiError> {
    let StageOrder { stage_ids } = parsed(body)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    timeline::lock_application(&mut tx, id).await?;
    let mut current: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM application_stages WHERE application_id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal("read stages"))?;
    let mut wanted = stage_ids.clone();
    current.sort_unstable();
    wanted.sort_unstable();
    if current != wanted {
        return Err(invalid("stage_ids", BAD_ORDER));
    }
    // The unique position is deferred to commit, so this one statement may swap positions.
    sqlx::query(
        "UPDATE application_stages AS s SET position = (o.ord - 1)::integer \
         FROM unnest($2::bigint[]) WITH ORDINALITY AS o(id, ord) \
         WHERE s.id = o.id AND s.application_id = $1",
    )
    .bind(id)
    .bind(&stage_ids)
    .execute(&mut *tx)
    .await
    .map_err(internal("reorder stages"))?;
    timeline::touch(&mut tx, id).await?;
    let stages = stages_of(&mut tx, id).await?;
    tx.commit().await.map_err(internal("reorder stages"))?;
    Ok(Json(stages))
}

/// Removes a stage, closes the gap in positions and records `stage_removed`.
#[utoipa::path(
    delete,
    path = "/applications/{id}/stages/{stage_id}",
    tag = "applications",
    operation_id = "remove_application_stage",
    params(
        ("id" = i64, Path, description = "The application's id"),
        ("stage_id" = i64, Path, description = "The stage's id"),
    ),
    responses(
        (status = 200, description = "The remaining stages of the application, by `position`", body = [Stage]),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id, or it has no stage with this id"),
        (status = 500, description = "The stage could not be removed"),
    )
)]
#[rocket::delete("/applications/<id>/stages/<stage_id>")]
pub async fn remove(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    stage_id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Vec<Stage>>, ApiError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    timeline::lock_application(&mut tx, id).await?;
    let (name, position): (String, i32) = sqlx::query_as(
        "DELETE FROM application_stages WHERE id = $1 AND application_id = $2 \
         RETURNING name, position",
    )
    .bind(stage_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal("remove a stage"))?
    .ok_or(ApiError::NotFound)?;
    sqlx::query(
        "UPDATE application_stages SET position = position - 1 \
         WHERE application_id = $1 AND position > $2",
    )
    .bind(id)
    .bind(position)
    .execute(&mut *tx)
    .await
    .map_err(internal("renumber stages"))?;
    timeline::record(
        &mut tx,
        id,
        EventKind::StageRemoved,
        None,
        None,
        Some(&name),
        None,
    )
    .await
    .map_err(internal("record a stage removal"))?;
    timeline::touch(&mut tx, id).await?;
    let stages = stages_of(&mut tx, id).await?;
    tx.commit().await.map_err(internal("remove a stage"))?;
    Ok(Json(stages))
}
