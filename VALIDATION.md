# Validation — November 11, 2026

The optimized implementation passes compilation, local unit/integration
tests, Linux microphone/desktop checks, and native Windows microphone/desktop
smoke tests. No live Groq transcription has been performed: an API key was
not supplied during development.

## Completed checks

- v0.1.13 candidate warm-up fix (November 13): **60 functional tests**, Clippy, formatting,
  release/system-DLL verification, update-helper integration, packaging/release
  checks, Inno Setup compilation, two MFCC/DTW equivalence checks and the native
  microphone smoke test pass. The primary still triggers probing after 300 ms;
  candidates get two seconds from their first native callback. Synthetic tests
  cover 1.1 seconds of zeros followed by a winning live frame with its prefix,
  the exact two-second boundary, a last candidate remaining open and buffering
  after exhaustion, energy selection at finish, and recovery after unresolved
  wake listening becomes a recording. Candidate silence never enters the dead
  cache. Release checks report zero allocations for 1,000 health frames per
  source and preserve capture/delivery allocation and bounded-buffer checks.
  Matched Windows resource profiles and hardware limits are recorded in
  `benchmarks/windows-mic-warmup.md`. Native Linux execution and the reported
  Intel DSP warm-up hardware remain unverified here.

- Digital-silence capture fallback: **57 functional tests**, two MFCC/DTW
  equivalence tests, Windows build/Clippy/import/package/installer checks,
  allocation benchmarks and the real microphone smoke test pass. Synthetic
  frames cover dead/recovered inputs, first-live selection, bounded buffers,
  all-dead energy choice, cache reset and queued-frame safety. Linux capture
  modules pass isolated cross-target type-checking/Clippy, with no native Linux
  execution here. Resource conditions and limits are recorded in
  `benchmarks/windows-microphone-fallback.md`. The user accepted the measured
  36–52 KiB active and 44 KiB post-capture working-set increase by instructing
  release after reviewing that cost on November 12, 2026. This is an accepted
  tradeoff under `AGENTS.md`, not evidence of zero added resource cost.

- November 12 WinHTTP/installer changes: **47 functional tests**, formatting,
  Clippy, release/system-DLL checks, package/release tests, update integration,
  streaming allocation checks and Inno Setup compilation pass through
  `scripts/build-windows.ps1 -Installer -Benchmark`. Windows dependencies omit
  ureq/rustls/ring/bundled roots; system WinHTTP and CNG supply networking and
  crypto. Native proxy refusal, auth preference, connection reuse, redirects,
  multipart bounds and absolute timeout regressions pass. Live Groq/GitHub
  HTTPS GET smoke tests pass without a real key. Corporate login, real-key
  transcription and Linux execution remain unverified here. The release EXE
  shrinks from 4,079,616 to 2,827,776 bytes. Measured transient resource cost
  was explicitly accepted by the user on November 12, 2026; evidence is in
  `benchmarks/windows-winhttp-efficiency.md`.

- v0.1.8 Windows interaction changes: **40 functional tests pass**, plus the
  opt-in desktop smoke test. Escape hook tests cover idle pass-through, active
  cancellation, held-key repeat/release after cancellation, and injected-key
  pass-through. Desktop checks receive exactly one cancellation event for a
  held Escape press. A real native menu test filters mouse/Enter selections
  without ending tracking, updates startup/mode/shortcut checkmarks on the
  same menu handles, ignores outside hits, exits through the menu's normal
  Escape handling, and verifies the menu is destroyed on dismissal.
  The release hook check reports zero Rust heap allocations across 40,000
  inactive/unmatched key events. Three before/after initial-idle samples record
  zero CPU cycles and unchanged median handle count; the small memory deltas
  and measurement limits are in `benchmarks/windows-interaction-efficiency.md`.
- v0.1.7 update/form changes: **37 Windows functional tests pass**, with seven
  opt-in checks; two vendored MFCC/DTW equivalence tests also pass. The full
  Windows build, Clippy, native executable checks, package/release tests and
  Inno Setup compilation pass. Native prompt checks exercise Ctrl+A selection,
  replacement/undo, Tab focus, multiline Enter, Save with Enter, Escape and
  Unicode Save/Cancel. Clipboard shortcuts explicitly send the native edit
  commands; the automated suite does not replace the user's clipboard.
  Update tests reject corrupt, truncated and oversized downloads, unexpected
  asset URLs and missing/invalid digests, and restore the previous file when
  replacement or process launch fails. A disposable Windows integration test
  executes the real helper with paths containing spaces/Unicode, waits for
  parent exit, stops lingering helpers only from the matching EXE, replaces
  and restarts the portable copy, preserves its key/prompt, and removes staging.
  This simulates a version replacement locally; no newer GitHub release was
  published or installed into the user's existing installation by these local checks.
  The release-only download allocation check retains a fixed 64 KiB stack
  buffer and one 110-byte tracked heap allocation for both 64 KiB and 8 MiB
  inputs (excludes HTTP/TLS and OS memory). Resource measurements and limits
  are in `benchmarks/windows-update-efficiency.md`.
- v0.1.6: a user-run short diagnostic in Notepad reproduced corruption with
  bulk Unicode input and once inserted the complete 17-character greeting
  with character-pair calls and no added delay. Repeated dictation in the
  installed v0.1.6 still produced missing/repeated letters and question marks
  in Notepad 11.2607.14.0. The installed binary matched the local v0.1.6 build.
  The user then confirmed that disabling both Notepad Spell check and
  Autocorrect stopped the corruption. The individual responsible feature and
  exact internal cause remain unisolated. The same user could not reproduce
  corruption in T3 Code or Chromium/Helium. Character-pair delivery alone
  therefore does not establish reliable Notepad compatibility.
  Protocol regressions cover the exact short and long reported
  sentences, multilingual text, combining/ZWJ sequences and surrogate pairs.
  A failed/partial submission stops further text and attempts only key-up
  cleanup, preventing an automatic duplicate retry. Ordinary Windows tests
  now total **31 passed**, with six opt-in checks. The offline insertion
  benchmark checks zero allocations with a simulated successful input sink;
  it does not measure Windows API/destination-app latency. Automatic live
  Notepad testing was unavailable because the Windows UI-control bridge was
  disconnected. These live Notepad observations are user reports, separate
  from the deterministic protocol tests and owned Win32 edit-field checks.
- v0.1.5: **30 Windows functional tests pass**, with five opt-in tests;
  two vendored MFCC/DTW equivalence tests also pass in debug and release.
  The retained original extractor verifies bit-identical features across
  sizes and resets, and the original matrix DTW checks identical terminal
  costs. New regressions preserve recording PCM, quiet pre-roll/hangover,
  and detection frame positions after long pauses. Capture conversion,
  both delivery modes and bounded-channel transfer allocate zero times in
  their warmed checks. The private voice replay still recognizes three
  repeats with the identical best score/target and rejects the silent control.
  See `benchmarks/windows-efficiency.md` for matched v0.1.4/v0.1.5 resource
  measurements, including their sampling and app-only limits.
- v0.1.4: native Windows raster checks cover the entire button rectangle,
  including enabled/disabled rounded corners, client bounds, and the prompt
  editor. Captures use actual Win32 controls rendered off-screen, not HTML
  mockups. Prompt fields no longer show an empty white scrollbar.
- Personal-reference calibration checks use other enrollment recordings,
  excluding each recording's self match, with bounded targets and invalid/
  silent-reference rejection. Migration preserves non-default numeric overrides.
  An explicitly requested live Windows phrase test recognized all three repeats
  with calibration; the old fixed target missed all three on the same recording.
  A separate silent recording produced no detections. These local samples are
  not bundled, and this is not a representative false-activation-rate study.

- v0.1.3: hidden native-window tests verify that consecutive Windows
  notifications retain only the latest result and do not stop the tray.
  Prompt Save/Cancel preserves Unicode. Enrollment tests cover the local
  phrase-test countdown, timeout/retry and explicit Enable action.
  A trained-reference regression recognizes a repeat at half volume;
  this synthetic check does not establish real microphone accuracy.
  Installer CI upgrades with two lingering helpers, preserves a saved key,
  confirms replacement input is initially disabled, and leaves a different
  EXE copy running. Uninstall also closes a lingering helper.
  Enrollment state tests cover per-sample clicks, countdowns without audio,
  silence timeouts, short/long retries, and three successful captures.
- Mock Groq uploads assert the custom prompt field and its exact text. Wake
  detector construction rejects a missing personal model; synthetic references
  are installed explicitly only by tests, never by production fallback logic.
  The Linux headless integration checks migration from saved wake=true to off
  and opening personal setup instead of starting wake listening without a model.
- Windows builds contain icon and group-icon resources, and the Windows shell
  can extract the icon. Inno Setup and Start Menu shortcuts use the embedded icon.

- Native Windows x64 MSVC v0.1.4: **28 functional tests pass**, with three explicit
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
  real text field could not be verified in that historical check. v0.1.3
  removes that focus guard and sends text to the current target unconditionally.
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
hold, shortcut capture, tray actions, Unicode typing into the current target
in the applications you use, sign-in startup, and your voice's wake-phrase
accuracy. The GitHub Actions workflow runs native Windows build/unit/packaging
tests on every push to `main`, version tag, pull request, and manual dispatch.
It tests a temporary Windows install/uninstall and Linux self-extraction,
then publishes installers, standalone binaries and archives to GitHub Releases
after both builds pass. Main builds are prereleases; version tags are stable.
Hardware tests remain opt-in locally. The built executable is unsigned.
