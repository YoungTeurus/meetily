//! Explicit import snapshots. VACUUM INTO includes committed WAL contents and never
//! opens the source writable. Caller retains the snapshot as the import backup.
use crate::ControlError;
use sqlx::{sqlite::SqliteConnectOptions, Connection};
use std::path::Path;
pub async fn snapshot_for_import(
    source: &Path,
    backup: &Path,
    max_version: i64,
) -> Result<(), ControlError> {
    if backup.exists() {
        return Err(ControlError::new(
            "conflict",
            "Backup already exists; refusing to overwrite it",
        ));
    }
    if !source.is_file() {
        return Err(ControlError::new(
            "not_found",
            "Source database does not exist",
        ));
    }
    let mut db = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(source).read_only(true),
    )
    .await?;
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut db)
        .await?;
    if integrity != "ok" {
        return Err(ControlError::new(
            "invalid_request",
            "Source database integrity check failed",
        ));
    }
    // These columns are the Community interchange contract. Reject unrelated databases.
    sqlx::query("SELECT id,title,created_at,updated_at FROM meetings LIMIT 0")
        .execute(&mut db)
        .await?;
    sqlx::query("SELECT id,meeting_id,transcript,timestamp FROM transcripts LIMIT 0")
        .execute(&mut db)
        .await?;
    let has_migrations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_one(&mut db)
    .await?;
    if has_migrations > 0 {
        let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&mut db)
            .await?;
        if version.unwrap_or(0) > max_version {
            return Err(ControlError::new(
                "invalid_request",
                "Source uses a newer database schema than this build",
            ));
        }
    }
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_string_lossy().as_ref())
        .execute(&mut db)
        .await?;
    Ok(())
}
