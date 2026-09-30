//! Single recording lifecycle shared by GUI, tray and local clients.
//! Transitions serialize; transcription checkpoints do not acquire that lock,
//! allowing stop to wait for the final worker without deadlocking.
#[path = "recording_store.rs"]
pub mod persistence;
use super::ControlError;
use crate::audio::{recording_commands as audio, transcription::TranscriptUpdate};
use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

static TRANSITION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static ACTIVE: Mutex<Option<persistence::Session>> = Mutex::new(None);
static FAILURE: Mutex<Option<String>> = Mutex::new(None);
static PENDING: Mutex<Vec<TranscriptUpdate>> = Mutex::new(Vec::new());
// Some(None) denotes fully drained capture with audio saving disabled.
static DRAINED: Mutex<Option<Option<String>>> = Mutex::new(None);
static CAPTURE_FOLDER: Mutex<Option<String>> = Mutex::new(None);
static FILE_DIRTY: Mutex<bool> = Mutex::new(false);

fn db<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, ControlError> {
    app.try_state::<AppState>()
        .map(|s| s.db_manager.pool().clone())
        .ok_or_else(|| ControlError::new("not_ready", "Complete initial setup before recording"))
}
fn db_error(error: sqlx::Error) -> ControlError {
    ControlError::new("persistence_failed", error.to_string())
}
fn active() -> Option<persistence::Session> {
    ACTIVE.lock().unwrap().clone()
}

pub async fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<(), ControlError> {
    let _guard = TRANSITION.lock().await;
    let pool = db(app)?;
    persistence::initialize(&pool).await.map_err(db_error)?;
    persistence::recover(&pool).await.map_err(db_error)?;
    let store = app
        .store("recording_preferences.json")
        .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
    if let Some(language) = store
        .get("language")
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        crate::set_language_preference_internal(language)?;
    }
    Ok(())
}

pub async fn set_language<R: Runtime>(
    app: &AppHandle<R>,
    language: String,
) -> Result<(), ControlError> {
    validate_language(app, &language).await?;
    let store = app
        .store("recording_preferences.json")
        .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
    store.set("language", json!(language));
    store
        .save()
        .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
    crate::set_language_preference_internal(language)?;
    Ok(())
}
async fn validate_language<R: Runtime>(
    app: &AppHandle<R>,
    language: &str,
) -> Result<(), ControlError> {
    if language.is_empty()
        || language.len() > 16
        || !language
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-')
    {
        return Err(ControlError::new(
            "invalid_language",
            "Use a language code such as ru, en or auto",
        ));
    }
    let config = SettingsRepository::get_transcript_config(&db(app)?)
        .await
        .map_err(db_error)?;
    if config.map_or(false, |c| c.provider == "parakeet") && language != "auto" {
        return Err(ControlError::new("unsupported_language","Parakeet does not support an explicit language override; select Whisper for this language"));
    }
    Ok(())
}

/// Called directly by the worker BEFORE the transcript becomes visible.
/// Failed writes retain the complete segment in memory and prevent finalization.
pub async fn checkpoint<R: Runtime>(
    app: &AppHandle<R>,
    update: &TranscriptUpdate,
) -> Result<(), ControlError> {
    let session = active()
        .ok_or_else(|| ControlError::new("no_recording", "No recording session for transcript"))?;
    let pool = db(app)?;
    match persist_update(&pool, &session, update).await {
        Ok(()) => {
            audio::remember_transcript(update);
            super::emit_event(app,"transcript.segment",Some(&session.recording_id),Some(&session.meeting_id),json!({"sequence_id":update.sequence_id,"text":update.text,"is_partial":update.is_partial,"audio_start_time":update.audio_start_time,"audio_end_time":update.audio_end_time})).await?;
            Ok(())
        }
        Err(error) => {
            PENDING.lock().unwrap().push(update.clone());
            *FILE_DIRTY.lock().unwrap() = true;
            Err(error)
        }
    }
}
async fn persist_update(
    pool: &SqlitePool,
    session: &persistence::Session,
    u: &TranscriptUpdate,
) -> Result<(), ControlError> {
    persistence::checkpoint(
        pool,
        session,
        u.sequence_id,
        &u.text,
        &u.timestamp,
        u.audio_start_time,
        u.audio_end_time,
        u.duration,
        u.confidence,
        u.is_partial,
    )
    .await
    .map_err(db_error)
}
pub fn mark_failed(message: String) {
    *FAILURE.lock().unwrap() = Some(message);
}

pub async fn dispatch<R: Runtime>(
    app: AppHandle<R>,
    method: &str,
    params: Value,
) -> Result<Value, ControlError> {
    // A command continues on a backend-owned task even if its HTTP caller closes.
    // This avoids leaving half a start/stop when a client timeout cancels its wait.
    super::initialize_storage(&app).await?;
    let mut params = params;
    if matches!(
        method,
        "recording.stop" | "recording.pause" | "recording.resume"
    ) && params.get("recording_id").is_none()
    {
        if let Some(session) = active() {
            params["recording_id"] = json!(session.recording_id);
        }
    }
    let method = method.to_owned();
    tokio::spawn(async move { dispatch_inner(app, &method, params).await })
        .await
        .map_err(|e| ControlError::new("internal", e.to_string()))?
}
async fn dispatch_inner<R: Runtime>(
    app: AppHandle<R>,
    method: &str,
    params: Value,
) -> Result<Value, ControlError> {
    if method == "recording.status" || method == "status" {
        return session_status(&app, &params).await;
    }
    let _guard = TRANSITION.lock().await;
    let pool = db(&app)?;
    persistence::initialize(&pool).await.map_err(db_error)?;
    match method {
        "recording.start" => {
            let key = string(&params, "idempotency_key");
            if let Some(key) = &key {
                if let Some(existing) = persistence::by_key(&pool, key).await.map_err(db_error)? {
                    return Ok(json!({"recording":existing,"idempotent":true}));
                }
            }
            if active().is_some() || capture_is_recording(&app).await {
                return Err(ControlError::new(
                    "recording_active",
                    "A recording is already active",
                ));
            }
            if let Some(session_id) = string(&params, "detection_session_id") {
                crate::detection::validate_reserved_start(&app, &session_id)
                    .map_err(|e| ControlError::new("stale_session", e))?;
            }

            if let Some(language) = string(&params, "language") {
                set_language(&app, language).await?;
            } else {
                validate_language(
                    &app,
                    &crate::get_language_preference_internal().unwrap_or_else(|| "auto".into()),
                )
                .await?;
            }
            let mut prefs = crate::audio::recording_preferences::load_recording_preferences(&app)
                .await
                .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
            let mic = if params.get("input_device").is_some()
                || params.get("mic_device_name").is_some()
            {
                string(&params, "input_device").or(string(&params, "mic_device_name"))
            } else {
                prefs.preferred_mic_device.clone()
            };
            let system = if params.get("output_device").is_some()
                || params.get("system_device_name").is_some()
            {
                string(&params, "output_device").or(string(&params, "system_device_name"))
            } else {
                prefs.preferred_system_device.clone()
            };
            prefs.preferred_mic_device = mic.clone();
            prefs.preferred_system_device = system.clone();
            crate::audio::recording_preferences::save_recording_preferences(&app, &prefs)
                .await
                .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
            let title = string(&params, "name")
                .or(string(&params, "meeting_name"))
                .unwrap_or_else(|| {
                    format!(
                        "Meeting {}",
                        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
                    )
                });
            let now = chrono::Utc::now().to_rfc3339();
            let session = persistence::Session {
                recording_id: uuid::Uuid::new_v4().to_string(),
                meeting_id: uuid::Uuid::new_v4().to_string(),
                state: "starting".into(),
                initiator: string(&params, "initiator").unwrap_or_else(|| "gui".into()),
                idempotency_key: key,
                started_at: now.clone(),
                updated_at: now,
                stopped_at: None,
                finalized_at: None,
                error: None,
                detection_session_id: string(&params, "detection_session_id"),
                application: string(&params, "application"),
            };
            persistence::create(&pool, &session, &title)
                .await
                .map_err(db_error)?;
            *ACTIVE.lock().unwrap() = Some(session.clone());
            *FAILURE.lock().unwrap() = None;
            PENDING.lock().unwrap().clear();
            *DRAINED.lock().unwrap() = None;
            *CAPTURE_FOLDER.lock().unwrap() = None;
            *FILE_DIRTY.lock().unwrap() = false;
            match capture_start(app.clone(), mic, system, Some(title)).await {
                Ok(()) => {
                    *CAPTURE_FOLDER.lock().unwrap() = capture_folder(&app).await?;
                    let publication = async {
                        persistence::transition(&pool, &session.recording_id, "recording", None)
                            .await
                            .map_err(db_error)?;
                        emit(&app, "recording.started", &session, json!({})).await
                    }
                    .await;
                    if let Err(error) = publication {
                        mark_failed(format!(
                            "Recording startup could not be published: {}",
                            error.message
                        ));
                        // Stop capture directly while holding the lifecycle lock; never recurse through dispatch.
                        let _ = capture_stop(app.clone()).await;
                        *DRAINED.lock().unwrap() = Some(CAPTURE_FOLDER.lock().unwrap().clone());
                        let pending = PENDING.lock().unwrap().clone();
                        let mut remaining = Vec::new();
                        for update in pending {
                            if persist_update(&pool, &session, &update).await.is_err() {
                                remaining.push(update);
                            }
                        }
                        *PENDING.lock().unwrap() = remaining;
                        if PENDING.lock().unwrap().is_empty() && !*FILE_DIRTY.lock().unwrap() {
                            if persistence::transition(
                                &pool,
                                &session.recording_id,
                                "failed",
                                Some(&error.message),
                            )
                            .await
                            .is_ok()
                            {
                                *ACTIVE.lock().unwrap() = None;
                            }
                        }
                        return Err(ControlError::new(
                            "recording_start_failed",
                            format!(
                                "{} (recording_id={}, meeting_id={})",
                                error.message, session.recording_id, session.meeting_id
                            ),
                        ));
                    }
                    Ok(
                        json!({"recording":persistence::get(&pool,&session.recording_id).await.map_err(db_error)?}),
                    )
                }
                Err(error) => {
                    persistence::transition(&pool, &session.recording_id, "failed", Some(&error))
                        .await
                        .map_err(db_error)?;
                    *ACTIVE.lock().unwrap() = None;
                    emit(&app, "recording.failed", &session, json!({"error":error})).await?;
                    Err(ControlError::new("recording_start_failed", error))
                }
            }
        }
        "recording.stop" => {
            let session = match active() {
                Some(s) => s,
                None => {
                    let old = if let Some(id) = string(&params, "recording_id") {
                        persistence::get(&pool, &id).await.map_err(db_error)?
                    } else {
                        persistence::latest(&pool).await.map_err(db_error)?
                    };
                    return Ok(json!({"recording":old,"idempotent":true}));
                }
            };
            require_match(&session, &params)?;
            let drained = DRAINED.lock().unwrap().clone();
            let folder = if let Some(folder) = drained {
                folder
            } else {
                persistence::transition(&pool, &session.recording_id, "stopping", None)
                    .await
                    .map_err(db_error)?;
                match capture_stop(app.clone()).await {
                    Ok(folder) => {
                        *DRAINED.lock().unwrap() = Some(folder.clone());
                        folder
                    }
                    Err(error) => {
                        // Capture and workers have already stopped; retain the session
                        // so pending checkpoints and file recovery can still be retried.
                        mark_failed(error);
                        let folder = CAPTURE_FOLDER.lock().unwrap().clone();
                        *DRAINED.lock().unwrap() = Some(folder.clone());
                        folder
                    }
                }
            };
            let pending = PENDING.lock().unwrap().clone();
            let mut remaining = Vec::new();
            for update in &pending {
                if persist_update(&pool, &session, update).await.is_err() {
                    remaining.push(update.clone());
                }
            }
            *PENDING.lock().unwrap() = remaining;
            if !PENDING.lock().unwrap().is_empty() {
                return Err(ControlError::new(
                    "persistence_failed",
                    "Transcript checkpoints could not be written; retry stopping to finish saving",
                ));
            }
            if *FILE_DIRTY.lock().unwrap() {
                if let Some(folder) = &folder {
                    let history = transcript_history(&app).await?;
                    let mut dirty = FILE_DIRTY.lock().unwrap();
                    persistence::retry_transcript_file(
                        std::path::Path::new(folder),
                        &history,
                        &mut *dirty,
                    )
                    .map_err(|e| ControlError::new("persistence_failed", e.to_string()))?;
                } else {
                    *FILE_DIRTY.lock().unwrap() = false;
                }
            }
            let failure = FAILURE.lock().unwrap().clone();
            if let Some(error) = failure {
                persistence::transition(&pool, &session.recording_id, "failed", Some(&error))
                    .await
                    .map_err(db_error)?;
                *ACTIVE.lock().unwrap() = None;
                emit(&app, "recording.failed", &session, json!({"error":error})).await?;
                return Err(ControlError::new("recording_finalize_failed", error));
            }
            // Persist the completed drain and folder separately from finalization.
            // A failed finalization can then retry without reopening or losing audio.
            persistence::mark_drained(&pool, &session.recording_id, folder.as_deref())
                .await
                .map_err(db_error)?;
            persistence::finalize(&pool, &session.recording_id, folder.as_deref())
                .await
                .map_err(db_error)?;
            *ACTIVE.lock().unwrap() = None;
            *DRAINED.lock().unwrap() = None;
            // The durable finalized event is committed in the same transaction.
            let _=app.emit(&super::native_event_name("meeting.finalized"),json!({"recording_id":session.recording_id,"meeting_id":session.meeting_id,"data":{"folder_path":folder}}));
            let finalized = persistence::get(&pool, &session.recording_id)
                .await
                .map_err(db_error)?;
            let _=app.emit("recording-stopped",json!({"message":"Recording saved","recording_id":session.recording_id,"meeting_id":session.meeting_id,"folder_path":folder,"state":"finalized"}));
            Ok(json!({"recording":finalized}))
        }
        "recording.pause" | "recording.resume" => {
            let session = active()
                .ok_or_else(|| ControlError::new("no_recording", "No recording is active"))?;
            require_match(&session, &params)?;
            let target = if method == "recording.pause" {
                "paused"
            } else {
                "recording"
            };
            let current = persistence::get(&pool, &session.recording_id)
                .await
                .map_err(db_error)?
                .unwrap();
            if current.state != target {
                let result = if method == "recording.pause" {
                    capture_pause(app.clone()).await
                } else {
                    capture_resume(app.clone()).await
                };
                result.map_err(|e| ControlError::new("recording_transition_failed", e))?;
                persistence::transition(&pool, &session.recording_id, target, None)
                    .await
                    .map_err(db_error)?;
                emit(
                    &app,
                    if target == "paused" {
                        "recording.paused"
                    } else {
                        "recording.resumed"
                    },
                    &session,
                    json!({}),
                )
                .await?;
            }
            Ok(
                json!({"recording":persistence::get(&pool,&session.recording_id).await.map_err(db_error)?}),
            )
        }
        _ => Err(ControlError::new("unknown_method", method)),
    }
}
fn string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}
fn require_match(s: &persistence::Session, params: &Value) -> Result<(), ControlError> {
    if string(params, "recording_id").map_or(false, |id| id != s.recording_id)
        || string(params, "detection_session_id")
            .map_or(false, |id| Some(id) != s.detection_session_id)
    {
        return Err(ControlError::new(
            "recording_mismatch",
            "The requested recording is not the active session",
        ));
    }
    Ok(())
}
async fn emit<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
    s: &persistence::Session,
    data: Value,
) -> Result<(), ControlError> {
    super::emit_event(app, event, Some(&s.recording_id), Some(&s.meeting_id), data).await
}

pub async fn transcript_history<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<crate::audio::recording_saver::TranscriptSegment>, String> {
    let session = active().ok_or_else(|| "No active recording session".to_string())?;
    let pool = db(app).map_err(|e| e.message)?;
    let rows:Vec<(i64,String,String,f64,f64,f64,f32)> =sqlx::query_as("SELECT s.sequence_id,t.transcript,t.timestamp,t.audio_start_time,t.audio_end_time,t.duration,s.confidence FROM recording_segments s JOIN transcripts t ON t.id=s.transcript_id WHERE s.recording_id=? ORDER BY s.sequence_id")
        .bind(&session.recording_id).fetch_all(&pool).await.map_err(|e|e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, text, time, start, end, duration, confidence)| {
            crate::audio::recording_saver::TranscriptSegment {
                id: format!("seg_{id}"),
                sequence_id: id as u64,
                text,
                display_time: time,
                audio_start_time: start,
                audio_end_time: end,
                duration,
                confidence,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn get_recording_session<R: Runtime>(app: AppHandle<R>) -> Result<Value, String> {
    dispatch(app, "recording.status", json!({}))
        .await
        .map_err(|e| e.message)
}

/// The capture boundary is distinct from worker drain and durable finalization.
pub async fn mark_capture_stopped<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let session = active().ok_or_else(|| "No active recording session".to_string())?;
    let pool = db(app).map_err(|e| e.message)?;
    persistence::transition(&pool, &session.recording_id, "processing", None)
        .await
        .map_err(|e| e.to_string())?;
    emit(app, "recording.stopped", &session, json!({}))
        .await
        .map_err(|e| e.message)
}

/// Lightweight status read also used by tray refreshes, without reentering dispatch.
pub async fn session_status<R: Runtime>(
    app: &AppHandle<R>,
    params: &Value,
) -> Result<Value, ControlError> {
    let pool = db(app)?;
    let selected = if let Some(id) = params.get("recording_id").and_then(Value::as_str) {
        persistence::get(&pool, id).await.map_err(db_error)?
    } else if let Some(s) = active() {
        persistence::get(&pool, &s.recording_id)
            .await
            .map_err(db_error)?
    } else {
        persistence::latest(&pool).await.map_err(db_error)?
    };
    Ok(json!({"recording":selected}))
}

pub fn validate_capture_start<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(session_id) = active().and_then(|s| s.detection_session_id) {
        crate::detection::validate_reserved_start(app, &session_id)?;
    }
    Ok(())
}

// These wrappers preserve the production capture calls. A test-only, per-app
// recorder fixture controls the hardware boundary while exercising dispatch,
// durable persistence, checkpointing and finalization unchanged.
async fn capture_start<R: Runtime>(
    app: AppHandle<R>,
    mic: Option<String>,
    system: Option<String>,
    title: Option<String>,
) -> Result<(), String> {
    #[cfg(test)]
    if let Some(recorder) = app.try_state::<headless::Recorder>() {
        return recorder.start().await;
    }
    audio::start_recording_raw(app, mic, system, title).await
}
async fn capture_stop<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    #[cfg(test)]
    if let Some(recorder) = app.try_state::<headless::Recorder>() {
        return recorder.stop(&app).await;
    }
    audio::stop_recording_raw(app).await
}
async fn capture_pause<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    #[cfg(test)]
    if app.try_state::<headless::Recorder>().is_some() {
        return Ok(());
    }
    audio::pause_recording_raw(app).await
}
async fn capture_resume<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    #[cfg(test)]
    if app.try_state::<headless::Recorder>().is_some() {
        return Ok(());
    }
    audio::resume_recording_raw(app).await
}
async fn capture_is_recording<R: Runtime>(app: &AppHandle<R>) -> bool {
    #[cfg(test)]
    if let Some(recorder) = app.try_state::<headless::Recorder>() {
        return recorder.is_recording();
    }
    let _ = app;
    audio::is_recording().await
}
async fn capture_folder<R: Runtime>(app: &AppHandle<R>) -> Result<Option<String>, String> {
    #[cfg(test)]
    if app.try_state::<headless::Recorder>().is_some() {
        return Ok(None);
    }
    let _ = app;
    audio::get_meeting_folder_path().await
}
#[cfg(test)]
#[path = "recording_headless.rs"]
mod headless;
