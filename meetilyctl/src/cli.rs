use crate::gateway::{Gateway, RpcError};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(version, about = "Manage the running local Meetily Calls application")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Emit machine-readable JSON; errors go to stderr"
    )]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Doctor,
    Status {
        #[arg(long)]
        recording_id: Option<String>,
    },
    Devices {
        #[command(subcommand)]
        command: DevicesCommand,
    },
    Recording {
        #[command(subcommand)]
        command: RecordingCommand,
    },
    Meetings {
        #[command(subcommand)]
        command: MeetingsCommand,
    },
    Transcript {
        #[command(subcommand)]
        command: TranscriptCommand,
    },
    Watch {
        #[arg(long, value_delimiter = ',')]
        events: Vec<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value = "500", value_parser = clap::value_parser!(u64).range(50..=60_000))]
        poll_ms: u64,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}
#[derive(Debug, Subcommand)]
pub enum DevicesCommand {
    List,
}
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    Serve {
        #[arg(
            long,
            help = "Expose recording tools only when a control credential is also present"
        )]
        allow_control: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum RecordingCommand {
    Start {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        input_device: Option<String>,
        #[arg(long)]
        output_device: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Stop {
        #[arg(long)]
        recording_id: Option<String>,
    },
    Pause {
        #[arg(long)]
        recording_id: Option<String>,
    },
    Resume {
        #[arg(long)]
        recording_id: Option<String>,
    },
    Wait {
        #[arg(long)]
        recording_id: String,
        #[arg(long, value_enum, default_value = "finalized")]
        until: WaitUntil,
        #[arg(long, default_value = "120", value_parser = clap::value_parser!(u64).range(1..=86400))]
        timeout: u64,
    },
}
#[derive(Debug, Clone, ValueEnum)]
pub enum WaitUntil {
    Finalized,
}
#[derive(Debug, Clone, ValueEnum)]
pub enum ExportFormat {
    Md,
    Txt,
    Json,
}
impl ExportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Txt => "txt",
            Self::Json => "json",
        }
    }
}
#[derive(Debug, Clone, ValueEnum)]
pub enum Order {
    Asc,
    Desc,
}
#[derive(Debug, Args)]
pub struct Page {
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long, default_value = "20", value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub limit: u32,
}
#[derive(Debug, Subcommand)]
pub enum MeetingsCommand {
    List {
        #[command(flatten)]
        page: Page,
        #[arg(long, value_enum, default_value = "desc")]
        order: Order,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        query: Option<String>,
    },
    Get {
        id: String,
    },
    Export {
        id: String,
        #[arg(long, value_enum, default_value = "md")]
        format: ExportFormat,
    },
}
#[derive(Debug, Subcommand)]
pub enum TranscriptCommand {
    Get {
        id: String,
        #[command(flatten)]
        page: Page,
    },
    Search {
        query: String,
        #[command(flatten)]
        page: Page,
    },
}

/// No command silently drains or truncates a page: the cursor remains in the output.
pub fn rpc_command(command: &Command) -> Option<(&'static str, Value, bool)> {
    Some(match command {
        Command::Doctor => ("doctor", json!({}), false),
        Command::Status { recording_id } => ("status", json!({"recording_id":recording_id}), false),
        Command::Devices { .. } => ("devices.list", json!({}), false),
        Command::Recording { command } => match command {
            RecordingCommand::Start {
                name,
                language,
                input_device,
                output_device,
                idempotency_key,
            } => (
                "recording.start",
                json!({"name":name,"language":language,"input_device":input_device,"output_device":output_device,"idempotency_key":idempotency_key,"initiator":"cli"}),
                true,
            ),
            RecordingCommand::Stop { recording_id } => {
                ("recording.stop", json!({"recording_id":recording_id}), true)
            }
            RecordingCommand::Pause { recording_id } => (
                "recording.pause",
                json!({"recording_id":recording_id}),
                true,
            ),
            RecordingCommand::Resume { recording_id } => (
                "recording.resume",
                json!({"recording_id":recording_id}),
                true,
            ),
            RecordingCommand::Wait { .. } => return None,
        },
        Command::Meetings { command } => match command {
            MeetingsCommand::List {
                page,
                order,
                state,
                query,
            } => (
                "meetings.list",
                json!({"cursor":page.cursor,"limit":page.limit,"order": match order { Order::Asc=>"asc",Order::Desc=>"desc"},"state":state,"query":query}),
                false,
            ),
            MeetingsCommand::Get { id } => ("meetings.get", json!({"meeting_id":id}), false),
            MeetingsCommand::Export { id, format } => (
                "meetings.export",
                json!({"meeting_id":id,"format":format.as_str()}),
                false,
            ),
        },
        Command::Transcript { command } => match command {
            TranscriptCommand::Get { id, page } => (
                "transcript.get",
                json!({"meeting_id":id,"cursor":page.cursor,"limit":page.limit}),
                false,
            ),
            TranscriptCommand::Search { query, page } => (
                "transcripts.search",
                json!({"query":query,"cursor":page.cursor,"limit":page.limit}),
                false,
            ),
        },
        _ => return None,
    })
}

/// Advances the durable event cursor even when filtering events out. Resuming
/// from it therefore does not duplicate already delivered event IDs.
pub fn event_page(
    page: Value,
    cursor: &mut Option<String>,
    events: &[String],
) -> Result<Vec<Value>, RpcError> {
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| RpcError::internal("Gateway returned an invalid events page"))?;
    let mut output = Vec::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(|id| {
                id.as_str()
                    .map(str::to_owned)
                    .or_else(|| id.as_u64().map(|id| id.to_string()))
            })
            .ok_or_else(|| RpcError::internal("Event lacks a stable ID"))?;
        if cursor.as_ref().is_some_and(|old| {
            old == &id
                || old
                    .parse::<u64>()
                    .ok()
                    .zip(id.parse::<u64>().ok())
                    .is_some_and(|(old, id)| id <= old)
        }) {
            continue;
        }
        *cursor = Some(id);
        if events.is_empty()
            || item
                .get("event")
                .and_then(Value::as_str)
                .is_some_and(|event| events.iter().any(|e| e == event))
        {
            output.push(item.clone());
        }
    }
    if let Some(next) = page.get("next_cursor").and_then(Value::as_str) {
        let regresses = cursor
            .as_deref()
            .and_then(|old| old.parse::<u64>().ok())
            .zip(next.parse::<u64>().ok())
            .is_some_and(|(old, next)| next < old);
        if !regresses {
            *cursor = Some(next.to_string());
        }
    }
    Ok(output)
}

/// Read-only finite wait. A cancelled wait never modifies the recorder.
pub async fn wait_finalized(
    gateway: &Gateway,
    recording_id: &str,
    timeout: Duration,
) -> Result<Value, RpcError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut cursor = None;
    let result = async {
        loop {
            let page = gateway
                .call("events.list", json!({"after":cursor,"limit":200}), false)
                .await?;
            let events = event_page(page, &mut cursor, &[])?;
            for event in events {
                if event.get("recording_id").and_then(Value::as_str) == Some(recording_id) {
                    if event.get("event").and_then(Value::as_str) == Some("meeting.finalized") {
                        return Ok(event);
                    }
                    if event.get("event").and_then(Value::as_str) == Some("recording.failed") {
                        return Err(RpcError::new(
                            "unavailable",
                            "Recording processing failed; inspect doctor and the partial meeting",
                        ));
                    }
                }
            }
            let status = gateway
                .call("status", json!({"recording_id":recording_id}), false)
                .await?;
            if let Some(recording) = status.get("recording") {
                if recording.get("recording_id").and_then(Value::as_str) == Some(recording_id)
                    && recording.get("state").and_then(Value::as_str) == Some("finalized")
                {
                    return Ok(recording.clone());
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    tokio::time::timeout_at(deadline, result)
        .await
        .map_err(|_| {
            RpcError::new(
                "timeout",
                format!(
                    "Recording {recording_id} did not finalize within {} seconds",
                    timeout.as_secs()
                ),
            )
        })?
}
