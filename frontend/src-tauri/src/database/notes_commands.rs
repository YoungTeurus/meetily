use super::repositories::notes::{MeetingNotes, NotesError, NotesRepository};
use crate::state::AppState;
use tauri::{AppHandle, Emitter, Runtime};

#[tauri::command]
pub async fn get_meeting_notes(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingNotes, NotesError> {
    NotesRepository::get(state.db_manager.pool(), &meeting_id).await
}

#[tauri::command]
pub async fn save_meeting_notes<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    notes: String,
    expected_revision: i64,
) -> Result<MeetingNotes, NotesError> {
    let _summary_guard = crate::summary::commands::SUMMARY_START_LOCK.lock().await;
    let saved = NotesRepository::save(state.db_manager.pool(), &meeting_id, &notes, expected_revision).await?;
    if saved.revision != expected_revision {
        if let Ok(Some(process)) = super::repositories::summary::SummaryProcessesRepository::get_summary_data(state.db_manager.pool(), &meeting_id).await {
            if process.status.eq_ignore_ascii_case("cancelled") {
                if let Some(started_at) = process.start_time {
                    crate::summary::service::SummaryService::cancel_summary(&meeting_id, started_at);
                }
            }
        }
        if let Err(error) = app.emit("meeting-notes-updated", serde_json::json!({"meeting_id": meeting_id, "revision": saved.revision})) {
            log::warn!("Notes saved, but update notification failed: {error}");
        }
    }
    Ok(saved)
}
