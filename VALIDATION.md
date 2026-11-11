# Validation — November 11, 2026

The optimized implementation passes compilation, local unit/integration
tests, Linux microphone/desktop checks, and native Windows microphone/desktop
smoke tests. No live Groq transcription has been performed: an API key was
not supplied during development.

## Completed checks

- v0.1.2: hidden native-window tests verify immediate notification replacement,
  expiry without stopping the tray, and Unicode prompt Save/Cancel behavior.
  Enrollment state tests cover per-sample clicks, countdowns without audio,
  silence timeouts, short/long retries, and three successful captures.
- Mock Groq uploads assert the custom prompt field and its exact text. Wake
  detector construction rejects a missing personal model; synthetic references
  are installed explicitly only by tests, never by production fallback logic.
  The Linux headless integration checks migration from saved wake=true to off
  and opening personal setup instead of starting wake listening without a model.
- Windows builds contain icon and group-icon resources, and the Windows shell
  can extract the icon. Inno Setup and Start Menu shortcuts use the embedded icon.

- Native Windows x64 MSVC: **23 functional tests pass**, with three explicit
  opt-in tests (offline benchmark, interactive desktop, physical microphone).
  Packaging tests cover Windows/Linux archives, standalone outputs, repeat
  runs, checksums, and missing inputs. On Linux, a fake app verifies the
  self-extracting installer installs its payload and invokes `init` without
  compiling or touching the real desktop. Release tests reject corrupt or
  incomplete platform payloads before publishing.
- v0.1.1: native menus checked in all three activation modes; selected mode
  appears in the root submenu label, Check for updates is present, and the
  removed actions are absent. Desktop smoke tests verify the smaller indicator
  ends at the monitor's full bottom edge, including above the taskbar, while
  retaining focus. Pixel tests verify no transparent bottom-center gap.
- Manual update checks compare semantic versions against the latest stable
  GitHub release; mock HTTP checks cover newer/equal/older versions, excluded
  prereleases/drafts, malformed/oversized replies, missing releases and rate
  limits. The network/TLS work runs in a short-lived helper with no idle poll.
  A live, unauthenticated GitHub API check also completed successfully.
- Startup checkbox: native Windows checkmark states verified, with isolated
  registry tests for enable/disable/idempotence and preservation of unrelated
  entries. The real user's Run entry is not changed by the tests. Linux toggles
  systemd enablement without starting/stopping the currently running session.
- Opt-in Windows microphone check: default device opened at **48 kHz stereo**,
  passed through FFT resampling, delivered **100 PCM frames in three seconds**,
  closed cleanly, and reported no queue overflow. No audio saved/uploaded.
- Opt-in Windows desktop check: real Unicode `SendInput` into an owned native
  edit field (accented French, Japanese, emoji at a batch boundary); native
  tray create/update/delete; indicator visibility for recording/transcribing/
  enrollment and removal at idle; focus retained during indicator updates;
  hook press/repeat/release handling using unflagged hook structures; IPC
  delivery and clean window shutdown. This is not a physical-keyboard trial.
- Windows v0.1.0 release EXE: GUI subsystem, x64, **3,609,088 bytes**, system-only DLL
  imports, static MSVC runtime. Redirected `--help`, config path, invalid
  command stderr/exit status, and offline worker JSON output pass.
- Windows `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings` pass.

- Linux v0.1.0 baseline `cargo test --locked`: **12 tests pass**; the offline benchmark is excluded
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
`benchmarks/results.html`. Native Windows samples are described below. Live
Groq upload latency/memory has not been measured.

## Native Windows resource sample

Windows build 26100, v0.1.0 x64 MSVC release, default tray and indicator enabled.
Each state has three seconds of warmup and a ten-second sample; no compiler
or benchmark ran during sampling. Default capture was 48 kHz stereo with
FFT resampling. CPU is percent of one logical core and excludes Windows audio
services. Private bytes are committed private memory; working set includes
shared pages and is not comparable to Linux PSS.

| State | CPU, one core | Private bytes | Working set |
|---|---:|---:|---:|
| Initial idle | 0.00% | 1.71 MiB | 13.11 MiB |
| Recording | 0.16% | 3.32 MiB | 21.05 MiB |
| Wake listening | 0.78% | 3.14 MiB | 20.99 MiB |
| Idle after capture | 0.00% | 2.92 MiB | 20.86 MiB |

Windows/driver pages and sleeping threads can remain resident after capture;
there is no artificial working-set trimming. These short samples establish
behavior on this device, not a whole-app before/after speedup or battery claim.
The ambient wake sample included a detection attempt blocked before recording
by the missing API key; it is not a quiet-room or false-positive-rate test.
Wake accuracy still needs personal enrollment/testing. No audio was uploaded.

Reproduce with `scripts/measure-windows.ps1`; raw samples and EXE SHA-256 are in
`benchmarks/windows-resources.json`. The Windows offline reference benchmark
also passes; raw results and methodology are in `benchmarks/windows-offline.json`.
It confirms zero allocations per 500 captured frames and 3.84 MB of audio
buffers for a 120-second recording. The reference implementation retains
11.52 MB of audio buffers; those values exclude OS, driver, and TLS memory.

## Before relying on the Windows release

Run it on Windows 10/11 with your actual microphone and API key. Check tap vs
hold, shortcut capture, tray actions, focus-change protection, Unicode typing
in the applications you use, sign-in startup, and your voice's wake-phrase
accuracy. The GitHub Actions workflow runs native Windows build/unit/packaging
tests on every push to `main`, version tag, pull request, and manual dispatch.
It tests a temporary Windows install/uninstall and Linux self-extraction,
then publishes installers, standalone binaries and archives to GitHub Releases
after both builds pass. Main builds are prereleases; version tags are stable.
Hardware tests remain opt-in locally. The built executable is unsigned.
