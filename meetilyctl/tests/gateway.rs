mod support;
use clap::Parser;
use meetilyctl::{
    cli::{self, Cli},
    gateway::{Credentials, Gateway, RpcError},
};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
async fn preserves_pages_uses_scope_and_omits_missing_params() {
    let fixture = support::fixture(
        |_, _| json!({"result":{"items":[{"id":"m"}],"next_cursor":"more"}}),
        true,
    )
    .await;
    let command = Cli::parse_from(["meetilyctl", "meetings", "list", "--limit", "1"]);
    let (method, params, control) = cli::rpc_command(&command.command).unwrap();
    let page = fixture.gateway.call(method, params, control).await.unwrap();
    assert_eq!(page["next_cursor"], "more");
    fixture
        .gateway
        .call("recording.stop", json!({"recording_id":"r"}), true)
        .await
        .unwrap();
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(
        calls[0]["authorization"],
        "authorization: Bearer read-secret"
    );
    assert!(calls[0]["params"].get("cursor").is_none());
    assert_eq!(
        calls[1]["authorization"],
        "authorization: Bearer control-secret"
    );
}

#[tokio::test]
async fn propagates_typed_errors_and_refuses_missing_control() {
    let fixture = support::fixture(
        |_, _| json!({"error":{"code":"unauthorized","message":"Revoked key"}}),
        false,
    )
    .await;
    let error = fixture
        .gateway
        .call("status", json!({}), false)
        .await
        .unwrap_err();
    assert_eq!(error.code, "unauthorized");
    assert_eq!(error.exit_code(), 4);
    let error = fixture
        .gateway
        .call("recording.start", json!({}), true)
        .await
        .unwrap_err();
    assert_eq!(error.code, "forbidden");
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}

#[test]
fn stable_exit_codes_and_invalid_discovery() {
    for (code, exit) in [
        ("invalid_request", 2),
        ("invalid_language", 2),
        ("unsupported_language", 2),
        ("not_ready", 3),
        ("no_recording", 5),
        ("recording_active", 6),
        ("recording_mismatch", 6),
        ("audio_busy", 6),
        ("unavailable", 3),
        ("forbidden", 4),
        ("not_found", 5),
        ("conflict", 6),
        ("timeout", 7),
        ("internal", 1),
    ] {
        assert_eq!(RpcError::new(code, "test").exit_code(), exit);
    }
    assert!(Gateway::new(Credentials {
        port: 0,
        read_token: "read".into(),
        control_token: None
    })
    .is_err());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "broken").unwrap();
    assert_eq!(
        Gateway::from_file(file.path()).err().unwrap().code,
        "invalid_request"
    );
}

#[tokio::test]
async fn finite_wait_is_read_only_and_ignores_other_recordings() {
    let fixture=support::fixture(|method,_|match method {
        "events.list"=>json!({"result":{"items":[{"id":1,"event":"meeting.finalized","recording_id":"other"}],"next_cursor":"1"}}),
        _=>json!({"result":{"recording":null}}),
    },true).await;
    let start = Instant::now();
    let error = cli::wait_finalized(&fixture.gateway, "expected", Duration::from_millis(80))
        .await
        .unwrap_err();
    assert_eq!(error.code, "timeout");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(fixture
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c["authorization"] == "authorization: Bearer read-secret"));
}

#[tokio::test]
async fn wait_follows_multiple_pages_to_the_requested_finalization() {
    let fixture=support::fixture(|method,params|match method {
        "events.list" if params.get("after").is_none()=>json!({"result":{"items":[{"id":1,"event":"meeting.finalized","recording_id":"other"}],"next_cursor":"1"}}),
        "events.list"=>json!({"result":{"items":[{"id":2,"event":"meeting.finalized","recording_id":"expected","meeting_id":"m"}],"next_cursor":"2"}}),
        _=>json!({"result":{"recording":null}}),
    },false).await;
    let result = cli::wait_finalized(&fixture.gateway, "expected", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(result["meeting_id"], "m");
}

#[test]
fn watch_filter_advances_cursor_and_does_not_repeat_inclusive_last_event() {
    let mut cursor = None;
    let page = json!({"items":[{"id":1,"event":"recording.started"},{"id":2,"event":"recording.stopped"}],"next_cursor":"2"});
    let output = cli::event_page(page, &mut cursor, &["recording.started".into()]).unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(cursor.as_deref(), Some("2"));
    let page = json!({"items":[{"id":1,"event":"recording.started"},{"id":2,"event":"recording.stopped"},{"id":3,"event":"meeting.finalized"}],"next_cursor":"3"});
    let output = cli::event_page(page, &mut cursor, &[]).unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0]["id"], 3);
    let replay = json!({"items":[{"id":1,"event":"recording.started"}],"next_cursor":"1"});
    assert!(cli::event_page(replay, &mut cursor, &[])
        .unwrap()
        .is_empty());
    assert_eq!(cursor.as_deref(), Some("3"));
}
