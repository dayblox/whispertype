# Native, repeatable Windows build. Does not install the app or change startup.
param(
    [switch]$Benchmark,
    [switch]$DesktopTests,
    [switch]$MicrophoneTest
)
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    function Invoke-Cargo {
        param([string[]]$Arguments)
        & cargo @Arguments
        if ($LASTEXITCODE -ne 0) { throw "cargo $Arguments failed ($LASTEXITCODE)" }
    }
    Invoke-Cargo -Arguments @('fmt', '--check')
    Invoke-Cargo -Arguments @('test', '--locked')
    Invoke-Cargo -Arguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    Invoke-Cargo -Arguments @('build', '--release', '--locked')
    & python scripts/verify-windows.py target/release/whispertype.exe
    if ($LASTEXITCODE -ne 0) { throw 'Windows executable validation failed' }
    if ($Benchmark) {
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'optimization_benchmark', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    if ($DesktopTests) {
        Invoke-Cargo -Arguments @('test', '--locked', 'windows_desktop_smoke', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    if ($MicrophoneTest) {
        Invoke-Cargo -Arguments @('test', '--locked', 'microphone_smoke', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    & python scripts/test_package.py
    if ($LASTEXITCODE -ne 0) { throw 'Packaging tests failed' }
    & python scripts/package.py --platform windows --windows-binary target/release/whispertype.exe
    if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }
    Write-Host 'Ready: target\release\whispertype.exe and dist\whispertype-windows-x64.zip'
} finally {
    Pop-Location
}
