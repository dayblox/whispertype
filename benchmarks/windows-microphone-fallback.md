# Digital-silence microphone fallback

Measured November 12, 2026 on Windows 11 build 26100, Intel i7-9700K,
Rust 1.99.1, unchanged compact release settings. Baseline is the v0.1.11
implementation before microphone fallback. Three fresh processes per build,
five seconds per state following one second of warmup; baseline group first.
No working-set trimming. These are app-only measurements, excluding audio
services/drivers and the existing HTTP helper. Profiling disables submission
and network warm-up; all captured audio is canceled, never saved or uploaded.

## Matched working input

Both builds explicitly select the same existing HD Pro Webcam C920 microphone
through the original `microphone` setting. The profiler's optional
`--microphone` argument only configures its disposable profile. No product
setting, device setting, or user profile is changed. Unlike a default-device
comparison, this measures the ordinary working-input path in both builds.

| State | Before Mcycles/s | After Mcycles/s | Before private MiB | After private MiB | Before working set MiB | After working set MiB |
|---|---:|---:|---:|---:|---:|---:|
| shortcut_idle | 0.000 | 0.000 | 1.832 | 1.812 | 10.652 | 10.656 |
| shortcut_recording | 7.004 | 6.555 | 2.566 | 2.543 | 14.414 | 14.465 |
| wake_listening | 29.325 | 26.494 | 2.805 | 2.781 | 14.590 | 14.625 |
| wake_recording | 7.219 | 6.474 | 2.805 | 2.781 | 14.730 | 14.777 |
| idle_after_capture | 0.000 | 0.000 | 2.387 | 2.387 | 14.402 | 14.445 |

CPU is millions of process cycles/second; memory is MiB. The table contains
medians of three trials. Initial idle records zero CPU cycles and zero audio
events in both builds. Active private bytes are 24 KiB lower and measured cycle
counts do not increase. However, working set is 36 KiB higher while wake
listening, 52 KiB higher during shortcut recording, 48 KiB higher during wake
recording, and 44 KiB higher after capture closes. Initial idle working set is
4 KiB higher (within overlapping trial ranges). The active working-set increase
persists after replacing the newer CPAL description's multiple property reads
with the original single Windows FriendlyName query.

The requested no-added-cost constraint has **not** been established. The user
accepted the reported active and post-capture working-set increase by instructing
release after reviewing these measurements on November 12, 2026. This is an
accepted tradeoff under AGENTS.md. No working-set trimming or memory-accounting
workaround was used. These short ambient recordings do
not prove an exact zero-instruction cost or a theoretical resource minimum.
CPU process-time accounting is coarse over five seconds; raw cycle counts and
trial ranges are retained. Wake recognition depends on ambient sound, so small
CPU differences are not controlled same-audio optimization results.

Windows release size is 2,827,776 bytes before and 2,886,656 bytes after
(+58,880 bytes, or 57.5 KiB).

## Default-device comparison

The initial default-device run is retained separately. The new build opens a
candidate while listening, then delivers audio where the baseline's quiet
gate delivered no frames. Wake CPU consequently rises from about 5.6 to
25.6 Mcycles/s because a different, live input now reaches the detector. This
is a changed workload, not a matched working-device CPU comparison. No physical
mute, USB power state, or driver setting was changed to produce this result.
The initial default-device after-profile predates final queued-frame handling
and Linux selector refinements; the matched working-input after-profile uses
the final Windows executable and records its SHA-256.

## Resource ownership and limits

The healthy committed callback uses direct delivery and the existing wake
gate. It performs no peak scan, clock read, new allocation or device query.
Startup/probing scans one existing 480-sample frame for peak magnitude; the
dead decision starts at the first native callback and waits at least 300 ms.
There is no probe timer or extra application thread. The existing engine event
queue remains bounded; probe frames carry source identity and a classification
epoch so losing streams and earlier wake/recording decisions cannot be reused.

At most two CPAL streams are active. The engine keeps the original recording
buffer and one candidate buffer, each capped at `max_seconds * 16000` samples.
That is at most 3,840,000 PCM bytes per buffer at the default 120-second limit,
or 19,200,000 bytes each at 600 seconds. They grow through the existing bounded
append routine. A dead candidate is closed before the next opens; its storage
is reused. Selection immediately closes the other stream and releases unused
buffers/device enumeration state. Only stable dead IDs and the default-input
identity can remain after capture closes. A recovered input clears its dead
entry; a changed default clears the cache at the next capture start.

Peak <= 2 means digital silence for this heuristic, not proof that hardware is
disconnected. A quiet DSP/noise-gated primary remains open while candidates are
tested and can recover and win. Live means any frame peak > 2, not recognized
speech. If nothing wins before finish, the higher-energy buffer is used and
the existing speech-range rejection still applies. All-dead handling only
writes stderr. Silence counters, the initial-speech timeout and wake detection
do not consume undecided/dead frames; the overall recording bound still applies.

The public CPAL stable-ID APIs require a dependency update. Windows uses CPAL
0.17.3, retaining its existing WASAPI capture model without a default-device
listener. Linux uses CPAL 0.18.2's ALSA backend, whose device-by-ID API supports
explicit server source PCMs. The native PulseAudio/pipewire-pulse lookup runs
only for a dead or remembered server alias. It uses a bounded synchronous
mainloop operation, no subscriptions or worker, and closes the context and
library before returning. Both `pipewire:NODE` and `pulse:DEVICE` selectors
are supported and share one source identity; an unavailable plugin can fall
through to the other without opening the same source twice. Monitor sources
are excluded. Sources: [CPAL IDs](https://docs.rs/cpal/0.18.2/cpal/traits/trait.DeviceTrait.html),
[PipeWire ALSA selectors](https://github.com/PipeWire/pipewire/blob/master/pipewire-alsa/conf/50-pipewire.conf),
[PulseAudio ALSA selectors](https://github.com/alsa-project/alsa-plugins/blob/master/pulse/50-pulseaudio.conf).

No enumeration or server lookup runs while shortcut idle, and no application
device-change listener, periodic poll, notification or setting was added.
Ordinary capture errors remain bounded by the existing event queue and driver
behavior; this is not a guarantee for every possible driver, permission state,
or unavailable audio-server/plugin combination.

## Validation

`scripts/build-windows.ps1 -Installer -Benchmark -MicrophoneTest` passes:
57 ordinary functional tests, two MFCC/DTW equivalence tests, formatting,
Clippy with warnings denied, release/system-DLL verification, update helper
integration, package/release checks and Inno Setup compilation. The real
microphone smoke test opens/delivers/closes without overflow. Existing PCM,
resampling and wake equivalence/allocation checks pass. One pre-existing
HTTP truncation fixture was held open until the client rejects its input,
removing an unrelated racing TCP-close assertion; HTTP production code is
unchanged.

New synthetic tests cover the exact <= 2 threshold including i16::MIN,
300 ms timing, an always-dead primary, late recovery, first-live selection,
closing the loser, preserving queued winner frames, stale-source/epoch
rejection, all-dead energy choice, bounded candidate storage, default-change
cache reset, selector identity and rearming a reused wake stream. The release
health-scan allocation check reports zero allocations over 1,000 probe frames;
healthy committed callbacks bypass that scan. There is no per-frame buffer
allocation in callbacks.

Linux capture modules pass cross-target type-checking and Clippy in an isolated
harness with Config/Event stubs and ALSA pkg-config metadata only. No Linux
native library was linked or executed: the full cross-build cannot run here
without a Linux C compiler/sysroot for the existing ring dependency. Native
Linux PulseAudio/PipeWire and unplugged/muted hardware tests remain unverified.
The synthetic and one-machine observations do not establish universal hardware
compatibility.

## Reproduction

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-windows.ps1 -Installer -Benchmark -MicrophoneTest
python scripts/measure-windows-efficiency.py --binary target/whispertype-before-mic-fallback.exe --label before-mic-fallback-working-input --microphone "HD Pro Webcam C920" --output benchmarks/windows-mic-working-before.json --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-efficiency.py --binary target/release/whispertype.exe --label after-mic-fallback-working-input-final --microphone "HD Pro Webcam C920" --output benchmarks/windows-mic-working-after.json --seconds 5 --warmup 1 --trials 3
```

Use an actual working microphone name on the test machine. Raw data:
[working before](windows-mic-working-before.json),
[working after](windows-mic-working-after.json),
[default before](windows-mic-fallback-before.json),
[default after](windows-mic-fallback-after.json).

Baseline working-profile SHA-256: `ae0ea1198e1e80de00460d340c88bf59765740880880d1a7d8cb69501c5c059b`.
Working-profile implementation SHA-256 (before the v0.1.12 version bump): `21aef5dbefd7011bcf2f5ec7f631c5f235ddeccdcbd179c4988cff51c2c754be`.
