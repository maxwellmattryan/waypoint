//! Learning signals: the raw evidence of what the user corrects and turns down, for the
//! memory writer to learn from. One `memory_signals` row per decision on something an agent
//! drafted: an approval (with the diff from the drafted to the approved input), a rejection
//! or a review-queue dismissal (with the user's optional reason).
//!
//! Signals are written only by the `record_*` helpers, each on the decision's own
//! transaction, so a stored decision always has its signal and a failed one leaves none.
//! Each helper also schedules a memory writer run in that transaction
//! ([`crate::workflows::memory_writer::schedule`], debounced), which claims the unprocessed
//! signals and marks them processed (see docs/memory-writer.md). The only route lists them;
//! nothing over HTTP writes, edits or deletes one.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::applications::{ApiError, ValidationErrors, internal};
use crate::events::EventBus;
use crate::host::LocalHost;
use crate::memories::{filter, text_enum};
use crate::runs::Run;
use crate::runs::approvals::PendingAction;
use crate::runs::review::ReviewItem;
use crate::workflows::memory_writer::{self, Trigger};

/// What decision a signal records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// The user approved a pending action, possibly after editing its input.
    Approval,
    /// The user rejected a pending action.
    Rejection,
    /// The user dismissed a review item.
    Dismissal,
}

text_enum!(SignalKind, "signal kind", [
    Approval => "approval",
    Rejection => "rejection",
    Dismissal => "dismissal",
]);

/// How one value changed between the drafted and the approved input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalOp {
    /// The key is only in the approved input; `before` is `null`.
    Add,
    /// The key is only in the drafted input; `after` is `null`.
    Remove,
    /// The value at `path` differs; both are given.
    Replace,
}

/// One entry of an approval's diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SignalChange {
    pub op: SignalOp,
    /// An RFC 6901 JSON Pointer: `/message`, `/a~1b`, or `""` for the whole input.
    pub path: String,
    /// The drafted value; `null` for an `add`.
    #[schema(required = true)]
    pub before: Option<Value>,
    /// The approved value; `null` for a `remove`.
    #[schema(required = true)]
    pub after: Option<Value>,
}

/// One `memory_signals` row: the body of `GET /memory-signals`' entries.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct MemorySignal {
    pub id: i64,
    pub kind: SignalKind,
    pub run_id: i64,
    // The `Option` fields are always serialized (as `null` when empty), so they are
    // required in the schema.
    /// The decided pending action, for an approval or a rejection.
    #[schema(required = true)]
    pub pending_action_id: Option<i64>,
    /// The dismissed review item, for a dismissal.
    #[schema(required = true)]
    pub review_item_id: Option<i64>,
    /// The tool name, or the review item's kind.
    pub subject: String,
    /// The action's requested input, or the item's payload.
    pub drafted: Value,
    /// Approvals only: the approved input.
    #[schema(required = true)]
    pub approved: Option<Value>,
    /// Approvals only: the diff from `drafted` to `approved`; `[]` when unedited.
    #[sqlx(json(nullable))]
    #[schema(required = true)]
    pub changes: Option<Vec<SignalChange>>,
    /// Dismissals only: the user's reason, when they gave one.
    #[schema(required = true)]
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub(crate) const COLUMNS: &str = "id, kind, run_id, pending_action_id, review_item_id, subject, drafted, \
                       approved, changes, reason, created_at";

/// The changes that turn `drafted` into `approved`. Objects are compared key by key,
/// recursively, in lexicographic key order, and the entries come out in that order; every
/// other value (arrays included, and a change of type) is compared and replaced whole. An
/// explicit `null` counts as present.
pub fn diff(drafted: &Value, approved: &Value) -> Vec<SignalChange> {
    let mut changes = Vec::new();
    walk(&mut changes, String::new(), drafted, approved);
    changes
}

fn walk(changes: &mut Vec<SignalChange>, path: String, before: &Value, after: &Value) {
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
            for key in keys {
                let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (before.get(key), after.get(key)) {
                    (Some(b), Some(a)) => walk(changes, path, b, a),
                    (Some(b), None) => changes.push(SignalChange {
                        op: SignalOp::Remove,
                        path,
                        before: Some(b.clone()),
                        after: None,
                    }),
                    (None, Some(a)) => changes.push(SignalChange {
                        op: SignalOp::Add,
                        path,
                        before: None,
                        after: Some(a.clone()),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ if before != after => changes.push(SignalChange {
            op: SignalOp::Replace,
            path,
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
        _ => {}
    }
}

/// Schedules the memory writer after a signal insert. Returns the run it created, for the
/// caller to publish after its commit; `None` when a waiting run was reused.
async fn schedule_writer(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
) -> sqlx::Result<Option<Run>> {
    let (run, reused) = memory_writer::schedule(tx, bus, Trigger::Decision).await?;
    Ok((!reused).then_some(run))
}

/// Records the approval of `action`, as returned by the approving UPDATE, and schedules the
/// memory writer ([`schedule_writer`]).
pub(crate) async fn record_approval(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    action: &PendingAction,
) -> sqlx::Result<Option<Run>> {
    let approved = action.approved_input.as_ref().unwrap_or(&action.input);
    let changes = sqlx::types::Json(diff(&action.input, approved));
    sqlx::query(
        "INSERT INTO memory_signals \
         (kind, run_id, pending_action_id, subject, drafted, approved, changes) \
         VALUES ('approval', $1, $2, $3, $4, $5, $6)",
    )
    .bind(action.run_id)
    .bind(action.id)
    .bind(&action.tool)
    .bind(&action.input)
    .bind(approved)
    .bind(changes)
    .execute(&mut **tx)
    .await?;
    schedule_writer(tx, bus).await
}

/// Records the rejection of `action`, the one the user rejected, and schedules the memory
/// writer.
pub(crate) async fn record_rejection(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    action: &PendingAction,
) -> sqlx::Result<Option<Run>> {
    sqlx::query(
        "INSERT INTO memory_signals (kind, run_id, pending_action_id, subject, drafted) \
         VALUES ('rejection', $1, $2, $3, $4)",
    )
    .bind(action.run_id)
    .bind(action.id)
    .bind(&action.tool)
    .bind(&action.input)
    .execute(&mut **tx)
    .await?;
    schedule_writer(tx, bus).await
}

/// Records the dismissal of `item`, with the user's reason (already trimmed, never blank),
/// and schedules the memory writer.
pub(crate) async fn record_dismissal(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    item: &ReviewItem,
    reason: Option<&str>,
) -> sqlx::Result<Option<Run>> {
    sqlx::query(
        "INSERT INTO memory_signals (kind, run_id, review_item_id, subject, drafted, reason) \
         VALUES ('dismissal', $1, $2, $3, $4, $5)",
    )
    .bind(item.run_id)
    .bind(item.id)
    .bind(&item.kind)
    .bind(&item.payload)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    schedule_writer(tx, bus).await
}

/// Up to `limit` signals with `id < before` (any id when `None`), of `kind` when given,
/// newest first.
pub async fn list_signals(
    pool: &PgPool,
    kind: Option<SignalKind>,
    limit: i64,
    before: Option<i64>,
) -> sqlx::Result<Vec<MemorySignal>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memory_signals \
         WHERE ($1::text IS NULL OR kind = $1) AND ($3::bigint IS NULL OR id < $3) \
         ORDER BY id DESC LIMIT $2"
    ))
    .bind(kind.map(SignalKind::as_str))
    .bind(limit)
    .bind(before)
    .fetch_all(pool)
    .await
}

/// `GET /memory-signals`: the learning signals, newest first, a page at a time.
#[utoipa::path(
    get,
    path = "/memory-signals",
    tag = "memories",
    operation_id = "list_memory_signals",
    params(
        ("kind" = Option<SignalKind>, Query, description = "Only signals of this kind; blank means any"),
        ("limit" = Option<i64>, Query, description = "Most signals to return; default 50, clamped to 1..=200"),
        ("before" = Option<i64>, Query, description = "Only signals with an `id` below this one, for the next page"),
    ),
    responses(
        (status = 200, description = "Signals, `id` descending", body = [MemorySignal]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 422, description = "`kind` is not a known value", body = ValidationErrors),
        (status = 500, description = "The signals could not be read"),
    )
)]
#[rocket::get("/memory-signals?<kind>&<limit>&<before>")]
pub async fn list(
    _host: LocalHost,
    kind: Option<&str>,
    limit: Option<i64>,
    before: Option<i64>,
    pool: &State<PgPool>,
) -> Result<Json<Vec<MemorySignal>>, ApiError> {
    let mut errors = Vec::new();
    let kind: Option<SignalKind> = filter(&mut errors, "kind", kind);
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }
    list_signals(pool, kind, crate::runs::page_size(limit), before)
        .await
        .map(Json)
        .map_err(internal("list memory signals"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn change(op: SignalOp, path: &str, before: Option<Value>, after: Option<Value>) -> Value {
        serde_json::to_value(SignalChange {
            op,
            path: path.into(),
            before,
            after,
        })
        .expect("serializes")
    }

    #[test]
    fn diff_cases() {
        use SignalOp::*;
        let cases: Vec<(&str, Value, Value, Vec<Value>)> = vec![
            (
                "equal values",
                json!({"a": 1, "b": {"c": [1, 2]}}),
                json!({"b": {"c": [1, 2]}, "a": 1}),
                vec![],
            ),
            (
                "a nested object key change",
                json!({"a": {"b": {"c": "x", "d": 1}}}),
                json!({"a": {"b": {"c": "y", "d": 1}}}),
                vec![change(
                    Replace,
                    "/a/b/c",
                    Some(json!("x")),
                    Some(json!("y")),
                )],
            ),
            (
                "an added key",
                json!({"a": 1}),
                json!({"a": 1, "b": 2}),
                vec![change(Add, "/b", None, Some(json!(2)))],
            ),
            (
                "a removed key",
                json!({"a": 1, "b": 2}),
                json!({"a": 1}),
                vec![change(Remove, "/b", Some(json!(2)), None)],
            ),
            (
                "an explicit null is present",
                json!({"a": null, "b": 1}),
                json!({"b": null, "c": null}),
                vec![
                    change(Remove, "/a", Some(Value::Null), None),
                    change(Replace, "/b", Some(json!(1)), Some(Value::Null)),
                    change(Add, "/c", None, Some(Value::Null)),
                ],
            ),
            (
                "an array is replaced whole",
                json!({"tags": [1, 2, 3]}),
                json!({"tags": [1, 3]}),
                vec![change(
                    Replace,
                    "/tags",
                    Some(json!([1, 2, 3])),
                    Some(json!([1, 3])),
                )],
            ),
            (
                "a type change at the root",
                json!({"a": 1}),
                json!("text"),
                vec![change(
                    Replace,
                    "",
                    Some(json!({"a": 1})),
                    Some(json!("text")),
                )],
            ),
            (
                "keys needing escapes",
                json!({"a/b": 1, "c~d": 1}),
                json!({"a/b": 2, "c~d": 2}),
                vec![
                    change(Replace, "/a~1b", Some(json!(1)), Some(json!(2))),
                    change(Replace, "/c~0d", Some(json!(1)), Some(json!(2))),
                ],
            ),
            (
                "lexicographic traversal order",
                json!({"z": 1, "m": {"y": 1, "b": 1}, "a": 1}),
                json!({"m": {"b": 2, "y": 2}, "a": 2, "z": 2}),
                vec![
                    change(Replace, "/a", Some(json!(1)), Some(json!(2))),
                    change(Replace, "/m/b", Some(json!(1)), Some(json!(2))),
                    change(Replace, "/m/y", Some(json!(1)), Some(json!(2))),
                    change(Replace, "/z", Some(json!(1)), Some(json!(2))),
                ],
            ),
        ];
        for (name, drafted, approved, expected) in cases {
            let got: Vec<Value> = diff(&drafted, &approved)
                .into_iter()
                .map(|c| serde_json::to_value(c).expect("serializes"))
                .collect();
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    fn a_change_always_carries_before_and_after() {
        let changes = diff(&json!({}), &json!({"a": 1}));
        assert_eq!(
            serde_json::to_value(&changes).expect("serializes"),
            json!([{"op": "add", "path": "/a", "before": null, "after": 1}])
        );
    }

    #[test]
    fn kinds_round_trip_through_text() {
        for kind in SignalKind::ALL {
            assert_eq!(SignalKind::try_from(kind.as_str().to_string()), Ok(*kind));
        }
        assert!(SignalKind::try_from("accept".to_string()).is_err());
    }
}
