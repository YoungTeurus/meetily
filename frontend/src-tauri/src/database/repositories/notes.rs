//! User-authored context is stored separately from recognized speech.
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

pub const MAX_NOTES_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, sqlx::FromRow)]
pub struct MeetingNotes {
    pub meeting_id: String,
    pub notes: String,
    pub revision: i64,
}

#[derive(Debug, Serialize)]
pub struct NotesError {
    pub code: &'static str,
    pub message: String,
}
impl NotesError {
    fn new(code: &'static str, message: &str) -> Self {
        Self { code, message: message.into() }
    }
}
impl From<sqlx::Error> for NotesError {
    fn from(error: sqlx::Error) -> Self {
        Self { code: "internal", message: format!("Could not access meeting notes: {error}") }
    }
}

pub struct NotesRepository;
impl NotesRepository {
    pub async fn get(pool: &SqlitePool, meeting_id: &str) -> Result<MeetingNotes, NotesError> {
        sqlx::query_as("SELECT m.id AS meeting_id,COALESCE(n.notes_markdown,'') AS notes,COALESCE(n.revision,0) AS revision FROM meetings m LEFT JOIN meeting_notes n ON n.meeting_id=m.id WHERE m.id=?")
            .bind(meeting_id).fetch_optional(pool).await?
            .ok_or_else(|| NotesError::new("not_found", "Meeting not found"))
    }

    /// Callers share SUMMARY_START_LOCK with summary starts. The transaction also
    /// invalidates any worker which already captured an older version of notes.
    pub async fn save(
        pool: &SqlitePool,
        meeting_id: &str,
        notes: &str,
        expected_revision: i64,
    ) -> Result<MeetingNotes, NotesError> {
        if notes.chars().count() > MAX_NOTES_CHARS || notes.contains('\0') || expected_revision < 0 {
            return Err(NotesError::new("invalid_notes", "Notes must contain at most 20000 characters, no null characters, and a valid revision"));
        }
        let mut tx = pool.begin().await?;
        let previous: MeetingNotes = sqlx::query_as("SELECT m.id AS meeting_id,COALESCE(n.notes_markdown,'') AS notes,COALESCE(n.revision,0) AS revision FROM meetings m LEFT JOIN meeting_notes n ON n.meeting_id=m.id WHERE m.id=?")
            .bind(meeting_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| NotesError::new("not_found", "Meeting not found"))?;
        if previous.revision != expected_revision {
            return Err(NotesError::new("notes_conflict", "Notes changed in another window. Your draft has not replaced the saved version."));
        }
        if previous.notes == notes {
            tx.commit().await?;
            return Ok(previous);
        }
        let revision = previous.revision + 1;
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meeting_notes(meeting_id,notes_markdown,notes_json,created_at,updated_at,revision) VALUES(?,?,NULL,?,?,?) ON CONFLICT(meeting_id) DO UPDATE SET notes_markdown=excluded.notes_markdown,notes_json=NULL,updated_at=excluded.updated_at,revision=excluded.revision")
            .bind(meeting_id).bind(notes).bind(&now).bind(&now).bind(revision)
            .execute(&mut *tx).await?;
        sqlx::query("UPDATE summary_processes SET result=CASE WHEN LOWER(status)='pending' THEN COALESCE(result_backup,result) ELSE result END,result_backup=NULL,result_backup_timestamp=NULL,status=CASE WHEN LOWER(status)='pending' THEN 'cancelled' ELSE status END,error=CASE WHEN LOWER(status)='pending' THEN 'Meeting notes changed; regenerate the summary' ELSE error END WHERE meeting_id=?")
            .bind(meeting_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO meeting_transcription_metadata(meeting_id,transcript_revision,summary_stale) VALUES(?,0,EXISTS(SELECT 1 FROM summary_processes WHERE meeting_id=? AND result IS NOT NULL)) ON CONFLICT(meeting_id) DO UPDATE SET summary_stale=excluded.summary_stale")
            .bind(meeting_id).bind(meeting_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE meetings SET updated_at=? WHERE id=?")
            .bind(now).bind(meeting_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(MeetingNotes { meeting_id: meeting_id.into(), notes: notes.into(), revision })
    }
}

/// This becomes part of the existing summary cache fingerprint as custom_prompt.
/// Keep notes distinct from the transcript and don't invent spoken quotations.
pub fn with_summary_notes(custom_prompt: &str, notes: &str) -> String {
    if notes.trim().is_empty() {
        return custom_prompt.to_owned();
    }
    format!("{custom_prompt}\n\nUser meeting notes (additional context, not verbatim transcript):\nUse these notes to clarify terms, decisions, and priorities. Distinguish the author's notes from what was actually said; do not fabricate quotations.\n{}", serde_json::to_string(notes).expect("serializing a string cannot fail"))
}
