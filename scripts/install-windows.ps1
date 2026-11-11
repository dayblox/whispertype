# Run from the extracted Windows release. No administrator rights required.
$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:LOCALAPPDATA 'Whispertype'
$config = Join-Path $env:APPDATA 'whispertype'
$source = Join-Path $PSScriptRoot 'whispertype.exe'
if (!(Test-Path $source)) { $source = Join-Path $PSScriptRoot '..\target\release\whispertype.exe' }
if (!(Test-Path $source)) { throw 'Build with cargo build --release, or place this script beside whispertype.exe.' }
Get-Process whispertype -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $destination 'whispertype.exe') } | Stop-Process
New-Item -ItemType Directory -Force $destination, $config | Out-Null
$target = Join-Path $destination 'whispertype.exe'
if ((Resolve-Path $source).Path -ne $target) { Copy-Item $source $target -Force }
if (!(Test-Path (Join-Path $config 'groq.key'))) {
    $secret = Read-Host 'Groq API key (https://console.groq.com/keys)' -AsSecureString
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
    try {
        $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer)
        if ($key -notmatch '^gsk_[a-zA-Z0-9_]{16,}$') { throw 'Expected a Groq API key beginning with gsk_.' }
        [IO.File]::WriteAllText((Join-Path $config 'groq.key'), $key, [Text.UTF8Encoding]::new($false))
    } finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
        $key = $null
        $secret.Dispose()
    }
}
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
New-Item $run -Force | Out-Null
New-ItemProperty -Path $run -Name Whispertype -Value ('"' + $target + '" run') -PropertyType String -Force | Out-Null
$shortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Whispertype.lnk'
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
$shortcut.TargetPath = $target
$shortcut.Arguments = 'run'
$shortcut.IconLocation = "$target,0"
$shortcut.Save()
Start-Process $target -ArgumentList 'run'
Write-Host 'Whispertype is running. Ctrl+Space: tap to toggle, hold to talk. Right-click its tray icon for settings.'
