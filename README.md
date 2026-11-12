# Whispertype

Native Rust voice typing for **Windows 10/11 x64** and **Linux / Hyprland on Wayland**. No always-open application window, browser engine, or local Whisper model. A tray menu handles settings; a small, static waveform capsule appears centered against the bottom screen edge while recording or transcribing. The indicator is 64x24 logical pixels (128x48 backing pixels), with no bottom gap or transparent padding.

**Ctrl+Space** is the default:

- Tap and release within 250 ms to start hands-free recording; tap again to submit.
- Hold longer than 250 ms, then release to submit.
- Change the shortcut through **right-click tray → Shortcut → Record shortcut…**, or choose a preset. Supported keys: Space, letters, digits, F1–F24, plus Ctrl/Alt/Shift/Win.
- Choose Automatic, Hold, or Toggle explicitly from the same menu. The top-level entry shows the current selection, for example **Activation: Hold to talk**.
- **Check for updates** checks the latest stable GitHub release on demand. On Windows x64 it downloads and verifies the newer executable, then installs and restarts once dictation and open prompt editing are finished. Installed and portable copies keep their existing location, settings, key, and startup registration. If automatic installation is unavailable (for example, the folder is read-only), it opens the download page instead. Linux opens the download page. There is no background update polling.
- **Launch at startup** is a checkbox reflecting your current sign-in registration. Ticking it enables automatic launch for your user account; unticking it leaves the current session running. Windows uses the current user's Run entry, and Linux uses a systemd user service. No administrator rights are needed.

On Windows, **Escape** cancels active recording, transcription, or personal phrase capture without submitting it. It remains available to other applications while Whispertype is idle/listening. Escape in the prompt editor cancels that editor. The tray menu stays open when changing shortcuts, activation mode, wake listening, or startup, and when checking for updates; checkmarks update in place. Click outside or press Escape to dismiss it. Editor/setup actions and Quit close the menu.

Dictation is uploaded after you stop; this is not streaming transcription. Completed text is sent as Unicode keystrokes to whatever is active when transcription finishes, without changing the clipboard. Whispertype does not inspect input fields, remember the original window, or require a focused text field. Both left- and right-clicking the tray icon open its menu; use the shortcut for dictation. Start/stop, Cancel, and Paste remain available as CLI commands for automation and recovery.

## Windows

Download from [GitHub Releases](https://github.com/dayblox/whispertype/releases/latest):

- **Installer:** `whispertype-windows-x64-setup.exe`. Choose the installation folder (defaults to `%LOCALAPPDATA%\Whispertype`). Installs for the current user, offers optional sign-in startup and hidden Groq key entry, and includes an uninstaller. No administrator rights required; choose a folder your account can write to so automatic updates can replace the EXE.
- **Standalone:** `whispertype-windows-x64.exe`, or `whispertype-windows-x64.zip` for the binary with documentation and install/uninstall scripts.

For the ZIP, extract it and run `install-windows.ps1` in PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install-windows.ps1
```

The script installs under `%LOCALAPPDATA%\Whispertype`, asks for a [Groq API key](https://console.groq.com/keys) without displaying it, and starts the app at sign-in. No administrator rights or additional runtime are needed. Allow microphone access for desktop apps in Windows Privacy settings. Windows may warn about the unsigned executable.

For a different folder, pass `-Destination "D:\Apps\Whispertype"` to the ZIP installation script, and the same argument to its uninstall script when removing it.

Corporate proxies are supported through the system/WinHTTP proxy settings (including machine-wide `netsh winhttp set proxy` configuration). Windows uses WinHTTP and Schannel for all HTTPS calls, with the signed-in Windows account for Negotiate/NTLM proxy login; no proxy password is saved and Basic authentication is never used. `HTTP_PROXY`/`HTTPS_PROXY` environment variables are ignored on Windows. Recording starts a short-lived helper to warm the Groq connection; cancellation, discarded audio, and completed transcription release it. Linux continues to use ureq.

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

**Automatic releases:** [Build and test](https://github.com/dayblox/whispertype/actions/workflows/build.yml) checks both platforms, tests installers, and publishes a [GitHub Release](https://github.com/dayblox/whispertype/releases) only after both builds pass. Every `main` push publishes an immutable `main-RUNNUMBER-SHORTSHA` prerelease. Push a `vVERSION` tag matching `Cargo.toml` for a stable release (for example, `v0.1.0`); stable releases appear at the [latest release](https://github.com/dayblox/whispertype/releases/latest) link. Pull requests run checks without publishing. Manual dispatch also produces a main prerelease. Each release includes installers, standalone binaries, portable archives, licenses, and a combined `SHA256SUMS`. No API key is needed for CI. Actions artifacts remain available for 30 days; release downloads persist independently.

**Validation:** native Windows unit, microphone, and desktop smoke tests now pass. The desktop check types Unicode into its own native edit field, exercises hook transitions and IPC, and checks the indicator does not steal focus. Physical shortcut use, personal wake accuracy, application-specific typing, sign-in startup, and live Groq transcription remain manual checks. See `VALIDATION.md`.

Windows v0.1.6 sends one Unicode character's key-down/up events per input call.
There is no artificial typing delay or clipboard replacement. Emoji surrogate
pairs stay together. The receiving app still determines repaint and undo behavior.

**Windows 11 Notepad compatibility:** repeated dictation in Notepad 11.2607.14.0
still produced missing/repeated letters and literal question marks in v0.1.6,
despite an initial successful character-pair diagnostic. The user confirmed that
turning off both **Spell check** and **Autocorrect** in Notepad's Settings stopped
the corruption. This implicates Notepad's proofreading/input interaction, not the
document's UTF-8 setting; which individual feature or internal component causes it
has not been isolated. The same user could not reproduce it in T3 Code or
Chromium/Helium. This is a confirmed workaround on that machine, not a guarantee
for every Notepad version. Whispertype does not change other apps' settings.

## Linux

Tested desktop: Omarchy / Hyprland with Lua configuration and PipeWire's ALSA compatibility layer. GNOME, X11, and generic non-Hyprland shortcut setup are not implemented. Release binaries are built on Ubuntu 22.04 (glibc 2.35+) and require ALSA/PipeWire, `libxkbcommon`, `wtype`, a StatusNotifier tray host, a layer-shell compositor and systemd user services. On Arch/Omarchy these runtime components are normally installed.

Download the **self-extracting installer** `whispertype-linux-x64-install.sh` from [Releases](https://github.com/dayblox/whispertype/releases/latest):

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

## Personal wake phrase

Wake listening starts **off on every launch**, including upgrades from profiles that previously saved it as enabled. Enabling it is an explicit action for the current session. If no personal model exists, selecting wake listening opens phrase setup instead of opening the microphone for background listening. There is no synthetic fallback or restore-default phrase option.

Choose **Record personal wake phrase...**. On Windows a single small panel shows the current step, countdown, speech capture, and saved sample count. Click **Record sample 1 / 3**, wait for **Say your phrase now**, speak a short distinctive phrase, then stay quiet. The sample finishes after 0.7 seconds of silence. Click Record for each of the remaining samples at your own pace. Quiet, short, or overlong samples can be retried individually. Cancel closes setup and preserves an existing model. The microphone is closed between samples. After saving, choose **Enable listening**, **Test phrase**, or **Close**. Closing leaves listening off.

**Test personal wake phrase...** runs recognition locally with a countdown and an eight-second attempt. It reports whether your saved model recognized the phrase, closes the microphone afterward, and never submits test audio to Groq or starts dictation. Retry or re-record if needed. Testing leaves wake listening off; choose **Enable listening** when ready.

Wake sensitivity is calibrated from agreement between your personal recordings. The old fixed default of 0.65 migrates to automatic calibration (`wake_threshold = 0`). The target has a floor of 0.50 and a ceiling of 0.60; inconsistent recordings are rejected rather than continually lowering sensitivity. Other explicit numeric overrides remain available. Match scores are internal detector scores, not probabilities or accuracy percentages. The local test now distinguishes missing/quiet microphone audio from a phrase mismatch and shows both the measured score and target. Detection fires after three confirmed matching frames without waiting for an additional silence period.

On Linux the **Personal phrase setup** submenu provides the same Record/Cancel steps and live status, with replacing desktop notifications. All enrollment audio stays local. Use your normal voice, actual microphone, and typical speaking distance. A two- or three-syllable phrase is a sensible starting point; you can personally record "Hey Groq" if you like. Personal enrollment is required, but detection accuracy still depends on your voice, microphone and noise. Test it in your environment. Untrusted playback can cause false activations.

The old eSpeak "Hey Groq" reference had only synthetic regression tests and an observed ambient activation attempt. It had no representative microphone/accent/noise evaluation, so no real-world accuracy rate was established. It is retained only as a test fixture, excluded from production fallback behavior.

For advanced enrollment from three to eight **mono, 16 kHz, PCM16 WAV** files:

```sh
whispertype train "my phrase" one.wav two.wav three.wav
whispertype reload
```

Wake listening uses Rustpotter MFCC/DTW matching, with a quiet-input gate, 240 ms pre-roll and approximately two seconds of hangover. It needs a continuously open microphone while enabled and uses more CPU than shortcut mode. There is no paid wake service, downloaded neural model or Python runtime requirement.

v0.1.5 reuses FFT/matching buffers and suppresses quiet frames before waking
the engine. Shortcut idle keeps the microphone closed; wake inference pauses
during recording in both modes. The [Windows efficiency report](benchmarks/windows-efficiency.md)
compares CPU work, private memory and allocations against v0.1.4, with raw
measurements and reproduction commands.

## Prompt, feedback and reloading

**Transcription prompt...** opens a small editor. Enter concise vocabulary, names, spelling or context hints in the language you dictate; this is not a chat/system instruction. Save applies the value immediately to future uploads and stores it in `config.toml`. Clear the field and Save to disable it. The app bounds it to 1024 UTF-8 bytes; Groq documents a 224-token prompt limit. See [Groq prompting guidance](https://console.groq.com/docs/speech-to-text). Linux uses `zenity` for this optional editor.

On Windows, Ctrl+A/C/X/V/Z select all, copy, cut, paste, and undo. Tab and Shift+Tab move between the field and buttons, Enter inserts a new line in the field, and Escape cancels without saving. Only one prompt editor opens at a time.

Simple Windows feedback uses the standard Windows notification appearance. A newer message dismisses the preceding Whispertype notification; bursts within 200 ms are coalesced to the latest message rather than queued. Windows controls notification duration and quiet-mode behavior. Update checks run silently and show only their result. Phrase setup/testing and prompt editing use compact dark native forms with Segoe UI text and clear buttons. Linux replaces its previous notification by ID with a four-second requested expiry.

There is no Reload menu entry and no background configuration watcher or polling. Tray and prompt-editor changes apply immediately. After manual `config.toml` edits, restart Whispertype or run `whispertype reload`. The CLI reload refuses active dictation/setup and never automatically enables wake listening. Legacy `protect_focus` settings are accepted for upgrades but ignored and removed on the next save.

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
wake_enabled = false            # wake starts off each launch; enable from tray
wake_threshold = 0               # automatic personal calibration; numeric override is optional
wake_phrase = "Personal phrase"         # label; enroll to change the actual acoustic phrase
prompt = ""                      # short vocabulary/context hints
# language = "en"                # omit for automatic language detection
# microphone = "USB"             # substring of input device name
```

Commands: `run`, `init`, `press`, `release`, `start`, `stop`, `toggle`, `cancel`, `wake-on`, `wake-off`, `enroll`, `wake-test`, `paste`, `reload`, `check-updates`, `startup-toggle`, `prompt-edit`, `enroll-next`, `enroll-cancel`, `quit`, `set-key`, `config`, `train`, `transcribe FILE.wav`. On Windows the GUI-subsystem executable writes CLI output only when handles are redirected. `init` registers sign-in startup on Windows; the installer script additionally starts it immediately.

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
