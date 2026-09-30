# Verification record

## Source and tools

- Fork: https://github.com/YoungTeurus/meetily (`main`, verified against remote HEAD on 2026-09-30).
- Upstream: https://github.com/Zackriya-Solutions/meetily.
- Base SHA: `a2cb62e827da7ef59f65064c97233efb2313878e`, Community 0.4.1.
- Task branch: `enhance/local-control-call-detection`.
- Linux cloud toolchain: Rust/Cargo 1.98.1; Node 24.19.0; pnpm 9.15.9; Bun 1.3.14. Native CI uses current stable Rust and Node 20.
- MIT license retained. No Pro Edition source used. No release or upstream changes authorized/performed.

## Baseline, before changes

| Command | Observed result |
|---|---|
| `cd frontend && bun test` | 45 passed, 0 failed, 158 assertions |
| `cd frontend && pnpm exec tsc --noEmit` | Exit 0 |
| `cd frontend && pnpm build` | Exit 0, Next.js static export |
| `cargo test -p meetily --lib --no-default-features` | Failed before project compilation: missing `glib-2.0 >= 2.70` development library |

The initial Rust failure was environmental, not a passing baseline. System prerequisites were subsequently extracted into a user-local sysroot; see [native-build.md](native-build.md). Official ONNX Runtime 1.22.0 was obtained from GitHub after the dependency CDN was blocked. The genuine llama-helper and FFmpeg sidecars were built/downloaded; no placeholder binaries were used.

## Current checks

The following are actual checks, with final counts/results updated before handoff:

- `scripts/check-linux-native.sh`: full Tauri desktop `cargo check -p meetily --lib` passed with existing warnings after fixing generated Tauri command registration paths. This includes the actual audio engine and platform libraries, not only common-code mocks.
- `cargo test --manifest-path local-control/Cargo.toml`: authentication/browser rejection, cancelled-request ownership, complete 503-segment export/pagination, date-order normalization, event resumption, source import WAL snapshot/schema checks, and the exact desktop persistence module.
- `cargo test --manifest-path meetilyctl/Cargo.toml`: CLI subprocess and official rmcp SDK client protocol tests, including real gateway+SQLite reads, pagination/export, revoked keys, finite wait, discovery-file rotation/restart, stdout protocol framing.
- `cargo test --manifest-path call-detection/Cargo.toml`: deterministic observation/session policy fixtures. These do not simulate native APIs or establish real-call accuracy.
- Cross-target `cargo check` for the native detector on `aarch64-apple-darwin` and `x86_64-pc-windows-gnu`: platform source/type checking; not native linking or execution.
- Frontend TypeScript/build/tests and full desktop test outcomes are recorded after their final runs below.

## Architecture and acceptance boundaries

| Area | Implementation | Verification boundary |
|---|---|---|
| Recording | `frontend/src-tauri/src/control/recording.rs`, `recording_store.rs`, audio worker and stop/drain integration; GUI/tray/CLI/MCP share serialization | SQL transaction/crash/idempotency/finalization tests; real audio/concurrent client acceptance still requires target systems |
| Local gateway | `local-control/src/gateway.rs`, desktop `control/gateway.rs` | Real HTTP router tests, scope/origin/Host rejection, cancellation semantics |
| Read/export/import | `local-control/src/store.rs`, `import.rs` | Real SQLite integration tests, mixed timestamp sorting, full transcript pages, explicit backup/version validation |
| CLI/MCP | `meetilyctl/` | Official SDK client initialize/list/call, executable subprocesses, actual gateway/database smoke |
| Detection | `call-detection/`, desktop `detection/mod.rs` | Pure state fixtures and native API cross-typechecking; no real Zoom/Discord call in this cloud |
| Notifications | `call_notifications.rs`, `CallPrompts.tsx`, `/call-action` | Frontend fixtures; Windows APIs cross-typechecked; installed app toast activation awaits native OS test |
| Settings/onboarding | `CallIntegrationSettings.tsx`, config/onboarding contexts | Frontend tests/typecheck; no required summary model |
| Packaging | `.github/workflows/calls-preview.yml`, `scripts/build-calls-preview.cjs` | Native CI/installer outcomes and exact blockers appended below |

macOS native minimum is 14.2 for public Core Audio process objects; 14.8.5 meets that API floor. The target is Apple Silicon. Windows minimum is Windows 10 version 2004, build 19041: pinned ONNX Runtime 1.22.0 has a non-delay import of DXCoreCreateAdapterFactory from dxcore.dll. Windows 11 x64 meets this floor. Installer/actual call execution on the minimum OS still requires native verification; a Windows GNU type-check or Windows Server CI runner is not that test. See DLL hash/import evidence in native-build.md.

Teams and browser-specific detection are `unsupported`, not guessed from a process or browser microphone. Optional `save_summary` is not implemented. macOS uses an immediately opened compact action window because the available notification plugin lacks an activation callback. Full real-call/permission/device/sleep/resume verification remains pending on both target OSes; [manual checklist](../CALLS_PREVIEW.ru.md#ручная-приёмка-на-обеих-ос).

## Final run results and artifacts

- Full Linux desktop check: passed. Full desktop library tests: 251 passed, 0 failed, 2 ignored (7.28 seconds after compilation). Final source check and test rerun also passed (251 passed, 2 ignored).
- Local gateway/storage: 13 passed; Clippy `--all-targets -- -D warnings` passed (3 access/cancellation, 1 import, 3 read/pagination, 6 exact desktop persistence tests).
- CLI/MCP: 17 passed. Clippy with `-D warnings`, formatter check and Linux release build passed. Installed `codex mcp add --help` confirms configuration command syntax; user configuration was not modified.
- Detector: 17 fixture tests passed; Apple Silicon and Windows GNU native source checks passed.
- Frontend: 49 top-level tests passed, including subprocess wrappers that execute all 13 new behavioral cases (33 child assertions). The first combined run exposed Bun module-mock contamination; isolation fixes preserve every behavioral assertion. TypeScript and Next.js static build passed.
- Native macOS/Windows installers and workflow URLs will be recorded after the draft PR starts CI. No signed release or real-call verification is claimed.
- Independent review found and prompted fixes for finalization retry/atomic events, pending transcript preservation, disk envelope recovery, stale/deselected/PID-reused call actions, observation during long stop, UI completion and mixed datetime ordering. After fixes, no concrete P1/P2 remained in the reviewed code. Pure persistence tests do not replace physical concurrent GUI/CLI/MCP audio acceptance.
