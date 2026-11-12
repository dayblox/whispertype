# Native, repeatable Windows build. Does not install the app or change startup.
param(
    [switch]$Installer,
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
    Invoke-Cargo -Arguments @('fmt', '--all', '--check')
    Invoke-Cargo -Arguments @('test', '--locked')
    Invoke-Cargo -Arguments @('test', '--locked', '-p', 'rustpotter', '--lib', 'mfcc::')
    Invoke-Cargo -Arguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    Invoke-Cargo -Arguments @('build', '--release', '--locked')
    & python scripts/verify-windows.py target/release/whispertype.exe
    if ($LASTEXITCODE -ne 0) { throw 'Windows executable validation failed' }
    & python scripts/test_windows_update.py target/release/whispertype.exe
    if ($LASTEXITCODE -ne 0) { throw 'Windows update helper validation failed' }
    if ($Benchmark) {
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'microphone_probe_allocates', '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'winhttp_upload_allocations', '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'optimization_benchmark', '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'unicode_insertion_benchmark', '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'update_download_allocations', '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-Cargo -Arguments @('test', '--release', '--locked', 'inactive_keyboard_hook_allocates_nothing', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    if ($DesktopTests) {
        Invoke-Cargo -Arguments @('test', '--locked', 'windows_desktop_smoke', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    if ($MicrophoneTest) {
        Invoke-Cargo -Arguments @('test', '--locked', 'microphone_smoke', '--', '--ignored', '--nocapture', '--test-threads=1')
    }
    & python scripts/test_package.py
    if ($LASTEXITCODE -ne 0) { throw 'Packaging tests failed' }
    & python scripts/test_release.py
    if ($LASTEXITCODE -ne 0) { throw 'Release tests failed' }
    $packageArguments = @('scripts/package.py', '--platform', 'windows', '--windows-binary', 'target/release/whispertype.exe')
    if ($Installer) {
        $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        $iscc = if ($compiler) { $compiler.Source } else {
            @(
                (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
                (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
                (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
            ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        }
        if (!$iscc) { throw 'Install Inno Setup 6 to build the Windows installer.' }
        $version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
        & $iscc "/DAppVersion=$version" scripts/windows-installer.iss
        if ($LASTEXITCODE -ne 0) { throw 'Windows installer compilation failed' }
        $packageArguments += @('--windows-installer', 'dist/whispertype-windows-x64-setup.exe')
    }
    & python @packageArguments
    if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }
    Write-Host 'Ready: target\release\whispertype.exe and dist\whispertype-windows-x64.zip'
} finally {
    Pop-Location
}
