//! Shared desktop integration entry point. No second recorder or SQLite connection.
pub mod gateway;
pub mod recording;
pub use gateway::{
    get_integration_settings, get_login_start, rotate_integration_keys, set_integration_settings,
    set_login_start,
};
pub use meetily_local_control::ControlError;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Dotted event identifiers belong to the durable gateway protocol. Tauri
/// restricts native event names, so its transport uses the colon equivalent.
pub fn native_event_name(event: &str) -> String {
    event.replace('.', ":")
}

pub async fn emit_event<R: Runtime>(
    app: &AppHandle<R>,
    event: &str,
    recording_id: Option<&str>,
    meeting_id: Option<&str>,
    data: Value,
) -> Result<(), ControlError> {
    let state = app
        .try_state::<crate::state::AppState>()
        .ok_or_else(|| ControlError::new("unavailable", "Database is not ready"))?;
    meetily_local_control::store::emit_event(
        state.db_manager.pool(),
        event,
        recording_id,
        meeting_id,
        data.clone(),
    )
    .await?;
    app.emit(
        &native_event_name(event),
        serde_json::json!({"recording_id":recording_id,"meeting_id":meeting_id,"data":data}),
    )
    .map_err(|e| ControlError::new("internal", e.to_string()))?;
    Ok(())
}
static READY: tokio::sync::Mutex<bool> = tokio::sync::Mutex::const_new(false);
pub async fn initialize_storage<R: Runtime>(app: &AppHandle<R>) -> Result<(), ControlError> {
    let mut ready = READY.lock().await;
    if *ready {
        return Ok(());
    }
    let state = app
        .try_state::<crate::state::AppState>()
        .ok_or_else(|| ControlError::new("unavailable", "Complete initial setup in Meetily"))?;
    meetily_local_control::store::initialize(state.db_manager.pool()).await?;
    recording::initialize(app).await?;
    *ready = true;
    Ok(())
}
pub async fn start<R: Runtime>(app: AppHandle<R>) -> Result<(), ControlError> {
    use tauri::Listener;
    if app.try_state::<crate::state::AppState>().is_some() {
        initialize_storage(&app).await?;
    }
    let on_ready = app.clone();
    app.listen("database-initialized", move |_| {
        let app = on_ready.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = initialize_storage(&app).await {
                log::error!("Control storage initialization failed: {e}")
            }
        });
    });
    gateway::start(app).await
}
