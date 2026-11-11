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
        "_stop-for-upgrade" => platform::stop_for_upgrade(std::path::Path::new(
            args.get(1).context("Missing installed executable path")?,
        )),
        "run" => engine::run(paths),
        "_worker" => {
            let result = groq::worker(&paths);
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
