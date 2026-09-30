use clap::Parser;
use meetilyctl::cli::{Cli, Command, RecordingCommand};

#[test]
fn parse_documented_commands_and_global_json() {
    for args in [
        vec!["meetilyctl", "doctor", "--json"],
        vec!["meetilyctl", "devices", "list", "--json"],
        vec![
            "meetilyctl",
            "recording",
            "start",
            "--name",
            "Zoom meeting",
            "--language",
            "ru",
            "--idempotency-key",
            "abc",
        ],
        vec![
            "meetilyctl",
            "meetings",
            "list",
            "--limit",
            "20",
            "--cursor",
            "20",
            "--json",
        ],
        vec!["meetilyctl", "meetings", "export", "id", "--format", "md"],
        vec![
            "meetilyctl",
            "transcript",
            "get",
            "id",
            "--cursor",
            "200",
            "--limit",
            "200",
            "--json",
        ],
        vec![
            "meetilyctl",
            "watch",
            "--events",
            "recording.started,meeting.finalized",
            "--json",
        ],
        vec!["meetilyctl", "mcp", "serve"],
    ] {
        assert!(Cli::try_parse_from(args).is_ok());
    }
}

#[test]
fn wait_requires_an_explicit_recording_and_finite_timeout() {
    assert!(Cli::try_parse_from(["meetilyctl", "recording", "wait"]).is_err());
    assert!(Cli::try_parse_from([
        "meetilyctl",
        "recording",
        "wait",
        "--recording-id",
        "r",
        "--timeout",
        "0"
    ])
    .is_err());
    let cli = Cli::try_parse_from([
        "meetilyctl",
        "recording",
        "wait",
        "--recording-id",
        "r",
        "--until",
        "finalized",
        "--timeout",
        "1",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Recording {
            command: RecordingCommand::Wait { .. }
        }
    ));
}

#[test]
fn rejects_unbounded_pages_and_unknown_export_formats() {
    assert!(Cli::try_parse_from(["meetilyctl", "meetings", "list", "--limit", "0"]).is_err());
    assert!(
        Cli::try_parse_from(["meetilyctl", "meetings", "export", "m", "--format", "html"]).is_err()
    );
}
