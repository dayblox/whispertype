# Isolated install, upgrade with lingering workers, and uninstall regression.
$ErrorActionPreference = 'Stop'
$appId = '{B2615475-8302-4834-AE3E-092AA419C04C}_is1'
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId"
if (Test-Path -LiteralPath $uninstallKey) { throw 'An installed Whispertype already exists; refusing to alter it for a test.' }
$setup = (Resolve-Path (Join-Path $PSScriptRoot '..\dist\whispertype-windows-x64-setup.exe')).Path
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('whispertype-setup-test-' + [guid]::NewGuid())
$previousProfile = $env:WHISPERTYPE_HOME
$env:WHISPERTYPE_HOME = Join-Path $testRoot 'test-profile'
$children = [Collections.Generic.List[Diagnostics.Process]]::new()
function Start-HeldWorker([string]$executable) {
    $info = [Diagnostics.ProcessStartInfo]::new($executable, '_worker')
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $process = [Diagnostics.Process]::Start($info)
    $children.Add($process)
    return $process
}
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$before = (Get-ItemProperty -LiteralPath $runKey -Name Whispertype -ErrorAction SilentlyContinue).Whispertype
try {
    New-Item -ItemType Directory -Path $env:WHISPERTYPE_HOME -Force | Out-Null
    $savedKey = Join-Path $env:WHISPERTYPE_HOME 'groq.key'
    [IO.File]::WriteAllText($savedKey, ('gsk_' + ('a' * 48)), [Text.Encoding]::ASCII)
    $keyHash = (Get-FileHash -LiteralPath $savedKey).Hash
    $log = Join-Path $testRoot 'setup.log'
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', '/TASKS=""', "/DIR=`"$testRoot`"", "/LOG=`"$log`"")
    $process = Start-Process -FilePath $setup -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
    if (!(Test-Path -LiteralPath (Join-Path $testRoot 'whispertype.exe'))) { throw 'Installed EXE missing' }
    $sourceHash = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot '..\target\release\whispertype.exe')).Hash
    $installedHash = (Get-FileHash -LiteralPath (Join-Path $testRoot 'whispertype.exe')).Hash
    if ($sourceHash -ne $installedHash) { throw 'Installed EXE differs from build' }
    $after = (Get-ItemProperty -LiteralPath $runKey -Name Whispertype -ErrorAction SilentlyContinue).Whispertype
    if ($before -ne $after) { throw 'Installer unexpectedly changed sign-in startup' }
    if (!(Test-Path -LiteralPath $uninstallKey)) { throw 'Uninstall registration missing' }
    if ((Get-FileHash -LiteralPath $savedKey).Hash -ne $keyHash) { throw 'Setup changed the existing API key' }
    if (!(Select-String -LiteralPath $log -SimpleMatch 'Existing credential detected; replacement entry disabled.')) {
        throw 'Setup did not detect and disable replacement input for the saved key'
    }
    $installed = Join-Path $testRoot 'whispertype.exe'
    $first = Start-HeldWorker $installed
    $second = Start-HeldWorker $installed
    $other = Start-HeldWorker (Join-Path $PSScriptRoot '..\target\release\whispertype.exe')
    Start-Sleep -Milliseconds 300
    if ($first.HasExited -or $second.HasExited -or $other.HasExited) { throw 'Could not hold regression workers open' }
    $process = Start-Process -FilePath $setup -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Upgrade failed: $($process.ExitCode)" }
    if (!$first.WaitForExit(5000) -or !$second.WaitForExit(5000)) { throw 'Upgrade left an installed helper running' }
    if ($other.HasExited) { throw 'Upgrade stopped a different copy of Whispertype' }
    if ((Get-FileHash -LiteralPath $installed).Hash -ne $sourceHash) { throw 'Upgrade EXE differs from build' }
    if ((Get-FileHash -LiteralPath $savedKey).Hash -ne $keyHash) { throw 'Upgrade changed the saved key' }
    $uninstallWorker = Start-HeldWorker $installed
    $process = Start-Process -FilePath (Join-Path $testRoot 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
    if (!$uninstallWorker.WaitForExit(5000)) { throw 'Uninstall left a helper running' }
    if ($other.HasExited) { throw 'Uninstall stopped a different copy of Whispertype' }
    if ((Test-Path -LiteralPath (Join-Path $testRoot 'whispertype.exe')) -or (Test-Path -LiteralPath $uninstallKey)) {
        throw 'Uninstaller left the application or uninstall registration behind'
    }
    Write-Host 'Windows install/upgrade/uninstall passed; saved key preserved, lingering helpers stopped, unrelated copy left running, installed hash verified.'
} finally {
    foreach ($child in $children) { if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() }; $child.Dispose() }
    if (Test-Path -LiteralPath (Join-Path $testRoot 'unins000.exe')) {
        Start-Process -FilePath (Join-Path $testRoot 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
    }
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedTest.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedTest -Leaf) -like 'whispertype-setup-test-*' -and (Test-Path -LiteralPath $resolvedTest)) {
        Remove-Item -LiteralPath $resolvedTest -Recurse -Force
    }
    $env:WHISPERTYPE_HOME = $previousProfile
}
