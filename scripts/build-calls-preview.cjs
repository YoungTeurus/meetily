#!/usr/bin/env node
// Run on the target OS. Installs no toolchains and publishes no releases.
const { spawnSync } = require('node:child_process');
const { readFileSync, writeFileSync, mkdirSync, copyFileSync } = require('node:fs');
const { resolve, join } = require('node:path');
const root = resolve(__dirname, '..');
const platform = process.platform;
const target = platform === 'darwin' ? 'aarch64-apple-darwin' : platform === 'win32' ? 'x86_64-pc-windows-msvc' : null;
if (!target || (platform === 'darwin' && process.arch !== 'arm64') || (platform === 'win32' && process.arch !== 'x64')) {
  throw new Error('Run this preview installer build on macOS ARM64 or Windows x64.');
}
const config = JSON.parse(readFileSync(join(root, 'frontend/src-tauri/tauri.conf.json'), 'utf8'));
if (config.identifier !== 'com.youngteurus.meetily.calls') {
  throw new Error('Refusing to package without the isolated Meetily Calls Preview application identifier.');
}
const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target'), RUST_BACKTRACE: '1' };
if (platform === 'darwin') env.MACOSX_DEPLOYMENT_TARGET = '14.2';
if (platform === 'win32') {
  env.CMAKE_PROJECT_INCLUDE = join(root, '.github/force-portable-ggml.cmake').replaceAll('\\', '/');
  env.RUSTFLAGS = '-C target-cpu=x86-64-v2';
  // cargo test does not run Tauri's setup, which selects the bundled runtime.
  // build.rs verifies/stages this DLL before launching the desktop test binary.
  env.ORT_DYLIB_PATH = join(root, 'frontend/src-tauri/binaries/onnxruntime/onnxruntime.dll');
}
function version(command, args) {
  const result = spawnSync(command, args, { cwd: root, env, encoding: 'utf8' });
  return result.status === 0 ? result.stdout.trim() : 'unavailable';
}
function run(command, args, cwd = root) {
  console.log(`> ${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
// Build the existing optional summary sidecar because Tauri bundles it. A model
// download or use of that sidecar is not required for transcription/CLI/MCP.
const helperArgs = ['build', '--locked', '--release', '-p', 'llama-helper', '--target', target];
if (platform === 'darwin') helperArgs.push('--features', 'metal');
run('cargo', helperArgs);
const extension = platform === 'win32' ? '.exe' : '';
const binaries = join(root, 'frontend/src-tauri/binaries');
mkdirSync(binaries, { recursive: true });
copyFileSync(join(root, `target/${target}/release/llama-helper${extension}`), join(binaries, `llama-helper-${target}${extension}`));
for (const manifest of ['local-control/Cargo.toml', 'call-detection/Cargo.toml', 'meetilyctl/Cargo.toml']) {
  run('cargo', ['test', '--locked', '--manifest-path', manifest, '--target', target]);
}
const desktopTests = ['test', '--locked', '-p', 'meetily', '--lib', '--target', target];
if (platform === 'win32') desktopTests.push('--', '--test-threads=1', '--nocapture');
run('cargo', desktopTests);
run('cargo', ['build', '--locked', '--release', '--manifest-path', 'meetilyctl/Cargo.toml', '--target', target]);
const overlay = join(root, '.calls-preview.config.json');
const previewConfig = { bundle: { createUpdaterArtifacts: false } };
if (platform === 'darwin') {
  previewConfig.bundle.macOS = { minimumSystemVersion: '14.2', signingIdentity: '-', hardenedRuntime: false };
} else {
  previewConfig.bundle.windows = { signCommand: null };
  previewConfig.build = {
    beforeBundleCommand: { script: `node .github/verify-portable-ggml.cjs target/${target}/release`, cwd: root }
  };
}
writeFileSync(overlay, JSON.stringify(previewConfig, null, 2) + '\n');
run(process.execPath, [join(root, 'frontend/node_modules/@tauri-apps/cli/tauri.js'), 'build', '--target', target, '--bundles', platform === 'darwin' ? 'app,dmg' : 'nsis,msi', '--config', overlay], join(root, 'frontend'));
const artifacts = join(root, `artifacts/calls-preview-${target}`);
mkdirSync(artifacts, { recursive: true });
copyFileSync(join(root, 'LICENSE.md'), join(artifacts, 'LICENSE.md'));
copyFileSync(join(root, 'docs/implementation/native-build.md'), join(artifacts, 'native-build.md'));
copyFileSync(join(root, `target/${target}/release/meetilyctl${extension}`), join(artifacts, `meetilyctl${extension}`));
writeFileSync(join(artifacts, 'BUILD-INFO.txt'), [
  'Meetily Calls Preview', `Target: ${target}`, `Version: ${config.version}`,
  `Source commit: ${version('git', ['rev-parse', 'HEAD'])}`,
  version('rustc', ['--version']), version('cargo', ['--version']), `Node: ${process.version}`,
  'No release has been published.',
  platform === 'darwin' ? 'Ad-hoc signature only; no Developer ID signature or notarization.' : 'Unsigned Windows installers and CLI.',
  'Real audio/call/notification behavior requires the documented manual verification.', ''
].join('\n'));
console.log(`CLI and build metadata: ${artifacts}`);
console.log(`Desktop installers: ${join(root, `target/${target}/release/bundle`)}`);
