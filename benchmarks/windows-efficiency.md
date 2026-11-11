# Windows efficiency comparison: v0.1.4 → v0.1.5

Measured on November 11, 2026, on Windows 11 build 26100, an Intel
i7-9700K (8 logical CPUs), and the default 48 kHz stereo microphone.
This device needs application-side FFT resampling; supported mono 16 kHz
devices use the existing direct capture path instead.

## Full application

| State | CPU Mcycles/s, v0.1.4 | CPU Mcycles/s, v0.1.5 | Private MiB, v0.1.4 | Private MiB, v0.1.5 |
|---|---:|---:|---:|---:|
| Shortcut idle | 0.000 | 0.000 | 1.793 | 1.789 |
| Shortcut record | 7.787 | 7.808 | 3.445 | 3.410 |
| Wake idle | 7.562 | 6.120 | 2.828 | 2.793 |
| Wake record | 7.907 | 7.660 | 3.418 | 3.383 |
| Idle after mic | 0.000 | 0.022 | 2.461 | 2.371 |

Quiet wake standby uses **19.1% fewer cycles** in these paired trials, and
engine audio deliveries fall from **33.3 per second to zero** after the
hangover. This does not stop the microphone or driver callback. Recording
CPU work is essentially unchanged: shortcut recording differs by +0.3%,
and wake recording by -3.1%, with overlapping trial ranges. Private memory
falls by about **36 KiB** in both recording modes and in wake standby;
initial shortcut idle is essentially unchanged. Working set during recording
is about **15.2 MiB** after the change (private memory is about **3.4 MiB**).
No CPU cycles were observed in the untouched initial shortcut-idle samples.
Small post-capture message/OS activity is retained in the results.


The table uses medians of three paired trials, with 15 seconds per state
after three seconds of warmup. Each run starts a fresh process and exercises
the same five states in order. Build order alternates AB, BA, AB. The tray,
indicator and real microphone capture are enabled. Recordings contain room
audio held in RAM and are canceled; nothing is saved or uploaded. The test
opens a disposable profile and addresses its own tray window by PID.

CPU work is measured with Windows `QueryProcessCycleTime`. Cycle counts can
vary with CPU frequency and system scheduling; small differences are not
established improvements. `GetProcessTimes` is also recorded, but its coarse
accounting produced zeros and inconsistent outliers on these short samples.
The JSON retains those values and their ranges rather than presenting zero
as the recording CPU cost. These are app-only measurements and exclude
Windows audio services, drivers, other processes and total system power.

Private bytes measure committed private memory. Working set includes shared
and resident pages; it is also recorded in the JSON. The app does not force
pages out of memory or request working-set trimming. Initial idle and idle
after capture are separate because audio initialization leaves reusable OS
and driver pages resident.

Both executables use Rust 1.99.1 and the same compact release settings. The
baseline is commit `b189bdf9a7efa471efe3fd55868c70aec5d2751e` plus the common
profiling hooks in `v0.1.4-profiling.patch`. The hooks count audio events and
report state, permit capture in a profile without an API key, prevent upload
workers from starting, and suppress automatic recording completion during a
measurement. Both builds execute their production capture, recording and
matching paths. They are not an unchanged release binary versus an
instrumented build.

## Recognition on identical audio

Each value is the median of five runs of 40 passes, after model loading and
one warmup pass. A frame contains 480 samples / 30 ms. The private positive
clip has three spoken phrase repeats and a three-second quiet tail; the
negative fixture is other speech with the same quiet tail. These timings
include the application's energy gate and recognition pipeline, but exclude
microphone capture and model-loading time.

| Workload | v0.1.4, µs/frame | v0.1.5, µs/frame | Less processing time |
|---|---:|---:|---:|
| Other speech | 369.82 | 110.10 | 70.2% |
| Personal phrase | 179.36 | 54.89 | 69.4% |
| Digital silence | 0.422 | 0.422 | Essentially unchanged |

| Allocations per complete audio pass | v0.1.4 | v0.1.5 | Reduction |
|---|---:|---:|---:|
| Other speech, 5.52 s | 471,352 | 884 | 99.81% |
| Personal phrase, 13.02 s | 518,414 | 1,690 | 99.67% |
| Digital silence, 30 s | 0 | 0 | Already allocation-free |

The positive clip produces **3 detections in both builds**; other speech and
the silent control produce zero. The best score on the private clip remains
identical (0.61548555), as does the automatically calibrated target
(0.52079213). Thresholds are unchanged by this optimization pass. This is one
person's local diagnostic, not a representative accuracy or false-activation
study. Audio and personal reference features are not included in this repo.

The allocation counter is test-only. Its `peak_incremental_live_bytes`
measures the peak net live allocation above the warmed state, excluding
retained workspaces, allocator caching, and OS/driver memory. It is not the
whole application's memory footprint. Live private-memory samples above
include retained workspaces.

## What changed

- The wake matcher reuses two DTW rows instead of allocating and copying
  full matrices. Vector magnitudes are computed once per comparison,
  normalization workspaces are reused, and unsuccessful candidates do not
  allocate a score map. Arithmetic order and upstream terminal-cell behavior
  are preserved.
- MFCC extraction caches its FFT plan, FFT scratch, spectra, mel workspaces
  and DCT weights. A test-only original extractor and original matrix DTW
  implementation verify bit-for-bit equivalence across sizes and resets.
  Both native CI jobs run these checks.
- Quiet wake frames are suppressed in the capture callback before waking
  the engine. The same 240 ms pre-roll, approximately two-second hangover,
  and detector-reset frame are preserved. Recording bypasses that delivery
  gate and retains every PCM frame. Regression tests compare detection frame
  positions across long pauses and check exact recording PCM.
- The engine directly owns the CPAL stream. The extra thread that only
  waited to hold that stream is removed; CPAL still owns its callback thread.
  Wake cancellation reuses the open stream, and the compiled detector is
  reset and reused between recordings. Disabling listening releases it.
- Shortcut recordings skip the live silence scan used only for wake-triggered
  recording completion. The final speech trim remains unchanged. Neither
  mode runs wake inference while recording.

Conversion, delivery and bounded-channel transfer are allocation-free after
initialization, including both new delivery modes. PCM storage grows in
bounded steps: a 120-second recording needs at most **3.84 MB of sample
capacity** in either build, without a duplicate WAV buffer. Shortcuts keep
the microphone closed at idle and use no periodic configuration/update poll.
Wake standby requires continuous microphone capture; the driver and any
required resampling remain its baseline cost. These measurements do not
prove a theoretical physical minimum.

## Reproduce

Apply `v0.1.4-profiling.patch` to a separate checkout of the baseline commit
and build with `cargo build --release --locked`. Build current main with the
same compiler. Keep the baseline executable in a separate directory. The
patch also supplies the same test-only benchmark and allocation counter.

```powershell
python scripts/compare-windows-efficiency.py `
  --before path/to/baseline/whispertype.exe `
  --after target/release/whispertype.exe --seconds 15 --trials 3

# Use your own personal wake model and local spoken diagnostic.
$env:WHISPERTYPE_BENCH_MODEL = "$env:APPDATA\whispertype\wake.rpw"
$env:WHISPERTYPE_BENCH_WAV = 'path/to/local-16khz-mono-pcm16.wav'
cargo test --release --locked efficiency_benchmark -- --ignored --nocapture --test-threads=1
cargo test --release --locked personal_capture_delivery_equivalence -- --ignored --nocapture --test-threads=1
cargo test --release --locked optimization_benchmark -- --ignored --nocapture --test-threads=1
cargo test --release --locked -p rustpotter --lib mfcc::
```

The recognition benchmark expects three recognized repeats. Do not compile
or run another benchmark during native measurements. The native runner sets
`WHISPERTYPE_PROFILE=1`, removes the API-key environment variable and copies
only the wake model into its disposable profile. It uses Ctrl+Alt+Shift+F24
to avoid intercepting ordinary typing. Existing installed instances are not
controlled. No live Groq transcription, TLS/upload latency, battery use or
Linux hardware resource comparison was measured in this pass.

Raw results: `windows-efficiency-before.json`, `windows-efficiency-after.json`,
`windows-efficiency-dsp-before.json`, `windows-efficiency-dsp-after.json`.
`windows-efficiency.html` visualizes the same values.
