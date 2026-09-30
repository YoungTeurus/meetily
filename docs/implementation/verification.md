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
| Recording | `frontend/src-tauri/src/control/recording.rs`, `recording_store.rs`, audio worker and stop/drain integration; GUI/tray/CLI/MCP share serialization | SQL transaction/crash/idempotency/finalization tests plus actual GUI/shared dispatch headless orchestration with fake capture; real audio acceptance still requires target systems |
| Local gateway | `local-control/src/gateway.rs`, desktop `control/gateway.rs` | Real HTTP router tests, scope/origin/Host rejection, cancellation semantics |
| Read/export/import | `local-control/src/store.rs`, `import.rs` | Real SQLite integration tests, mixed timestamp sorting, full transcript pages, explicit backup/version validation |
| CLI/MCP | `meetilyctl/` | Official SDK client initialize/list/call, executable subprocesses, actual gateway/database smoke |
| Detection | `call-detection/`, desktop `detection/mod.rs` | Pure state fixtures and native API cross-typechecking; no real Zoom/Discord call in this cloud |
| Notifications | `call_notifications.rs`, `CallPrompts.tsx`, `/call-action` | Frontend fixtures; Windows APIs cross-typechecked; installed app toast activation awaits native OS test |
| Settings/onboarding | `CallIntegrationSettings.tsx`, config/onboarding contexts | Frontend tests/typecheck; no required summary model |
| Packaging | `.github/workflows/calls-preview.yml`, `scripts/build-calls-preview.cjs` | Native CI/installer outcomes and exact blockers appended below |

macOS native minimum is 14.2 for public Core Audio process objects; 14.8.5 meets that API floor. The target is Apple Silicon. Windows minimum is Windows 10 version 2004, build 19041: pinned ONNX Runtime 1.22.0 has a non-delay import of DXCoreCreateAdapterFactory from dxcore.dll. Windows 11 x64 meets this floor. Installer/actual call execution on the minimum OS still requires native verification; a Windows GNU type-check or Windows Server CI runner is not that test. See DLL hash/import evidence in native-build.md.

Teams and browser-specific detection are `unsupported`, not guessed from a process or browser microphone. Optional `save_summary` is not implemented. macOS uses an immediately opened compact action window because the available notification plugin lacks an activation callback. Full real-call/permission/device/sleep/resume verification remains pending on both target OSes; [manual checklist](../CALLS_PREVIEW.ru.md#ручная-приёмка-на-обеих-ос).

### User preview observation

On 2026-09-30 the user tested the macOS Preview with **Zoom 7.1.9, Russian UI**. A real Zoom call offer appeared; after switching to the compact Meetily window it disappeared with a delay, sometimes allowing a click before it vanished. Screenshots show the offer followed by the empty-offer state. The user reports automatic completion “seems to work.” This is useful real-client feedback, but not a completed end-to-end audio/transcript/permissions acceptance run. The implemented fix retains an offer during transient unknown state, adds enabled Zoom meeting-menu evidence and revalidates an explicit Start. Regression fixtures pass; the new build still needs the user's real-client retest. The detector does not inspect window titles, so a title change has not been established as the cause.

## Final run results and artifacts

### Codex CLI, automatic retranscription and live notes follow-up

The user confirmed successful manual retranscription in the current macOS Preview and requested Codex CLI inside Generate summary, optional automatic processing after **every** finalized recording, and notes during the call. These additions are implemented in the follow-up tree; the `cb1337e` artifact below does not contain them.

- Full Linux desktop: **274 passed, 0 failed, 3 ignored**, 7.45 seconds (`/tmp/meetily-followup-final-native.log`); final production check passed in 12.81 seconds (`/tmp/meetily-followup-final-production.log`). Includes Codex subprocess failures/timeouts/cancellation/descendant pipes, persisted configuration, shared recording priority and engine exclusion. Nine Codex module tests also passed independently; Apple Silicon and Windows MSVC source checks passed, including the Windows job-object test's compilation.
- Gateway/storage: **27 passed**, including durable automatic-queue handoff/snapshots/recovery/atomic cancel and four notes persistence/summary-context regressions (`/tmp/meetily-followup-control-tests.log`). Clippy with warnings denied passed.
- Frontend: **56 top-level tests passed, 0 failed, 198 assertions**, plus isolated component scenarios including notes autosave/conflicts, authoritative recording identity, settings and summary flush ordering. Seven dedicated auto-summary readiness scenarios verify waiting for background processing, failure/cancellation fallback and fail-closed status-query errors. Explicit Generate/Regenerate/Stop consumes deferred automatic intent. TypeScript and the final production Next.js export passed after the coordination change. Logs: `/tmp/meetily-auto-summary-full-ui.log`, `/tmp/meetily-followup-final-next.log`.
- Actual installed Codex **0.159.0-alpha.3** was run against a localhost fake Responses endpoint. Exactly one request arrived, with **`tools: []`**; the endpoint deliberately returned HTTP 400 to stop execution. This proves tool advertisement for that tested version/configuration, not real-account summary quality. No paid model request or user-authentication changes were performed.
- Windows resource-link fixture passed: exactly one generated Tauri resource reaches the library test harness and exactly one reaches the production application's linker. Actual MSVC execution and packaging require the next native run.
- Independent integration review found and prompted fixes for explicit Codex path clearing, native npm executable discovery with Finder's restricted PATH, and automatic-processing admission. No remaining concrete P1/P2 finding at handoff to native CI.

Automatic processing remains cooperative at native audio-fragment boundaries: recording start waits up to 30 seconds, then returns a retryable `audio_busy` error if the current native operation has not yielded. Real logged-in Codex summary generation, after-call inference and live notes with physical audio remain local acceptance checks.

### Current native preview

[Native CI run 36735969234](https://github.com/YoungTeurus/meetily/actions/runs/36735969234) builds implementation **`cb1337e2d8345f6ff56957468b9ce2ae70857cde`**, with PR merge checkout **`e103c880251701f4c0d47e1231cb122a8a025b55`**. Later documentation-only commits do not change this tested implementation.

- **macOS ARM64: succeeded.** Native desktop tests: **264 passed, 0 failed, 4 ignored** (9.20 seconds). Gateway/storage 19, detector 30, CLI/MCP 17 passed. Frontend tests and types passed. Release `.app`, DMG and standalone CLI built.
- [Download macOS DMG and CLI archive](https://github.com/YoungTeurus/meetily/actions/runs/36735969234/artifacts/11108266151), 51,322,358 bytes, expires **2026-10-14**. GitHub archive SHA256: `be2b236ced072c57fd7db7d1bbeb39688d526c8d022cdd717f0f5de16f0c77f6`.
- **Windows x64 native tests: succeeded**, desktop **264 passed, 0 failed, 3 ignored** (20.15 seconds), including sequential/concurrent device enumeration after caller teardown; gateway/storage 19, detector 30 and CLI/MCP 17 passed. Release linking then failed with `CVT1100: duplicate resource, type VERSION, name 1` and `LNK1123`. No installer was produced by this run. The follow-up patch links the Tauri resource only into the library test harness; the production application retains Tauri's sole resource argument. Cross-target linker-argument fixtures pass; native packaging must be rerun.

These are Preview artifacts, not a published release: macOS has an ad-hoc signature only, with no Developer ID/notarization; Windows packages have no publisher signature. Successful native CI proves compilation, automated tests and packaging, not installation on the minimum OS or real Zoom/Discord audio/notification behavior. See the [physical acceptance checklist](../CALLS_PREVIEW.ru.md#ручная-приёмка-на-обеих-ос).

### Preview feedback follow-up

- Complete Linux desktop suite after retranscription, vocabulary, focus-policy and model-guard integration: **263 passed, 0 failed, 3 ignored** (7.75 seconds), `/tmp/meetily-rerun-final-native.log`. This includes actual shared recorder orchestration, job-specific cancellation/commit boundary tests, model mutation exclusion, and summary invalidation/fresh regeneration. Tests touching production recorder globals share a test-only serial boundary.
- Gateway/storage: **19 passed**, including five new vocabulary/replacement transaction cases and a transcript cursor regression. The regression first reproduced mixed old/new pages after SQLite reused rowids; cursors now bind a transcript revision and rows/revision are read in one SQLite snapshot. Clippy with warnings denied passed.
- CLI/MCP: **17 passed** after the storage change; the stable exit-code test also passed after mapping new `audio_busy` conflicts to exit 6. Detector policy/native evidence fixtures: **30 passed**, with Apple Silicon and Windows source checks passed.
- Frontend: **50 top-level tests passed**, with all **30 isolated component scenarios** executed, including 17 retranscription/recovery scenarios (60 child assertions). TypeScript passed. Review fixes cover hidden-dialog completion, terminal status recovery with retained warnings, page refresh, and model readiness commands that can implicitly load an engine.
- CPAL 0.15.3 is patched at its root cause with thread-local Windows enumeration, preserving version and dependency graph; Linux and Windows MSVC source checks passed. Native run 36735969234 subsequently passed the actual Windows runtime regression. Windows packaging remains a separate linker-resource issue, described above.
- Whisper prompt tokenization and native API integration compile against the actual locked engine; helper tests cover Unicode, empty prompts, ordering, cap and tokenizer errors. No model inference or accuracy comparison on the user's recorded audio has been executed in this cloud.

### Initial implementation and native build history

- Full Linux desktop check: passed. Full desktop library tests: 251 passed, 0 failed, 3 ignored (7.21 seconds after compilation), including the headless shared lifecycle test. The immediately preceding run passed 252 with 2 ignored; the physical output-device assertion is now an explicitly manual test because the Windows CI runner has no default audio device.
- Local gateway/storage: 13 passed; Clippy `--all-targets -- -D warnings` passed (3 access/cancellation, 1 import, 3 read/pagination, 6 exact desktop persistence tests).
- CLI/MCP: 17 passed. Clippy with `-D warnings`, formatter check and Linux release build passed. Installed `codex mcp add --help` confirms configuration command syntax; user configuration was not modified.
- Detector: 17 fixture tests passed; Apple Silicon and Windows GNU native source checks passed.
- Frontend: 49 top-level tests passed, including subprocess wrappers that execute all 13 new behavioral cases (33 child assertions). The first combined run exposed Bun module-mock contamination; isolation fixes preserve every behavioral assertion. TypeScript and Next.js static build passed.
- [Draft PR #1](https://github.com/YoungTeurus/meetily/pull/1), first [native CI run 36723904371](https://github.com/YoungTeurus/meetily/actions/runs/36723904371), implementation `d9a43d85c06574e9a797a6f99422759ad7962437`: macOS 14.8.9 ARM64 compiled native code and passed gateway 13, detector 17, CLI/MCP 17 and desktop 252 tests (3 ignored); the CLI release binary built. Desktop packaging then failed Tauri minor-version preflight (Rust 2.12.0 vs npm API 2.11.0). npm API is now pinned to 2.12.0 and native packaging is being rerun with the lifecycle event fix. No signed release or real-call verification is claimed.
- Independent review found and prompted fixes for finalization retry/atomic events, pending transcript preservation, disk envelope recovery, stale/deselected/PID-reused call actions, observation during long stop, UI completion and mixed datetime ordering. After fixes, no concrete P1/P2 remained in the reviewed code. The additional headless orchestration test exercises real GUI commands/shared dispatch, SQLite, Store and Tauri event delivery with test-only capture hardware. It exposed invalid dotted Tauri event names: native transport now uses colons while durable gateway/CLI event names retain dots. It covers competing starts/stops, idempotent retry, caller cancellation, processing-before-finalized and a post-stop tail segment; physical concurrent GUI/CLI/MCP audio acceptance remains pending.
- [Native run 36727038135](https://github.com/YoungTeurus/meetily/actions/runs/36727038135), implementation `3059839efdc5b40b362819d7b3c126847bac1100`, PR merge checkout `4489b0e33d3ba6ef4da5abb6d6266bfe8de46a2c`: **macOS succeeded**, desktop 252 passed / 0 failed / 4 ignored, gateway 13, detector 17, CLI/MCP 17. Actual ARM64 `.app`, `.dmg` and release CLI were built. [Download unsigned/ad-hoc macOS preview](https://github.com/YoungTeurus/meetily/actions/runs/36727038135/artifacts/11103254035); artifact expires 2026-10-14. Archive SHA256: `2237af16c9e9d7ec7c77bb7e0164189e8b81486fdd1ada88a34cfa843e7d0272`. Windows passed the standalone suites and compiled/linked the desktop test executable, but its loader failed before running tests with `STATUS_ENTRYPOINT_NOT_FOUND` (`0xc0000139`); no Windows installer was produced by this run.
- The Windows library-test executable did not receive Tauri's Common Controls v6 manifest: locked `embed-resource` emits `rustc-link-arg-bins`, while retained `muda` code imports `TaskDialogIndirect`. `build.rs` now also links the existing generated resource into library tests on MSVC. The follow-up native run must verify this diagnosis and installer linking; failure-only loader diagnostics are included. Linux production check after this change passed (6.07 seconds).
- [Follow-up run 36730915943](https://github.com/YoungTeurus/meetily/actions/runs/36730915943), implementation `05010213e993ae35d013d74140795db0eeb40439`: Windows now launches the desktop tests, confirming that embedding the resource fixes the pre-main loader failure. Diagnostics show `cargo --list exit: 0` and `mt manifest extraction exit: 0`. Serial tests then terminate with `STATUS_ACCESS_VIOLATION` in `audio::system_audio_commands::tests::test_list_system_audio_devices`; this native enumeration fault is under investigation, not skipped. Standalone gateway/detector/CLI suites passed again. [Windows diagnostic artifact](https://github.com/YoungTeurus/meetily/actions/runs/36730915943/artifacts/11104424083).
