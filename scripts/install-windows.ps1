# Run from the extracted Windows release. No administrator rights required.
$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:LOCALAPPDATA 'Whispertype'
$config = Join-Path $env:APPDATA 'whispertype'
$source = Join-Path $PSScriptRoot 'whispertype.exe'
if (!(Test-Path $source)) { $source = Join-Path $PSScriptRoot '..\target\release\whispertype.exe' }
if (!(Test-Path $source)) { throw 'Build with cargo build --release, or place this script beside whispertype.exe.' }
$target = Join-Path $destination 'whispertype.exe'
if (Test-Path -LiteralPath $target) {
    $stop = Start-Process -FilePath $source -ArgumentList @('_stop-for-upgrade', ('"' + $target + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($stop.ExitCode -ne 0) { throw 'Could not stop the installed Whispertype. Close it and retry.' }
}
New-Item -ItemType Directory -Force $destination, $config | Out-Null
if ((Resolve-Path $source).Path -ne $target) { Copy-Item $source $target -Force }
$savedKey = Join-Path $config 'groq.key'
$configured = ($env:GROQ_API_KEY -match '^gsk_[a-zA-Z0-9_]{16,}$')
if (Test-Path -LiteralPath $savedKey) { $configured = $configured -or ([IO.File]::ReadAllText($savedKey).Trim() -match '^gsk_[a-zA-Z0-9_]{16,}$') }
if ($configured) { Write-Host 'Existing Groq key detected; keeping it. No key entry needed.' }
else {
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
Start-Process $target -ArgumentList 'run' -WindowStyle Hidden
Write-Host 'Whispertype is running. Ctrl+Space: tap to toggle, hold to talk. Right-click its tray icon for settings.'
