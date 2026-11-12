param([string]$Destination = (Join-Path $env:LOCALAPPDATA 'Whispertype'))
$ErrorActionPreference = 'Stop'
$destination = [IO.Path]::GetFullPath($Destination)
$target = Join-Path $destination 'whispertype.exe'
if (Test-Path -LiteralPath $target) {
    $stop = Start-Process -FilePath $target -ArgumentList @('_stop-for-upgrade', ('"' + $target + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($stop.ExitCode -ne 0) { throw 'Could not stop Whispertype. Close it and retry uninstalling.' }
}
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Whispertype -ErrorAction SilentlyContinue
$shortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Whispertype.lnk'
if (Test-Path -LiteralPath $shortcutPath) { Remove-Item -LiteralPath $shortcutPath }
$resolved = [IO.Path]::GetFullPath($destination)
# A custom destination may contain unrelated files. Remove only our EXE and
# then the directory if empty; never recursively delete a user-chosen folder.
if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
if ((Test-Path -LiteralPath $resolved) -and !(Get-ChildItem -LiteralPath $resolved -Force | Select-Object -First 1)) {
    Remove-Item -LiteralPath $resolved -Force
}
Write-Host 'Uninstalled. Your key, settings, wake model and last transcript remain in %APPDATA%\whispertype; delete that folder to remove them.'
