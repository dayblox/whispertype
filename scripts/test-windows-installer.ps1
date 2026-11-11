# Install into a temporary directory, with no startup entry, icons, key, or app launch.
$ErrorActionPreference = 'Stop'
$appId = '{B2615475-8302-4834-AE3E-092AA419C04C}_is1'
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId"
if (Test-Path -LiteralPath $uninstallKey) { throw 'An installed Whispertype already exists; refusing to alter it for a test.' }
$setup = (Resolve-Path (Join-Path $PSScriptRoot '..\dist\whispertype-windows-x64-setup.exe')).Path
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('whispertype-setup-test-' + [guid]::NewGuid())
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$before = (Get-ItemProperty -LiteralPath $runKey -Name Whispertype -ErrorAction SilentlyContinue).Whispertype
try {
    $process = Start-Process -FilePath $setup -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', '/TASKS=""', "/DIR=`"$testRoot`"") -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
    if (!(Test-Path -LiteralPath (Join-Path $testRoot 'whispertype.exe'))) { throw 'Installed EXE missing' }
    $sourceHash = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot '..\target\release\whispertype.exe')).Hash
    $installedHash = (Get-FileHash -LiteralPath (Join-Path $testRoot 'whispertype.exe')).Hash
    if ($sourceHash -ne $installedHash) { throw 'Installed EXE differs from build' }
    $after = (Get-ItemProperty -LiteralPath $runKey -Name Whispertype -ErrorAction SilentlyContinue).Whispertype
    if ($before -ne $after) { throw 'Installer unexpectedly changed sign-in startup' }
    if (!(Test-Path -LiteralPath $uninstallKey)) { throw 'Uninstall registration missing' }
    $process = Start-Process -FilePath (Join-Path $testRoot 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
    if ((Test-Path -LiteralPath (Join-Path $testRoot 'whispertype.exe')) -or (Test-Path -LiteralPath $uninstallKey)) {
        throw 'Uninstaller left the application or uninstall registration behind'
    }
    Write-Host 'Windows installer/uninstaller passed; installed EXE hash matches the build.'
} finally {
    if (Test-Path -LiteralPath (Join-Path $testRoot 'unins000.exe')) {
        Start-Process -FilePath (Join-Path $testRoot 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
    }
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedTest.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedTest -Leaf) -like 'whispertype-setup-test-*' -and (Test-Path -LiteralPath $resolvedTest)) {
        Remove-Item -LiteralPath $resolvedTest -Recurse -Force
    }
}
