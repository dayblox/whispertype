$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:LOCALAPPDATA 'Whispertype'
$target = Join-Path $destination 'whispertype.exe'
if (Test-Path -LiteralPath $target) {
    $stop = Start-Process -FilePath $target -ArgumentList @('_stop-for-upgrade', ('"' + $target + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($stop.ExitCode -ne 0) { throw 'Could not stop Whispertype. Close it and retry uninstalling.' }
}
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Whispertype -ErrorAction SilentlyContinue
$shortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Whispertype.lnk'
if (Test-Path -LiteralPath $shortcutPath) { Remove-Item -LiteralPath $shortcutPath }
$resolved = [IO.Path]::GetFullPath($destination)
if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($env:LOCALAPPDATA) -or (Split-Path $resolved -Leaf) -ne 'Whispertype') {
    throw 'Unexpected uninstall destination'
}
if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
Write-Host 'Uninstalled. Your key, settings, wake model and last transcript remain in %APPDATA%\whispertype; delete that folder to remove them.'
