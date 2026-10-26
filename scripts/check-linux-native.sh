#!/usr/bin/env bash
set -euo pipefail
# Optional cloud check using a user-local Debian sysroot. No system installation.
# Prepare packages as documented in docs/implementation/native-build.md.
meetily_sysroot="${MEETILY_LINUX_SYSROOT:-/workspace/linux-deps/sysroot}"
export PKG_CONFIG_PATH="$meetily_sysroot/usr/lib/x86_64-linux-gnu/pkgconfig:$meetily_sysroot/usr/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export PKG_CONFIG_SYSROOT_DIR="$meetily_sysroot"
export BINDGEN_EXTRA_CLANG_ARGS="-resource-dir=$meetily_sysroot/usr/lib/llvm-19/lib/clang/19${BINDGEN_EXTRA_CLANG_ARGS:+ $BINDGEN_EXTRA_CLANG_ARGS}"
export LIBCLANG_PATH="$meetily_sysroot/usr/lib/llvm-19/lib"
export LD_LIBRARY_PATH="$meetily_sysroot/usr/lib/x86_64-linux-gnu${ORT_LIB_LOCATION:+:$ORT_LIB_LOCATION}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$meetily_sysroot/usr/bin:$PATH"
export CMAKE_ROOT="$meetily_sysroot/usr/share/cmake-3.31"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"
cd "$(dirname "$0")/.."
cargo build --locked -p llama-helper
meetily_target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p frontend/src-tauri/binaries
cp "$meetily_target_dir/debug/llama-helper" frontend/src-tauri/binaries/llama-helper-x86_64-unknown-linux-gnu
if [[ "${1:-}" == "--test" ]]; then
  shift
  exec cargo test --locked -p meetily --lib "$@"
fi
exec cargo check --locked -p meetily --lib "$@"
