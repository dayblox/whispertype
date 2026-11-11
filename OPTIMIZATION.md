# Optimization pass — November 11, 2026

This pass keeps the tray, automatic tap/hold, optional wake phrase,
and recording indicator. It optimizes actual processing and data
ownership instead of shrinking the working set artificially or removing
features. No memory-trimming calls, constant GC/cleanup timers, or forced
working-set eviction are used.

v0.1.3 removes foreground-window capture/checks and the redundant detector
VAD after the existing energy gate. No configuration watcher or file polling
was added. Windows notification bursts use a single 200 ms one-shot timer,
which stops after the newest message is delivered. The styled native forms
allocate their fonts/brush only when a form is opened; no UI framework or
notification process is added to the idle app.

v0.1.4 calculates personal-reference calibration only when loading/training
a wake model. Normal listening retains the existing energy gate; the extra
score probe exists only during an explicit eight-second local phrase test.
There is no periodic microphone sampling or calibration task at idle.

v0.1.5 caches MFCC FFT plans, DCT weights and matching workspaces, borrows
feature windows, and replaces full DTW matrices with two reusable rows.
The upstream extraction and terminal-cell arithmetic are checked for exact
compatibility. Quiet wake frames are suppressed before engine delivery;
recording keeps a direct, ungated PCM path. CPAL streams are owned by the
engine, and wake cancellation/model reuse avoids repeated setup work.
Shortcut recordings omit the silence scan used only for wake auto-stop.
See `benchmarks/windows-efficiency.md` for the paired Windows comparison,
same-audio benchmarks, allocation counts, raw data and reproduction commands.

## Implemented

- Prefer supported **mono 16 kHz** microphone input, with default-format
  fallback. At 16 kHz, omit the FFT resampler entirely. Native PCM16 is kept
  bit-exact, including minimum/maximum sample values.
- Emit fixed **480-sample / 30 ms** frames into a bounded channel. Buffers
  are allocated at startup; neither conversion nor successful channel
  delivery allocates a new vector per frame. The fallback batches stereo
  mixing and sample conversion instead of branching per input sample.
- Use squared-energy comparisons for silence checks, avoiding per-frame
  square roots and normalized-sample conversions.
- Skip wake recognition during sustained quiet. Retain 240 ms of pre-roll
  and approximately two seconds of hangover, reset stale detector state,
  and test reactivation after long pauses. Speech/noise above the gate still
  runs the full detector; adjust `speech_rms` for quiet voices.
- Stream the original PCM buffer and a **44-byte WAV header** through the
  bounded upload pipe. The HTTPS worker and CLI file path stream their
  input instead of collecting a whole WAV. Reject truncated streams and
  preserve worker error messages when it closes the pipe early.
- Bound recording capacity growth to the configured limit, avoiding the old
  power-of-two allocation overshoot on long recordings.
- Use the existing **Windows tray thread/message loop** for its indicator.
  There is no extra indicator process or indicator thread during normal use;
  GDI drawing resources are released immediately after committing its image.
- Run the **Linux tray service on its existing executor**, eliminating a
  sleeping service thread. Preserve the smaller asynchronous backend: a
  tested three-thread alternative used more memory and was rejected.
- Retain compact release code generation. Speed-oriented compiler settings
  were benchmarked but increased idle memory. The algorithmic improvements
  work without retaining that cost.
- Cache the Windows `TaskbarCreated` message ID once instead of registering it
  again on ordinary window messages. Keep Unicode surrogate pairs within
  bounded `SendInput` batches, including emoji on a batch boundary.
- Link the MSVC runtime statically for a self-contained native Windows EXE.
  Validate the GUI subsystem and system-only DLL imports on every build.

## Reproduce

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --release --locked optimization_benchmark -- --ignored --nocapture --test-threads=1
cargo build --release --locked
python3 scripts/profile-linux.py --binary target/release/whispertype --mode idle --seconds 20
python3 scripts/profile-linux.py --binary target/release/whispertype --mode record --seconds 20
python3 scripts/profile-linux.py --binary target/release/whispertype --mode wake --seconds 20 --wake-model ~/.config/whispertype/wake.rpw
```

`record` and `wake` profiling deliberately open the microphone. They use an
isolated temporary profile, exclude real API keys, cancel recordings, and
never upload audio. `idle` does not open it. Capture configurations can be
logged with `WHISPERTYPE_TRACE_AUDIO=1`; normal operation does not enable
that logging. The offline benchmark needs no microphone or network.

The benchmark retains the original conversion algorithm only in test code.
It compares both algorithms under identical final compiler settings and
reports the median of five runs. Its allocation counter is compiled only
into tests. The capture allocation check includes bounded-channel delivery.
The upload-buffer test tracks allocator peaks for a 120-second recording,
not the entire app, driver, TLS stack, or OS.

Raw results are in `benchmarks/results.json`. `VALIDATION.md` summarizes the
measured values and platform limitations. Hardware CPU samples include app
threads, but exclude the external PipeWire service; they are not measurements
of total system power or battery consumption. Synthetic digital silence
illustrates the wake gate and does not represent every real room.

## Remaining limits

Native Windows build, desktop, microphone, and release allocation benchmarks
now pass. Reproduce with `scripts/build-windows.ps1 -Benchmark -DesktopTests
-MicrophoneTest`. Profile the release app with `scripts/measure-windows.ps1`;
this opens the microphone in a disposable profile and cancels all recordings.
See `benchmarks/windows-resources.json` and `VALIDATION.md` for the historical
hardware sample and its limits; the v0.1.5 paired comparison is in
`benchmarks/windows-efficiency.md`. A live Groq upload was not measured because no API
key was supplied. Synthetic wake references are retained only as test fixtures; production
requires personal enrollment and wake listening starts off each launch. This is a
measured optimization pass, not proof of a theoretical resource minimum.
