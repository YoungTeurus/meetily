//! Durable recording state and transcript checkpoints. Kept independent of Tauri
//! so crash recovery, ordering and transaction guarantees can be fixture-tested.
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Session {
    pub recording_id: String,
    pub meeting_id: String,
    pub state: String,
    pub initiator: String,
    pub idempotency_key: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub stopped_at: Option<String>,
    pub finalized_at: Option<String>,
    pub error: Option<String>,
    pub detection_session_id: Option<String>,
    pub application: Option<String>,
}

pub async fn initialize(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("CREATE TABLE IF NOT EXISTS recording_sessions (
        recording_id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE CASCADE,
        state TEXT NOT NULL, initiator TEXT NOT NULL, idempotency_key TEXT UNIQUE,
        started_at TEXT NOT NULL, updated_at TEXT NOT NULL, stopped_at TEXT, finalized_at TEXT, error TEXT,
        detection_session_id TEXT, application TEXT
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS recording_segments (
        recording_id TEXT NOT NULL REFERENCES recording_sessions(recording_id) ON DELETE CASCADE,
        sequence_id INTEGER NOT NULL, transcript_id TEXT NOT NULL UNIQUE REFERENCES transcripts(id) ON DELETE CASCADE,
        confidence REAL NOT NULL, is_partial INTEGER NOT NULL,
        PRIMARY KEY(recording_id,sequence_id)
    )").execute(pool).await?;
    Ok(())
}

pub async fn recover(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE recording_sessions SET state='incomplete',error='Application exited before finalization',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state IN ('starting','recording','paused','stopping','processing')").execute(pool).await?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM recording_sessions WHERE recording_id=?")
        .bind(id)
        .fetch_optional(pool)
        .await
}
pub async fn by_key(pool: &SqlitePool, key: &str) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM recording_sessions WHERE idempotency_key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
}
pub async fn latest(pool: &SqlitePool) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM recording_sessions ORDER BY started_at DESC,recording_id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
}

pub async fn create(pool: &SqlitePool, s: &Session, title: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at) VALUES(?,?,?,?)")
        .bind(&s.meeting_id)
        .bind(title)
        .bind(&s.started_at)
        .bind(&s.updated_at)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO recording_sessions(recording_id,meeting_id,state,initiator,idempotency_key,started_at,updated_at,detection_session_id,application) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(&s.recording_id).bind(&s.meeting_id).bind(&s.state).bind(&s.initiator).bind(&s.idempotency_key).bind(&s.started_at).bind(&s.updated_at).bind(&s.detection_session_id).bind(&s.application).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn transition(
    pool: &SqlitePool,
    id: &str,
    state: &str,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE recording_sessions SET state=?, error=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), stopped_at=CASE WHEN ?='processing' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE stopped_at END WHERE recording_id=? AND state!='finalized'")
        .bind(state).bind(error).bind(state).bind(id).execute(pool).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn checkpoint(
    pool: &SqlitePool,
    s: &Session,
    sequence: u64,
    text: &str,
    _timestamp: &str,
    start: f64,
    end: f64,
    duration: f64,
    confidence: f32,
    partial: bool,
) -> Result<(), sqlx::Error> {
    if sequence > i64::MAX as u64
        || !start.is_finite()
        || !end.is_finite()
        || !duration.is_finite()
        || start < 0.0
        || end < start
        || duration < 0.0
    {
        return Err(sqlx::Error::Protocol(
            "Invalid transcript sequence or audio timing".into(),
        ));
    }
    let started = chrono::DateTime::parse_from_rfc3339(&s.started_at)
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    let timestamp = started
        .checked_add_signed(chrono::TimeDelta::milliseconds((start * 1000.0) as i64))
        .ok_or_else(|| sqlx::Error::Protocol("Transcript timestamp is out of range".into()))?
        .to_rfc3339();
    let mut tx = pool.begin().await?;
    let state: Option<(String,)> =
        sqlx::query_as("SELECT state FROM recording_sessions WHERE recording_id=?")
            .bind(&s.recording_id)
            .fetch_optional(&mut *tx)
            .await?;
    if !state.as_ref().is_some_and(|s| {
        matches!(
            s.0.as_str(),
            "starting" | "recording" | "paused" | "stopping" | "processing" | "failed"
        )
    }) {
        return Err(sqlx::Error::Protocol("Recording is not writable".into()));
    }
    let id = format!("{}:{}", s.recording_id, sequence);
    sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp,audio_start_time,audio_end_time,duration) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET transcript=excluded.transcript,timestamp=excluded.timestamp,audio_start_time=excluded.audio_start_time,audio_end_time=excluded.audio_end_time,duration=excluded.duration")
        .bind(&id).bind(&s.meeting_id).bind(text).bind(timestamp).bind(start).bind(end).bind(duration).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO recording_segments(recording_id,sequence_id,transcript_id,confidence,is_partial) VALUES(?,?,?,?,?) ON CONFLICT(recording_id,sequence_id) DO UPDATE SET confidence=excluded.confidence,is_partial=excluded.is_partial")
        .bind(&s.recording_id).bind(sequence as i64).bind(id).bind(confidence).bind(partial).execute(&mut *tx).await?;
    sqlx::query("UPDATE meetings SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
        .bind(&s.meeting_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

pub async fn finalize(
    pool: &SqlitePool,
    id: &str,
    folder: Option<&str>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let affected=sqlx::query("UPDATE recording_sessions SET state='finalized',finalized_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),error=NULL WHERE recording_id=? AND state='processing'")
        .bind(id).execute(&mut *tx).await?.rows_affected();
    if affected != 1 {
        return Err(sqlx::Error::Protocol(
            "Only a drained recording can finalize".into(),
        ));
    }
    sqlx::query("UPDATE meetings SET folder_path=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=(SELECT meeting_id FROM recording_sessions WHERE recording_id=?)")
        .bind(folder).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO control_events(time,event,recording_id,meeting_id,data) SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now'),'meeting.finalized',recording_id,meeting_id,? FROM recording_sessions WHERE recording_id=?")
        .bind(serde_json::json!({"folder_path":folder}).to_string()).bind(id).execute(&mut *tx).await?;
    tx.commit().await
}

/// Save the drain boundary before finalization. Repeating this never reopens capture.
pub async fn mark_drained(
    pool: &SqlitePool,
    id: &str,
    folder: Option<&str>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE recording_sessions SET state='processing',stopped_at=COALESCE(stopped_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE recording_id=? AND state IN ('stopping','processing')")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE meetings SET folder_path=? WHERE id=(SELECT meeting_id FROM recording_sessions WHERE recording_id=?)").bind(folder).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO control_events(time,event,recording_id,meeting_id,data) SELECT stopped_at,'recording.stopped',recording_id,meeting_id,'{}' FROM recording_sessions WHERE recording_id=? AND NOT EXISTS (SELECT 1 FROM control_events WHERE recording_id=? AND event='recording.stopped')")
        .bind(id).bind(id).execute(&mut *tx).await?;
    tx.commit().await
}

/// The same versioned envelope and atomic replacement is used by live saves and
/// retry recovery. A failed replacement retains `dirty`, preventing finalization.
pub fn retry_transcript_file<T: Serialize>(
    folder: &std::path::Path,
    segments: &[T],
    dirty: &mut bool,
) -> Result<(), std::io::Error> {
    use std::io::Write;
    if !*dirty {
        return Ok(());
    }
    let bytes=serde_json::to_vec_pretty(&serde_json::json!({"version":"1.0","segments":segments,"last_updated":chrono::Utc::now().to_rfc3339(),"total_segments":segments.len()}))
        .map_err(|e|std::io::Error::new(std::io::ErrorKind::InvalidData,e))?;
    let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    // tempfile uses platform-specific overwrite semantics (including Windows).
    temporary
        .persist(folder.join("transcripts.json"))
        .map_err(|e| e.error)?;
    *dirty = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> SqlitePool {
        let p = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE meetings(id TEXT PRIMARY KEY,title TEXT,created_at TEXT,updated_at TEXT,folder_path TEXT)").execute(&p).await.unwrap();
        sqlx::query("CREATE TABLE transcripts(id TEXT PRIMARY KEY,meeting_id TEXT,transcript TEXT,timestamp TEXT,audio_start_time REAL,audio_end_time REAL,duration REAL)").execute(&p).await.unwrap();
        sqlx::query("CREATE TABLE control_events(id INTEGER PRIMARY KEY AUTOINCREMENT,time TEXT NOT NULL,event TEXT NOT NULL,recording_id TEXT,meeting_id TEXT,data TEXT NOT NULL)").execute(&p).await.unwrap();
        initialize(&p).await.unwrap();
        p
    }
    fn session() -> Session {
        Session {
            recording_id: "r1".into(),
            meeting_id: "m1".into(),
            state: "starting".into(),
            initiator: "cli".into(),
            idempotency_key: Some("same".into()),
            started_at: "2026-09-30T00:00:00Z".into(),
            updated_at: "2026-09-30T00:00:00Z".into(),
            stopped_at: None,
            finalized_at: None,
            error: None,
            detection_session_id: None,
            application: None,
        }
    }
    #[tokio::test]
    async fn crash_marks_active_incomplete_without_losing_checkpoint() {
        let p = fixture().await;
        create(&p, &session(), "Call").await.unwrap();
        checkpoint(&p, &session(), 7, "tail", "12:00", 1., 2., 1., 0.9, false)
            .await
            .unwrap();
        recover(&p).await.unwrap();
        assert_eq!(get(&p, "r1").await.unwrap().unwrap().state, "incomplete");
        let text: (String,) = sqlx::query_as("SELECT transcript FROM transcripts")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(text.0, "tail");
    }
    #[tokio::test]
    async fn checkpoint_upsert_and_finalization_are_guarded() {
        let p = fixture().await;
        let s = session();
        create(&p, &s, "Call").await.unwrap();
        checkpoint(&p, &s, 2, "partial", "12:00", 0., 1., 1., 0.9, true)
            .await
            .unwrap();
        checkpoint(&p, &s, 2, "final tail", "12:00", 0., 2., 2., 0.9, false)
            .await
            .unwrap();
        assert!(finalize(&p, "r1", Some("folder")).await.is_err());
        transition(&p, "r1", "processing", None).await.unwrap();
        finalize(&p, "r1", Some("folder")).await.unwrap();
        assert_eq!(get(&p, "r1").await.unwrap().unwrap().state, "finalized");
        let events: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM control_events WHERE event='meeting.finalized'")
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(events.0, 1);
        let rows: Vec<(String,)> = sqlx::query_as("SELECT transcript FROM transcripts")
            .fetch_all(&p)
            .await
            .unwrap();
        assert_eq!(rows, vec![("final tail".into(),)]);
        assert!(
            checkpoint(&p, &s, 3, "late", "12:00", 2., 3., 1., 0.9, false)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn finalize_event_failure_rolls_back_and_retry_preserves_folder() {
        let p = fixture().await;
        let s = session();
        create(&p, &s, "Call").await.unwrap();
        transition(&p, "r1", "stopping", None).await.unwrap();
        mark_drained(&p, "r1", Some("durable-folder"))
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_finalized BEFORE INSERT ON control_events WHEN NEW.event='meeting.finalized' BEGIN SELECT RAISE(ABORT,'disk fault'); END").execute(&p).await.unwrap();
        assert!(finalize(&p, "r1", Some("durable-folder")).await.is_err());
        assert_eq!(get(&p, "r1").await.unwrap().unwrap().state, "processing");
        sqlx::query("DROP TRIGGER fail_finalized")
            .execute(&p)
            .await
            .unwrap();
        mark_drained(&p, "r1", Some("durable-folder"))
            .await
            .unwrap();
        finalize(&p, "r1", Some("durable-folder")).await.unwrap();
        assert!(finalize(&p, "r1", None).await.is_err());
        let counts: Vec<(String, i64)> = sqlx::query_as(
            "SELECT event,COUNT(*) FROM control_events GROUP BY event ORDER BY event",
        )
        .fetch_all(&p)
        .await
        .unwrap();
        assert_eq!(
            counts,
            vec![
                ("meeting.finalized".into(), 1),
                ("recording.stopped".into(), 1)
            ]
        );
        let folder: (String,) = sqlx::query_as("SELECT folder_path FROM meetings")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(folder.0, "durable-folder");
    }
    #[tokio::test]
    async fn new_checkpoints_use_absolute_iso_time_and_audio_seconds() {
        let p = fixture().await;
        let s = session();
        create(&p, &s, "Call").await.unwrap();
        checkpoint(&p, &s, 0, "tail", "12:00", 1.5, 2.0, 0.5, 0.9, false)
            .await
            .unwrap();
        let timing: (String, f64) =
            sqlx::query_as("SELECT timestamp,audio_start_time FROM transcripts")
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(timing.0, "2026-09-30T00:00:01.500+00:00");
        assert_eq!(timing.1, 1.5);
    }
    #[test]
    fn transcript_file_recovery_keeps_envelope_and_dirty_until_success() {
        let folder = tempfile::tempdir().unwrap();
        let broken = folder.path().join("missing");
        let segments = vec![serde_json::json!({"sequence_id":1,"text":"tail"})];
        let mut dirty = true;
        assert!(retry_transcript_file(&broken, &segments, &mut dirty).is_err());
        assert!(dirty);
        std::fs::create_dir(&broken).unwrap();
        std::fs::write(broken.join("transcripts.json"), "old file").unwrap();
        retry_transcript_file(&broken, &segments, &mut dirty).unwrap();
        assert!(!dirty);
        let payload: serde_json::Value =
            serde_json::from_slice(&std::fs::read(broken.join("transcripts.json")).unwrap())
                .unwrap();
        assert_eq!(payload["version"], "1.0");
        assert_eq!(payload["total_segments"], 1);
        assert_eq!(payload["segments"][0]["text"], "tail");
        assert!(payload["last_updated"].as_str().unwrap().contains('T'));
    }
    #[tokio::test]
    async fn idempotency_is_durable_across_restart() {
        let p = fixture().await;
        create(&p, &session(), "Call").await.unwrap();
        recover(&p).await.unwrap();
        assert_eq!(
            by_key(&p, "same").await.unwrap().unwrap().recording_id,
            "r1"
        );
        let mut second = session();
        second.recording_id = "r2".into();
        second.meeting_id = "m2".into();
        assert!(create(&p, &second, "Duplicate").await.is_err());
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }
}
