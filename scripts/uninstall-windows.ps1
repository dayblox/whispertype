$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:LOCALAPPDATA 'Whispertype'
Get-Process whispertype -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $destination 'whispertype.exe') } | Stop-Process
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Whispertype -ErrorAction SilentlyContinue
if (Test-Path $destination) { Remove-Item $destination -Recurse -Force }
Write-Host 'Uninstalled. Your key, settings, wake model and last transcript remain in %APPDATA%\whispertype; delete that folder to remove them.'
