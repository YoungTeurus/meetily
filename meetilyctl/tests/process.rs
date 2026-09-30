mod support;
use serde_json::{json, Value};
use std::{
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

#[tokio::test]
async fn cli_json_export_errors_and_finite_wait_work_as_processes() {
    let fixture = support::fixture(
        |method, _| match method {
            "meetings.export" => json!({"result":{"format":"md","content":"# Meeting\nHello"}}),
            "meetings.get" => json!({"error":{"code":"not_found","message":"No such meeting"}}),
            "events.list" => json!({"result":{"items":[],"next_cursor":null}}),
            _ => json!({"result":{"recording":null}}),
        },
        false,
    )
    .await;
    async fn run(f: &support::Fixture, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_meetilyctl"))
            .args(args)
            .env("MEETILY_INTEGRATION_FILE", f.file.path())
            .output()
            .await
            .unwrap()
    }
    let invalid = run(&fixture, &["meetings", "list", "--limit", "0", "--json"]).await;
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stderr).unwrap()["error"]["code"],
        "invalid_request"
    );
    let output = run(&fixture, &["status", "--json"]).await;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"recording":null})
    );
    assert!(output.stderr.is_empty());
    let output = run(&fixture, &["meetings", "export", "m", "--format", "md"]).await;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "# Meeting\nHello\n"
    );
    let output = run(&fixture, &["meetings", "get", "missing", "--json"]).await;
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "not_found"
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        run(
            &fixture,
            &[
                "recording",
                "wait",
                "--recording-id",
                "expected",
                "--timeout",
                "1",
                "--json",
            ],
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn watch_reconnects_keeps_cursor_and_emits_filtered_ndjson_without_duplicates() {
    let count = Arc::new(AtomicUsize::new(0));
    let responses = count.clone();
    let fixture=support::fixture(move |_,_|match responses.fetch_add(1,Ordering::SeqCst) {
        0=>json!({"result":{"items":[{"id":1,"time":"2026-09-30T10:00:00Z","event":"recording.started","recording_id":"r"},{"id":2,"time":"2026-09-30T10:00:01Z","event":"recording.stopped","recording_id":"r"}],"next_cursor":"2"}}),
        1=>json!({"error":{"code":"unavailable","message":"Temporary restart"}}),
        2=>json!({"result":{"items":[{"id":1,"time":"2026-09-30T10:00:00Z","event":"recording.started","recording_id":"r"},{"id":2,"time":"2026-09-30T10:00:01Z","event":"recording.stopped","recording_id":"r"},{"id":3,"time":"2026-09-30T10:00:02Z","event":"meeting.finalized","recording_id":"r","meeting_id":"m"}],"next_cursor":"3"}}),
        _=>json!({"result":{"items":[],"next_cursor":"3"}}),
    },false).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_meetilyctl"))
        .args([
            "watch",
            "--events",
            "recording.started,meeting.finalized",
            "--poll-ms",
            "50",
            "--json",
        ])
        .env("MEETILY_INTEGRATION_FILE", fixture.file.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(3), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&first).unwrap()["id"], 1);
    assert_eq!(serde_json::from_str::<Value>(&second).unwrap()["id"], 3);
    assert_eq!(fixture.calls.lock().unwrap()[1]["params"]["after"], "2");
    assert_eq!(fixture.calls.lock().unwrap()[2]["params"]["after"], "2");
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn watch_follows_app_restart_port_and_key_rotation_from_discovery_file() {
    let first=support::fixture(|_,params|if params.get("after").is_none() {
        json!({"result":{"items":[{"id":1,"event":"recording.started","recording_id":"r"}],"next_cursor":"1"}})
    } else { json!({"result":{"items":[],"next_cursor":"1"}}) },false).await;
    let second=support::fixture(|_,params| {
        assert_eq!(params["after"],"1");
        json!({"result":{"items":[{"id":2,"event":"meeting.finalized","recording_id":"r","meeting_id":"m"}],"next_cursor":"2"}})
    },false).await;
    let mut rotated: Value =
        serde_json::from_slice(&std::fs::read(second.file.path()).unwrap()).unwrap();
    rotated["read_token"] = json!("rotated-secret");
    std::fs::write(second.file.path(), rotated.to_string()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_meetilyctl"))
        .args(["watch", "--poll-ms", "50", "--json"])
        .env("MEETILY_INTEGRATION_FILE", first.file.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let started = tokio::time::timeout(std::time::Duration::from_secs(3), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&started).unwrap()["id"], 1);
    // Update the same credential file as the app does on restart. The original
    // listener remains reachable, so only actual discovery reload finds server 2.
    std::fs::write(
        first.file.path(),
        std::fs::read(second.file.path()).unwrap(),
    )
    .unwrap();
    let finalized = tokio::time::timeout(std::time::Duration::from_secs(3), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&finalized).unwrap()["id"], 2);
    assert_eq!(second.calls.lock().unwrap()[0]["params"]["after"], "1");
    assert_eq!(
        second.calls.lock().unwrap()[0]["authorization"],
        "authorization: Bearer rotated-secret"
    );
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
