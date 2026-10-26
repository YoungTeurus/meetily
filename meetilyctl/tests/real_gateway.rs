//! Real v1 gateway and SQLx store, exercised through the official MCP SDK.
//! No recorder mock is used and this test makes no native audio claim.
use meetily_local_control::{
    gateway::{self, Dispatcher},
    store, ControlError,
};
use meetilyctl::gateway::{Credentials, Gateway};
use rmcp::{model::CallToolRequestParams, transport::TokioChildProcess, ServiceExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::{process::Command, sync::RwLock};

struct DatabaseDispatcher(sqlx::SqlitePool);
#[async_trait::async_trait]
impl Dispatcher for DatabaseDispatcher {
    async fn dispatch(&self, method: &str, params: Value) -> Result<Value, ControlError> {
        store::dispatch(&self.0, method, &params).await
    }
}

#[tokio::test]
async fn sdk_reads_all_503_segments_through_actual_gateway_and_database() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE meetings(id TEXT PRIMARY KEY,title TEXT,created_at TEXT,updated_at TEXT,folder_path TEXT); CREATE TABLE transcripts(id TEXT PRIMARY KEY,meeting_id TEXT,transcript TEXT,timestamp TEXT,audio_start_time REAL,audio_end_time REAL,duration REAL); CREATE TABLE recording_sessions(recording_id TEXT PRIMARY KEY,meeting_id TEXT,state TEXT,initiator TEXT,detection_session_id TEXT,application TEXT);").execute(&pool).await.unwrap();
    store::initialize(&pool).await.unwrap();
    sqlx::query("INSERT INTO meetings VALUES('m','Русская встреча','2026-09-30T10:00:00Z','2026-09-30T10:00:00Z',NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO recording_sessions VALUES('r','m','finalized','gui',NULL,NULL)")
        .execute(&pool)
        .await
        .unwrap();
    for index in (0..503).rev() {
        sqlx::query("INSERT INTO transcripts VALUES(?, 'm', ?, '2026-09-30T10:00:00Z', ?, ?, 1)")
            .bind(format!("t{index:04}"))
            .bind(format!("Текст {index}"))
            .bind(index as f64)
            .bind((index + 1) as f64)
            .execute(&pool)
            .await
            .unwrap();
    }
    store::emit_event(&pool, "meeting.finalized", Some("r"), Some("m"), json!({}))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let credentials = Arc::new(RwLock::new(gateway::Credentials {
        enabled: true,
        read_token: "actual-read".into(),
        control_token: Some("actual-control".into()),
    }));
    let router = gateway::router(
        Arc::new(DatabaseDispatcher(pool)),
        credentials.clone(),
        port,
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        json!({"port":port,"read_token":"actual-read","control_token":"actual-control"})
            .to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_meetilyctl"));
    command
        .args(["mcp", "serve"])
        .env("MEETILY_INTEGRATION_FILE", file.path());
    let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 7);
    let meetings = client
        .call_tool(
            CallToolRequestParams::new("list_meetings").with_arguments(
                json!({"state":"finalized","order":"desc","limit":20})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(meetings.structured_content.unwrap()["items"][0]["id"], "m");
    let mut params = json!({"meeting_id":"m","limit":200});
    let mut items = Vec::new();
    let mut pages = 0;
    loop {
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_transcript")
                    .with_arguments(params.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        let page = result.structured_content.unwrap();
        assert_eq!(page["partial"], false);
        items.extend(page["items"].as_array().unwrap().clone());
        pages += 1;
        if page["next_cursor"].is_null() {
            break;
        }
        params["cursor"] = page["next_cursor"].clone();
    }
    assert_eq!(pages, 3);
    assert_eq!(items.len(), 503);
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item["text"], format!("Текст {index}"));
    }
    let export = client
        .call_tool(
            CallToolRequestParams::new("export_meeting").with_arguments(
                json!({"meeting_id":"m","format":"txt"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(export.structured_content.unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("Текст 502"));
    let gateway = Gateway::new(Credentials {
        port,
        read_token: "actual-read".into(),
        control_token: None,
    })
    .unwrap();
    let finalized =
        meetilyctl::cli::wait_finalized(&gateway, "r", std::time::Duration::from_secs(1))
            .await
            .unwrap();
    assert_eq!(finalized["meeting_id"], "m");
    credentials.write().await.read_token = "revoked".into();
    let error = client
        .call_tool(
            CallToolRequestParams::new("get_meeting")
                .with_arguments(json!({"meeting_id":"m"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(error.is_error, Some(true));
    assert_eq!(
        error.structured_content.unwrap()["error"]["code"],
        "unauthorized"
    );
    client.cancel().await.unwrap();
    server.abort();
}
