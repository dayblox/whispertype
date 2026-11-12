#![cfg_attr(windows, windows_subsystem = "windows")]

mod activation;
mod audio;
mod config;
mod engine;
mod enrollment;
mod groq;
mod platform;
mod updates;
mod wake;

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};

fn main() {
    if let Err(error) = run() {
        eprintln!("whispertype: {error:#}");
        if cfg!(windows) {
            platform::notify(&format!("{error:#}"));
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = config::Paths::new()?;
    match args.first().map(String::as_str).unwrap_or("run") {
        #[cfg(windows)]
        "_wake-diagnostic" => wake::diagnose(
            &paths,
            std::path::Path::new(args.get(1).context("Missing local report path")?),
            args.get(2).map(std::path::Path::new),
        ),
        #[cfg(windows)]
        "_stop-for-upgrade" => platform::stop_for_upgrade(std::path::Path::new(
            args.get(1).context("Missing installed executable path")?,
        )),
        #[cfg(windows)]
        "_apply-update" => {
            let result = updates::apply(
                std::path::Path::new(args.get(1).context("Missing update target")?),
                args.get(2).context("Missing parent process")?.parse()?,
            );
            if let Err(error) = &result {
                // There is no tray in this helper to display a notification.
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let message: Vec<u16> = format!("Could not install the update: {error:#}\nDownload the latest version from github.com/dayblox/whisper/releases/latest.")
                        .encode_utf16().chain(Some(0)).collect();
                    let title: Vec<u16> =
                        "Whispertype update".encode_utf16().chain(Some(0)).collect();
                    MessageBoxW(
                        std::ptr::null_mut(),
                        message.as_ptr(),
                        title.as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            result
        }
        "run" => {
            #[cfg(windows)]
            if args.get(1).is_some_and(|a| a == "--updated-from") {
                if let (Some(stage), Some(helper)) = (args.get(2), args.get(3)) {
                    if let Ok(helper) = helper.parse() {
                        let _ = updates::cleanup(std::path::Path::new(stage), helper);
                    }
                }
            }
            engine::run(paths)
        }
        "_worker" => {
            let result = groq::worker(&paths, args.get(1).is_some_and(|a| a == "--warm"));
            let reply = match result {
                Ok(text) => engine::Transcript { text, error: None },
                Err(err) => engine::Transcript {
                    text: String::new(),
                    error: Some(format!("{err:#}")),
                },
            };
            serde_json::to_writer(std::io::stdout(), &reply)?;
            Ok(())
        }
        "_indicator" => platform::indicator(args.get(1).map(String::as_str).unwrap_or("recording")),
        "_capture-shortcut" => platform::capture_shortcut(),
        "_check-updates" => {
            serde_json::to_writer(std::io::stdout(), &updates::check())?;
            Ok(())
        }
        "_edit-prompt" => {
            let config = config::Config::load(&paths)?;
            serde_json::to_writer(std::io::stdout(), &platform::edit_prompt(&config.prompt)?)?;
            Ok(())
        }
        "init" => {
            let config = config::Config::load(&paths)?;
            config.save(&paths)?;
            platform::install(&paths, &config)?;
            println!("Configuration: {}", paths.config.display());
            Ok(())
        }
        "set-key" => {
            let mut key = String::new();
            std::io::stdin().take(4096).read_to_string(&mut key)?;
            config::validate_key(key.trim())?;
            config::private_write(&paths.key, key.trim().as_bytes())?;
            println!("Groq API key saved.");
            Ok(())
        }
        "train" => {
            anyhow::ensure!(
                args.len() >= 5,
                "Usage: whispertype train NAME SAMPLE1.wav SAMPLE2.wav SAMPLE3.wav [...]"
            );
            wake::train_files(&args[1], &args[2..], &paths.model)?;
            println!("Wake model saved: {}", paths.model.display());
            Ok(())
        }
        "transcribe" => {
            let name = args
                .get(1)
                .context("Usage: whispertype transcribe FILE.wav")?;
            let config = config::Config::load(&paths)?;
            print!(
                "{}",
                groq::transcribe_file(&paths, &config, std::path::Path::new(name))?
            );
            std::io::stdout().flush()?;
            Ok(())
        }
        "config" => {
            println!("{}", paths.config.display());
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("Whispertype — Windows / Linux voice typing\n\nwhispertype init          Install startup and shortcut integration\nwhispertype run           Run tray and dictation service\nwhispertype set-key       Read Groq API key from standard input\nwhispertype transcribe WAV\nwhispertype train NAME SAMPLE1.wav SAMPLE2.wav SAMPLE3.wav\nwhispertype config        Print configuration path\nwhispertype press|release|toggle|start|stop|cancel|wake-on|wake-off|enroll|paste|reload|check-updates|startup-toggle|quit");
            Ok(())
        }
        command if engine::valid_command(command) => platform::send(command),
        unknown => bail!("Unknown command: {unknown}. Use whispertype --help."),
    }
}
