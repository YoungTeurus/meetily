use sqlx::SqlitePool;

async fn database() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/src-tauri/migrations");
    let mut migrations: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|f| f.unwrap().path())
        .collect();
    migrations.sort();
    for file in migrations {
        sqlx::raw_sql(&std::fs::read_to_string(file).unwrap())
            .execute(&pool)
            .await
            .unwrap();
    }
    pool
}

#[tokio::test]
async fn global_vocabulary_and_replacement_revision_are_persisted() {
    let pool = database().await;
    for table in ["transcription_vocabulary", "meeting_transcription_metadata"] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "missing persisted {table}");
    }
}

#[path = "../../frontend/src-tauri/src/audio/retranscription_store.rs"]
mod replacement;
#[path = "../../frontend/src-tauri/src/database/repositories/vocabulary.rs"]
mod vocabulary;

async fn seed(pool: &SqlitePool) {
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at,folder_path) VALUES('m','Call','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z','/saved/call')").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp) VALUES('old','m','Original','2026-09-30T10:00:00Z')").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO summary_processes(meeting_id,status,created_at,updated_at,result) VALUES('m','completed','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z','User edited summary')").execute(pool).await.unwrap();
}
fn new_segment(id: &str) -> replacement::ReplacementSegment {
    replacement::ReplacementSegment {
        id: id.into(),
        text: "New wording".into(),
        timestamp: "2026-09-30T10:00:01Z".into(),
        audio_start_time: Some(1.0),
        audio_end_time: Some(2.0),
        duration: Some(1.0),
    }
}
#[tokio::test]
async fn vocabulary_persists_normalized_global_without_per_run_mutation() {
    let pool = database().await;
    let saved = vocabulary::VocabularyRepository::save_global(&pool, Some("Tauri, Meetily\nTAURI"))
        .await
        .unwrap();
    assert_eq!(saved.as_deref(), Some("Tauri\nMeetily"));
    assert_eq!(
        vocabulary::VocabularyRepository::merge(Some("Phoenix,tauri"), saved.as_deref()).as_deref(),
        Some("Phoenix, tauri, Meetily")
    );
    assert_eq!(
        vocabulary::VocabularyRepository::get_global(&pool)
            .await
            .unwrap(),
        saved
    );
    assert!(
        vocabulary::VocabularyRepository::save_global(&pool, Some(&"é".repeat(1001)))
            .await
            .is_err()
    );
    assert!(vocabulary::VocabularyRepository::normalize("bad\0term").is_err());
    vocabulary::VocabularyRepository::save_global(&pool, None)
        .await
        .unwrap();
    assert_eq!(
        vocabulary::VocabularyRepository::get_global(&pool)
            .await
            .unwrap(),
        None
    );
}
#[tokio::test]
async fn replacement_rolls_back_everything_on_insert_failure_and_keeps_empty_result() {
    let pool = database().await;
    seed(&pool).await;
    assert!(replacement::replace(&pool, "m", &[]).await.is_err());
    sqlx::query("CREATE TRIGGER deny_replacement BEFORE INSERT ON transcripts BEGIN SELECT RAISE(ABORT,'disk unavailable'); END").execute(&pool).await.unwrap();
    assert!(replacement::replace(&pool, "m", &[new_segment("new")])
        .await
        .is_err());
    let text: String =
        sqlx::query_scalar("SELECT transcript FROM transcripts WHERE meeting_id='m'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(text, "Original");
    let info = replacement::info(&pool, "m").await.unwrap();
    assert_eq!(info.transcript_revision, 0);
    assert!(!info.summary_stale);
    sqlx::query("DROP TRIGGER deny_replacement")
        .execute(&pool)
        .await
        .unwrap();
    let info = replacement::replace(&pool, "m", &[new_segment("new")])
        .await
        .unwrap();
    assert_eq!(info.transcript_revision, 1);
    assert!(info.summary_stale);
    let summary: String =
        sqlx::query_scalar("SELECT result FROM summary_processes WHERE meeting_id='m'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(summary, "User edited summary");
    let info = replacement::replace(&pool, "m", &[new_segment("newer")])
        .await
        .unwrap();
    assert_eq!(info.transcript_revision, 2);
}
#[tokio::test]
async fn deleted_meeting_cannot_receive_replacement_or_orphan_revision() {
    let pool = database().await;
    assert!(replacement::source(&pool, "missing").await.is_err());
    assert!(
        replacement::replace(&pool, "missing", &[new_segment("new")])
            .await
            .is_err()
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM meeting_transcription_metadata")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
#[tokio::test]
async fn replacement_cancels_old_pending_summary_and_restores_user_backup() {
    let pool = database().await;
    seed(&pool).await;
    sqlx::query("UPDATE summary_processes SET status='PENDING',result='partial',result_backup='Manual summary',start_time='2026-09-30T11:00:00Z' WHERE meeting_id='m'").execute(&pool).await.unwrap();
    replacement::replace(&pool, "m", &[new_segment("new")])
        .await
        .unwrap();
    let row: (String, String, Option<String>) = sqlx::query_as(
        "SELECT status,result,result_backup FROM summary_processes WHERE meeting_id='m'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("cancelled".into(), "Manual summary".into(), None));
}
