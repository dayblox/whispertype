# Windows WinHTTP and installation folder validation

Measured November 12, 2026, Windows 11 build 26100, Intel i7-9700K.
Both release executables use the same compiler and compact release settings.
The machine has direct WinHTTP access, without a configured corporate proxy.

## Binary and dependencies

| Windows release EXE | Bytes |
|---|---:|
| Before | 4,079,616 |
| After | 2,827,776 |

Reduction: 1,251,840 bytes (30.7%). Baseline SHA-256:
`87c59e0c4c029e4e13dc0dea57547c039313fe7a438607eb9a647a8b560ef964`.
Measured implementation SHA-256 (before the v0.1.11 release version bump):
`505334d9cdd4327d1b24a8fb43e714a515d443f22e472255cf747d59f0a023ca`.
The resource profiles identify that pre-release-version executable. The release
version bump changes version metadata and the HTTP User-Agent. Release URLs
also use the renamed dayblox/whispertype repository to avoid the old API's HTTP
301 response; the measured networking and capture algorithms are unchanged.
The Windows normal dependency graph contains no ureq, rustls, ring or webpki
roots. Linux retains ureq and its TLS dependencies. The notices retain the
shared resolved graph and explain the platform split. Windows imports the
system `winhttp.dll` and `bcrypt.dll`; MSVC remains statically linked.

## Tray, capture and wake states

Three fresh processes per build, five seconds per state following one second
of warmup. Baseline group first, then final group. The existing profiling mode
opens the real microphone and cancels audio; it disables submission **and the
warm network helper**. These figures compare the tray/capture paths only.
Private/working-set values are medians in MiB; CPU is millions of cycles/second.

| State | Before CPU | After CPU | Before private | After private | Before working set | After working set |
|---|---:|---:|---:|---:|---:|---:|
| shortcut_idle | 0.000 | 0.135 | 1.816 | 1.832 | 10.590 | 10.633 |
| shortcut_recording | 6.932 | 6.888 | 2.555 | 2.566 | 14.357 | 14.311 |
| wake_listening | 4.944 | 5.022 | 2.793 | 2.805 | 14.484 | 14.465 |
| wake_recording | 6.877 | 6.580 | 2.793 | 2.816 | 14.633 | 14.637 |
| idle_after_capture | 0.000 | 0.000 | 2.430 | 2.434 | 14.359 | 14.348 |

All initial-idle samples recorded zero process-time increments and zero audio
events; two final samples had small nonzero cycle counts. Separate ten-second
idle rechecks after three seconds of warmup are retained in the idle JSON files.
Both recheck medians are zero CPU cycles/second, zero process CPU time and zero
audio events. Median private memory is 1.816 MiB before and 1.828 MiB after;
the 12 KiB difference is within the observed fresh-process variation.
Small memory differences and ambient wake CPU variation do not establish a
regression or improvement. No idle session, helper, microphone, polling loop,
animation or new timer is introduced. Handles fall from 160 to 156 at initial
idle. Capture uses the same 48 kHz stereo input and FFT resampling.

## Transient network cost — accepted

Three interleaved before/after trials against real HTTPS endpoints. Each process
has a disposable profile and an intentionally invalid Groq key. Upload inputs
are one second of generated digital silence, never microphone recordings.
Groq rejects the key with the same 401 message in both builds. GitHub checks
return no newer release. Poison HTTP_PROXY/HTTPS_PROXY environment values do
not interfere with Windows requests. No proxy or user settings are modified.

| Helper operation | Before peak private MiB | After peak private MiB | Before Mcycles | After Mcycles | Before elapsed ms | After elapsed ms |
|---|---:|---:|---:|---:|---:|---:|
| Groq rejected upload, cold helper | 1.691 | 2.715 | 65.399 | 113.357 | 57.95 | 73.66 |
| GitHub release check | 1.766 | 2.793 | 62.612 | 114.801 | 82.29 | 99.86 |

These measurements show about **1.0 MiB more transient private memory** and
more CPU cycles with WinHTTP/Schannel on this machine. Network latency is
uncontrolled; these invalid-key responses do not establish actual transcription
latency, proxy-login latency, or a universal CPU difference. Coarse Windows
process-time accounting is noisy for these short requests; retain cycle counts
and individual trials in the raw data. Peak working sets increase from about
11.2 to 14.5 MiB; shared system DLL pages are included and must not be counted
as unique physical RAM for every process.

Warm-up starts the same upload helper earlier, while recording. At one second
its median private memory is **2.711 MiB**, working set **13.469 MiB**, and
startup/network work about **97.925 million CPU cycles**. The baseline has no
helper during recording, so this is an additional recording-state cost, not
an idle cost. There is one helper, not separate warm-up and upload processes;
it waits on the audio pipe, reuses its session/connection, and exits after
transcription. Discard/cancel/shutdown kills and reaps the recording's helper.
Warm-up failures are ignored and do not reserve transcription usage.

The required WinHTTP migration and early warm-up therefore have a measured
resource tradeoff. On November 12, 2026, the user explicitly accepted the measured
cost and instructed: "Accept the cost and keep WinHTTP + warm-up." This satisfies
the agreement requirement in AGENTS.md for these changes; future resource
regressions still require separate agreement. No claim of a physical resource
minimum is made.

## Correctness and allocation checks

`scripts/build-windows.ps1 -Installer -Benchmark` passes formatting, **47
functional tests**, two MFCC/DTW equivalence tests, Clippy with warnings denied,
release build, system DLL/icon/GUI checks, update replacement/rollback/helper
integration, package/release tests and Inno Setup compilation. Ten tests are
opt-in; the relevant allocation benchmarks run with `-Benchmark`.

WinHTTP regressions cover exact Content-Length and chunked writes, reuse of
one actual TCP connection for a warm GET followed by POST, truncated input,
redirect handling, Basic-only proxy refusal, bounded integrated-proxy refusal,
Negotiate-before-NTLM selection, unchanged Groq HTTP errors and the response
cap. A slow-drip response verifies the absolute deadline and closes pending
reads before releasing their buffers. Real unauthenticated HTTPS GETs to Groq
(401) and GitHub (200) pass separately. A successful corporate proxy handshake
and real-key transcription remain unverified on this direct-access machine.

The 64 KiB and 8 MiB WinHTTP upload checks both track five Rust allocations and
a 192-byte peak, including the mock server's header parsing. Upload data uses
one fixed 64 KiB stack buffer. Update downloads retain one 110-byte Rust
allocation for both sizes, with a fixed 64 KiB stack buffer. These counters
exclude native Windows HTTP/TLS/CNG allocations; the process profiles above
include those. The canonical SHA-256 `abc` vector passes against Windows CNG.
Existing PCM, bounded recording, delivery and wake-equivalence checks pass;
the inactive keyboard hook remains allocation-free.

Inno Setup explicitly enables folder selection, retaining the previous folder
on upgrades. ZIP scripts accept the same custom Destination for install and
uninstall; uninstall removes only its EXE and an empty directory. The isolated
installer installation regression was not run because a real installation
already exists and that test refuses to alter it. Linux execution was not
available (no WSL/Linux toolchain); the Linux ureq implementation is retained.

## Reproduce and raw results

```powershell
cargo build --release --locked
# Save baseline before applying changes, then build the final revision.
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows.ps1 -Installer -Benchmark
cargo test --locked winhttp_live_https_smoke -- --ignored --nocapture --test-threads=1
python scripts/measure-windows-efficiency.py --binary target/whispertype-before-winhttp.exe --label before-winhttp --output benchmarks/windows-winhttp-before.json --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-efficiency.py --binary target/release/whispertype.exe --label after-winhttp --output benchmarks/windows-winhttp-after.json --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-http.py --before target/whispertype-before-winhttp.exe --after target/release/whispertype.exe --output benchmarks/windows-winhttp-network.json
```

Raw data: [before](windows-winhttp-before.json), [after](windows-winhttp-after.json),
[HTTP helpers](windows-winhttp-network.json),
[idle recheck before](windows-winhttp-idle-before.json),
[idle recheck after](windows-winhttp-idle-after.json).
