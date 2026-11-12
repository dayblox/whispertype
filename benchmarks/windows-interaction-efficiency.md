# Escape and persistent-menu resource checks

Measured on Windows 11 build 26100, Intel i7-9700K, with the existing compact
release settings. Three fresh processes per build; five seconds of initial
shortcut idle after one second of warmup. An isolated profile and PID-specific
controls leave other installed copies alone. The profiler subsequently checks
capture transitions and cancels them; no audio is saved or uploaded.

| Median, initial shortcut idle | Before | After |
|---|---:|---:|
| CPU cycles/second | 0 | 0 |
| Audio events/second | 0 | 0 |
| Private memory, MiB | 1.8125 | 1.8164 |
| Working set, MiB | 10.6992 | 10.7070 |
| Handles | 160 | 160 |

The private-memory median differs by one 4 KiB page, within the baseline's
fresh-process variation. Working set includes shared pages. This short sample
does not establish a small memory improvement/regression or a theoretical
minimum. No new idle worker, timer or polling loop exists. Capture, resampling,
wake recognition and upload ownership are unchanged by these interaction fixes.

The release-only hook check processes 40,000 inactive/unmatched key events
with **zero tracked Rust heap allocations**. It calls the hook using synthetic
key structures, without sending physical keypresses to the user's applications;
it does not measure end-to-end OS keyboard latency. Cancellation uses an inline
event in the existing bounded queue. Other keys avoid snapshot locks.

Persistent menu handling installs a thread-local `WH_MSGFILTER` hook only
during native menu tracking. The existing UI thread dispatches setting clicks;
state-change messages refresh checkmarks. Dismissal unhooks and destroys the
menu, as verified by a real native menu test. The native menu's message loop
waits for events; there is no periodic repaint. Menu-open CPU/RAM and live
transcription cancellation were not separately profiled. The desktop smoke
test verifies cancellation-event delivery and repeat suppression.

Raw profiles: [before](windows-interaction-before.json),
[after](windows-interaction-after.json). They include binary hashes, per-trial
values, capture configuration and the first-run variation.

```powershell
python scripts/measure-windows-efficiency.py --binary PATH_TO_BASELINE --label before-escape-menu --output benchmarks/windows-interaction-before.json --states shortcut_idle --seconds 5 --warmup 1 --trials 3
python scripts/measure-windows-efficiency.py --binary target/release/whispertype.exe --label after-escape-menu --output benchmarks/windows-interaction-after.json --states shortcut_idle --seconds 5 --warmup 1 --trials 3
cargo test --release --locked inactive_keyboard_hook_allocates_nothing -- --ignored --nocapture --test-threads=1
cargo test --locked native_menu_keeps -- --test-threads=1
cargo test --locked windows_desktop_smoke -- --ignored --test-threads=1
```
