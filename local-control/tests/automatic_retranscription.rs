#[allow(dead_code)]
#[path = "../../frontend/src-tauri/src/audio/automatic_retranscription_store.rs"]
mod queue;
#[allow(dead_code)]
#[path = "../../frontend/src-tauri/src/audio/retranscription_store.rs"]
mod replacement;
use sqlx::SqlitePool;
async fn database() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/src-tauri/migrations");
    let mut files: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    files.sort();
    for f in files {
        sqlx::raw_sql(&std::fs::read_to_string(f).unwrap())
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "CREATE TABLE recording_sessions(recording_id TEXT PRIMARY KEY,meeting_id TEXT,state TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    queue::initialize(&pool).await.unwrap();
    pool
}
async fn seed(p: &SqlitePool, id: &str) {
    sqlx::query("INSERT INTO meetings(id,title,created_at,updated_at) VALUES(?,'Call','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z')").bind(id).execute(p).await.unwrap();
    sqlx::query("INSERT INTO recording_sessions VALUES(?,?,'processing')")
        .bind(id)
        .bind(id)
        .execute(p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcripts(id,meeting_id,transcript,timestamp) VALUES(?,?,'Original','2026-09-30T10:00:00Z')").bind(format!("old-{id}")).bind(id).execute(p).await.unwrap();
}
async fn finish(p: &SqlitePool, id: &str) {
    sqlx::query("UPDATE recording_sessions SET state='finalized' WHERE recording_id=?")
        .bind(id)
        .execute(p)
        .await
        .unwrap();
}
async fn enable(p: &SqlitePool) {
    queue::save_settings(
        p,
        &queue::Settings {
            enabled: true,
            provider: "whisper".into(),
            model: "small".into(),
            language: "ru".into(),
        },
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn finalization_is_atomic_idempotent_and_never_scans_history() {
    let p = database().await;
    assert!(!queue::settings(&p).await.unwrap().enabled);
    seed(&p, "old").await;
    finish(&p, "old").await;
    enable(&p).await;
    queue::initialize(&p).await.unwrap();
    assert!(queue::jobs(&p, None).await.unwrap().is_empty());
    seed(&p, "new").await;
    let mut tx = p.begin().await.unwrap();
    sqlx::query("UPDATE recording_sessions SET state='finalized' WHERE recording_id='new'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(queue::jobs(&p, None).await.unwrap().is_empty());
    finish(&p, "new").await;
    finish(&p, "new").await;
    assert_eq!(queue::jobs(&p, None).await.unwrap().len(), 1);
}
#[tokio::test]
async fn each_call_snapshots_settings_and_global_terms_and_disable_only_cancels_waiting() {
    let p = database().await;
    enable(&p).await;
    sqlx::query("INSERT INTO transcription_vocabulary VALUES('global','Hermes')")
        .execute(&p)
        .await
        .unwrap();
    seed(&p, "a").await;
    finish(&p, "a").await;
    let a = queue::next(&p).await.unwrap().unwrap();
    assert!(queue::transition(&p, &a.job_id, "queued", "running", None)
        .await
        .unwrap());
    sqlx::query("UPDATE transcription_vocabulary SET vocabulary='Other'")
        .execute(&p)
        .await
        .unwrap();
    seed(&p, "b").await;
    finish(&p, "b").await;
    assert_eq!(a.config["vocabulary"], "Hermes");
    assert_eq!(
        queue::next(&p).await.unwrap().unwrap().config["vocabulary"],
        "Other"
    );
    let mut settings = queue::settings(&p).await.unwrap();
    settings.enabled = false;
    queue::save_settings(&p, &settings).await.unwrap();
    assert_eq!(
        queue::jobs(&p, Some("a")).await.unwrap()[0].state,
        "running"
    );
    assert_eq!(
        queue::jobs(&p, Some("b")).await.unwrap()[0].state,
        "cancelled"
    );
}
fn segment() -> replacement::ReplacementSegment {
    replacement::ReplacementSegment {
        id: "new-text".into(),
        text: "Updated".into(),
        timestamp: "2026-09-30T10:00:01Z".into(),
        audio_start_time: Some(1.0),
        audio_end_time: Some(2.0),
        duration: Some(1.0),
    }
}
#[tokio::test]
async fn committed_replacement_survives_crash_but_unfinished_work_is_failed() {
    let p = database().await;
    enable(&p).await;
    seed(&p, "a").await;
    finish(&p, "a").await;
    let a = queue::next(&p).await.unwrap().unwrap();
    queue::transition(&p, &a.job_id, "queued", "running", None)
        .await
        .unwrap();
    replacement::replace_for_job(&p, "a", &[segment()], Some(&a.job_id))
        .await
        .unwrap();
    seed(&p, "b").await;
    finish(&p, "b").await;
    let b = queue::next(&p).await.unwrap().unwrap();
    queue::transition(&p, &b.job_id, "queued", "running", None)
        .await
        .unwrap();
    queue::initialize(&p).await.unwrap();
    assert_eq!(
        queue::jobs(&p, Some("a")).await.unwrap()[0].state,
        "completed"
    );
    assert_eq!(queue::jobs(&p, Some("b")).await.unwrap()[0].state, "failed");
    let old: String = sqlx::query_scalar("SELECT transcript FROM transcripts WHERE meeting_id='b'")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(old, "Original");
}
#[tokio::test]
async fn accepted_cancel_rolls_back_replacement_and_manual_run_supersedes_waiting() {
    let p = database().await;
    enable(&p).await;
    seed(&p, "a").await;
    finish(&p, "a").await;
    let a = queue::next(&p).await.unwrap().unwrap();
    queue::transition(&p, &a.job_id, "queued", "running", None)
        .await
        .unwrap();
    queue::transition(&p, &a.job_id, "running", "cancelled", None)
        .await
        .unwrap();
    assert!(
        replacement::replace_for_job(&p, "a", &[segment()], Some(&a.job_id))
            .await
            .is_err()
    );
    let old: String = sqlx::query_scalar("SELECT transcript FROM transcripts WHERE meeting_id='a'")
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(old, "Original");
    seed(&p, "b").await;
    finish(&p, "b").await;
    queue::supersede(&p, "b").await.unwrap();
    assert!(queue::next(&p).await.unwrap().is_none());
}
