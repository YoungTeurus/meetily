# Native preview builds and verified build limits

The preview workflow is `.github/workflows/calls-preview.yml`. It has macOS ARM64 (`macos-14`) and Windows x64 (`windows-2022`) jobs. Each runs the standalone gateway, detector and CLI/MCP tests, the full desktop library tests, the existing summary sidecar build, a separate release CLI build, and Tauri desktop packaging. CI is configured to upload DMG or MSI/NSIS installers with `meetilyctl` and build metadata. It does not create releases, request write permissions, or use signing credentials.

The macOS artifact has an ad-hoc signature needed by Apple Silicon; it has no Developer ID signature and is not notarized. Windows artifacts are unsigned. Updater artifact signing is disabled. Preview identity is `com.youngteurus.meetily.calls`, separate from Community's `com.meetily.ai`; the build script refuses to package under the Community identifier. Never point the preview integration/data path at the Community database. Data migration must be an explicit import with backup, rather than shared access to the same SQLite file.

## Reproduce on macOS ARM64 or Windows x64

Install Rust stable (the current verification toolchain is 1.98.1; standalone crates require at least 1.88), Node 20+, pnpm 9.15.9, and the native compiler prerequisites described in `docs/BUILDING.md`. macOS needs Xcode command line tools; Windows needs Visual Studio 2022 C++ tools, Windows SDK, CMake and LLVM/libclang for the existing audio dependencies. No transcription or summary model download is needed for compilation.

From the repository root:

```sh
pnpm --dir frontend install --frozen-lockfile
node scripts/build-calls-preview.cjs
```

The script runs only on a native ARM64 macOS or x64 Windows host. Standalone crates use their own lockfiles; their artifacts share the root `target` directory during this script. Windows Whisper retains the existing CPU portability checks (`GGML_NATIVE=OFF`, no AVX-512; Rust `x86-64-v2`). This CPU build still requires the native Whisper AVX2 instruction set. It does not require Vulkan or CUDA. macOS uses the existing Metal/CoreML acceleration.

Desktop installers are in `target/<target>/release/bundle`; CLI/build metadata are in `artifacts/calls-preview-<target>`. The optional bundled summary helper is compiled because Tauri's existing `externalBin` references it. Its presence does not make summary models necessary for transcription or MCP.

## Linux cloud prerequisites and actual results (2026-09-30)

Host: Debian 13 (trixie), x86_64. Tools: `rustc 1.98.1 (48a229cea 2026-09-01)`, `cargo 1.98.1 (797e8a9bc 2026-08-05)`, Node `v24.19.0`. Rust is installed in `/workspace/tooling`; it was absent from the default PATH.

The earlier baseline `cargo test -p meetily --lib` stopped in the GLib dependency build because GLib pkg-config/development packages were absent. An initial `apt-get download` attempt also failed because the host has no usable local package index:

```text
E: Can't select candidate version from package libglib2.0-dev as it has no candidate
E: Unable to locate package libwebkit2gtk-4.1-dev
E: Unable to locate package libasound2-dev
E: Unable to locate package libayatana-appindicator3-dev
```

We downloaded a signed Debian package index from the network-policy-allowed `deb.debian.org`, resolved 145 prerequisites plus the libclang runtime, and extracted their .deb files into `/workspace/linux-deps/sysroot`. No sudo, package installation, or system modification was used. APT configuration, lists, archives, and extracted files are all user-local. Packages downloaded successfully (110 MB). Prerequisite verification returned:

```text
pkg-config --modversion glib-2.0 gtk+-3.0 webkit2gtk-4.1 alsa ayatana-appindicator3-0.1
2.84.4
3.24.49
2.52.6
1.2.14
0.5.94
cmake version 3.31.6
```

A reproducible setup on this Debian cloud host:

```sh
mkdir -p /workspace/linux-deps/apt/empty /workspace/linux-deps/apt/lists/partial /workspace/linux-deps/apt/cache/archives/partial /workspace/linux-deps/debs /workspace/linux-deps/sysroot
printf '%s\n' 'deb https://deb.debian.org/debian trixie main' > /workspace/linux-deps/apt/sources.list
cat > /workspace/linux-deps/apt/config <<'CONFIG'
Dir::Etc::parts "/workspace/linux-deps/apt/empty";
Dir::Etc::main "/workspace/linux-deps/apt/empty/apt.conf";
Dir::Etc::sourcelist "/workspace/linux-deps/apt/sources.list";
Dir::Etc::sourceparts "/workspace/linux-deps/apt/empty";
Dir::State::lists "/workspace/linux-deps/apt/lists";
Dir::Cache "/workspace/linux-deps/apt/cache";
Acquire::https::Proxy "http://proxy:8080";
CONFIG
APT_CONFIG=/workspace/linux-deps/apt/config apt-get update
APT_CONFIG=/workspace/linux-deps/apt/config apt-get -o Debug::NoLocking=true -o Dir::Cache::archives=/workspace/linux-deps/debs --download-only --assume-yes --no-install-recommends install libglib2.0-dev libgtk-3-dev libwebkit2gtk-4.1-dev libasound2-dev libayatana-appindicator3-dev libclang-dev cmake
(cd /workspace/linux-deps/debs && APT_CONFIG=/workspace/linux-deps/apt/config apt-get download libclang1-19)
for meetily_deb in /workspace/linux-deps/debs/*.deb; do
  dpkg-deb --extract "$meetily_deb" /workspace/linux-deps/sysroot
done
curl -fL https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-x64-1.22.0.tgz -o /workspace/linux-deps/onnxruntime-linux-x64-1.22.0.tgz
printf '%s\n' '8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3  /workspace/linux-deps/onnxruntime-linux-x64-1.22.0.tgz' | sha256sum --check
mkdir -p /workspace/linux-deps/onnxruntime
tar -xzf /workspace/linux-deps/onnxruntime-linux-x64-1.22.0.tgz -C /workspace/linux-deps/onnxruntime
# Development symlinks may target runtime libraries APT skipped as already installed.
# Link only to real installed libraries; all new links remain inside the sysroot.
python3 - <<'PYTHON'
from pathlib import Path
sysroot = Path('/workspace/linux-deps/sysroot')
for p in (sysroot / 'usr/lib/x86_64-linux-gnu').glob('*.so'):
    if p.is_symlink() and not p.exists():
        target = (p.parent / p.readlink()).absolute()
        host = Path('/') / target.relative_to(sysroot)
        if host.is_file() and not target.exists():
            target.symlink_to(host)
PYTHON
export ORT_LIB_LOCATION=/workspace/linux-deps/onnxruntime/onnxruntime-linux-x64-1.22.0/lib
export ORT_PREFER_DYNAMIC_LINK=1
export CARGO_HOME=/workspace/tooling/cargo RUSTUP_HOME=/workspace/tooling/rustup
export PATH=/workspace/tooling/cargo/bin:/workspace/tooling/node_modules/.bin:$PATH
./scripts/check-linux-native.sh
```

`check-linux-native.sh` sets the sysroot's pkg-config paths, library search path, libclang path, Clang builtin header resource directory, and CMake resource directory. It first builds/stages the real existing `llama-helper` sidecar required by Tauri. It performs `cargo check --locked -p meetily --lib`, without replacing dependencies or disabling desktop modules. `MEETILY_LINUX_SYSROOT` overrides the default sysroot location.

The first attempt after resolving GUI prerequisites stopped before Rust compilation with:

```text
error: multiple workspace roots found in the same workspace:
  /workspace/meetily/call-detection
  /workspace/meetily/local-control
  /workspace/meetily
```

This exposed the need for the root workspace to exclude the standalone path-dependent crates. The root workspace now excludes the standalone crates. The resumed check compiled the GTK dependencies, then stopped at ONNX Runtime's default binary download:

```text
Failed to GET `https://cdn.pyke.io/0/pyke:ort-rs/ms@1.22.0/x86_64-unknown-linux-gnu.tgz`:
CONNECT proxy failed: proxy server responded 403/403
```

The cloud network policy does not allow `cdn.pyke.io`. The supported `ORT_LIB_LOCATION`/`ORT_PREFER_DYNAMIC_LINK=1` settings instead use the official Microsoft ONNX Runtime 1.22.0 GitHub release from an allowed domain; no dependencies were stubbed or features disabled. Its downloaded archive SHA256 is `8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3`.

Bindgen next reported `Unable to find libclang ... /workspace/linux-deps/sysroot/usr/lib/llvm-19/lib/libclang.so: No such file or directory`. APT omitted the runtime package because it already exists on the host, leaving the extracted development symlink unresolved. Explicitly downloading and extracting `libclang1-19` resolved that sysroot issue. Tauri then reported the missing existing sidecar `resource path binaries/llama-helper-x86_64-unknown-linux-gnu doesn't exist`. Building the real `llama-helper` exposed Clang's missing builtin `stdbool.h`, fixed by `BINDGEN_EXTRA_CLANG_ARGS=-resource-dir=/workspace/linux-deps/sysroot/usr/lib/llvm-19/lib/clang/19`. The check script includes this setting and builds/stages the actual helper. After the main task corrected generated Tauri command registration paths to their original `control::gateway` module, the full desktop library check **passed**:

```text
warning: `meetily` (lib) generated 9 warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.42s
```

This compiles the complete Linux desktop library against real GTK/audio/Whisper/ONNX dependencies and its real bundled helper/FFmpeg resources. It does not prove native macOS/Windows compilation or real-call behavior. Log: `/workspace/linux-deps/cargo-check-native.log`. The complete desktop library test binary compiled and linked successfully in 3m 27s. Its initial launch reported `libonnxruntime.so.1: cannot open shared object file`; the script now adds `ORT_LIB_LOCATION` to `LD_LIBRARY_PATH` and the cached rerun **passed**:

```text
test result: ok. 251 passed; 0 failed; 3 ignored; 0 measured;
0 filtered out; finished in 7.21s
```

Command: `./scripts/check-linux-native.sh --test` with the documented ORT environment. Final log including the headless recorder orchestration test and explicitly manual physical-device test: `/tmp/meetily-final-native-tests.log` (earlier run before the hardware-test annotation: `/tmp/meetily-headless-full-native.log`). APT skips already-installed runtime packages, so 78 user-local sysroot links point to genuine host runtime libraries; the preparation command above reproduces those links without changing host files.

Build script JavaScript syntax (`node --check`) and workflow YAML parsing passed. These checks prove parsing only. Native CI was started by [draft PR #1](https://github.com/YoungTeurus/meetily/pull/1): [run 36723904371](https://github.com/YoungTeurus/meetily/actions/runs/36723904371), implementation commit `d9a43d85c06574e9a797a6f99422759ad7962437`. Installer outcomes are recorded in [verification.md](verification.md). No real-call validation is claimed. Native installers, audio permissions, notification activation, actual Zoom/Discord detection, and older OS compatibility remain pending their documented native/manual checks.

## OS compatibility evidence and remaining floor qualification

macOS preview packaging sets `minimumSystemVersion=14.2` and `MACOSX_DEPLOYMENT_TARGET=14.2`. Existing system audio capture uses Core Audio process taps, introduced in macOS 14.2; the detector uses audio-process properties and Accessibility. macOS 14.8.5 is newer than these API requirements. A successful compile/installer does not verify audio capture or permissions on the user's 14.8.5 machine; that manual check remains required.

The new Windows detector uses WASAPI audio sessions/`IAudioSessionControl2::GetProcessId` and UI Automation; WASAPI session APIs predate Windows 10; the selected `CUIAutomation8`/`IUIAutomation2` interfaces require Windows 8 or newer. The running-app Toast Activated callback uses Windows Runtime notification APIs. Windows 10/11 x64 are the intended targets. The current locked GUI stack is Tao 0.37.1, Wry 0.57.0, Tauri runtime Wry 2.12.0; `GetDpiForWindow` and `AdjustWindowRectExForDpi` are dynamically loaded with older API fallbacks, so their Windows 10 1607/build 14393 availability does **not** prove a mandatory 14393 floor.

The minimum supported stable Windows release is **Windows 10 version 2004, x64, build 19041**; Windows 11 x64 is supported by the same API floor. This is derived from the exact ONNX Runtime DLL bundled for Parakeet, rather than a guessed detector-only requirement. We downloaded the pinned official `onnxruntime-win-x64-1.22.0.zip` and verified SHA256 `174c616efc0271194488642a72f1a514e01487da4dfe84c49296d66e40ebe0da`, matching `frontend/src-tauri/build/onnxruntime.rs`. `objdump -p onnxruntime.dll` returned:

```text
Entry 1 ... Import Directory [parts of .idata]
Entry d 0000000000000000 00000000 Delay Import Directory
DLL Name: dxcore.dll
DXCoreCreateAdapterFactory
```

This is a mandatory normal import, with no delay-import directory. Microsoft documents the API's initial availability at Windows Insider build 18936; its first generally released Windows version is Windows 10 2004/build 19041. Shipping builds such as 1809/17763 cannot load the bundled DLL. The floor applies to the preview application as a whole, including users who select Whisper, because both transcription engines are included.

We also inspected the exact Windows FFmpeg 8.0.1 essentials archive used by the build helper (SHA256 `e2aaeaa0fdbc397d4794828086424d4aaa2102cef1fb6874f6ffd29c0b88b673`); its normal imported APIs do not establish a higher floor. Its archive and PE import inspection logs are under `/workspace/linux-deps`; ONNX PE imports are in `/workspace/linux-deps/onnxruntime-win-imports.txt`. Runtime dependencies also include Microsoft's VC++ 2015–2022 x64 runtime and WebView2; packaging handles WebView2 through the existing Tauri installer mechanism.

`installer/windows-build-floor.nsh` reads the OS registry `CurrentBuildNumber` and aborts the NSIS preinstall hook below 19041. `installer/windows-build-floor.wxs` supplies the corresponding MSI registry search and launch condition; a referenced hidden FeatureGroup links the entire condition fragment. An installed MSI may still be repaired/uninstalled on an older OS. Tauri config points to both gates. The full config validates against the installed Tauri 2.11.1 schema; the WiX fragment validates against the official WiX3 XSD; the actual hook compiled successfully with user-local NSIS 3.11 into a minimal verification installer.

**Pending native qualification:** these API/import and installer syntax checks establish and enforce the intended floor; they do not prove a successful installed application on build 19041. Install/run both transcription engines, notifications, CLI and detection on a Windows 10 2004 VM before declaring runtime qualification complete. Run both NSIS and MSI on 1809/17763 and confirm rejection, on 2004/19041 and Windows 11 and confirm acceptance, including silent installer modes. Native run 36745310096 compiled/linked both full generated installers successfully; actual calls and installation on the minimum OS must still be tested.

References: [Core Audio process taps](https://developer.apple.com/documentation/coreaudio/audiohardwarecreateprocesstap(_:_:)), [audio-session process ID](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid), [Tao source at the locked version](https://github.com/tauri-apps/tao/blob/tao-v0.37.1/src/platform_impl/windows/util.rs), [DXCoreCreateAdapterFactory API minimum](https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/dxcore/nf-dxcore-dxcorecreateadapterfactory.md), [ONNX Runtime 1.22.0 source](https://github.com/microsoft/onnxruntime/tree/v1.22.0), [current runner labels](https://github.com/actions/runner-images).

## Native CI test environment

The first Windows job compiled and linked the desktop library and passed gateway (13), detector (17), and CLI/MCP (17) tests, then failed the pre-existing `audio::playback_monitor::tests::test_get_output_device` assertion because the runner has no default audio output. That physical-device test is explicitly ignored in the default suite; run it on an interactive target desktop with:

```sh
cargo test --locked -p meetily --lib audio::playback_monitor::tests::test_get_output_device -- --ignored --nocapture
```

The Windows test process also terminated with `STATUS_ACCESS_VIOLATION` (`0xc0000005`), independently unresolved by the missing-device assertion. Unlike application startup, the test binary never runs Tauri setup's `ort::init_from`. The native build script now explicitly selects the SHA-verified bundled ONNX Runtime via `ORT_DYLIB_PATH`, and runs Windows desktop tests serially with visible test output to identify any remaining native crash. No VAD or ONNX tests are skipped. Runtime selection was a discrepancy; it is not yet proven to be the access-violation cause.

The subsequent manifest fix allows the desktop test executable to launch: Tauri's existing Common Controls v6 resource is now linked into MSVC library tests as well as application binaries. Run 36730915943 identified the access violation specifically in system-audio device enumeration, before VAD tests. CPAL 0.15.3 cached an STA `IMMDeviceEnumerator` process-wide while its COM guard was thread-local; a later caller reused the interface after the first caller's apartment was torn down. The exact published crate is now vendored with a single Windows source patch that keeps an enumerator per calling thread. The root workspace patch fixes all CPAL enumeration consumers; it does not upgrade the dependency graph. See [origin, hash, change and regression details](../../vendor/cpal/MEETILY-PATCH.md). Its Apache-2.0 license and patch notice accompany preview artifacts. Linux CPAL compilation and Windows MSVC source checks passed; native execution of the fix is tracked in verification.md. The original device-list test remains enabled, alongside a new regression exercising sequential and concurrent caller threads on a runner with no audio endpoints.

Run 36735969234 passed the entire Windows desktop suite (264 passed, 3 ignored), including the CPAL regression. Release linking then exposed duplicate VERSION resources: a generic link argument also reached the app binary, where Tauri had already supplied its resource. The fix uses a Windows/MSVC/test-only native link declaration in `src/windows_test_resource.rs`; `build.rs` supplies the resource search directory. The production library adds no resource. Cross-target linker fixtures verify one resource argument for each intended executable. Native run [36745310096](https://github.com/YoungTeurus/meetily/actions/runs/36745310096), implementation `0f28ea01bf1d769208285e82b4ba37a85c1f29de`, confirmed both: Windows desktop **269 passed, 0 failed, 3 ignored**, including the CPAL regression and Codex descendant-timeout test. Production release linking and portable GGML validation passed; NSIS, MSI and standalone CLI artifacts uploaded successfully. macOS also passed (275 desktop tests, 4 ignored) and produced its DMG/CLI. Current downloads and archive hashes are in [verification.md](verification.md#current-native-preview).
