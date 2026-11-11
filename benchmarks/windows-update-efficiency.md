# Windows update and prompt efficiency checks

Measured November 11, 2026 on Windows-11-10.0.26100-SP0, Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz.
The baseline is the pre-change v0.1.6 release executable; the updated build
uses the same release settings and profiling hooks. Three fresh processes
per build, five seconds per state after one second of warmup. The baseline
group ran first, then the final build; this is not an alternating paired trial.
The app uses its real tray, indicator and 48 kHz stereo/FFT-resampling capture.
Disposable profiles are addressed by PID. Audio is canceled, never uploaded
or saved. Other installed copies are left alone.

| State | Before Mcycles/s | After Mcycles/s | Before private MiB | After private MiB | Before working set MiB | After working set MiB |
|---|---:|---:|---:|---:|---:|---:|
| shortcut_idle | 0.000 | 0.000 | 1.789 | 1.820 | 10.668 | 10.691 |
| shortcut_recording | 7.117 | 6.863 | 2.527 | 2.551 | 14.434 | 14.451 |
| wake_listening | 12.455 | 5.109 | 2.766 | 2.789 | 14.609 | 14.598 |
| wake_recording | 6.790 | 7.183 | 2.766 | 2.789 | 14.754 | 14.746 |
| idle_after_capture | 0.000 | 0.000 | 2.352 | 2.441 | 14.422 | 14.422 |

Entries are medians of three trials. Untouched initial shortcut idle recorded
zero CPU cycles and zero audio events in both builds. Post-capture idle retains
small OS/message activity in the raw samples; its median cycle count is zero.
No polling, additional idle thread, timer, or retained network worker was added.
Memory remains in the same small range, with private-memory median differences
of tens of KiB. Those small differences should not be treated as an established
improvement or regression: fresh-process/allocator variation is visible in the
raw samples. Recording-buffer growth also falls close to the six-second
measurement boundary; some trials retain 192,000 bytes and others 288,000 bytes
at the final snapshot. This can affect post-capture allocator retention.

Wake CPU samples are especially sensitive to ambient sound. The before and
after audio-delivery counts differ, so the wake-listening CPU comparison is
not a controlled same-input performance result. Windows process-time accounting
is coarse over five seconds; cycle counts are retained as the more useful
CPU-work observation. Measurements exclude audio services, drivers, other
processes and system power. They do not establish a physical resource minimum.
Live Groq uploads and a real newer-release HTTPS installation were not profiled.

The update path uses no new crate packages: hashing and secure staging names
reuse `ring`, already present for HTTPS. Downloads stream through a fixed
64 KiB stack buffer in a short-lived worker, with a 100 MiB maximum asset size.
TLS/HTTP allocations are released when that process exits. The idle engine
retains only the optional update path/flags, and installs on an existing event
once dictation, enrollment, startup changes and prompt editing are finished.
Prompt controls remain native, with shortcuts handled only on keyboard events.

The release-only `update_download_allocations` check measured one 110-byte
Rust heap allocation for both a 64 KiB and an 8 MiB streamed binary. It also
asserts that tracked heap use stays below 4 KiB, independently of file size.
This excludes the fixed stack buffer, HTTP/TLS, filesystem caches and OS memory.
The native helper integration replaces a disposable old executable, restarts,
removes staging, preserves its key/prompt and leaves other EXE copies running.

Reproduce:

```powershell
python scripts/measure-windows-efficiency.py --binary PATH_TO_BASELINE --label before-update-and-form-fixes --output benchmarks/windows-update-before.json --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-efficiency.py --binary target/release/whispertype.exe --label after-update-and-form-fixes --output benchmarks/windows-update-after.json --seconds 5 --warmup 1 --trials 3
cargo test --release --locked update_download_allocations -- --ignored --nocapture --test-threads=1
python scripts/test_windows_update.py target/release/whispertype.exe
```

Raw samples: [before](windows-update-before.json), [after](windows-update-after.json).
Executable hashes: before `1de9798c94d330ff7c6e639e9999fbcf9c83856a2d6e8af72c3777cf108e87c9`, after
`4c2a646848cd5cccaf24c2ba2d1a804dad2e7bbebf4df31045ad43c5008c1fed`. The full profiles include executable sizes,
per-trial values, capacity, handles, audio deliveries and capture configuration.
Permanent requirements for future changes are in [AGENTS.md](../AGENTS.md).
