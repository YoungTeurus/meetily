#[path = "../../frontend/src-tauri/src/database/repositories/notes.rs"]
mod notes;
use notes::NotesRepository;
use sqlx::SqlitePool;

async fn database() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/src-tauri/migrations");
    let mut migrations: Vec<_> = std::fs::read_dir(path).unwrap()
        .map(|f| f.unwrap().path()).collect();
    migrations.sort();
    for file in migrations {
        sqlx::raw_sql(&std::fs::read_to_string(file).unwrap()).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at) VALUES('m','Call','2026-09-30','2026-09-30')")
        .execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn notes_round_trip_and_stale_autosave_cannot_replace_latest_text() {
    let pool = database().await;
    let empty = NotesRepository::get(&pool, "m").await.unwrap();
    assert_eq!(empty.notes, "");
    assert_eq!(empty.revision, 0);
    let first = NotesRepository::save(&pool, "m", "Решение: Гермес\n\n- Проверить HRMS", 0).await.unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(NotesRepository::get(&pool, "m").await.unwrap(), first);
    let err = NotesRepository::save(&pool, "m", "stale window", 0).await.unwrap_err();
    assert_eq!(err.code, "notes_conflict");
    assert_eq!(NotesRepository::get(&pool, "m").await.unwrap(), first);
    assert_eq!(NotesRepository::save(&pool, "m", &first.notes, 1).await.unwrap().revision, 1);
    let cleared = NotesRepository::save(&pool, "m", "", 1).await.unwrap();
    assert_eq!(cleared.notes, "");
    assert_eq!(cleared.revision, 2);
}

#[tokio::test]
async fn notes_validate_unicode_limit_and_never_create_missing_meetings() {
    let pool = database().await;
    let text = "Я".repeat(20_000);
    NotesRepository::save(&pool, "m", &text, 0).await.unwrap();
    for invalid in ["Я".repeat(20_001), "bad\0note".into()] {
        assert_eq!(NotesRepository::save(&pool, "m", &invalid, 1).await.unwrap_err().code, "invalid_notes");
    }
    assert_eq!(NotesRepository::get(&pool, "absent").await.unwrap_err().code, "not_found");
    assert_eq!(NotesRepository::save(&pool, "absent", "text", 0).await.unwrap_err().code, "not_found");
    assert_eq!(NotesRepository::get(&pool, "m").await.unwrap().notes, text);
}

#[tokio::test]
async fn edited_notes_invalidate_pending_summary_without_losing_previous_result() {
    let pool = database().await;
    sqlx::query("INSERT INTO summary_processes(meeting_id,status,created_at,updated_at,result,result_backup,start_time) VALUES('m','PENDING','2026-09-30','2026-09-30','old summary','edited prior summary','2026-09-30T12:00:00Z')")
        .execute(&pool).await.unwrap();
    NotesRepository::save(&pool, "m", "New decision", 0).await.unwrap();
    let row: (String, String) = sqlx::query_as("SELECT status,result FROM summary_processes WHERE meeting_id='m'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row, ("cancelled".into(), "edited prior summary".into()));
    let stale: i64 = sqlx::query_scalar("SELECT summary_stale FROM meeting_transcription_metadata WHERE meeting_id='m'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(stale, 1);
    let updated = sqlx::query("UPDATE summary_processes SET result='obsolete worker' WHERE meeting_id='m' AND LOWER(status)='pending'")
        .execute(&pool).await.unwrap();
    assert_eq!(updated.rows_affected(), 0);
}

#[test]
fn notes_are_separate_context_and_part_of_the_prompt_cache_input() {
    assert_eq!(notes::with_summary_notes("Use bullets", ""), "Use bullets");
    let prompt = notes::with_summary_notes("Use bullets", "Гермес = internal HRMS");
    assert!(prompt.starts_with("Use bullets"));
    assert!(prompt.contains("User meeting notes"));
    assert!(prompt.contains("Гермес = internal HRMS"));
    assert!(prompt.contains("not verbatim transcript"));
}
