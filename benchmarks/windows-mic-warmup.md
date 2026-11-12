# Candidate microphone warm-up fix

Measured November 13, 2026 on Windows 11 build 26100, Intel i7-9700K,
Rust 1.99.1, with unchanged compact release settings. The baseline is the
unmodified v0.1.12 executable built from this checkout before the fix. Profiling
precedes the v0.1.13 version bump; the raw reports identify both measured
executables by SHA-256. Each group uses three fresh processes, five seconds
per state after one second
of warmup, initial baseline groups first. A further three baseline processes
run after the implementation profiles to check variation between groups.
No compilation or other benchmarks ran
during profiling. The existing profiler uses disposable configuration,
disables uploads and connection warm-up, cancels recordings, and addresses
only its own process. No audio is stored or uploaded, and no working-set
trimming is used. These measurements exclude audio services/drivers and
the HTTP helper.

## Functional and allocation checks

`cargo test --locked` passes 60 functional tests (11 opt-in tests excluded).
New regressions advance synthetic callback times without sleeping. They
cover a dead primary with a candidate delivering zeros through 1,080 ms and
signal at 1,100 ms, preserving all 37 prefix frames; the two-second candidate
deadline measured from the first callback; recovery after that deadline;
and continued buffering, unchanged dead-primary memory, and finish-time
energy selection after exhausting the device list. Starting recording from
unresolved wake listening also retains the last candidate. Existing primary
300 ms timing, queued-frame, epoch, PCM and buffer-bound checks still pass.

Formatting, Clippy with warnings denied, release/system-DLL verification,
and two vendored MFCC/DTW bit-equivalence tests pass. The native microphone
smoke test delivers 102 frames and closes without overflow. The release
health check reports zero heap allocations for 1,000 frames **per source**.
The existing optimization benchmark also reports zero allocations for
conversion/channel delivery (500 frames) and shortcut/wake delivery (1,000
frames), with the unchanged 3,840,000-byte bound for one PCM buffer at the
120-second recording limit. These checks exclude OS/driver allocation.

## Matched working microphone

Both builds explicitly select the existing HD Pro Webcam C920 input through
the disposable profile's `microphone` setting: 48 kHz stereo, FFT resampling.
The entries are medians of three trials. Memory is KiB; CPU is millions of
process cycles per second, with ambient wake-detector activity.

| State | Before Mcycles/s | After Mcycles/s | Before private KiB | After private KiB | Before working set KiB | After working set KiB |
|---|---:|---:|---:|---:|---:|---:|
| shortcut_idle | 0.000 | 0.000 | 1868 | 1860 | 10924 | 10920 |
| shortcut_recording | 6.808 | 6.853 | 2608 | 2608 | 14822 | 14816 |
| wake_listening | 25.124 | 26.813 | 2856 | 2856 | 14980 | 14988 |
| wake_recording | 6.947 | 7.024 | 2856 | 2856 | 15128 | 15136 |
| idle_after_capture | 0.000 | 0.000 | 2452 | 2356 | 14780 | 14776 |

Private memory and handle counts during active states match. Working-set
deltas are within overlapping trial ranges; the first baseline process has
an unusually high working set, retained in the raw data rather than removed.
Wake cycle medians differ with ambient input: before 23.489–27.301 Mcycles/s,
after 26.712–27.773 Mcycles/s. This is not controlled same-audio CPU evidence
and does not establish a wake CPU improvement or regression. Initial and
post-capture idle have zero audio events and median zero CPU cycles. A single
initial-idle sample in each group includes a small burst of non-audio work;
no recurring idle activity is observed. Both release EXEs are 2,886,656 bytes.

Raw data: [before](windows-mic-warmup-before.json),
[after](windows-mic-warmup-after.json).

## Default-device profile

The default-device runs open a primary and a candidate for each capture in
both builds (four 48 kHz stereo stream openings across recording and wake
listening). This exercises native fallback on this machine, but does not
reproduce the reported slow DSP. The same units and sampling conditions apply.

| State | Before Mcycles/s | After Mcycles/s | Before private KiB | After private KiB | Before working set KiB | After working set KiB |
|---|---:|---:|---:|---:|---:|---:|
| shortcut_idle | 0.000 | 0.000 | 1852 | 1864 | 10908 | 10932 |
| shortcut_recording | 6.876 | 7.181 | 2636 | 2652 | 14812 | 14830 |
| wake_listening | 26.085 | 27.160 | 2688 | 2716 | 14996 | 15012 |
| wake_recording | 6.761 | 7.172 | 2920 | 2948 | 15222 | 15238 |
| idle_after_capture | 0.000 | 0.000 | 2456 | 2408 | 14764 | 14744 |

Default-input active medians are 16–28 KiB higher in private memory and
16–18 KiB higher in working set; initial idle is already 12 KiB higher in
private memory and 24 KiB higher in working set. Post-capture memory is lower.
CPU cycle ranges overlap in recording states; ambient wake-listening ranges
are 25.845–26.746 before and 27.044–27.172 after. These short sequential,
ambient samples do not isolate those differences or establish zero added
cost. They are retained alongside the explicit working-device comparison,
whose active private bytes and handle counts match. Both groups return to
zero post-capture CPU cycles and audio events, with unchanged handle counts
in every state.

Repeating the **unchanged baseline** after both implementation groups gives
active cycle medians of 7.233 / 27.410 / 7.707 Mcycles/s for shortcut recording,
wake listening, and wake recording. Its active private medians are 2644 / 2712 /
2928 KiB, and working sets are 14834 / 15016 / 15234 KiB. These reproduce much
of the first comparison's shifts with the same old binary: all three active
working-set medians are within 4 KiB of the new build. Recording capacities
also cross existing growth thresholds at different points in these short
samples (192,000 or 288,000 bytes in the raw records). The profiles therefore
do not establish a persistent per-build regression; they also cannot prove
an exact zero-instruction or zero-byte cost.

Raw data: [default before](windows-mic-warmup-default-before.json),
[default after](windows-mic-warmup-default-after.json),
[repeated baseline](windows-mic-warmup-default-control.json).

## Scope and limits

The fix uses a role bit in the existing stream mode, with no larger audio
event or additional clock read, timer, thread, dependency, or idle operation.
After a live stream wins, callbacks retain the existing direct delivery path.
The two recording buffers remain bounded by `max_seconds * 16000` samples
each. Keeping a quiet candidate alive intentionally extends its capture and
buffering work; replacing it intentionally permits a brief third stream
until the successful open returns and the old candidate closes. These are
the requested lifecycle changes, not evidence that warm-up and handoff have
zero resource cost. This machine does not reproduce a silent final candidate
or a three-stream replacement, so their native driver costs are not isolated
by these profiles. No theoretical minimum or universal hardware claim is made.

The reported Intel Smart Sound 48 kHz four-channel DSP microphone is not
available here. Its delayed-signal case is verified synthetically, not by a
live hardware reproduction. Native Linux execution, hardware disconnection,
and real Groq transcription were not performed for this fix.

## Reproduction

```powershell
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -p rustpotter --lib mfcc::
cargo test --release --locked microphone_probe_allocates -- --ignored --nocapture --test-threads=1
cargo test --release --locked optimization_benchmark -- --ignored --nocapture --test-threads=1
cargo test --locked microphone_smoke -- --ignored --nocapture --test-threads=1
cargo build --release --locked
python scripts/verify-windows.py target/release/whispertype.exe
python scripts/measure-windows-efficiency.py --binary target/whispertype-before-mic-warmup.exe --label before-mic-warmup --microphone "HD Pro Webcam C920" --output benchmarks/windows-mic-warmup-before.json --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-efficiency.py --binary target/release/whispertype.exe --label after-mic-warmup --microphone "HD Pro Webcam C920" --output benchmarks/windows-mic-warmup-after.json --seconds 5 --warmup 1 --trials 3
```

Use an actual working microphone name on the test machine. The profiler
requires an existing personal wake model; it copies that model into each
disposable profile. Omit `--microphone` for the default-device profiles.
