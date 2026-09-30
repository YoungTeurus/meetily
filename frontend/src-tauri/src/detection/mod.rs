//! Detection observes; the shared backend recorder owns every recording operation.
use call_detection::{
    session::{Engine, Prompt},
    CallState, Observation, Settings,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

struct Inner {
    settings: Settings,
    engine: Engine,
    observations: Vec<Observation>,
    latest_sample_ms: u64,
    recording: Option<Value>,
    error: Option<String>,
}
pub struct DetectionState {
    inner: Mutex<Inner>,
    actions: Arc<tokio::sync::Mutex<()>>,
    settings_path: PathBuf,
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn utc(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
fn snapshot(inner: &Inner) -> Value {
    let observations: Vec<Value> = inner
        .observations
        .iter()
        .map(|o| {
            let mut value = serde_json::to_value(o).unwrap_or(Value::Null);
            if let Some(map) = value.as_object_mut() {
                map.remove("observed_at_ms");
                map.insert("observed_at".into(), json!(utc(o.observed_at_ms)));
            }
            value
        })
        .collect();
    let prompts: Vec<Prompt> = if inner.settings.enabled {
        inner
            .engine
            .prompts()
            .into_iter()
            .filter(|p| inner.settings.applications.contains(&p.application))
            .collect()
    } else {
        vec![]
    };
    json!({"settings":inner.settings,"observations":observations,"sessions":inner.engine.sessions(),
        "prompts":prompts,
        "platform":std::env::consts::OS,"recording":inner.recording,"error":inner.error,
        "observed_at":utc(inner.latest_sample_ms),"observer_interval_ms":1000})
}
/// Snapshot is generic so the gateway can use it without depending on the UI runtime.
#[tauri::command]
pub fn get_detection_status<R: Runtime>(app: AppHandle<R>) -> Result<Value, String> {
    let state = app
        .try_state::<DetectionState>()
        .ok_or("Detection observer has not initialized")?;
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Detection state unavailable")?;
    Ok(snapshot(&inner))
}
#[tauri::command]
pub fn set_detection_settings(app: AppHandle, settings: Value) -> Result<Value, String> {
    let settings: Settings =
        serde_json::from_value(settings).map_err(|e| format!("Invalid detection settings: {e}"))?;
    settings.validate()?;
    let state = app
        .try_state::<DetectionState>()
        .ok_or("Detection observer has not initialized")?;
    let data = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    if let Some(parent) = state.settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Settings contain no secrets; a failed write must not silently change the effective settings.
    std::fs::write(&state.settings_path, &data)
        .map_err(|e| format!("Could not save detection settings: {e}"))?;
    let value = {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "Detection state unavailable")?;
        if !settings.enabled {
            inner.engine.disable();
        }
        for session in inner.engine.sessions() {
            if !settings.applications.contains(&session.application) {
                inner.engine.suppress(&session.session_id)?;
            }
        }
        inner.settings = settings;
        snapshot(&inner)
    };
    let _ = app.emit("detection-changed", &value);
    Ok(value)
}
fn active_recording(value: &Value) -> Option<Value> {
    let r = value.get("recording")?;
    if r.is_null() {
        return None;
    }
    if [
        "recording",
        "paused",
        "starting",
        "stopping",
        "finalizing",
        "processing",
    ]
    .contains(&r.get("state").and_then(Value::as_str).unwrap_or(""))
    {
        Some(r.clone())
    } else {
        None
    }
}
async fn current_recording(app: &AppHandle) -> Result<Option<Value>, String> {
    crate::control::recording::dispatch(app.clone(), "recording.status", json!({}))
        .await
        .map(|v| active_recording(&v))
        .map_err(|e| format!("Cannot inspect recording: {e:?}"))
}
fn validate_current_observation(inner: &Inner, id: &str, state: CallState) -> Result<(), String> {
    let session = inner
        .engine
        .sessions()
        .into_iter()
        .find(|s| s.session_id == id)
        .ok_or("stale_detection_session")?;
    if !inner.settings.enabled || !inner.settings.applications.contains(&session.application) {
        return Err("Detection for this application is disabled".into());
    }
    let observation = inner
        .observations
        .iter()
        .find(|o| o.application == session.application)
        .ok_or("No current application observation")?;
    if state == CallState::ConfirmedCall {
        inner
            .engine
            .validate_identity(id, observation.process_identity.as_deref())?;
    }
    if observation.state != state || now_ms().saturating_sub(observation.observed_at_ms) > 5_000 {
        return Err("Call observation is stale or changed; wait for a fresh observation".into());
    }
    Ok(())
}
/// Called by the recorder before a notification start and immediately before capture,
/// including after model loading. GUI/CLI starts do not require a detector session.
pub fn validate_reserved_start<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
) -> Result<(), String> {
    let state = app
        .try_state::<DetectionState>()
        .ok_or("Detection observer has not initialized")?;
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Detection state unavailable")?;
    validate_current_observation(&inner, session_id, CallState::ConfirmedCall)?;
    let session = inner
        .engine
        .sessions()
        .into_iter()
        .find(|s| s.session_id == session_id)
        .ok_or("stale_detection_session")?;
    let observation = inner
        .observations
        .iter()
        .find(|o| o.application == session.application)
        .ok_or("No current application observation")?;
    inner
        .engine
        .validate_reserved_start(session_id, observation.process_identity.as_deref())?;
    Ok(())
}
#[tauri::command]
pub async fn detection_action(
    app: AppHandle,
    session_id: String,
    action: String,
) -> Result<Value, String> {
    let state = app
        .try_state::<DetectionState>()
        .ok_or("Detection observer has not initialized")?;
    let actions = state.actions.clone();
    let action_guard = actions.lock().await;
    match action.as_str() {
        "skip" | "dismiss" => {
            state
                .inner
                .lock()
                .map_err(|_| "Detection state unavailable")?
                .engine
                .suppress(&session_id)?;
        }
        "start" => {
            let current = current_recording(&app).await?;
            if current.is_some() {
                return Err("A recording is already in progress".into());
            }
            // Restore focus only after the user's Start click, and only if native
            // evidence is currently inconclusive. Cached audio never authorizes capture.
            let (application, identity) = {
                let inner = state
                    .inner
                    .lock()
                    .map_err(|_| "Detection state unavailable")?;
                let identity = inner.engine.revalidation_identity(&session_id)?;
                let session = inner
                    .engine
                    .sessions()
                    .into_iter()
                    .find(|s| s.session_id == session_id)
                    .ok_or("stale_detection_session")?;
                if !inner.settings.enabled
                    || !inner.settings.applications.contains(&session.application)
                {
                    return Err("Detection for this application is disabled".into());
                }
                (session.application, identity)
            };
            log::info!(target:"call_detection","explicit start revalidation session={} app={}",session_id,application);
            let fresh = tauri::async_runtime::spawn_blocking(move || {
                call_detection::native::revalidate(&application, &identity, now_ms())
            })
            .await
            .map_err(|e| format!("Could not verify call: {e}"))??;
            log::info!(target:"call_detection","explicit revalidation session={} state={:?} process={:?} evidence={:?} limitations={:?}",session_id,fresh.state,fresh.process_identity,fresh.evidence,fresh.limitations);
            {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "Detection state unavailable")?;
                let settings = inner.settings.clone();
                let at = now_ms();
                let accepted =
                    call_detection::merge_latest_observations(&mut inner.observations, vec![fresh]);
                inner.engine.update(&accepted, &settings, at);
                inner.latest_sample_ms = inner.latest_sample_ms.max(at);
            }
            let application = {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "Detection state unavailable")?;
                if !inner.settings.enabled
                    || now_ms().saturating_sub(inner.latest_sample_ms) > 5_000
                {
                    return Err(
                        "Call observation is disabled or stale; wait for a fresh observation"
                            .into(),
                    );
                }
                let application = inner.engine.authorize_start(&session_id)?;
                validate_current_observation(&inner, &session_id, CallState::ConfirmedCall)?;
                if inner.engine.sessions().iter().any(|s| s.starting) {
                    return Err("Another recording is starting".into());
                }
                inner.engine.reserve_start(&session_id)?;
                application
            };
            drop(action_guard);
            let result=crate::control::recording::dispatch(app.clone(),"recording.start",json!({"initiator":"notification","detection_session_id":session_id,"application":application,"name":format!("{application} call")})).await;
            let value = match result {
                Ok(v) => v,
                Err(e) => {
                    state
                        .inner
                        .lock()
                        .map_err(|_| "Detection state unavailable")?
                        .engine
                        .start_failed(&session_id);
                    return Err(format!("Could not start recording: {e:?}"));
                }
            };
            let id = value
                .get("recording_id")
                .or_else(|| value.get("recording").and_then(|r| r.get("recording_id")))
                .and_then(Value::as_str)
                .ok_or("Recording service did not return a recording ID")?;
            let mut inner = state
                .inner
                .lock()
                .map_err(|_| "Detection state unavailable")?;
            inner.engine.attach_recording(&session_id, id)?;
            inner.recording = active_recording(&value);
        }
        "stop" => {
            let current = current_recording(&app).await?;
            let id = {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "Detection state unavailable")?;
                if now_ms().saturating_sub(inner.latest_sample_ms) > 5_000 {
                    return Err("Call observation is stale; wait for a fresh observation".into());
                }
                validate_current_observation(&inner, &session_id, CallState::NoCall)?;
                inner.engine.authorize_stop(
                    &session_id,
                    current
                        .as_ref()
                        .and_then(|r| r.get("recording_id"))
                        .and_then(Value::as_str),
                )?
            };
            drop(action_guard);
            let result = crate::control::recording::dispatch(
                app.clone(),
                "recording.stop",
                json!({"recording_id":id,"detection_session_id":session_id}),
            )
            .await
            .map_err(|e| format!("Could not stop recording: {e:?}"))?;
            state
                .inner
                .lock()
                .map_err(|_| "Detection state unavailable")?
                .engine
                .recording_finished(&id);
            let status = get_detection_status(app.clone())?;
            let _ = app.emit("detection-changed", status);
            return Ok(result);
        }
        _ => return Err("Unknown detection action; use start, skip, dismiss, or stop".into()),
    }
    let value = get_detection_status(app.clone())?;
    let _ = app.emit("detection-changed", &value);
    Ok(value)
}
async fn apply_observations(app: AppHandle, observations: Vec<Observation>, at: u64) {
    let Some(state) = app.try_state::<DetectionState>() else {
        return;
    };
    let recording_result = current_recording(&app).await;
    let (fresh, value, settings) = {
        let Ok(mut inner) = state.inner.lock() else {
            return;
        };
        for observation in &observations {
            let changed = inner
                .observations
                .iter()
                .find(|o| o.application == observation.application)
                .map(|prior| {
                    prior.state != observation.state
                        || prior.process_identity != observation.process_identity
                        || prior.limitations != observation.limitations
                })
                .unwrap_or(true);
            if changed {
                log::info!(target:"call_detection","observation app={} state={:?} process={:?} evidence={:?} limitations={:?}",observation.application,observation.state,observation.process_identity,observation.evidence,observation.limitations);
            }
        }
        let current = match recording_result {
            Ok(r) => {
                inner.error = None;
                r
            }
            Err(e) => {
                inner.error = Some(e);
                inner.recording.clone()
            }
        };
        let current_id = current
            .as_ref()
            .and_then(|r| r.get("recording_id"))
            .and_then(Value::as_str);
        for s in inner.engine.sessions() {
            if let Some(id) = s.recording_id {
                if Some(id.as_str()) != current_id {
                    inner.engine.recording_finished(&id);
                }
            }
        }
        inner.recording = current.clone();
        let accepted =
            call_detection::merge_latest_observations(&mut inner.observations, observations);
        inner.latest_sample_ms = inner.latest_sample_ms.max(at);
        let settings = inner.settings.clone();
        let previous_sessions = inner.engine.sessions();
        let mut fresh = inner.engine.update(&accepted, &settings, at);
        // Any active recording consumes the current session's offer. Its manual stop cannot re-offer.
        if current.is_some() {
            for s in inner.engine.sessions() {
                if s.recording_id.is_none() && !s.starting {
                    let _ = inner.engine.suppress(&s.session_id);
                }
            }
            fresh.retain(|p| p.kind != "start");
        }
        for session in inner.engine.sessions() {
            if previous_sessions
                .iter()
                .find(|s| s.session_id == session.session_id)
                .map(|prior| prior.phase != session.phase || prior.suppressed != session.suppressed)
                .unwrap_or(true)
            {
                log::info!(target:"call_detection","session={} phase={:?} suppressed={} starting={} prompts={:?}",session.session_id,session.phase,session.suppressed,session.starting,inner.engine.prompts());
            }
        }
        (fresh, snapshot(&inner), settings)
    };
    let _ = app.emit("detection-changed", &value);
    for Prompt {
        kind,
        session_id,
        application,
    } in fresh
    {
        if kind == "stop" && settings.auto_stop {
            // Same guarded command checks the current owner again, so an independent recorder is safe.
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = detection_action(app, session_id, "stop".into()).await {
                    log::warn!("Call-linked auto-stop declined: {e}");
                }
            });
        } else if settings.notification_mode == "system" {
            if let Err(e) =
                crate::call_notifications::show_prompt(&app, &session_id, &kind, &application)
            {
                log::warn!("Call notification unavailable: {e}");
            }
        }
    }
}
/// App lifetime owns the observer thread. No background process survives application exit.
pub fn start(app: AppHandle) -> Result<(), String> {
    if app.try_state::<DetectionState>().is_some() {
        return Ok(());
    }
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("detection.json");
    let settings = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice::<Settings>(&d).ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default();
    app.manage(DetectionState {
        inner: Mutex::new(Inner {
            settings,
            engine: Engine::default(),
            observations: vec![],
            latest_sample_ms: 0,
            recording: None,
            error: None,
        }),
        actions: Arc::new(tokio::sync::Mutex::new(())),
        settings_path: path,
    });
    std::thread::Builder::new()
        .name("call-observer".into())
        .spawn(move || {
            let mut observer = call_detection::native::create();
            loop {
                let enabled = app
                    .try_state::<DetectionState>()
                    .and_then(|s| s.inner.lock().ok().map(|i| i.settings.enabled))
                    .unwrap_or(false);
                if enabled {
                    let observations = observer.observe(now_ms());
                    let at = now_ms();
                    // Await integration before sampling again; never build an unbounded queue of observations.
                    tauri::async_runtime::block_on(apply_observations(
                        app.clone(),
                        observations,
                        at,
                    ));
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
