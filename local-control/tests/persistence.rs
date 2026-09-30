use meetily_local_control::store::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
async fn db() -> sqlx::SqlitePool {
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE meetings(id TEXT PRIMARY KEY,title TEXT,created_at TEXT,updated_at TEXT,folder_path TEXT); CREATE TABLE transcripts(id TEXT PRIMARY KEY,meeting_id TEXT,transcript TEXT,timestamp TEXT,audio_start_time REAL,audio_end_time REAL,duration REAL); CREATE TABLE recording_sessions(recording_id TEXT PRIMARY KEY,meeting_id TEXT,state TEXT,initiator TEXT,detection_session_id TEXT,application TEXT);").execute(&p).await.unwrap();
    initialize(&p).await.unwrap();
    p
}
#[tokio::test]
async fn transcript_pages_cover_large_meeting_in_audio_order_without_truncation() {
    let p = db().await;
    sqlx::query("INSERT INTO meetings VALUES('m','Русская встреча','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z',NULL)").execute(&p).await.unwrap();
    for i in (0..503).rev() {
        sqlx::query("INSERT INTO transcripts VALUES(?, 'm', ?, '2026-09-30T10:00:00Z', ?, ?, 1)")
            .bind(format!("t{i:04}"))
            .bind(format!("Текст {i}"))
            .bind(i as f64)
            .bind((i + 1) as f64)
            .execute(&p)
            .await
            .unwrap();
    }
    let mut params = json!({"meeting_id":"m","limit":200});
    let mut items = vec![];
    loop {
        let v = dispatch(&p, "transcript.get", &params).await.unwrap();
        items.extend(v["items"].as_array().unwrap().clone());
        if v["next_cursor"].is_null() {
            break;
        };
        params["cursor"] = v["next_cursor"].clone();
    }
    assert_eq!(items.len(), 503);
    assert_eq!(items[0]["text"], "Текст 0");
    assert_eq!(items[502]["text"], "Текст 502");
    let export = dispatch(
        &p,
        "meetings.export",
        &json!({"meeting_id":"m","format":"txt"}),
    )
    .await
    .unwrap();
    assert!(export["content"].as_str().unwrap().contains("Текст 502"));
    assert!(dispatch(
        &p,
        "transcript.get",
        &json!({"meeting_id":"m","cursor":"broken"})
    )
    .await
    .is_err());
}
#[tokio::test]
async fn events_resume_with_stable_ids_and_missing_meetings_are_errors() {
    let p = db().await;
    emit_event(&p, "recording.started", Some("r"), Some("m"), json!({}))
        .await
        .unwrap();
    let a = dispatch(&p, "events.list", &json!({"limit":1}))
        .await
        .unwrap();
    emit_event(&p, "meeting.finalized", Some("r"), Some("m"), json!({}))
        .await
        .unwrap();
    let b = dispatch(&p, "events.list", &json!({"after":a["next_cursor"]}))
        .await
        .unwrap();
    assert_eq!(b["items"].as_array().unwrap().len(), 1);
    assert_eq!(b["items"][0]["event"], "meeting.finalized");
    assert!(dispatch(&p, "meetings.get", &json!({"meeting_id":"none"}))
        .await
        .is_err());
}
#[tokio::test]
async fn newest_meeting_uses_actual_datetime_across_legacy_and_rfc3339_formats() {
    let p = db().await;
    for (id, time) in [
        ("early", "2026-09-30T08:00:00+00:00"),
        ("late", "2026-09-30 12:00:00"),
    ] {
        sqlx::query("INSERT INTO meetings VALUES(?, ?, ?, ?, NULL)")
            .bind(id)
            .bind(id)
            .bind(time)
            .bind(time)
            .execute(&p)
            .await
            .unwrap();
    }
    let a = dispatch(&p, "meetings.list", &json!({"limit":1}))
        .await
        .unwrap();
    assert_eq!(a["items"][0]["meeting_id"], "late");
    let b = dispatch(
        &p,
        "meetings.list",
        &json!({"limit":1,"cursor":a["next_cursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(b["items"][0]["meeting_id"], "early");
    assert!(b["next_cursor"].is_null());
}

#[tokio::test]
async fn transcript_cursor_rejects_replacement_even_when_sqlite_reuses_rowids() {
    let p = db().await;
    sqlx::raw_sql("CREATE TABLE meeting_transcription_metadata(meeting_id TEXT PRIMARY KEY,transcript_revision INTEGER NOT NULL,summary_stale INTEGER NOT NULL); INSERT INTO meetings VALUES('m','Meeting','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z',NULL);").execute(&p).await.unwrap();
    for i in 0..3 {
        sqlx::query(
            "INSERT INTO transcripts VALUES(?, 'm', 'old', '2026-09-30T10:00:00Z', ?, ?, 1)",
        )
        .bind(format!("old{i}"))
        .bind(i as f64)
        .bind((i + 1) as f64)
        .execute(&p)
        .await
        .unwrap();
    }
    let first = dispatch(&p, "transcript.get", &json!({"meeting_id":"m","limit":1}))
        .await
        .unwrap();
    let mut legacy: serde_json::Value =
        serde_json::from_str(first["next_cursor"].as_str().unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("revision");
    let old_page = dispatch(
        &p,
        "transcript.get",
        &json!({"meeting_id":"m","limit":1,"cursor":legacy.to_string()}),
    )
    .await
    .unwrap();
    assert_eq!(old_page["items"][0]["id"], "old1");
    let mut replacement = p.begin().await.unwrap();
    sqlx::query("DELETE FROM transcripts WHERE meeting_id='m'")
        .execute(&mut *replacement)
        .await
        .unwrap();
    for i in 0..3 {
        sqlx::query(
            "INSERT INTO transcripts VALUES(?, 'm', 'new', '2026-09-30T10:00:00Z', ?, ?, 1)",
        )
        .bind(format!("new{i}"))
        .bind(i as f64)
        .bind((i + 1) as f64)
        .execute(&mut *replacement)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO meeting_transcription_metadata VALUES('m',1,1)")
        .execute(&mut *replacement)
        .await
        .unwrap();
    replacement.commit().await.unwrap();
    let error = dispatch(
        &p,
        "transcript.get",
        &json!({"meeting_id":"m","limit":1,"cursor":first["next_cursor"]}),
    )
    .await
    .expect_err("old cursor must not mix transcript revisions");
    assert_eq!(error.code, "conflict");
    assert_eq!(
        dispatch(
            &p,
            "transcript.get",
            &json!({"meeting_id":"m","cursor":legacy.to_string()})
        )
        .await
        .unwrap_err()
        .code,
        "conflict"
    );
    let fresh = dispatch(&p, "transcript.get", &json!({"meeting_id":"m","limit":1}))
        .await
        .unwrap();
    assert_eq!(fresh["transcript_revision"], 1);
    assert_eq!(fresh["items"][0]["id"], "new0");
    let next = dispatch(
        &p,
        "transcript.get",
        &json!({"meeting_id":"m","limit":1,"cursor":fresh["next_cursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(next["items"][0]["id"], "new1");
}
