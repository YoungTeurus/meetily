# Meetily local control and call detection implementation plan

**Goal:** User-confirmed recording of desktop calls, durable backend transcripts, CLI and read-first MCP for local Codex.
**Spec:** [requirements.ru.md](requirements.ru.md). User explicitly requests implementation after this plan without another design approval.
**Architecture:** The existing Rust audio engine remains the only recorder. Backend lifecycle owns incremental database persistence and finalization. A versioned authenticated loopback gateway delegates to that same lifecycle; a separate Rust CLI/MCP client never opens SQLite. Native observation feeds a pure call-session state machine; recording requires an explicit user action.
**Stack:** Tauri/Rust, Next.js/TypeScript, SQLx/SQLite, official MCP SDK.

## Baseline

- Fork: https://github.com/YoungTeurus/meetily; default branch `main` confirmed via `git ls-remote --symref origin HEAD` on 2026-09-30.
- Upstream: https://github.com/Zackriya-Solutions/meetily.
- Base: `a2cb62e827da7ef59f65064c97233efb2313878e` (Community 0.4.1).
- Clean initial checkout; isolated task branch `enhance/local-control-call-detection` in the supplied cloud checkout.
- Existing stop explicitly delegates database persistence to React. Transcript listener is removed before the transcription task drains. Both must change.
- Host Linux; native interactive audio verification requires macOS/Windows. Initial Rust tools are installed under `/workspace/tooling`, outside PATH; existing setup log reports missing GLib development libraries. Fresh baseline commands/results will be recorded separately.

## Global constraints

- macOS Apple Silicon including 14.8.5; Windows 10/11 x64 with documented dependency-derived build floor.
- No automatic recording on detection/restart; no upstream changes, releases or merge.
- MIT preserved, no Pro code, no new FastAPI recorder, no required summary model.
- Integrations opt-in, independently revocable read/control keys; reject browser origins. Secrets in protected local files.
- Distinguish written, compiled, fixture-tested, MCP-client-tested and real-call-tested.

## Implementation tasks

- [x] Backend lifecycle (`audio/recording_commands.rs`, `control/recording.rs`, persistence): test concurrent/idempotent start/stop, tail segments, crash recovery; serialize transitions, persist segments before events, finalize transaction before `meeting.finalized`; remove React save duplication.
- [x] Gateway (`control/mod.rs`, `control/gateway.rs`, `control/store.rs`): authenticated POST `/v1/rpc` with `{method,params}`; success `{result:...}` / failure `{error:{code,message}}`. Per-request scope and origin checks. Methods `doctor`, `status`, `devices.list`, `recording.start/stop/pause/resume`, `meetings.list/get/export`, `transcript.get`, `transcripts.search`, `events.list`, `detection.status`. Persisted event IDs and bounded pages; test access and pagination.
- [x] CLI/MCP (`meetilyctl/`): same RPC contract and discovery file `integration.json` in fork app data, optional `MEETILY_INTEGRATION_FILE` override. Official SDK stdio server; read tools default, control tools only with control credential. Test CLI and real SDK initialize/list/call, stdout purity, finite wait and event reconnect.
- [x] Detection (`call-detection/`, `frontend/src-tauri/src/detection/`): native Core Audio + Accessibility and WASAPI + UI Automation observations, honest evidence/permission limitations. Pure state tests for 3s debounce, 20s grace, skip, late action, mute, process/device/reconnect, independent recordings. No process-only confirmed calls.
- [x] UI/integration: shared backend commands, settings, compact action window, tray lifecycle, autostart opt-in, no required summarization; backend language/devices settings. Reuse existing visible recording indicators.
- [ ] Packaging/CI/docs: isolated fork identity/data/update channel; native macOS/Windows tests and installer/CLI artifacts without release publication; local setup/MCP examples/capability matrix/manual audio test checklist. Verify and attempt draft PR in fork.

## Review focus

1. Late segments and UI absent: database remains authoritative and no early finalization.
2. Cancelled network request: operation continues under backend ownership without duplicate recording.
3. Reused PID or stale notification: session identity checked before action.
4. Browser-origin request, revoked credentials, disabled integrations: denied before dispatch.
5. Large/in-progress transcripts: stable order/cursors and explicit partial status; no silent truncation.

## Execution

Inline integration with independent, file-owned CLI/MCP and detector work delegated under `dispatching-parallel-agents`; all use the contracts above. Test-first for shared logic. Native build/real-call limitations are reported, never inferred from Linux mocks. Record actual verification in `verification.md`.

Implementation review completed with fixes for persistence retry and stale action cases. Native installers/real-call acceptance remain separate tracked verification steps; see verification.md.
