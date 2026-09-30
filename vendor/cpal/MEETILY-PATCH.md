# CPAL 0.15.3 Windows COM apartment fix

This directory preserves the published CPAL 0.15.3 crate, its package metadata,
source files, and Apache-2.0 `LICENSE`. Source: https://crates.io/crates/cpal/0.15.3
(upstream https://github.com/RustAudio/cpal, revision
`ac6cbb2ba55e61665a35ab88ae136a83380d1354` from `.cargo_vcs_info.json`).
The published crate SHA256 is
`873dab07c8f743075e57f524c583985fbaf745602acbe916a01539364369a779`.

The only upstream source change is in `src/host/wasapi/device.rs`: the cached
`IMMDeviceEnumerator` belongs to the calling thread rather than to a process-wide
`OnceLock`. Each thread initializes COM before creating its enumerator, and drops
the enumerator before its COM guard uninitializes the apartment. Calls receive
a cloned interface on the same thread. The CPAL public API and other platforms
are unchanged.

In the original code, a short-lived caller initialized an STA apartment, cached
its interface globally, then uninitialized COM when the caller exited. A later
caller reused that interface from another thread without initializing COM.
Windows native CI crashed with `STATUS_ACCESS_VIOLATION` when the system-audio
permission test was followed by the device enumeration test. The patch fixes
the dependency used by all Meetily device enumeration/default-device paths.

The desktop Windows regression
`windows_device_enumeration_survives_caller_thread_teardown` exercises the real
CPAL API from sequential and concurrent threads, including default devices.
An empty device list is valid on a headless runner; no physical device is
required. The existing native device enumeration test stays enabled.
