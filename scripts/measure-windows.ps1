# Measures a disposable profile. Recordings are canceled, never submitted.
param(
    [ValidateRange(5, 30)][int]$Seconds = 10,
    [string]$Output = (Join-Path $PSScriptRoot '..\benchmarks\windows-resources.json')
)
$ErrorActionPreference = 'Stop'
if (Get-Process whispertype -ErrorAction SilentlyContinue) {
    throw 'Quit any running Whispertype instance before profiling.'
}
$exe = (Resolve-Path (Join-Path $PSScriptRoot '..\target\release\whispertype.exe')).Path
$profileRoot = Join-Path ([IO.Path]::GetTempPath()) ('whispertype-profile-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $profileRoot | Out-Null
$previousHome = $env:WHISPERTYPE_HOME
$previousTrace = $env:WHISPERTYPE_TRACE_AUDIO
$previousKey = $env:GROQ_API_KEY
$app = $null
try {
    $env:WHISPERTYPE_HOME = $profileRoot
    $env:WHISPERTYPE_TRACE_AUDIO = '1'
    $env:GROQ_API_KEY = $null
    [IO.File]::WriteAllText((Join-Path $profileRoot 'config.toml'), "protect_focus = false`nmax_seconds = 120`n")
    # Existence permits capture. This placeholder is removed before wake mode;
    # the recording is explicitly canceled well before the 120-second limit.
    [IO.File]::WriteAllText((Join-Path $profileRoot 'groq.key'), 'local-profile-no-api-key')
    $app = Start-Process -FilePath $exe -ArgumentList run -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $profileRoot 'stderr.log') -RedirectStandardOutput (Join-Path $profileRoot 'stdout.log')
    function Send-Control([string]$Command) {
        $control = Start-Process -FilePath $exe -ArgumentList $Command -WindowStyle Hidden -PassThru
        if (!$control.WaitForExit(5000) -or $control.ExitCode -ne 0) { throw "Control failed: $Command" }
    }
    function Measure-State([string]$State) {
        Start-Sleep -Seconds 3
        $app.Refresh()
        if ($app.HasExited) { throw 'Whispertype exited unexpectedly' }
        $cpu = $app.TotalProcessorTime.TotalSeconds
        $watch = [Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $Seconds
        $app.Refresh()
        [ordered]@{
            state = $State
            elapsed_seconds = $watch.Elapsed.TotalSeconds
            cpu_percent_one_core = 100 * ($app.TotalProcessorTime.TotalSeconds - $cpu) / $watch.Elapsed.TotalSeconds
            working_set_bytes = $app.WorkingSet64
            private_bytes = $app.PrivateMemorySize64
            threads = $app.Threads.Count
            handles = $app.HandleCount
        }
    }
    $samples = @()
    $samples += Measure-State idle
    Send-Control start
    $samples += Measure-State recording
    Send-Control cancel
    Remove-Item -LiteralPath (Join-Path $profileRoot 'groq.key')
    Send-Control wake-on
    $samples += Measure-State listening
    Send-Control wake-off
    $samples += Measure-State idle_after_capture
    Send-Control quit
    if (!$app.WaitForExit(5000)) { throw 'Whispertype did not exit cleanly' }
    $trace = Get-Content -LiteralPath (Join-Path $profileRoot 'stderr.log') -Raw
    if (($trace | Select-String 'Audio capture:' -AllMatches).Matches.Count -lt 2) {
        throw "Capture did not open for both recording and wake listening: $trace"
    }
    if ($trace -match 'Microphone:|fell behind|Could not|No default') { throw $trace }
    $result = [ordered]@{
        measured_at = [DateTime]::UtcNow.ToString('o')
        os = [Environment]::OSVersion.VersionString
        architecture = $env:PROCESSOR_ARCHITECTURE
        executable_bytes = (Get-Item $exe).Length
        executable_sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        sample_seconds = $Seconds
        warmup_seconds = 3
        notes = 'App only; excludes Windows audio services. Ambient audio, not a controlled silent room. No audio saved or uploaded. CPU percent is of one logical core; working set includes shared pages.'
        audio_trace = $trace.Trim()
        samples = $samples
    }
    $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Output -Encoding UTF8
    Write-Host "Measurements: $Output"
} finally {
    if ($app -and !$app.HasExited) {
        $app.Kill()
        $app.WaitForExit()
    }
    $env:WHISPERTYPE_HOME = $previousHome
    $env:WHISPERTYPE_TRACE_AUDIO = $previousTrace
    $env:GROQ_API_KEY = $previousKey
    $resolvedProfile = [IO.Path]::GetFullPath($profileRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedProfile.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedProfile -Leaf) -like 'whispertype-profile-*') {
        Remove-Item -LiteralPath $resolvedProfile -Recurse -Force
    }
}
