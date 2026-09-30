//! Tests the actual shared desktop dispatch and SQLite lifecycle. Only capture
//! hardware is replaced; no duplicate lifecycle or mutex implementation exists.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::Listener;
use tokio::sync::Semaphore;

pub(super) struct Recorder {
    pub starts: AtomicUsize,
    pub stops: AtomicUsize,
    live: AtomicBool,
    start_entered: Semaphore,
    start_release: Semaphore,
    stop_entered: Semaphore,
    drain_release: Semaphore,
}
impl Recorder {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            live: AtomicBool::new(false),
            start_entered: Semaphore::new(0),
            start_release: Semaphore::new(0),
            stop_entered: Semaphore::new(0),
            drain_release: Semaphore::new(0),
        }
    }
    pub async fn start(&self) -> Result<(), String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.start_entered.add_permits(1);
        self.start_release.acquire().await.unwrap().forget();
        self.live.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub async fn stop<R: Runtime>(&self, app: &AppHandle<R>) -> Result<Option<String>, String> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        assert!(self.live.swap(false,Ordering::SeqCst),"Stop must only reach capture after warmup completed, even if the start waiter was cancelled");
        mark_capture_stopped(app).await?;
        self.stop_entered.add_permits(1);
        // The final transcript deliberately arrives after capture has stopped.
        self.drain_release.acquire().await.unwrap().forget();
        let update = TranscriptUpdate {
            text: "last words after capture stopped".into(),
            timestamp: "12:00".into(),
            source: "Audio".into(),
            sequence_id: 0,
            chunk_start_time: 1.,
            is_partial: false,
            confidence: 0.9,
            audio_start_time: 1.,
            audio_end_time: 2.,
            duration: 1.,
        };
        checkpoint(app, &update).await.map_err(|e| e.message)?;
        Ok(None)
    }
    pub fn is_recording(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }
}
async fn await_boundary(semaphore: &Semaphore) {
    tokio::time::timeout(std::time::Duration::from_secs(5), semaphore.acquire())
        .await
        .expect("lifecycle boundary timed out")
        .unwrap()
        .forget();
}
async fn join<T>(task: tokio::task::JoinHandle<T>) -> Result<T, tokio::task::JoinError> {
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .expect("Recording command did not finish within the test deadline")
}
async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn gui_cli_mcp_share_one_capture_and_finish_tails_after_caller_cancellation() {
    let _test_boundary = super::TEST_BOUNDARY.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    // Absolute joining confines the real Store plugin's app-data directory to
    // the fixture; no environment variables or user's preferences are changed.
    context.config_mut().identifier = temp.path().to_string_lossy().to_string();
    let native_app = tauri::test::mock_builder()
        .plugin(tauri_plugin_store::Builder::new().build())
        .build(context)
        .unwrap();
    let app = native_app.handle().clone();
    let path = temp
        .path()
        .join("meetings.sqlite")
        .to_string_lossy()
        .to_string();
    let database = crate::database::manager::DatabaseManager::new(&path, &path)
        .await
        .unwrap();
    let pool = database.pool().clone();
    app.manage(AppState {
        db_manager: database,
    });
    app.manage(Recorder::new());
    let mut preferences = crate::audio::recording_preferences::RecordingPreferences::default();
    preferences.save_folder = temp.path().join("recordings");
    preferences.auto_save = false;
    crate::audio::recording_preferences::save_recording_preferences(&app, &preferences)
        .await
        .unwrap();
    crate::set_language_preference_internal("auto".into()).unwrap();
    super::super::initialize_storage(&app).await.unwrap();
    let recorder = app.state::<Recorder>();
    let native_starts = Arc::new(AtomicUsize::new(0));
    let native_finals = Arc::new(AtomicUsize::new(0));
    let starts = native_starts.clone();
    app.listen("recording:started", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        assert!(payload["recording_id"].as_str().is_some());
        starts.fetch_add(1, Ordering::SeqCst);
    });
    let finals = native_finals.clone();
    app.listen("meeting:finalized", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        assert!(payload["meeting_id"].as_str().is_some());
        finals.fetch_add(1, Ordering::SeqCst);
    });

    // GUI capture is held during warmup while CLI and MCP submit competing
    // starts through the same public dispatch used by their gateway requests.
    let gui_app = app.clone();
    let gui = tokio::spawn(async move {
        audio::start_recording_with_meeting_name(gui_app, Some("GUI call".into())).await
    });
    await_boundary(&recorder.start_entered).await;
    let cli_app = app.clone();
    let mcp_app = app.clone();
    let cli = tokio::spawn(async move {
        dispatch(
            cli_app,
            "recording.start",
            json!({"initiator":"cli","idempotency_key":"cli-competing"}),
        )
        .await
    });
    let mcp = tokio::spawn(async move {
        dispatch(
            mcp_app,
            "recording.start",
            json!({"initiator":"mcp","idempotency_key":"mcp-competing"}),
        )
        .await
    });
    let warming = session_status(&app, &json!({})).await.unwrap();
    assert_eq!(warming["recording"]["state"], "starting");
    assert_eq!(recorder.starts.load(Ordering::SeqCst), 1);
    recorder.start_release.add_permits(1);
    join(gui).await.unwrap().unwrap();
    assert_eq!(
        join(cli).await.unwrap().unwrap_err().code,
        "recording_active"
    );
    assert_eq!(
        join(mcp).await.unwrap().unwrap_err().code,
        "recording_active"
    );
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM meetings").await, 1);
    let current = session_status(&app, &json!({})).await.unwrap();
    let first = current["recording"]["recording_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(current["recording"]["initiator"], "gui");

    // GUI's stop waiter is cancelled while CLI/MCP stops overlap the same
    // drain. The backend-owned operation must still checkpoint and finalize.
    let gui_app = app.clone();
    let gui_stop = tokio::spawn(async move {
        audio::stop_recording(
            gui_app,
            audio::RecordingArgs {
                save_path: String::new(),
            },
        )
        .await
    });
    await_boundary(&recorder.stop_entered).await;
    let stopped = session_status(&app, &json!({})).await.unwrap();
    assert_eq!(stopped["recording"]["state"], "processing");
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM control_events WHERE event='meeting.finalized'"
        )
        .await,
        0
    );
    let cli_app = app.clone();
    let cli_id = first.clone();
    let cli_stop = tokio::spawn(async move {
        dispatch(
            cli_app,
            "recording.stop",
            json!({"recording_id":cli_id,"initiator":"cli"}),
        )
        .await
    });
    let mcp_app = app.clone();
    let mcp_id = first.clone();
    let mcp_stop = tokio::spawn(async move {
        dispatch(
            mcp_app,
            "recording.stop",
            json!({"recording_id":mcp_id,"initiator":"mcp"}),
        )
        .await
    });
    gui_stop.abort();
    assert!(join(gui_stop).await.unwrap_err().is_cancelled());
    recorder.drain_release.add_permits(1);
    for result in [join(cli_stop).await.unwrap(), join(mcp_stop).await.unwrap()] {
        let value = result.unwrap();
        assert_eq!(value["recording"]["recording_id"], first);
        assert_eq!(value["recording"]["state"], "finalized");
    }
    assert_eq!(recorder.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM transcripts WHERE transcript='last words after capture stopped'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM control_events WHERE event='meeting.finalized'"
        )
        .await,
        1
    );

    // Cancel a CLI start waiter during warmup. An MCP retry with the same key
    // and a GUI stop during STARTING must use that one surviving operation.
    let cli_app = app.clone();
    let cli_start = tokio::spawn(async move {
        dispatch(
            cli_app,
            "recording.start",
            json!({"name":"shared retry","initiator":"cli","idempotency_key":"retry-key"}),
        )
        .await
    });
    await_boundary(&recorder.start_entered).await;
    let warming = session_status(&app, &json!({})).await.unwrap();
    let second = warming["recording"]["recording_id"]
        .as_str()
        .unwrap()
        .to_owned();
    cli_start.abort();
    assert!(join(cli_start).await.unwrap_err().is_cancelled());
    let mcp_app = app.clone();
    let mcp_retry = tokio::spawn(async move {
        dispatch(
            mcp_app,
            "recording.start",
            json!({"initiator":"mcp","idempotency_key":"retry-key"}),
        )
        .await
    });
    let gui_app = app.clone();
    let gui_stop = tokio::spawn(async move {
        audio::stop_recording(
            gui_app,
            audio::RecordingArgs {
                save_path: String::new(),
            },
        )
        .await
    });
    recorder.start_release.add_permits(1);
    await_boundary(&recorder.stop_entered).await;
    assert_eq!(recorder.starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        session_status(&app, &json!({})).await.unwrap()["recording"]["state"],
        "processing"
    );
    recorder.drain_release.add_permits(1);
    join(gui_stop).await.unwrap().unwrap();
    let retry = join(mcp_retry).await.unwrap().unwrap();
    assert_eq!(retry["recording"]["recording_id"], second);
    assert_eq!(retry["idempotent"], true);
    assert_eq!(recorder.starts.load(Ordering::SeqCst), 2);
    assert_eq!(recorder.stops.load(Ordering::SeqCst), 2);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM meetings").await, 2);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM transcripts").await, 2);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM control_events WHERE event='meeting.finalized'"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM recording_sessions WHERE state!='finalized'"
        )
        .await,
        0
    );
    assert!(active().is_none());
    assert_eq!(native_starts.load(Ordering::SeqCst), 2);
    assert_eq!(native_finals.load(Ordering::SeqCst), 2);
}
