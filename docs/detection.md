# Desktop call detection

Detection is opt-in. It never starts recording automatically. A current call offer must be accepted with **Start recording**, which delegates to the same backend recorder as the GUI, tray, CLI, and MCP. Login launch starts the application and its observer, never a recorder. Fully quitting the application ends observation.

## Implemented adapters and verification

| Application | Platform | Required evidence | Capability / verification |
| --- | --- | --- | --- |
| Zoom desktop | macOS Apple Silicon, including 14.8.5 | Registered `us.zoom.xos` application; call-specific Accessibility controls such as Leave Meeting / End Meeting, or End / Leave together with Participants; CoreAudio process input/output activity raises confidence | Native code written and `aarch64-apple-darwin` Rust type-check passed. Real calls and current Zoom accessibility labels await interactive testing. |
| Discord desktop / Canary / PTB | macOS Apple Silicon | Registered Discord bundle; Disconnect control together with Voice Connected / RTC Connected status in its adjacent call-control group; CoreAudio process activity | Native code written and Apple target type-check passed. Voice-channel/direct-call accessibility exposure must be checked against installed Discord version. |
| Zoom desktop | Windows 10/11 x64 | Zoom executable and process creation identity; UI Automation Leave Meeting / End Meeting or End / Leave together with Participants; WASAPI active capture/render session mapped to root or helper PID | Native code written and `x86_64-pc-windows-gnu` Rust type-check passed. Native linking, installer activation, and real calls await Windows testing. |
| Discord desktop / Canary / PTB | Windows 10/11 x64 | Discord executable and creation identity; UI Automation Disconnect plus adjacent Voice Connected / RTC Connected status; WASAPI capture/render session activity | Native code written and Windows target type-check passed. Real calls await interactive testing. |
| Microsoft Teams desktop | Both | No adapter has sufficient verified call-specific evidence yet | `unsupported`; no process/audio-only confirmation. |
| Browser meetings | Both | Browser microphone activity cannot identify the service or distinguish a meeting from another capture | `unsupported`; no heuristic confirmation is advertised. |
| Any application | Linux | Native adapters target macOS and Windows | `unsupported`. |

The call-control vocabulary currently covers English and selected Russian labels. Changed/localized labels, missing accessibility trees, or scan limits reduce capability; an executable name or active audio session alone never confirms a call. The current implementation needs desktop clients to expose their call controls. It does not use private client RPC, credentials, injected code, undocumented Pro code, or screen OCR.

CoreAudio process objects were added in macOS 14.2. The adapter checks `AudioObjectHasProperty` before querying `kAudioHardwarePropertyProcessObjectList`; the user's macOS 14.8.5 satisfies this requirement. Selectors are checked against SDK-derived `objc2-core-audio` 0.3.2 generated declarations: `prs#`, `ppid`, `piri`, `piro`. No process audio tap API requiring a later OS is used. This detector does not add a Windows build requirement beyond the application's Windows 10/11 packaging: WASAPI audio session interfaces predate Windows 10, and the selected `CUIAutomation8`/`IUIAutomation2` interfaces require Windows 8 or newer. See the installation documentation for the product's supported Windows build floor.

These are source/type-check results, not proof of a real call. Linux cannot execute Apple frameworks, Windows COM, native desktop notifications, or physical audio devices. Apple and Windows native CI plus the manual checklist below remain required before describing the adapters as verified on calls.

## Permissions, evidence, and privacy

On macOS enable Meetily under **System Settings → Privacy & Security → Accessibility**. Without that permission, an installed/running client's call state is `unknown`, with an explicit limitation explaining that audio cannot distinguish a meeting, music, microphone tests, or text chat. Microphone and system-audio recording permissions are separate and remain owned by the existing recorder.

The macOS adapter reads a bounded accessibility tree for call-control roles, button names, and descriptions. Discord status text values are limited to two enclosing groups around the Disconnect button; message lists, text-input fields, and document text are excluded from that targeted scan. Windows uses the control view, reads button names, and reads status names only around Disconnect. No chat bodies are retained in observations or persisted. Missing/unresponsive provider data, no accessible window, and exhausted scan limits yield `unknown`, rather than declaring that a call ended.

Every observation reports application, process ID and creation identity, state, confidence, evidence strings, limitations, and UTC ISO 8601 `observed_at` in the Tauri/gateway result. `unknown` means that the adapter could not decide; `unsupported` means that no adequate adapter is implemented; `no_call` means readable absence of supported call controls or absence of the identified desktop process. `confirmed_call` always includes app-specific UI evidence. Correlated active audio gives `high` confidence; UI evidence while muted or silent gives `medium` confidence.

CoreAudio activity is correlated to root applications through their process ancestry, so an Electron/Zoom helper can supply audio evidence. Windows enumerates active capture and render endpoints, reads `IAudioSessionControl2::GetProcessId`, maps helper ancestry, and refreshes endpoint/session enumeration after device changes. OS process creation time prevents a reused PID from retaining an unrecorded old call offer. Mute, deafen, silence, and headphones do not end a call while its UI controls remain present. Observation itself does not capture audio. Meetily's recorder may capture audio from the whole computer; these observations do not imply per-application audio isolation.

## Session policy and settings

The pure policy is in `call-detection/src/session.rs`; integration is in `frontend/src-tauri/src/detection/mod.rs`. Observation runs on one dedicated OS thread, while recording continues under backend ownership. The observer samples, integrates the result, then waits one second; scans are bounded to 512 main control nodes and 500 ms on macOS / 600 ms on Windows, with native messaging timeouts and smaller bounded Discord status scans. The cadence includes scan time. This polling deliberately refreshes both UI and audio ownership: neither platform's audio events alone describe semantic call membership, and client accessibility providers can replace Electron control trees. There are no persistent capture streams, screen reads, or growing background queues. CPU cost and false-negative behavior need measurement on the actual clients; event-driven audio/AX/UIA wakeups are a future optimization rather than a claimed implementation.

Settings are persisted as `detection.json` in the fork's application data directory:

```json
{
  "enabled": false,
  "applications": ["zoom", "discord"],
  "debounce_ms": 3000,
  "grace_ms": 20000,
  "auto_stop": false,
  "notification_mode": "system"
}
```

`notification_mode` accepts `system` or `in_app`. Debounce accepts 500–60000 ms; grace accepts 5000–300000 ms. Confirmation must persist through consecutive observations for the debounce interval. `unknown` breaks a candidate's confirmation interval and cancels an exit interval. A positively observed exit must persist for the grace interval before a linked recording gets **Finish and save**. An absent process is positive exit evidence; lost accessibility permission is not.

Skip, user-dismissed notifications, and manual recording stop suppress another start offer for the same call session. A new call following confirmed exit gets a new session ID. A current independent GUI/CLI/MCP recording consumes pending offers and can never be stopped by this detector. Start reservations block duplicate actions across apps while keeping native observation running during model loading. A stale notification, deselected application, changed observation, or per-app observation older than five seconds cannot start capture. The backend checks the reservation again immediately before capture. Optional auto-stop is off by default and checks both the active recording ID and detected-session ID through the backend before stopping.

`detection-changed` carries `{settings, observations, sessions, prompts, platform, recording, error, observed_at, observer_interval_ms}`. Each prompt is `{kind:"start"|"stop", session_id, application}`. Commands are `get_detection_status`, `set_detection_settings`, and `detection_action` with `start`, `skip`, `dismiss`, or `stop`. The native notification integration is `call_notifications::show_prompt`, invoked only for a new system-mode offer after publishing the state. A notification opens the compact action window; activation does not itself start recording.

## Reproducible verification

```sh
cargo test --manifest-path call-detection/Cargo.toml
rustup target add aarch64-apple-darwin x86_64-pc-windows-gnu
cargo check --manifest-path call-detection/Cargo.toml --target aarch64-apple-darwin
cargo check --manifest-path call-detection/Cargo.toml --target x86_64-pc-windows-gnu
```

The fixture suite covers debounce, Unknown breaking debounce, configurable timing, simultaneous apps, skip/dismiss, stale start, PID reuse, mute/reconnect/device-change sequences, ownership-protected stop, manual-stop suppression, deselection, and a resumed/unknown call invalidating a pending stop. Native target `cargo check` type-checks platform code; it does not link frameworks or run clients.

On macOS 14.8.5 Apple Silicon and the supported Windows build, repeat these steps for current Zoom and Discord desktop versions and record the exact versions, observed evidence, timings, and outcome:

1. Enable detection. For macOS, first deny Accessibility: verify `unknown` with permission explanation, and no start offer. Grant permission and retry. Test unreadable/minimized/closed call UI separately.
2. Open Zoom idle, open Discord text chat, play a video/music, and run each client's microphone test. None may be labeled a confirmed call or start recording.
3. Join an actual call with the microphone already muted and headphones on. Verify that app-specific call controls confirm the call after the debounce interval. Listen to the other participant. Accept Start recording and confirm the single backend recording ID and correct initiator/session linkage.
4. Mute/unmute, Discord deafen/undeafen, remain silent, change default input/output devices, and reconnect for less than the grace interval. Verify that the recording continues and no duplicate start/stop offer appears.
5. Skip a fresh offer; close another with the user's dismissal action; manually stop another recording. Stay in the call and verify that no new start offer appears. Leave, wait for grace, rejoin, and verify a new session ID.
6. Leave a recorded call. Verify Finish and save appears only after confirmed exit plus grace. Rejoin before clicking an old stop offer and verify rejection. Click an old start offer after exit or after deselecting that client and verify rejection.
7. Start an independent GUI/CLI/MCP recording, then join/leave calls in both clients. Verify no second recorder and no detector stop of the independent recording. Enable auto-stop only for a linked-session test; confirm its backend ownership checks.
8. Hide Meetily to tray and repeat notification click/dismiss/start/skip/finish. On Windows use the installed packaged app to verify native toast activation. Exit Meetily entirely, then verify no observation and no recording at next login launch.
