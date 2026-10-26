use super::{recording, ControlError};
use meetily_local_control::gateway::{Credentials, Dispatcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{Mutex, RwLock};

#[derive(Default, Serialize, Deserialize, Clone)]
struct Preferences {
    enabled: bool,
    allow_control: bool,
}
pub struct IntegrationState {
    credentials: Arc<RwLock<Credentials>>,
    port: u16,
    directory: PathBuf,
    update: Mutex<()>,
}
struct Desktop<R: Runtime>(AppHandle<R>);
#[async_trait::async_trait]
impl<R: Runtime> Dispatcher for Desktop<R> {
    async fn dispatch(&self, method: &str, params: Value) -> Result<Value, ControlError> {
        match method {
            "doctor" => {
                let devices = crate::audio::list_audio_devices().await;
                let model = if self.0.try_state::<crate::state::AppState>().is_some() {
                    crate::audio::transcription::validate_transcription_model_ready(&self.0).await
                } else {
                    Err("Complete initial setup in Meetily".into())
                };
                let (devices, device_error) = match devices {
                    Ok(d) => (Some(d), None),
                    Err(e) => (None, Some(e.to_string())),
                };
                Ok(
                    json!({"version":1,"platform":std::env::consts::OS,"app_version":self.0.package_info().version.to_string(),"database_ready":self.0.try_state::<crate::state::AppState>().is_some(),"model_ready":model.is_ok(),"model_error":model.err().map(|e|e.to_string()),"devices":devices,"device_error":device_error,"system_audio_mode":"whole_computer","microphone_permission":"checked_when_recording_starts","screen_recording_permission":crate::audio::permissions::check_screen_recording_permission(),"detection":crate::detection::get_detection_status(self.0.clone())?}),
                )
            }
            "devices.list" => serde_json::to_value(
                crate::audio::list_audio_devices()
                    .await
                    .map_err(|e| ControlError::new("unavailable", e.to_string()))?,
            )
            .map_err(|e| ControlError::new("internal", e.to_string())),
            "detection.status" => {
                crate::detection::get_detection_status(self.0.clone()).map_err(Into::into)
            }
            _ => {
                super::initialize_storage(&self.0).await?;
                if method == "status" || method.starts_with("recording.") {
                    recording::dispatch(self.0.clone(), method, params).await
                } else {
                    let state = self.0.state::<crate::state::AppState>();
                    meetily_local_control::store::dispatch(state.db_manager.pool(), method, &params)
                        .await
                }
            }
        }
    }
}
fn io_error(e: impl std::fmt::Display) -> ControlError {
    ControlError::new("internal", e.to_string())
}
/// Create restricted temporary file before writing any secret. Rename into place only
/// after permissions are correct. Windows uses the account SID rather than a name.
fn private_json(path: &Path, value: &Value) -> Result<(), ControlError> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("Invalid credential directory"))?;
    std::fs::create_dir_all(parent).map_err(io_error)?;
    let tmp = parent.join(format!(".integration-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(io_error)?;
    #[cfg(windows)]
    {
        let account = std::process::Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .map_err(io_error)?;
        let output = String::from_utf8_lossy(&account.stdout);
        let sid = output
            .trim()
            .split(',')
            .nth(1)
            .map(|s| s.trim_matches('"'))
            .filter(|s| s.starts_with("S-1-"))
            .ok_or_else(|| {
                io_error("Unable to determine account SID for credential permissions")
            })?;
        let acl = std::process::Command::new("icacls.exe")
            .arg(&tmp)
            .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(F)")])
            .output()
            .map_err(io_error)?;
        if !acl.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_error("Unable to protect integration credentials"));
        }
    }
    let result = (|| {
        file.write_all(
            serde_json::to_string_pretty(value)
                .map_err(io_error)?
                .as_bytes(),
        )
        .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        // Windows rename cannot replace; the old token has already been revoked in memory.
        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(path).map_err(io_error)?;
        }
        std::fs::rename(&tmp, path).map_err(io_error)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
async fn apply(state: &IntegrationState, p: Preferences) -> Result<Value, ControlError> {
    let _serial = state.update.lock().await;
    // Revoke old keys first; fail closed if writing the new credential file fails.
    let mut auth = state.credentials.write().await;
    *auth = Credentials::default();
    private_json(
        &state.directory.join("integration-settings.json"),
        &serde_json::to_value(&p).map_err(io_error)?,
    )?;
    let file = state.directory.join("integration.json");
    if p.enabled {
        let next = Credentials {
            enabled: true,
            read_token: meetily_local_control::gateway::token(),
            control_token: if p.allow_control {
                Some(meetily_local_control::gateway::token())
            } else {
                None
            },
        };
        private_json(
            &file,
            &json!({"version":1,"port":state.port,"read_token":next.read_token,"control_token":next.control_token}),
        )?;
        *auth = next;
    } else if file.exists() {
        std::fs::remove_file(&file).map_err(io_error)?;
    }
    Ok(
        json!({"enabled":auth.enabled,"allow_control":auth.control_token.is_some(),"credential_file":file,"port":state.port}),
    )
}
pub async fn start<R: Runtime>(app: AppHandle<R>) -> Result<(), ControlError> {
    let directory = app.path().app_data_dir().map_err(io_error)?;
    let prefs = match std::fs::read(directory.join("integration-settings.json")) {
        Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes).map_err(io_error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
        Err(e) => return Err(io_error(e)),
    };
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(io_error)?;
    let port = listener.local_addr().map_err(io_error)?.port();
    let credentials = Arc::new(RwLock::new(Credentials::default()));
    let state = IntegrationState {
        credentials: credentials.clone(),
        port,
        directory,
        update: Mutex::new(()),
    };
    apply(&state, prefs).await?;
    app.manage(state);
    let router = meetily_local_control::gateway::router(Arc::new(Desktop(app)), credentials, port);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            log::error!("Local control server stopped: {e}")
        }
    });
    Ok(())
}
#[tauri::command]
pub async fn get_integration_settings<R: Runtime>(app: AppHandle<R>) -> Result<Value, String> {
    let state = app
        .try_state::<IntegrationState>()
        .ok_or("Local control is starting")?;
    let auth = state.credentials.read().await;
    Ok(
        json!({"enabled":auth.enabled,"allow_control":auth.control_token.is_some(),"credential_file":state.directory.join("integration.json"),"port":state.port}),
    )
}
#[tauri::command]
pub async fn set_integration_settings<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
    allow_control: bool,
) -> Result<Value, String> {
    let state = app
        .try_state::<IntegrationState>()
        .ok_or("Local control is starting")?;
    apply(
        &state,
        Preferences {
            enabled,
            allow_control,
        },
    )
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn rotate_integration_keys<R: Runtime>(app: AppHandle<R>) -> Result<Value, String> {
    let state = app
        .try_state::<IntegrationState>()
        .ok_or("Local control is starting")?;
    let prefs = {
        let a = state.credentials.read().await;
        Preferences {
            enabled: a.enabled,
            allow_control: a.control_token.is_some(),
        }
    };
    apply(&state, prefs).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub fn get_login_start<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}
#[tauri::command]
pub fn set_login_start<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|e| e.to_string())
}
