use clap::Parser;
use meetilyctl::{
    cli::{self, Cli, Command, McpCommand, MeetingsCommand, RecordingCommand},
    gateway::{Gateway, RpcError},
    mcp::MeetilyMcp,
};
use rmcp::{transport::stdio, ServiceExt};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    time::Duration,
};

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                error.exit();
            }
            if std::env::args_os().any(|arg| arg == "--json") {
                eprintln!(
                    "{}",
                    json!({"error":RpcError::new("invalid_request",error.to_string())})
                );
                std::process::exit(2);
            }
            error.exit();
        }
    };
    if let Err(error) = run(&cli).await {
        if cli.json {
            eprintln!("{}", json!({"error":error}));
        } else {
            eprintln!("{error}");
        }
        std::process::exit(error.exit_code());
    }
}
fn print_json(value: &Value) -> Result<(), RpcError> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value)
        .map_err(|_| RpcError::internal("Unable to write stdout"))?;
    writeln!(out).map_err(|_| RpcError::internal("Unable to write stdout"))?;
    out.flush()
        .map_err(|_| RpcError::internal("Unable to flush stdout"))
}
async fn run(cli: &Cli) -> Result<(), RpcError> {
    let gateway = Gateway::discover()?;
    match &cli.command {
        Command::Mcp {
            command: McpCommand::Serve { allow_control },
        } => {
            let server = MeetilyMcp::new(gateway, *allow_control)
                .serve(stdio())
                .await
                .map_err(|e| RpcError::internal(format!("MCP initialization failed: {e}")))?;
            server
                .waiting()
                .await
                .map_err(|e| RpcError::internal(format!("MCP transport failed: {e}")))?;
            Ok(())
        }
        Command::Recording {
            command:
                RecordingCommand::Wait {
                    recording_id,
                    timeout,
                    ..
                },
        } => {
            let result = tokio::select! { result = cli::wait_finalized(&gateway,recording_id,Duration::from_secs(*timeout)) => result?, _ = tokio::signal::ctrl_c() => return Ok(()) };
            output(&result, cli.json)
        }
        Command::Watch {
            events,
            cursor,
            poll_ms,
        } => {
            let mut cursor = cursor.clone();
            let mut failures = 0u32;
            loop {
                let response = tokio::select! {
                    result = gateway.call("events.list",json!({"after":cursor,"limit":200}),false) => result,
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                };
                match response {
                    Ok(page) => {
                        failures = 0;
                        for event in cli::event_page(page, &mut cursor, events)? {
                            print_json(&event)?;
                        }
                    }
                    Err(error)
                        if matches!(error.code.as_str(), "unavailable" | "timeout")
                            || (error.code == "unauthorized" && failures == 0) =>
                    {
                        failures = failures.saturating_add(1);
                        if failures == 1 {
                            eprintln!(
                                "{error}; reconnecting with cursor {}",
                                cursor.as_deref().unwrap_or("start")
                            );
                        }
                    }
                    Err(error) => return Err(error),
                }
                let delay = if failures == 0 {
                    *poll_ms
                } else {
                    (*poll_ms)
                        .saturating_mul(2u64.pow(failures.min(5)))
                        .min(30_000)
                };
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(delay)) => (), _ = tokio::signal::ctrl_c() => return Ok(()) }
            }
        }
        command => {
            let (method, params, control) =
                cli::rpc_command(command).ok_or_else(|| RpcError::internal("Unhandled command"))?;
            let result = gateway.call(method, params, control).await?;
            if let Command::Meetings {
                command: MeetingsCommand::Export { .. },
            } = command
            {
                if !cli.json {
                    let content = result
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| RpcError::internal("Gateway export lacks content"))?;
                    let mut out = io::stdout().lock();
                    writeln!(out, "{content}")
                        .map_err(|_| RpcError::internal("Unable to write stdout"))?;
                    return Ok(());
                }
            }
            output(&result, cli.json)
        }
    }
}
fn output(result: &Value, json: bool) -> Result<(), RpcError> {
    if json {
        print_json(result)
    } else {
        let mut out = io::stdout().lock();
        serde_json::to_writer_pretty(&mut out, result)
            .map_err(|_| RpcError::internal("Unable to write stdout"))?;
        writeln!(out).map_err(|_| RpcError::internal("Unable to write stdout"))
    }
}
