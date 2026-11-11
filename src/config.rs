use crate::activation::Mode;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub key: PathBuf,
    pub state: PathBuf,
    pub model: PathBuf,
    pub lock: PathBuf,
    pub pending: PathBuf,
}
impl Paths {
    pub fn new() -> Result<Self> {
        let root = match std::env::var_os("WHISPERTYPE_HOME") {
            Some(root) => PathBuf::from(root),
            None => directories::BaseDirs::new()
                .context("No user configuration directory")?
                .config_dir()
                .join("whispertype"),
        };
        Self::at(root)
    }
    pub fn at(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            config: root.join("config.toml"),
            key: root.join("groq.key"),
            state: root.join("usage.json"),
            model: root.join("wake.rpw"),
            lock: root.join("instance.lock"),
            pending: root.join("pending.txt"),
            root,
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub shortcut: String,
    pub mode: Mode,
    pub hold_ms: u64,
    pub language: Option<String>,
    pub prompt: String,
    pub max_seconds: u32,
    pub silence_ms: u32,
    pub speech_rms: f32,
    pub microphone: Option<String>,
    pub tray: bool,
    pub indicator: bool,
    pub wake_enabled: bool,
    pub wake_threshold: f32,
    pub wake_phrase: String,
    // Accepted only for compatibility with old configuration files; never used.
    #[serde(skip_serializing)]
    pub protect_focus: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            shortcut: "CTRL + SPACE".into(),
            mode: Mode::Auto,
            hold_ms: 250,
            language: None,
            prompt: String::new(),
            max_seconds: 120,
            silence_ms: 1500,
            speech_rms: 0.006,
            microphone: None,
            tray: true,
            indicator: true,
            wake_enabled: false,
            wake_threshold: 0.65,
            wake_phrase: "Personal phrase".into(),
            protect_focus: false,
        }
    }
}
impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let config: Self = if paths.config.exists() {
            toml::from_str(&fs::read_to_string(&paths.config)?)?
        } else {
            Self::default()
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        Shortcut::parse(&self.shortcut)?;
        ensure!(
            (100..=2000).contains(&self.hold_ms),
            "hold_ms must be 100..2000"
        );
        ensure!(
            (2..=600).contains(&self.max_seconds),
            "max_seconds must be 2..600"
        );
        ensure!(
            (400..=10000).contains(&self.silence_ms),
            "silence_ms must be 400..10000"
        );
        ensure!(
            self.speech_rms.is_finite() && (0.0001..=0.2).contains(&self.speech_rms),
            "speech_rms must be 0.0001..0.2"
        );
        ensure!(
            self.wake_threshold.is_finite() && (0.1..=1.0).contains(&self.wake_threshold),
            "wake_threshold must be 0.1..1.0"
        );
        if let Some(lang) = &self.language {
            ensure!(
                lang.len() == 2 && lang.bytes().all(|x| x.is_ascii_lowercase()),
                "language must be an ISO 639-1 code, e.g. en or fr"
            );
        }
        ensure!(
            self.prompt.len() <= 1024,
            "prompt must be at most 1024 bytes"
        );
        Ok(())
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        self.validate()?;
        private_write(&paths.config, toml::to_string_pretty(self)?.as_bytes())
    }
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = path.with_extension("new");
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // MoveFileEx provides atomic replacement on Windows, including existing destinations.
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            if windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0x1 | 0x8,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    #[cfg(not(windows))]
    fs::rename(temp, path)?;
    Ok(())
}
pub fn validate_key(key: &str) -> Result<()> {
    ensure!(
        key.starts_with("gsk_")
            && key.len() >= 20
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "Expected a Groq API key beginning with gsk_"
    );
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
    pub key: String,
}
impl Shortcut {
    pub fn parse(value: &str) -> Result<Self> {
        let mut result = Self {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
            key: String::new(),
        };
        for part in value.split('+').map(|x| x.trim().to_uppercase()) {
            match part.as_str() {
                "CTRL" | "CONTROL" => result.ctrl = true,
                "ALT" => result.alt = true,
                "SHIFT" => result.shift = true,
                "SUPER" | "WIN" => result.super_key = true,
                key => {
                    ensure!(
                        result.key.is_empty(),
                        "Shortcut must contain exactly one non-modifier key"
                    );
                    ensure!(
                        key == "SPACE"
                            || key.len() == 1 && key.bytes().all(|b| b.is_ascii_alphanumeric())
                            || key
                                .strip_prefix('F')
                                .and_then(|k| k.parse::<u8>().ok())
                                .is_some_and(|n| (1..=24).contains(&n)),
                        "Supported keys: Space, A-Z, 0-9, F1-F24"
                    );
                    result.key = key.to_string();
                }
            }
        }
        ensure!(!result.key.is_empty(), "Shortcut needs a key");
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_rejects_bad_values() {
        let mut c = Config::default();
        assert!(c.validate().is_ok());
        c.max_seconds = 10000;
        assert!(c.validate().is_err());
        assert!(Shortcut::parse("CTRL + SPACE").unwrap().ctrl);
        assert!(Shortcut::parse("CTRL + A + B").is_err());
        assert!(Shortcut::parse("CTRL + ';touch /tmp/test'").is_err());
    }
}
