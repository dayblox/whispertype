# Validation — November 11, 2026

The optimized implementation passes compilation, local unit/integration
tests, and Linux microphone/desktop smoke checks. It has **not** been run on
a real Windows desktop and no live Groq transcription has been performed:
an API key was not supplied during development.

## Completed checks

- `cargo test --locked`: **12 tests pass**; the offline benchmark is excluded
  from ordinary test runs and passes separately in release mode. Tests cover
  tap/hold and explicit modes, config validation, native PCM preservation,
  resampling/alias rejection, silence trimming, bounded recording capacity,
  byte-identical streamed WAVs, truncated-input handling, rolling quotas,
  multipart uploads and 200/401/429 mock responses, wake matching/rejection,
  and rearming after long quiet periods.
- `cargo clippy --locked --all-targets -- -D warnings` on Linux.
- Equivalent Clippy check for `x86_64-pc-windows-gnullvm`.
- Optimized Linux x64 build and Windows x64 cross-build. Windows GUI subsystem
  confirmed, so launching the app does not open a console window.
- Windows import table inspected: only Windows system DLLs; LLVM's unwind
  runtime is linked statically. No extra DLL, model download, or Python
  environment is needed for the Windows application.
- Linux tray registration confirmed through StatusNotifierWatcher.
- CPAL opened the default PipeWire/ALSA microphone, recording was canceled,
  the capture threads stopped, wake listening toggled on/off, and the app
  quit cleanly. No recording was submitted to Groq during this check.
- Hyprland reported the click-through layer-shell indicator at 88×36 logical
  pixels. Screenshot capture could not finish while the current desktop was
  inactive; the compositor confirms mapping, not final pixel appearance.
- Unicode text was accepted by `wtype` after correcting its argument handling.
  There was no focused window in this desktop session, so insertion into a
  real text field could not be verified. The app refuses automatic insertion
  when it cannot identify the focused window.
- `cargo fmt --check`, `git diff --check`, shell syntax checks, Python script
  syntax checks, and archive contents checks.

## Measured optimization results

Machine: x86_64 Omarchy 4.5.0, Linux 7.2.15 / PipeWire ALSA. Sequential
20-second active samples after three seconds of warmup, with no compilation
or competing benchmark during those final samples. Default tray and
indicator are enabled. Recordings are canceled; no audio is uploaded.
CPU is percent of one core and excludes the external PipeWire service.

| Measurement | Before | Optimized |
|---|---:|---:|
| Recording CPU | 1.75% | 1.10% |
| Quiet wake listening CPU | 2.85% | 1.05% |
| Recording app + indicator PSS | 8.91 MiB | 8.03 MiB |
| Idle RSS (10-second sample) | 6.26 MiB | 6.23 MiB |
| Idle threads | 6 | 5 |
| Idle CPU ticks during sample | 0 | 0 |

Idle RAM is essentially unchanged, not a statistically established reduction.
Idle PSS was 3,852 vs 3,911 KiB: fixed audio frames trade a small bounded queue
allocation for eliminating repeated allocations while recording. Audio/driver
initialization can leave reusable pages resident. Short CPU samples are
indicative, not power-consumption or battery-life measurements.

The final hardware capture path reports **16 kHz mono, direct**, avoiding
an application-side FFT resampler on this microphone. Unsupported formats
still use the default-format/resampling path.

Offline tests retain the original algorithms only as test references and
use identical final compiler settings (median of five runs):

| Offline workload | Before | Optimized |
|---|---:|---:|
| Convert 150 seconds, native 16 kHz mono | 36.33 ms | 1.85 ms |
| Convert 150 seconds, 48 kHz stereo fallback | 77.52 ms | 66.37 ms |
| Process 30 seconds of digital silence for wake detection | 48.81 ms | 0.49 ms |
| Capture allocations per 500 frames | 500 | 0 |
| Peak audio buffers for a 120-second clip | 11.52 MB | 3.84 MB |

The zero-allocation check includes optimized bounded-channel delivery.
Audio-buffer peaks exclude OS, driver, TLS, and allocator cache memory; this
is a **67% reduction in the tested audio buffers**, not in whole-app RAM.
Synthetic digital silence is not representative of every room. Speech/noise
above the gate still runs MFCC/reference recognition.

See `OPTIMIZATION.md` for design/reproduction commands and
`benchmarks/results.json` for raw measurements. The chart is available as
`benchmarks/results.html`. Windows performance and live Groq upload latency/
memory have not been measured.

## Before relying on the Windows release

Run it on Windows 10/11 with your actual microphone and API key. Check tap vs
hold, shortcut capture, tray actions, focus-change protection, Unicode typing
in the applications you use, sign-in startup, and your voice's wake-phrase
accuracy. The provided GitHub Actions workflow adds native Windows build/unit
tests when this repository is pushed; those tests do not replace desktop and
microphone testing. The built executable is unsigned.
