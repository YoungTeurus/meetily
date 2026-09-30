# Meetily Calls CLI and Codex MCP

`meetilyctl` controls the **running local Meetily Calls app**. Its gateway uses the app's recorder, speech settings and database. It never opens SQLite or records audio independently. Enable **Local integrations** in the app first; enable the separate recording control permission if you intend to start/stop from CLI or MCP. These permissions can be revoked in the app.

## Build and install

Rust 1.88 or newer is required by the official [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK (locked at 3.5.0). This independent workspace builds without desktop audio libraries:

```sh
cargo test --locked --manifest-path meetilyctl/Cargo.toml
cargo build --locked --release --manifest-path meetilyctl/Cargo.toml
```

On macOS, copy `meetilyctl/target/release/meetilyctl` to a user-owned directory such as `/Users/alice/bin/meetilyctl`. On Windows, use `meetilyctl\target\release\meetilyctl.exe`, for example `C:\Users\Alice\bin\meetilyctl.exe`. Replace example usernames with your actual absolute paths. The packaged desktop app and CLI must run on the same machine.

## Discovery and credentials

The app creates `integration.json` in its isolated fork app-data directory:

| OS | Default location |
| --- | --- |
| macOS | `~/Library/Application Support/com.youngteurus.meetily.calls/integration.json` |
| Windows | `%APPDATA%\com.youngteurus.meetily.calls\integration.json` |
| Linux | `$XDG_DATA_HOME/com.youngteurus.meetily.calls/integration.json`, falling back to `~/.local/share/…` |

`MEETILY_INTEGRATION_FILE` overrides the location. The file contains a loopback port, a read credential and an optional separate control credential. Secrets never belong in command-line arguments. The CLI connects only to `127.0.0.1`, ignores proxy environment variables and refuses HTTP redirects. If the app is closed or integrations are disabled, commands report a diagnostic error; they do not launch it implicitly. The client rereads its discovery file for each request, following app port/key changes after a restart. Restart MCP after changing whether recording control should be exposed.

## Commands

```sh
meetilyctl doctor --json
meetilyctl status --json
meetilyctl devices list --json
meetilyctl recording start --name "Zoom meeting" --language ru --idempotency-key my-meeting-001
meetilyctl recording pause --recording-id RECORDING_ID
meetilyctl recording resume --recording-id RECORDING_ID
meetilyctl recording stop --recording-id RECORDING_ID
meetilyctl recording wait --recording-id RECORDING_ID --until finalized --timeout 120 --json
meetilyctl meetings list --limit 20 --order desc --state finalized --json
meetilyctl meetings get MEETING_ID --json
meetilyctl meetings export MEETING_ID --format md
meetilyctl transcript get MEETING_ID --limit 200 --json
meetilyctl transcript search "decision" --limit 20 --json
meetilyctl watch --events recording.started,recording.stopped,meeting.finalized --json
```

Start supports `--input-device`, `--output-device` and `--idempotency-key`. IDs/device names and language validation belong to the app. `--language ru` requires Whisper; Parakeet uses automatic language detection. Commands for recording control require the separate control credential. `status --recording-id RECORDING_ID` selects a specific historical or active session. Stop/pause/resume may target the current recording when `--recording-id` is omitted; supply the returned ID when automating them. Wait always requires an explicit recording ID and has a finite timeout (default 120 seconds, allowed 1–86400). It observes saved finalization events, never stops or starts recording. A successful stop means capture stopped; wait for finalization before assuming all transcript segments are ready.

`--json` is accepted before or after subcommands. Successful output is JSON on stdout; typed errors and diagnostics use stderr. Without `--json`, results are readable JSON and exports print their content. List/search/transcript commands return one explicitly bounded page with `next_cursor`; pass `--cursor NEXT_CURSOR` until it is null. No result silently claims a complete transcript after its first page. In-progress transcripts include `partial`.

Watch emits NDJSON regardless of `--json`, with durable event IDs, ISO 8601 event times, recording/meeting IDs and event payloads. `--cursor EVENT_ID` resumes a saved cursor; filters still advance it. It reconnects on transient unavailability with bounded backoff and suppresses replay of previous numeric event IDs. Ctrl+C cancels watching/waiting without changing recording. An authentication failure is retried once to cover rotation during an in-flight request; persistent authentication errors and forbidden scope are terminal. Polling defaults to 500 ms; `--poll-ms` accepts 50–60000 ms. Save the last consumed event ID to resume after restarting the CLI.

| Exit | Meaning |
| --- | --- |
| 0 | Success or watch/wait cancelled with Ctrl+C |
| 1 | Internal/protocol/output/persistence failure; unknown future error code |
| 2 | Invalid arguments/request, invalid language or unsupported language |
| 3 | App/gateway unavailable, setup incomplete, recording start/transition failed |
| 4 | Unauthorized, revoked key or forbidden scope |
| 5 | Meeting/recording/method not found, no active recording |
| 6 | Conflicting lifecycle operation, audio batch/model operation active, mismatched ID, or transcript revision changed during pagination |
| 7 | Gateway request/wait timed out |

## Connect local Codex

The stdio server uses the official MCP Rust SDK; stdout contains protocol messages only. Connecting grants read tools by default, even when a control token exists. Neither connecting nor cancelling an MCP request starts/stops recording.

macOS:

```sh
codex mcp add meetily -- /Users/alice/bin/meetilyctl mcp serve
```

Windows PowerShell:

```powershell
codex mcp add meetily -- C:\Users\Alice\bin\meetilyctl.exe mcp serve
```

Equivalent entries in your existing Codex `config.toml` (choose the appropriate platform; do not replace the rest of the file):

```toml
# macOS
[mcp_servers.meetily]
command = "/Users/alice/bin/meetilyctl"
args = ["mcp", "serve"]
```

```toml
# Windows: TOML literal strings preserve backslashes.
[mcp_servers.meetily]
command = 'C:\Users\Alice\bin\meetilyctl.exe'
args = ["mcp", "serve"]
```

For a non-default credential path, add the override to this server's environment:

```toml
[mcp_servers.meetily.env]
MEETILY_INTEGRATION_FILE = "/absolute/path/integration.json"
```

Read tools: `list_meetings`, `get_meeting`, `get_transcript`, `search_transcripts`, `export_meeting`, `get_recording_status`, `get_detection_status`. They expose filters, stable pagination, processing/partial state and timestamps returned by the app. Exports return content and never write arbitrary files. The optional `save_summary` tool is not implemented.

To deliberately expose audio control, enable control permission in Meetily and change the configured args to `["mcp", "serve", "--allow-control"]`. Only the combination of flag **and** available control credential exposes `start_recording` and `stop_recording`. Stop requires a recording ID. Tools carry MCP annotations and gateway errors appear as structured tool results with `isError: true` and an `error.code`.

Example Codex request:

> Summarize my latest finalized meeting. Use the returned meeting date and processing state to choose it. Read every transcript page. Give a concise overview, decisions, tasks with owners only when stated, and open questions. Include timestamps; preserve unknown speakers and do not infer their identity.

The bridge runs locally, but meeting text returned to Codex is submitted to the AI service selected in your Codex configuration. The app itself does not need an OpenAI API key or a summary/LLM model to provide transcripts.

## Verification scope

`cargo test` includes a real `rmcp` client spawning the binary over stdio (initialize, tools/list, tools/call, pagination, errors, control gating), a raw JSON-line stdout test, authenticated HTTP fixtures, CLI process tests, finite wait and watch reconnection/cursor tests. A separate official-SDK integration test uses the production gateway and actual SQLx storage to read all 503 timestamped segments across three pages, export content and verify revocation. HTTP fixtures test the client contract; they do not prove native recording or real Zoom/Discord detection. Run the desktop app/native platform checklist separately.
