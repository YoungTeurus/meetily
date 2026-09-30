# Read-only diagnostics for a failed native test launch; the original CI step
# remains failed. No test is skipped, and no machine-wide loader setting changes.
$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$report = Join-Path $repo 'artifacts/windows-native-diagnostics'
$deps = Join-Path $repo 'target/x86_64-pc-windows-msvc/debug/deps'
New-Item -ItemType Directory -Force $report | Out-Null
$testExe = Get-ChildItem $deps -Filter 'app_lib-*.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $testExe) {
    'No compiled desktop test executable was found.' | Tee-Object (Join-Path $report 'summary.txt')
    exit 0
}
Copy-Item $testExe.FullName $report
Get-FileHash $testExe.FullName | Format-List | Out-File (Join-Path $report 'test-executable.txt')

# Keep Cargo's own runtime DLL search path for the --list reproduction.
$env:CARGO_TARGET_DIR = Join-Path $repo 'target'
$env:RUSTFLAGS = '-C target-cpu=x86-64-v2'
$env:CMAKE_PROJECT_INCLUDE = (Join-Path $repo '.github/force-portable-ggml.cmake').Replace('\', '/')
$env:ORT_DYLIB_PATH = Join-Path $repo 'frontend/src-tauri/binaries/onnxruntime/onnxruntime.dll'
Push-Location $repo
try {
    & cargo test --locked -p meetily --lib --target x86_64-pc-windows-msvc -- --list 2>&1 |
        Tee-Object (Join-Path $report 'test-list.txt')
    "cargo --list exit: $LASTEXITCODE" | Tee-Object (Join-Path $report 'summary.txt')
} finally {
    Pop-Location
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -property installationPath
$dumpbin = Get-ChildItem (Join-Path $vs 'VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe') |
    Sort-Object FullName -Descending | Select-Object -First 1
if ($dumpbin) {
    & $dumpbin.FullName /DEPENDENTS $testExe.FullName 2>&1 |
        Tee-Object (Join-Path $report 'dependents.txt')
    & $dumpbin.FullName /IMPORTS $testExe.FullName 2>&1 |
        Out-File (Join-Path $report 'imports.txt')
    Select-String -Path (Join-Path $report 'imports.txt') -Pattern 'comctl32|TaskDialog|onnxruntime|WebView2' |
        Tee-Object -Append (Join-Path $report 'summary.txt')
    & $dumpbin.FullName /EXPORTS (Join-Path $env:SystemRoot 'System32/comctl32.dll') 2>&1 |
        Out-File (Join-Path $report 'system-comctl32-exports.txt')
    $hasTaskDialog = [bool](Select-String -Path (Join-Path $report 'system-comctl32-exports.txt') -Pattern '\bTaskDialogIndirect\b')
    "System32 comctl32.dll exports TaskDialogIndirect: $hasTaskDialog" |
        Tee-Object -Append (Join-Path $report 'summary.txt')
}
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
$mt = Get-ChildItem (Join-Path $kits 'bin/*/x64/mt.exe') | Sort-Object FullName -Descending | Select-Object -First 1
if ($mt) {
    & $mt.FullName "-inputresource:$($testExe.FullName);#1" "-out:$report/test-executable.manifest" 2>&1 |
        Tee-Object (Join-Path $report 'manifest-extraction.txt')
    "mt manifest extraction exit: $LASTEXITCODE" | Tee-Object -Append (Join-Path $report 'summary.txt')
}

$dllPaths = @(
    Get-ChildItem $deps -Filter '*.dll'
    Get-ChildItem (Split-Path $deps) -Filter '*.dll'
    Get-ChildItem (Join-Path $repo 'frontend/src-tauri/binaries/onnxruntime') -Filter '*.dll'
    Get-Item (Join-Path $env:SystemRoot 'System32/comctl32.dll'), (Join-Path $env:SystemRoot 'System32/vcruntime140*.dll'), (Join-Path $env:SystemRoot 'System32/msvcp140*.dll')
    Get-Item (Join-Path $env:SystemRoot 'WinSxS/amd64_microsoft.windows.common-controls_*/comctl32.dll')
)
$dllPaths | Select-Object FullName, Length, @{Name='FileVersion';Expression={$_.VersionInfo.FileVersion}} |
    Format-Table -AutoSize | Out-String -Width 250 | Out-File (Join-Path $report 'dll-versions.txt')
foreach ($log in @('Application', 'System')) {
    Get-WinEvent -FilterHashtable @{LogName=$log; StartTime=(Get-Date).AddMinutes(-20)} -ErrorAction SilentlyContinue |
        Where-Object { $_.Level -le 3 -and $_.Message -match 'app_lib|entry point|entrypoint|comctl32|Application Popup' } |
        Select-Object TimeCreated, Id, ProviderName, Message | Format-List |
        Out-File (Join-Path $report "$log-events.txt")
}
$cdb = Join-Path $kits 'Debuggers/x64/cdb.exe'
if (Test-Path $cdb) {
    $debug = Start-Process $cdb -ArgumentList @('-G', '-c', '".lastevent;lm;kb;q"', "`"$($testExe.FullName)`"", '--list') -PassThru -NoNewWindow `
        -RedirectStandardOutput (Join-Path $report 'debugger.txt') -RedirectStandardError (Join-Path $report 'debugger-errors.txt')
    if (-not $debug.WaitForExit(45000)) { $debug.Kill($true) }
} else {
    'Windows SDK cdb is not installed.' | Tee-Object -Append (Join-Path $report 'summary.txt')
}
'Diagnostics completed; the failed build result is preserved.'
exit 0
