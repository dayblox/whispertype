# Whispertype

Native Rust voice typing for **Windows 10/11 x64** and **Linux / Hyprland on Wayland**. No application window, browser engine, or local Whisper model. A tray menu handles settings; a small, static waveform capsule appears while recording or transcribing.

**Ctrl+Space** is the default:

- Tap and release within 250 ms to start hands-free recording; tap again to submit.
- Hold longer than 250 ms, then release to submit.
- Change the shortcut through **right-click tray → Shortcut → Record shortcut…**, or choose a preset. Supported keys: Space, letters, digits, F1–F24, plus Ctrl/Alt/Shift/Win.
- Choose Automatic, Hold, or Toggle explicitly from the same menu.

Dictation is uploaded after you stop; this is not streaming transcription. Results are typed into the foreground application using Unicode input, without changing the clipboard. If the focused window changed during recording/transcription, the text is retained for manual insertion using **Paste last transcript**. Focus protection checks windows, not individual fields within a window.

## Windows

Download from [GitHub Releases](https://github.com/dayblox/whisper/releases/latest):

- **Installer:** `whispertype-windows-x64-setup.exe`. Installs for the current user, offers optional sign-in startup and hidden Groq key entry, and includes an uninstaller. No administrator rights required.
- **Standalone:** `whispertype-windows-x64.exe`, or `whispertype-windows-x64.zip` for the binary with documentation and install/uninstall scripts.

For the ZIP, extract it and run `install-windows.ps1` in PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install-windows.ps1
```

The script installs under `%LOCALAPPDATA%\Whispertype`, asks for a [Groq API key](https://console.groq.com/keys) without displaying it, and starts the app at sign-in. No administrator rights or additional runtime are needed. Allow microphone access for desktop apps in Windows Privacy settings. Windows may warn about the unsigned executable.

For portable use, run `whispertype.exe` after saving your key as `%APPDATA%\whispertype\groq.key`. The tray icon may initially be in the taskbar overflow. Do not run as administrator: input into elevated applications or secure desktops is blocked by Windows. Some games and custom text controls do not accept Unicode `SendInput`.

To build on Windows, install Rust, the Visual Studio C++ build tools, and Python 3.11+ (packaging only), then run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows.ps1
# Optional local checks (opens a test window and the microphone):
.\scripts\build-windows.ps1 -Benchmark -DesktopTests -MicrophoneTest
# Also build Setup.exe (requires Inno Setup 6):
.\scripts\build-windows.ps1 -Installer
```

The script checks formatting, runs unit tests and Clippy, builds the native release EXE, verifies its GUI subsystem/system-only DLL imports and redirected CLI/worker output, then tests and creates the installable ZIP plus `SHA256SUMS`. Outputs: `target\release\whispertype.exe` and `dist\whispertype-windows-x64.zip`. The MSVC runtime is linked statically; end users need no Python or C++ runtime installation. For just the binary, `cargo build --release --locked` still works. `uninstall-windows.ps1` removes startup registration and the installed binary while retaining settings.

**Automatic releases:** [Build and test](https://github.com/dayblox/whisper/actions/workflows/build.yml) checks both platforms, tests installers, and publishes a [GitHub Release](https://github.com/dayblox/whisper/releases) only after both builds pass. Every `main` push publishes an immutable `main-RUNNUMBER-SHORTSHA` prerelease. Push a `vVERSION` tag matching `Cargo.toml` for a stable release (for example, `v0.1.0`); stable releases appear at the [latest release](https://github.com/dayblox/whisper/releases/latest) link. Pull requests run checks without publishing. Manual dispatch also produces a main prerelease. Each release includes installers, standalone binaries, portable archives, licenses, and a combined `SHA256SUMS`. No API key is needed for CI. Actions artifacts remain available for 30 days; release downloads persist independently.

**Validation:** native Windows unit, microphone, and desktop smoke tests now pass. The desktop check types Unicode into its own native edit field, exercises hook transitions and IPC, and checks the indicator does not steal focus. Physical shortcut use, personal wake accuracy, application-specific typing, sign-in startup, and live Groq transcription remain manual checks. See `VALIDATION.md`.

## Linux

Tested desktop: Omarchy / Hyprland with Lua configuration and PipeWire's ALSA compatibility layer. GNOME, X11, and generic non-Hyprland shortcut setup are not implemented. Release binaries are built on Ubuntu 22.04 (glibc 2.35+) and require ALSA/PipeWire, `libxkbcommon`, `wtype`, a StatusNotifier tray host, a layer-shell compositor and systemd user services. On Arch/Omarchy these runtime components are normally installed.

Download the **self-extracting installer** `whispertype-linux-x64-install.sh` from [Releases](https://github.com/dayblox/whisper/releases/latest):

```sh
bash whispertype-linux-x64-install.sh
~/.local/bin/whispertype set-key
```

It installs the bundled binary without Rust, configures Hyprland/user startup, and preserves settings. Alternatively, extract `whispertype-linux-x64.tar.gz` and run its `install-linux.sh`. For standalone use, download `whispertype-linux-x64`, make it executable with `chmod +x`, and run it with `run` after configuring your key. `init` enables desktop/startup integration. Standalone means no installation is required; Linux system libraries and desktop support requirements still apply.

To build from source, also install ALSA development headers and `pkg-config`:

```sh
cargo build --release --locked
scripts/install-linux.sh
# Paste the API key, then Ctrl+D; terminal input is visible with this simple CLI.
whispertype set-key
```

For hidden terminal entry instead:

```sh
read -rs -p 'Groq API key: ' groq_key
printf '%s' "$groq_key" | whispertype set-key
unset groq_key
```

For an extracted Linux binary, copy it to `~/.local/bin/whispertype` and run `whispertype init` instead of the source build script.

Installation backs up `~/.config/hypr/bindings.lua`, adds a `dofile` for generated Whispertype bindings, validates Hyprland reloads, and enables a user systemd service. It refuses a shortcut already assigned to another Hyprland action. Shortcut changes made through the tray rewrite the generated binding file and reload Hyprland.

For foreground troubleshooting: `systemctl --user stop whispertype; whispertype run`. Logs: `journalctl --user -u whispertype`. To uninstall, disable the service, remove its unit and binary, and remove the Whispertype `dofile` line from the Hyprland bindings. Your original bindings backup is preserved.

## Optional local wake phrase

Wake-word listening is **off by default**. Enable it from the tray. Say **“Hey Groq”**, wait for the green capsule, then dictate. A 1.5-second pause submits the utterance. Audio before activation stays local and is not uploaded. Wake listening pauses during upload and resumes afterward.

The included small acoustic reference model is generated from synthesized voices. It is experimental and needs microphone testing with your accent and environment. For a useful personal model, choose **Record personal wake phrase…** and say the same short phrase three times, with two-second gaps. Enrollment is local and replaces the default acoustic model; the phrase can be anything, including another language. Cancel from the tray if needed. A cheap energy gate skips recognition during sustained quiet, with 240 ms of pre-roll and approximately two seconds of hangover. It uses half of `speech_rms` as its threshold; lower that setting if your voice is too quiet to activate it. Keep wake mode off in environments with untrusted playback: false activation can record and upload speech.

Advanced enrollment from three to eight **mono, 16 kHz, PCM16 WAV** files:

```sh
whispertype train "my phrase" one.wav two.wav three.wav
whispertype reload
```

There is no paid wake-word service, access key, Python runtime, or downloaded neural model. Detection uses Rustpotter's MFCC/DTW reference matching. Its dependency graph includes Candle, but the app uses the reference detector, not neural inference. Wake mode necessarily keeps the microphone open and uses CPU; it cannot have the same idle cost as shortcuts.

## Groq and free-tier limits

The fixed endpoint is `https://api.groq.com/openai/v1/audio/transcriptions`; the fixed model is **`whisper-large-v3`**, not Turbo. Recordings are 16 kHz mono PCM16 WAV. The local limiter conservatively reserves at least ten audio seconds for every upload attempt and enforces rolling limits:

- 28,800 audio seconds/day (eight hours), 7,200/hour.
- 2,000 requests/day, 20/minute.

These are the [published Groq free-plan limits](https://console.groq.com/docs/rate-limits), checked November 10, 2026. Limits belong to the whole organization, can change, and are also used by other clients. Your [account limits](https://console.groq.com/settings/limits) are authoritative. The app cannot guarantee free use if the account has a paid plan, and it does not change billing settings. It handles HTTP 429 without automatic retries. Failed/canceled requests remain reserved locally, so its limiter may be stricter than Groq's.

No API key is bundled. `GROQ_API_KEY` overrides the local key file. Keys are sent only to the fixed HTTPS endpoint, never put on subprocess command lines or written to logs. Recordings remain in RAM and are discarded after each request; failed uploads are not recoverable. One successful transcript is retained in `pending.txt` for recovery until overwritten or deleted. Wake models and the quota ledger are also stored locally. Speech sent to Groq is subject to [Groq's data policies](https://console.groq.com/docs/your-data).

## Configuration and commands

`whispertype config` prints the config path. Default locations: `%APPDATA%\whispertype\config.toml` on Windows and `~/.config/whispertype/config.toml` on Linux. `WHISPERTYPE_HOME` overrides the directory for tests or isolated profiles. Reload through the tray after editing.

```toml
shortcut = "CTRL + SPACE"
mode = "auto"                    # auto | hold | toggle
hold_ms = 250
max_seconds = 120                # bounded to 2..600, safely below Groq's 25 MB cap
silence_ms = 1500                # wake-mode end of utterance
speech_rms = 0.006               # energy gate; lower for quiet microphones
tray = true
indicator = true
wake_enabled = false
wake_threshold = 0.65            # higher reduces false activations
wake_phrase = "Hey Groq"         # label; enroll to change the actual acoustic phrase
protect_focus = true
prompt = ""                      # short vocabulary/context hints
# language = "en"                # omit for automatic language detection
# microphone = "USB"             # substring of input device name
```

Commands: `run`, `init`, `press`, `release`, `start`, `stop`, `toggle`, `cancel`, `wake-on`, `wake-off`, `enroll`, `paste`, `reload`, `quit`, `set-key`, `config`, `train`, `transcribe FILE.wav`. On Windows the GUI-subsystem executable writes CLI output only when handles are redirected. `init` registers sign-in startup on Windows; the installer script additionally starts it immediately.

## Resource design and checks

Idle shortcut mode closes the microphone and removes the overlay entirely. The core sleeps waiting for events; the tray and Windows keyboard hook are event-driven. There are no animation loops, idle network requests, periodic microphone probes, or continuously running transcription workers. Capture prefers driver-supported mono 16 kHz input and skips resampling at that rate, falling back to the default format and an anti-aliasing FFT resampler when needed. Conversion uses preallocated buffers and fixed 30 ms frames in a bounded channel; it does not allocate a vector per callback. Recording capacity is bounded (default 120 seconds, 3.84 MB PCM); silence-only input is rejected before upload.

Only one upload runs at a time. The existing PCM recording is streamed with a small WAV header through a bounded pipe to the HTTPS worker, which uploads it without reading the whole clip into RAM. There are no second and third full-size WAV buffers. CLI file transcription also streams directly from the file. The worker's HTTPS allocations are reclaimed on exit. The Windows indicator uses the tray's existing native message loop, without an additional process or thread. Linux renders it in a small layer-shell helper only while active. Both draw once per state with no animation timer. The Linux tray shares its asynchronous executor instead of starting another service thread.

This minimizes common sources of idle/active overhead; “physically minimal” cannot be proven across OSes, audio drivers, hardware, and tray hosts. The measured optimization results and reproducible benchmark commands are in `OPTIMIZATION.md`; platform validation and remaining limitations are in `VALIDATION.md`. Faster compiler settings were tested and rejected when they increased idle memory; the default build retains compact code generation.

```sh
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

CI builds and tests Windows and Linux. Built-in wake-model generation is documented in `assets/README.md`.
