//! Read/export and durable event history over the application's existing SQLite pool.
//! This library never opens a second database; the desktop supplies its own pool.
use crate::ControlError;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

pub async fn initialize(pool: &SqlitePool) -> Result<(), ControlError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS control_events(id INTEGER PRIMARY KEY AUTOINCREMENT,time TEXT NOT NULL,event TEXT NOT NULL,recording_id TEXT,meeting_id TEXT,data TEXT NOT NULL)").execute(pool).await?;
    Ok(())
}
pub async fn emit_event(
    pool: &SqlitePool,
    event: &str,
    recording_id: Option<&str>,
    meeting_id: Option<&str>,
    data: Value,
) -> Result<(), ControlError> {
    sqlx::query(
        "INSERT INTO control_events(time,event,recording_id,meeting_id,data) VALUES(?,?,?,?,?)",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(event)
    .bind(recording_id)
    .bind(meeting_id)
    .bind(data.to_string())
    .execute(pool)
    .await?;
    Ok(())
}
fn invalid(message: &str) -> ControlError {
    ControlError::new("invalid_request", message)
}
fn limit(p: &Value) -> Result<i64, ControlError> {
    let n = match p.get("limit") {
        None => 200,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| invalid("limit must be an integer"))?,
    };
    if !(1..=1000).contains(&n) {
        Err(invalid("limit must be between 1 and 1000"))
    } else {
        Ok(n)
    }
}
fn required<'a>(p: &'a Value, key: &str) -> Result<&'a str, ControlError> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() < 4096)
        .ok_or_else(|| invalid(&format!("Missing or invalid {key}")))
}
fn utc(s: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc).to_rfc3339())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|d| d.and_utc().to_rfc3339())
        })
}
fn meeting(row: &SqliteRow) -> Value {
    let created: String = row.get("created_at");
    let updated: String = row.get("updated_at");
    json!({"meeting_id":row.get::<String,_>("id"),"id":row.get::<String,_>("id"),"title":row.get::<String,_>("title"),"created_at":utc(&created),"updated_at":utc(&updated),"state":row.get::<String,_>("state"),"recording_id":row.get::<Option<String>,_>("recording_id"),"initiator":row.get::<Option<String>,_>("initiator"),"source": {"application":row.get::<Option<String>,_>("application"),"detection_session_id":row.get::<Option<String>,_>("detection_session_id")}})
}
const MEETING_SELECT:&str="SELECT m.*,COALESCE(r.state,'finalized') AS state,r.recording_id,r.initiator,r.application,r.detection_session_id FROM meetings m LEFT JOIN recording_sessions r ON r.meeting_id=m.id";
async fn get_meeting(pool: &SqlitePool, id: &str) -> Result<Value, ControlError> {
    let row = sqlx::query(&format!("{MEETING_SELECT} WHERE m.id=?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ControlError::new("not_found", "Meeting not found"))?;
    Ok(meeting(&row))
}
#[derive(Serialize, Deserialize)]
struct TranscriptCursor {
    meeting_id: String,
    #[serde(default)]
    revision: i64,
    snapshot: i64,
    time: f64,
    id: String,
}
fn segment(row: &SqliteRow) -> Value {
    let timestamp: String = row.get("timestamp");
    json!({"id":row.get::<String,_>("id"),"meeting_id":row.get::<String,_>("meeting_id"),"text":row.get::<String,_>("transcript"),"timestamp":utc(&timestamp),"audio_start_time":row.get::<Option<f64>,_>("audio_start_time"),"audio_end_time":row.get::<Option<f64>,_>("audio_end_time"),"duration":row.get::<Option<f64>,_>("duration"),"speaker":null})
}
async fn transcript(pool: &SqlitePool, p: &Value) -> Result<Value, ControlError> {
    let id = required(p, "meeting_id")?;
    let meta = get_meeting(pool, id).await?;
    let n = limit(p)?;
    // Keep revision, rowid boundary and rows in one SQLite read snapshot. A
    // retranscription replaces rows transactionally and may reuse their rowids.
    let mut tx = pool.begin().await?;
    let has_metadata: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meeting_transcription_metadata'")
        .fetch_one(&mut *tx).await?;
    let revision: i64 = if has_metadata > 0 {
        sqlx::query_scalar(
            "SELECT transcript_revision FROM meeting_transcription_metadata WHERE meeting_id=?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0)
    } else {
        0
    };
    let c: TranscriptCursor = if let Some(s) = p.get("cursor").filter(|v| !v.is_null()) {
        serde_json::from_str(
            s.as_str()
                .filter(|s| s.len() < 2048)
                .ok_or_else(|| invalid("Invalid cursor"))?,
        )
        .map_err(|_| invalid("Invalid transcript cursor"))?
    } else {
        let snapshot: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(rowid),0) FROM transcripts WHERE meeting_id=?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        TranscriptCursor {
            meeting_id: id.into(),
            revision,
            snapshot,
            time: -1.0,
            id: String::new(),
        }
    };
    if c.meeting_id != id || c.snapshot < 0 || !c.time.is_finite() {
        return Err(invalid("Cursor does not belong to this meeting"));
    }
    if c.revision != revision {
        return Err(ControlError::new(
            "conflict",
            "Transcript changed; restart pagination without a cursor",
        ));
    }
    let rows=sqlx::query("SELECT * FROM transcripts WHERE meeting_id=? AND rowid<=? AND (COALESCE(audio_start_time,0)>? OR (COALESCE(audio_start_time,0)=? AND id>?)) ORDER BY COALESCE(audio_start_time,0),id LIMIT ?").bind(id).bind(c.snapshot).bind(c.time).bind(c.time).bind(&c.id).bind(n+1).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let more = rows.len() > n as usize;
    let rows = &rows[..rows.len().min(n as usize)];
    let next = if more {
        let last = rows.last().unwrap();
        Some(
            serde_json::to_string(&TranscriptCursor {
                meeting_id: id.into(),
                revision,
                snapshot: c.snapshot,
                time: last
                    .get::<Option<f64>, _>("audio_start_time")
                    .unwrap_or(0.0),
                id: last.get("id"),
            })
            .unwrap(),
        )
    } else {
        None
    };
    Ok(
        json!({"meeting_id":id,"items":rows.iter().map(segment).collect::<Vec<_>>(),"next_cursor":next,"partial":meta["state"]!="finalized","snapshot":c.snapshot,"transcript_revision":revision,"state":meta["state"]}),
    )
}
#[derive(Serialize, Deserialize)]
struct MeetingCursor {
    snapshot: i64,
    created: String,
    id: String,
    order: String,
    query: String,
    state: String,
}
async fn meetings(pool: &SqlitePool, p: &Value) -> Result<Value, ControlError> {
    let n = limit(p)?;
    let order = p.get("order").and_then(Value::as_str).unwrap_or("desc");
    if order != "desc" && order != "asc" {
        return Err(invalid("order must be asc or desc"));
    }
    let query = p.get("query").and_then(Value::as_str).unwrap_or("");
    let state = p.get("state").and_then(Value::as_str).unwrap_or("");
    let cursor: Option<MeetingCursor> = p
        .get("cursor")
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_str(
                v.as_str()
                    .filter(|s| s.len() < 8192)
                    .ok_or_else(|| invalid("Invalid cursor"))?,
            )
            .map_err(|_| invalid("Invalid meeting cursor"))
        })
        .transpose()?;
    let snapshot = if let Some(c) = &cursor {
        if c.order != order || c.query != query || c.state != state {
            return Err(invalid("Cursor filters changed"));
        }
        c.snapshot
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(rowid),0) FROM meetings")
            .fetch_one(pool)
            .await?
    };
    let cmp = if order == "asc" { ">" } else { "<" };
    let sql=format!("{MEETING_SELECT} WHERE m.rowid<=? AND (?='' OR instr(lower(m.title),lower(?))>0) AND (?='' OR COALESCE(r.state,'finalized')=?) AND (?=0 OR COALESCE(julianday(m.created_at),0) {cmp} COALESCE(julianday(?),0) OR (COALESCE(julianday(m.created_at),0)=COALESCE(julianday(?),0) AND m.id {cmp} ?)) ORDER BY COALESCE(julianday(m.created_at),0) {order},m.id {order} LIMIT ?");
    let created = cursor.as_ref().map(|c| c.created.as_str()).unwrap_or("");
    let id = cursor.as_ref().map(|c| c.id.as_str()).unwrap_or("");
    let rows = sqlx::query(&sql)
        .bind(snapshot)
        .bind(query)
        .bind(query)
        .bind(state)
        .bind(state)
        .bind(i32::from(cursor.is_some()))
        .bind(created)
        .bind(created)
        .bind(id)
        .bind(n + 1)
        .fetch_all(pool)
        .await?;
    let more = rows.len() > n as usize;
    let rows = &rows[..rows.len().min(n as usize)];
    let next = if more {
        let last = rows.last().unwrap();
        Some(
            serde_json::to_string(&MeetingCursor {
                snapshot,
                created: last.get("created_at"),
                id: last.get("id"),
                order: order.into(),
                query: query.into(),
                state: state.into(),
            })
            .unwrap(),
        )
    } else {
        None
    };
    Ok(json!({"items":rows.iter().map(meeting).collect::<Vec<_>>(),"next_cursor":next}))
}
pub async fn dispatch(pool: &SqlitePool, method: &str, p: &Value) -> Result<Value, ControlError> {
    match method {
        "meetings.get" => get_meeting(pool, required(p, "meeting_id")?).await,
        "meetings.list" => meetings(pool, p).await,
        "transcript.get" => transcript(pool, p).await,
        "meetings.export" => {
            let id = required(p, "meeting_id")?;
            let meta = get_meeting(pool, id).await?;
            let format = p.get("format").and_then(Value::as_str).unwrap_or("md");
            if !matches!(format, "md" | "txt" | "json") {
                return Err(invalid("format must be md, txt, or json"));
            }
            let mut params = json!({"meeting_id":id,"limit":1000});
            let mut items = Vec::new();
            loop {
                let page = transcript(pool, &params).await?;
                items.extend(page["items"].as_array().unwrap().clone());
                if page["next_cursor"].is_null() {
                    break;
                }
                params["cursor"] = page["next_cursor"].clone();
            }
            let content = if format == "json" {
                serde_json::to_string_pretty(
                    &json!({"meeting":meta,"segments":items,"partial":meta["state"]!="finalized"}),
                )
                .unwrap()
            } else {
                let prefix = if format == "md" { "# " } else { "" };
                let mut text = format!(
                    "{prefix}{}\n\n{} · {}\n\n",
                    meta["title"].as_str().unwrap_or("Meeting"),
                    meta["created_at"].as_str().unwrap_or("Date unknown"),
                    meta["state"].as_str().unwrap_or("unknown")
                );
                for s in items {
                    let start = s["audio_start_time"]
                        .as_f64()
                        .map(|v| format!("{v:.2}s"))
                        .unwrap_or_else(|| "time unknown".into());
                    text.push_str(&format!(
                        "[{start}] Unknown speaker: {}\n\n",
                        s["text"].as_str().unwrap_or("")
                    ));
                }
                text
            };
            Ok(
                json!({"meeting_id":id,"format":format,"content":content,"partial":meta["state"]!="finalized"}),
            )
        }
        "transcripts.search" => {
            let query = required(p, "query")?;
            let n = limit(p)?;
            let after = parse_id(p.get("cursor"))?;
            let rows=sqlx::query("SELECT rowid AS seq,* FROM transcripts WHERE rowid>? AND instr(lower(transcript),lower(?))>0 ORDER BY rowid LIMIT ?").bind(after).bind(query).bind(n+1).fetch_all(pool).await?;
            let more = rows.len() > n as usize;
            let rows = &rows[..rows.len().min(n as usize)];
            let next = if more {
                rows.last().map(|r| r.get::<i64, _>("seq").to_string())
            } else {
                None
            };
            let items: Vec<_> = rows
                .iter()
                .map(|r| {
                    let mut v = segment(r);
                    v["snippet"] =
                        Value::String(r.get::<String, _>("transcript").chars().take(600).collect());
                    v
                })
                .collect();
            Ok(json!({"items":items,"next_cursor":next}))
        }
        "events.list" => {
            let after = parse_id(p.get("after"))?;
            let n = limit(p)?;
            let rows = sqlx::query("SELECT * FROM control_events WHERE id>? ORDER BY id LIMIT ?")
                .bind(after)
                .bind(n)
                .fetch_all(pool)
                .await?;
            let next = rows
                .last()
                .map(|r| r.get::<i64, _>("id"))
                .unwrap_or(after)
                .to_string();
            Ok(
                json!({"items":rows.iter().map(|r|json!({"id":r.get::<i64,_>("id").to_string(),"time":r.get::<String,_>("time"),"event":r.get::<String,_>("event"),"recording_id":r.get::<Option<String>,_>("recording_id"),"meeting_id":r.get::<Option<String>,_>("meeting_id"),"data":serde_json::from_str::<Value>(&r.get::<String,_>("data")).unwrap_or(Value::Null)})).collect::<Vec<_>>(),"next_cursor":next}),
            )
        }
        _ => Err(ControlError::new("not_found", "Unknown store method")),
    }
}
fn parse_id(v: Option<&Value>) -> Result<i64, ControlError> {
    match v.filter(|v| !v.is_null()) {
        None => Ok(0),
        Some(v) => v
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| *n >= 0)
            .ok_or_else(|| invalid("Invalid cursor")),
    }
}
