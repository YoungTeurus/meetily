//! Backend-owned post-call queue. GUI lifetime never controls accepted work.
use super::{automatic_retranscription_store as store, retranscription};
use crate::state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Runtime};

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static PREEMPTED: AtomicBool = AtomicBool::new(false);
static USER_CANCELLED: AtomicBool = AtomicBool::new(false);
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<sqlx::SqlitePool, String> {
    Ok(app
        .try_state::<AppState>()
        .ok_or("Database not initialized")?
        .db_manager
        .pool()
        .clone())
}
pub(crate) fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}
pub(crate) fn preempt_for_recording() {
    if is_running() {
        PREEMPTED.store(true, Ordering::SeqCst);
        retranscription::preempt_automatic();
    }
}
pub(crate) fn mark_user_cancelled() {
    USER_CANCELLED.store(true, Ordering::SeqCst);
}
pub(crate) fn was_preempted() -> bool {
    PREEMPTED.load(Ordering::SeqCst)
}

pub async fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let db = match pool(app) {
        Ok(p) => p,
        Err(e) => {
            INITIALIZED.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    if let Err(e) = store::initialize(&db).await {
        INITIALIZED.store(false, Ordering::SeqCst);
        return Err(e);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = process_next(&app, &db).await {
                log::error!("Automatic retranscription queue: {e}");
            }
            tokio::select! { _=WAKE.notified()=>{}, _=tokio::time::sleep(std::time::Duration::from_secs(2))=>{} }
        }
    });
    Ok(())
}
async fn process_next<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
) -> Result<(), String> {
    let Some(job) = store::next(pool).await? else {
        return Ok(());
    };
    let Ok(_batch) = crate::control::recording::reserve_automatic_batch() else {
        return Ok(());
    };
    if !store::transition(pool, &job.job_id, "queued", "running", None).await? {
        return Ok(());
    }
    PREEMPTED.store(false, Ordering::SeqCst);
    USER_CANCELLED.store(false, Ordering::SeqCst);
    RUNNING.store(true, Ordering::SeqCst);
    // A start may have arrived between the batch reservation and publishing RUNNING.
    if crate::control::recording::has_pending_start() {
        PREEMPTED.store(true, Ordering::SeqCst);
    }
    let result = retranscription::execute_automatic(app.clone(), &job).await;
    RUNNING.store(false, Ordering::SeqCst);
    match result {
        Ok(result) => {
            // State was committed with the transcript. Enrich durable status with file warnings.
            sqlx::query("UPDATE automatic_retranscription_jobs SET result=? WHERE job_id=? AND state='completed'").bind(serde_json::to_string(&result).map_err(|e|e.to_string())?).bind(&job.job_id).execute(pool).await.map_err(|e|e.to_string())?;
        }
        Err(error) => {
            let state = if USER_CANCELLED.load(Ordering::SeqCst)
                || (error.contains("cancelled") && !PREEMPTED.load(Ordering::SeqCst))
            {
                "cancelled"
            } else if PREEMPTED.load(Ordering::SeqCst) && store::settings(pool).await?.enabled {
                "queued"
            } else if PREEMPTED.load(Ordering::SeqCst) {
                "cancelled"
            } else {
                "failed"
            };
            store::transition(pool, &job.job_id, "running", state, Some(&error)).await?;
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn get_automatic_retranscription_settings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<store::Settings, String> {
    store::settings(&pool(&app)?).await
}
#[tauri::command]
pub async fn set_automatic_retranscription_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: store::Settings,
) -> Result<store::Settings, String> {
    if !matches!(settings.provider.as_str(), "whisper" | "parakeet") {
        return Err("Choose Whisper or Parakeet".into());
    }
    if settings.provider == "parakeet" && settings.language != "auto" {
        return Err("Parakeet supports automatic language only".into());
    }
    if settings.language != "auto"
        && (settings.language.contains('\0')
            || (settings.language != "auto-translate"
                && whisper_rs::get_lang_id(&settings.language).is_none()))
    {
        return Err("Unsupported language".into());
    }
    if settings.enabled {
        let available = if settings.provider == "whisper" {
            crate::whisper_engine::commands::whisper_get_available_models()
                .await?
                .iter()
                .any(|m| {
                    m.name == settings.model
                        && matches!(m.status, crate::whisper_engine::ModelStatus::Available)
                })
        } else {
            crate::parakeet_engine::commands::parakeet_get_available_models()
                .await?
                .iter()
                .any(|m| {
                    m.name == settings.model
                        && matches!(m.status, crate::parakeet_engine::ModelStatus::Available)
                })
        };
        if !available {
            return Err(
                "Choose an installed model before enabling automatic retranscription".into(),
            );
        }
    }
    store::save_settings(&pool(&app)?, &settings).await?;
    WAKE.notify_one();
    Ok(settings)
}
#[tauri::command]
pub async fn get_automatic_retranscription_status<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: Option<String>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({"jobs":store::jobs(&pool(&app)?,meeting_id.as_deref()).await?}))
}
#[tauri::command]
pub async fn cancel_automatic_retranscription<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<(), String> {
    let db = pool(&app)?;
    // This compare-and-set shares SQLite's write boundary with transcript replacement.
    // An accepted cancel cannot commit a replacement, even before the worker publishes JOB.
    let changed=sqlx::query("UPDATE automatic_retranscription_jobs SET state='cancelled',error='Cancelled by user',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE job_id=? AND state IN ('queued','running')")
        .bind(&job_id).execute(&db).await.map_err(|e|e.to_string())?.rows_affected();
    if changed != 1 {
        return Err("This automatic job has already completed or stopped".into());
    }
    if retranscription::cancel_retranscription_command(job_id)
        .await
        .is_ok()
    {
        USER_CANCELLED.store(true, Ordering::SeqCst);
    }
    Ok(())
}
