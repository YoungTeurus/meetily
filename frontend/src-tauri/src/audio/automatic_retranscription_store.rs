//! Durable post-call work. The trigger is installed after recording storage exists.
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub job_id: String,
    pub recording_id: String,
    pub meeting_id: String,
    pub state: String,
    pub config: serde_json::Value,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}
pub async fn initialize(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("CREATE TRIGGER IF NOT EXISTS enqueue_automatic_retranscription AFTER UPDATE OF state ON recording_sessions WHEN NEW.state='finalized' AND OLD.state!='finalized' AND json_extract((SELECT config FROM automatic_retranscription_settings WHERE id=1),'$.enabled')=1 BEGIN INSERT OR IGNORE INTO automatic_retranscription_jobs(job_id,recording_id,meeting_id,state,config) SELECT lower(hex(randomblob(16))),NEW.recording_id,NEW.meeting_id,'queued',json_set(config,'$.vocabulary',CASE WHEN json_extract(config,'$.provider')='parakeet' THEN NULL ELSE (SELECT vocabulary FROM transcription_vocabulary WHERE id='global') END) FROM automatic_retranscription_settings WHERE id=1; END").execute(pool).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE automatic_retranscription_jobs SET state='failed',error='Application stopped before retranscription completed; the previous transcript was kept',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='running'").execute(pool).await.map_err(|e|e.to_string())?;
    Ok(())
}
pub async fn settings(pool: &SqlitePool) -> Result<Settings, String> {
    let raw: String =
        sqlx::query_scalar("SELECT config FROM automatic_retranscription_settings WHERE id=1")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}
pub async fn save_settings(pool: &SqlitePool, settings: &Settings) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE automatic_retranscription_settings SET config=? WHERE id=1")
        .bind(serde_json::to_string(settings).map_err(|e| e.to_string())?)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if !settings.enabled {
        sqlx::query("UPDATE automatic_retranscription_jobs SET state='cancelled',error='Automatic retranscription disabled',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='queued'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}
pub async fn jobs(pool: &SqlitePool, meeting: Option<&str>) -> Result<Vec<Job>, String> {
    let rows=sqlx::query("SELECT * FROM automatic_retranscription_jobs WHERE (? IS NULL OR meeting_id=?) ORDER BY created_at DESC,rowid DESC LIMIT 20").bind(meeting).bind(meeting).fetch_all(pool).await.map_err(|e|e.to_string())?;
    rows.into_iter()
        .map(|r| {
            Ok(Job {
                job_id: r.get("job_id"),
                recording_id: r.get("recording_id"),
                meeting_id: r.get("meeting_id"),
                state: r.get("state"),
                config: serde_json::from_str(&r.get::<String, _>("config"))
                    .map_err(|e| e.to_string())?,
                error: r.get("error"),
                result: r
                    .get::<Option<String>, _>("result")
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|e| e.to_string())?,
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
        })
        .collect()
}
pub async fn next(pool: &SqlitePool) -> Result<Option<Job>, String> {
    let id: Option<String>=sqlx::query_scalar("SELECT meeting_id FROM automatic_retranscription_jobs WHERE state='queued' ORDER BY created_at,rowid LIMIT 1").fetch_optional(pool).await.map_err(|e|e.to_string())?;
    match id {
        Some(id) => Ok(jobs(pool, Some(&id))
            .await?
            .into_iter()
            .find(|j| j.state == "queued")),
        None => Ok(None),
    }
}
pub async fn transition(
    pool: &SqlitePool,
    id: &str,
    from: &str,
    to: &str,
    error: Option<&str>,
) -> Result<bool, String> {
    Ok(sqlx::query("UPDATE automatic_retranscription_jobs SET state=?,error=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE job_id=? AND state=?").bind(to).bind(error).bind(id).bind(from).execute(pool).await.map_err(|e|e.to_string())?.rows_affected()==1)
}
pub async fn supersede(pool: &SqlitePool, meeting: &str) -> Result<(), String> {
    sqlx::query("UPDATE automatic_retranscription_jobs SET state='cancelled',error='Superseded by manual retranscription',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE meeting_id=? AND state='queued'").bind(meeting).execute(pool).await.map_err(|e|e.to_string())?;
    Ok(())
}
