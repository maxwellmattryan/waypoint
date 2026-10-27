//! Explains migration failures at startup. A database migrated by another branch or
//! worktree fails with a bare sqlx message; this adds what the database recorded, what
//! the local migration is, the likely cause and how to reset the dev database.

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

const CAUSE: &str =
    "Another branch or worktree probably ran different migrations against this database.";
const FIX: &str = "To start over (deletes all dev data): scripts/db-reset.sh, or: docker compose down -v && ./scripts/dev.sh";

/// Builds the startup message for a failed migration run. `recorded` is the database's
/// description for the failing version, if it could be read; `local` is the embedded
/// migration's description, if one has that version. Errors other than a mismatched or
/// missing version pass through as `database migrations failed: <err>`.
pub fn format_failure(err: &MigrateError, recorded: Option<&str>, local: Option<&str>) -> String {
    let first = format!("database migrations failed: {err}");
    let Some(version) = failing_version(err) else {
        return first;
    };
    let local_side = match local {
        Some(local) => format!("the local migration is \"{local}\""),
        None => "no local migration has this version".to_string(),
    };
    let detail = match recorded {
        Some(recorded) => {
            let differs = if Some(recorded) == local {
                ""
            } else {
                " They differ."
            };
            format!("This database recorded {version} as \"{recorded}\"; {local_side}.{differs}")
        }
        None => format!("For {version}, {local_side}."),
    };
    [first, detail, CAUSE.to_string(), FIX.to_string()].join("\n  ")
}

/// The version a mismatched or missing migration error names.
fn failing_version(err: &MigrateError) -> Option<i64> {
    match err {
        MigrateError::VersionMismatch(v) | MigrateError::VersionMissing(v) => Some(*v),
        _ => None,
    }
}

/// Reads the database's description for the failing version and the local one, then
/// builds the message with [`format_failure`]. A failed lookup leaves that side out.
pub async fn explain(pool: &PgPool, migrator: &Migrator, err: &MigrateError) -> String {
    let Some(version) = failing_version(err) else {
        return format_failure(err, None, None);
    };
    let recorded: Option<String> =
        sqlx::query_scalar("SELECT description FROM _sqlx_migrations WHERE version = $1")
            .bind(version)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let local = migrator
        .iter()
        .find(|m| m.version == version)
        .map(|m| m.description.as_ref());
    format_failure(err, recorded.as_deref(), local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatch_names_both_sides() {
        let err = MigrateError::VersionMismatch(20261022000005);
        let message = format_failure(&err, Some("resume versions"), Some("stages and events"));
        assert!(message.starts_with(&format!("database migrations failed: {err}\n  ")));
        assert!(message.contains("\"resume versions\""));
        assert!(message.contains("\"stages and events\""));
        assert!(message.contains("They differ."));
        assert!(message.contains("scripts/db-reset.sh"));
    }

    #[test]
    fn mismatch_without_recorded_keeps_local_side() {
        let err = MigrateError::VersionMismatch(20261022000005);
        let message = format_failure(&err, None, Some("stages and events"));
        assert!(!message.contains("This database recorded"));
        assert!(message.contains("\"stages and events\""));
        assert!(message.contains("scripts/db-reset.sh"));
    }

    #[test]
    fn missing_version_has_no_local_side() {
        let err = MigrateError::VersionMissing(20261022000099);
        let message = format_failure(&err, Some("resume versions"), None);
        assert!(message.starts_with(&format!("database migrations failed: {err}\n  ")));
        assert!(message.contains("20261022000099"));
        assert!(message.contains("no local migration has this version"));
        assert!(message.contains("scripts/db-reset.sh"));
    }

    #[test]
    fn other_errors_pass_through() {
        let err = MigrateError::VersionTooOld(1, 2);
        assert_eq!(
            format_failure(&err, Some("x"), Some("y")),
            format!("database migrations failed: {err}")
        );
    }
}
