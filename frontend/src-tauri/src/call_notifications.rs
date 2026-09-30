//! Call notifications never start capture. Activation opens a separate action window;
//! every button delegates to the detector's session-validated recording service.
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
#[cfg(not(target_os = "windows"))]
use tauri_plugin_notification::NotificationExt;

static WINDOW_SESSIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn open_action_window(app: &AppHandle, session_id: &str) -> Result<(), String> {
    if let Ok(mut sessions) = WINDOW_SESSIONS.lock() {
        if !sessions.iter().any(|session| session == session_id) {
            sessions.push(session_id.to_string());
        }
    }
    if let Some(window) = app.get_webview_window("call-action") {
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    let route = if tauri::is_dev() {
        "call-action"
    } else {
        "call-action.html"
    };
    let window = WebviewWindowBuilder::new(app, "call-action", WebviewUrl::App(route.into()))
        .title("Meetily — звонок")
        .inner_size(470.0, 310.0)
        .min_inner_size(390.0, 260.0)
        .resizable(true)
        .always_on_top(true)
        .build()
        .map_err(|error| error.to_string())?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::CloseRequested { .. }) {
            // Closing the shared compact window dismisses every visible proposal,
            // including simultaneous apps whose toast was not individually clicked.
            let mut sessions = WINDOW_SESSIONS
                .lock()
                .map(|mut ids| std::mem::take(&mut *ids))
                .unwrap_or_default();
            if let Ok(status) = crate::detection::get_detection_status(handle.clone()) {
                if let Some(prompts) = status.get("prompts").and_then(|value| value.as_array()) {
                    for prompt in prompts {
                        if let Some(id) = prompt.get("session_id").and_then(|value| value.as_str())
                        {
                            if !sessions.iter().any(|session| session == id) {
                                sessions.push(id.to_string());
                            }
                        }
                    }
                }
            }
            let app = handle.clone();
            tauri::async_runtime::spawn(async move {
                for session_id in sessions {
                    let _ = crate::detection::detection_action(
                        app.clone(),
                        session_id,
                        "dismiss".into(),
                    )
                    .await;
                }
            });
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_call_action(app: AppHandle, session_id: String) -> Result<(), String> {
    open_action_window(&app, &session_id)
}

#[tauri::command]
pub async fn close_call_action(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("call-action") {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn show_prompt(
    app: &AppHandle,
    session_id: &str,
    kind: &str,
    application: &str,
) -> Result<(), String> {
    let application = match application {
        "zoom" => "Zoom",
        "discord" => "Discord",
        "teams" => "Microsoft Teams",
        "browser" => "Браузер",
        other => other,
    };
    let title = if kind == "stop" {
        "Звонок завершён"
    } else {
        "Обнаружен звонок"
    };
    let body = if kind == "stop" {
        format!("{application}: нажмите, чтобы завершить запись и сохранить встречу.")
    } else {
        format!("{application}: нажмите, чтобы начать запись или пропустить. Записывается звук всего компьютера.")
    };
    #[cfg(target_os = "windows")]
    {
        match windows_toast(app, session_id, title, &body) {
            Ok(()) => return Ok(()),
            Err(error) => {
                log::warn!("Native call notification unavailable; opening actions: {error}")
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Desktop Tauri notification API has no macOS activation callback. Expose actions
        // immediately instead of advertising buttons that cannot invoke the recorder.
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            log::warn!("Call notification failed; opening actions: {error}");
        }
    }
    open_action_window(app, session_id)
}

#[cfg(target_os = "windows")]
fn windows_toast(app: &AppHandle, session_id: &str, title: &str, body: &str) -> Result<(), String> {
    use windows::{
        core::{IInspectable, HSTRING},
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{
            ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification,
            ToastNotificationManager,
        },
    };
    // Keep the WinRT objects alive while Action Center can activate the notification.
    static TOASTS: Mutex<Vec<ToastNotification>> = Mutex::new(Vec::new());
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
    let document = XmlDocument::new().map_err(|error| error.to_string())?;
    document.LoadXml(&HSTRING::from(format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>", escape(title), escape(body)))).map_err(|error| error.to_string())?;
    let notification =
        ToastNotification::CreateToastNotification(&document).map_err(|error| error.to_string())?;
    let clicked_app = app.clone();
    let clicked_session = session_id.to_string();
    notification
        .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                let app = clicked_app.clone();
                let session_id = clicked_session.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = open_action_window(&app, &session_id) {
                        log::error!("Cannot open call actions after toast activation: {error}");
                    }
                });
                Ok(())
            },
        ))
        .map_err(|error| error.to_string())?;
    let dismissed_app = app.clone();
    let dismissed_session = session_id.to_string();
    notification
        .Dismissed(&TypedEventHandler::<
            ToastNotification,
            ToastDismissedEventArgs,
        >::new(move |_, args| {
            if args.as_ref().and_then(|args| args.Reason().ok())
                == Some(ToastDismissalReason::UserCanceled)
            {
                let app = dismissed_app.clone();
                let session_id = dismissed_session.clone();
                tauri::async_runtime::spawn(async move {
                    let _ =
                        crate::detection::detection_action(app, session_id, "dismiss".into()).await;
                });
            }
            Ok(())
        }))
        .map_err(|error| error.to_string())?;
    let failed_app = app.clone();
    let failed_session = session_id.to_string();
    notification
        .Failed(
            &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, _| {
                let app = failed_app.clone();
                let session_id = failed_session.clone();
                tauri::async_runtime::spawn(async move {
                    log::warn!("Windows rejected call notification; opening compact actions");
                    if let Err(error) = open_action_window(&app, &session_id) {
                        log::error!("Call action fallback failed: {error}");
                    }
                });
                Ok(())
            }),
        )
        .map_err(|error| error.to_string())?;
    // NSIS/WiX shortcut registers this product's AppUserModelID. Development runs may
    // lack that identity, in which case we fall back to the compact window.
    let application_id = HSTRING::from(&app.config().identifier);
    unsafe {
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(windows::core::PCWSTR(
            application_id.as_ptr(),
        ))
        .map_err(|error| error.to_string())?;
    }
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
        &app.config().identifier,
    ))
    .map_err(|error| error.to_string())?;
    notifier
        .Show(&notification)
        .map_err(|error| error.to_string())?;
    if let Ok(mut notifications) = TOASTS.lock() {
        notifications.push(notification);
        // Bounded retention: old notifications remain harmless because the detector
        // rejects ended sessions, even if the OS delivers a delayed activation.
        if notifications.len() > 32 {
            notifications.remove(0);
        }
    }
    Ok(())
}

/// Opens the fixed OS permission pane only in response to an explicit UI action.
#[tauri::command]
pub fn open_detection_permissions() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("Не удалось открыть настройки Универсального доступа".into());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Настройка Универсального доступа требуется только на macOS".into())
}
