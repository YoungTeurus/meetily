mod support;
use rmcp::{model::CallToolRequestParams, transport::TokioChildProcess, ServiceExt};
use serde_json::json;
use tokio::process::Command;

async fn client(
    fixture: &support::Fixture,
    args: &[&str],
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_meetilyctl"));
    cmd.args(args)
        .env("MEETILY_INTEGRATION_FILE", fixture.file.path());
    let transport = TokioChildProcess::new(cmd).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), ().serve(transport))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn official_sdk_child_process_initializes_lists_calls_and_pages() {
    let fixture=support::fixture(|method,params|match method {
        "meetings.list"=>json!({"result":{"items":[{"id":"m","created_at":"2026-09-30T10:00:00Z","state":"finalized"}],"next_cursor":null}}),
        "transcript.get" if params.get("cursor").is_none()=>json!({"result":{"items":[{"id":1,"text":"hello","audio_start_seconds":0}],"next_cursor":"1","partial":false}}),
        "transcript.get"=>json!({"result":{"items":[{"id":2,"text":"world","audio_start_seconds":1}],"next_cursor":null,"partial":false}}),
        _=>json!({"result":{"recording":null}}),
    },true).await;
    let client = client(&fixture, &["mcp", "serve"]).await;
    assert_eq!(
        client
            .peer_info()
            .unwrap()
            .server_info
            .as_ref()
            .unwrap()
            .name,
        "meetilyctl"
    );
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 7);
    assert!(tools
        .iter()
        .all(|tool| tool.annotations.as_ref().unwrap().read_only_hint == Some(true)));
    assert!(!tools.iter().any(|tool| tool.name == "start_recording"));
    let result = client
        .call_tool(CallToolRequestParams::new("list_meetings"))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["items"][0]["id"],
        "m"
    );
    let first = client
        .call_tool(
            CallToolRequestParams::new("get_transcript").with_arguments(
                json!({"meeting_id":"m","limit":1})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let first = first.structured_content.unwrap();
    let second = client
        .call_tool(
            CallToolRequestParams::new("get_transcript").with_arguments(
                json!({"meeting_id":"m","cursor":first["next_cursor"],"limit":1})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        second.structured_content.unwrap()["items"][0]["text"],
        "world"
    );
    assert!(client
        .call_tool(CallToolRequestParams::new("start_recording"))
        .await
        .is_err());
    client.cancel().await.unwrap();
    assert!(fixture
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call["authorization"] == "authorization: Bearer read-secret"));
}

#[tokio::test]
async fn control_requires_both_explicit_flag_and_credential() {
    let read_fixture = support::fixture(|_, _| json!({"result":{}}), false).await;
    let read_client = client(&read_fixture, &["mcp", "serve", "--allow-control"]).await;
    assert_eq!(read_client.list_all_tools().await.unwrap().len(), 7);
    read_client.cancel().await.unwrap();
    let fixture = support::fixture(|_, params| json!({"result":params}), true).await;
    let client = client(&fixture, &["mcp", "serve", "--allow-control"]).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 9);
    assert_eq!(
        tools
            .iter()
            .find(|t| t.name == "start_recording")
            .unwrap()
            .annotations
            .as_ref()
            .unwrap()
            .idempotent_hint,
        Some(false)
    );
    assert_eq!(
        tools
            .iter()
            .find(|t| t.name == "start_recording")
            .unwrap()
            .annotations
            .as_ref()
            .unwrap()
            .read_only_hint,
        Some(false)
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("start_recording").with_arguments(
                json!({"idempotency_key":"retry"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.structured_content.unwrap()["initiator"], "mcp");
    let error = client
        .call_tool(
            CallToolRequestParams::new("stop_recording").with_arguments(
                json!({"recording_id":"r","recordingid":"typo"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(error.is_error, Some(true));
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.calls.lock().unwrap()[0]["authorization"],
        "authorization: Bearer control-secret"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn gateway_errors_become_typed_tool_results() {
    let fixture = support::fixture(
        |_, _| json!({"error":{"code":"not_found","message":"Meeting missing"}}),
        false,
    )
    .await;
    let client = client(&fixture, &["mcp", "serve"]).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_meeting")
                .with_arguments(json!({"meeting_id":"missing"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.structured_content.unwrap()["error"]["code"],
        "not_found"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn raw_stdio_contains_only_protocol_json_lines() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let fixture = support::fixture(|_, _| json!({"result":{"recording":null}}), false).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_meetilyctl"))
        .args(["mcp", "serve"])
        .env("MEETILY_INTEGRATION_FILE", fixture.file.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    async fn send(input: &mut tokio::process::ChildStdin, value: serde_json::Value) {
        input
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
        input.flush().await.unwrap();
    }
    async fn read(
        output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    ) -> serde_json::Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).expect("Every stdout line must be protocol JSON")
    }
    send(&mut input,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"stdout-smoke","version":"1"}}})).await;
    let response = read(&mut output).await;
    assert_eq!(response["id"], 1);
    assert!(response.get("result").is_some());
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert_eq!(
        read(&mut output).await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    send(&mut input,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_recording_status","arguments":{}}})).await;
    let response = read(&mut output).await;
    assert_eq!(response["id"], 3);
    assert!(response["result"]["structuredContent"]["recording"].is_null());
    drop(input);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert!(output.next_line().await.unwrap().is_none());
}
