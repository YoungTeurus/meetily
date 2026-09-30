//! Commit only a complete, validated replacement. SQLite remains authoritative;
//! its transcript revision identifies file mirrors and invalidates old page cursors.
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Serialize)]
pub struct TranscriptionInfo {
    pub transcript_revision: i64,
    pub summary_stale: bool,
}
#[derive(Debug)]
pub struct ReplacementSegment {
    pub id: String,
    pub text: String,
    pub timestamp: String,
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
}
pub async fn info(pool: &SqlitePool, meeting_id: &str) -> Result<TranscriptionInfo, String> {
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM meetings WHERE id=?")
        .bind(meeting_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err("Meeting not found".into());
    }
    let row = sqlx::query("SELECT transcript_revision,summary_stale FROM meeting_transcription_metadata WHERE meeting_id=?").bind(meeting_id).fetch_optional(pool).await.map_err(|e|e.to_string())?;
    Ok(TranscriptionInfo {
        transcript_revision: row
            .as_ref()
            .map(|r| r.get("transcript_revision"))
            .unwrap_or(0),
        summary_stale: row
            .map(|r| r.get::<i64, _>("summary_stale") != 0)
            .unwrap_or(false),
    })
}
pub async fn source(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<(String, DateTime<Utc>), String> {
    let sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='recording_sessions'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if sessions > 0 {
        let busy: i64=sqlx::query_scalar("SELECT count(*) FROM recording_sessions WHERE meeting_id=? AND state IN ('starting','recording','paused','stopping','processing')").bind(meeting_id).fetch_one(pool).await.map_err(|e|e.to_string())?;
        if busy > 0 {
            return Err("This recording has not finished processing".into());
        }
    }
    let row = sqlx::query("SELECT folder_path,created_at FROM meetings WHERE id=?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let folder = row
        .get::<Option<String>, _>("folder_path")
        .filter(|s| !s.is_empty())
        .ok_or("This meeting has no saved audio folder")?;
    let start: DateTime<Utc> = row.try_get("created_at").map_err(|e| e.to_string())?;
    Ok((folder, start))
}
pub async fn replace(
    pool: &SqlitePool,
    meeting_id: &str,
    segments: &[ReplacementSegment],
) -> Result<TranscriptionInfo, String> {
    if segments.is_empty() || segments.iter().all(|s| s.text.trim().is_empty()) {
        return Err("No transcript was produced; existing transcript was kept".into());
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM meetings WHERE id=?")
        .bind(meeting_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err("Meeting not found".into());
    }
    sqlx::query("DELETE FROM transcripts WHERE meeting_id=?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    for s in segments {
        sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp,audio_start_time,audio_end_time,duration) VALUES(?,?,?,?,?,?,?)")
            .bind(&s.id).bind(meeting_id).bind(&s.text).bind(&s.timestamp).bind(s.audio_start_time).bind(s.audio_end_time).bind(s.duration)
            .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    // A pre-replacement summary worker may still finish; invalidate its pending
    // compare-and-set token while preserving the user's previous summary.
    sqlx::query("UPDATE summary_processes SET result=CASE WHEN LOWER(status)='pending' THEN COALESCE(result_backup,result) ELSE result END, result_backup=NULL,result_backup_timestamp=NULL,status=CASE WHEN LOWER(status)='pending' THEN 'cancelled' ELSE status END,error=CASE WHEN LOWER(status)='pending' THEN 'Transcript replaced; regenerate the summary' ELSE error END WHERE meeting_id=?")
        .bind(meeting_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO meeting_transcription_metadata(meeting_id,transcript_revision,summary_stale) VALUES(?,1,EXISTS(SELECT 1 FROM summary_processes WHERE meeting_id=? AND result IS NOT NULL)) ON CONFLICT(meeting_id) DO UPDATE SET transcript_revision=transcript_revision+1,summary_stale=excluded.summary_stale")
        .bind(meeting_id).bind(meeting_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE meetings SET updated_at=? WHERE id=?")
        .bind(Utc::now())
        .bind(meeting_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let row=sqlx::query("SELECT transcript_revision,summary_stale FROM meeting_transcription_metadata WHERE meeting_id=?").bind(meeting_id).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let result = TranscriptionInfo {
        transcript_revision: row.get("transcript_revision"),
        summary_stale: row.get::<i64, _>("summary_stale") != 0,
    };
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(result)
}
