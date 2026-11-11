mod overlay;
use crate::{
    activation::Mode,
    config::{Config, Paths, Shortcut},
    engine::{Event, Snapshot},
};
use anyhow::{ensure, Context, Result};
use ksni::{menu::*, TrayMethods};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::SyncSender,
    time::Duration,
};

fn socket_path(paths: &Paths) -> PathBuf {
    paths.root.join("control.sock")
}
pub fn send(command: &str) -> Result<()> {
    let paths = Paths::new()?;
    let mut stream =
        UnixStream::connect(socket_path(&paths)).context("Whispertype is not running")?;
    stream.write_all(command.as_bytes())?;
    Ok(())
}

pub struct Desktop {
    tray: Option<ksni::Handle<Tray>>,
    socket: PathBuf,
    indicator: Option<Child>,
    indicator_state: String,
    enrollment_view: Option<crate::enrollment::View>,
}
impl Desktop {
    pub fn start(
        paths: &Paths,
        config: &Config,
        sender: SyncSender<Event>,
        startup: bool,
    ) -> Result<Self> {
        let socket = socket_path(paths);
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        let events = sender.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut bytes = Vec::new();
                if Read::by_ref(&mut stream)
                    .take(257)
                    .read_to_end(&mut bytes)
                    .is_ok()
                    && bytes.len() <= 256
                {
                    if let Ok(s) = String::from_utf8(bytes) {
                        let event = match s.as_str() {
                            "press" => Event::Key {
                                down: true,
                                at: std::time::Instant::now(),
                            },
                            "release" => Event::Key {
                                down: false,
                                at: std::time::Instant::now(),
                            },
                            _ => Event::Command(s),
                        };
                        let _ = events.send(event);
                    }
                }
            }
        });
        let tray = if config.tray {
            match async_io::block_on(
                (Tray {
                    snapshot: Snapshot {
                        config: config.clone(),
                        state: "idle".into(),
                        wake: false,
                        startup,
                        enrollment: None,
                    },
                    sender,
                })
                .assume_sni_available(true)
                .spawn(),
            ) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("Tray unavailable: {e}");
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            tray,
            socket,
            indicator: None,
            indicator_state: String::new(),
            enrollment_view: None,
        })
    }
    pub fn update(&mut self, snapshot: Snapshot) {
        if snapshot.enrollment != self.enrollment_view {
            self.enrollment_view = snapshot.enrollment.clone();
            if let Some(view) = &self.enrollment_view {
                notify(&view.text);
            }
        }
        let active = snapshot.config.indicator
            && matches!(
                snapshot.state.as_str(),
                "recording" | "transcribing" | "enrolling"
            );
        let desired = if active { snapshot.state.as_str() } else { "" };
        if desired != self.indicator_state {
            self.hide_indicator();
            if active {
                match super::spawn_hidden(
                    Command::new(std::env::current_exe().unwrap())
                        .args(["_indicator", desired])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit()),
                ) {
                    Ok(child) => {
                        self.indicator = Some(child);
                        self.indicator_state = desired.into();
                    }
                    Err(error) => eprintln!("Indicator unavailable: {error}"),
                }
            }
        }
        if let Some(tray) = &self.tray {
            async_io::block_on(tray.update(move |t| t.snapshot = snapshot));
        }
    }
    fn hide_indicator(&mut self) {
        if let Some(mut child) = self.indicator.take() {
            drop(child.stdin.take());
            let _ = child.wait();
        }
        self.indicator_state.clear();
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.hide_indicator();
        let _ = std::fs::remove_file(&self.socket);
    }
}

struct Tray {
    snapshot: Snapshot,
    sender: SyncSender<Event>,
}
impl Tray {
    fn item(&self, label: &str, command: &str) -> ksni::MenuItem<Self> {
        let command = command.to_string();
        StandardItem {
            label: label.into(),
            activate: Box::new(move |this: &mut Self| {
                let _ = this.sender.try_send(Event::Command(command.clone()));
            }),
            ..Default::default()
        }
        .into()
    }
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "whispertype".into()
    }
    fn title(&self) -> String {
        format!("Whispertype · {}", self.snapshot.state)
    }
    fn icon_name(&self) -> String {
        if self.snapshot.state == "recording" {
            "microphone-sensitivity-high-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        }
        .into()
    }
    const MENU_ON_ACTIVATE: bool = true;
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let modes = [Mode::Auto, Mode::Hold, Mode::Toggle];
        let mut items = vec![
            StandardItem {
                label: format!("Whispertype · {}", self.snapshot.state),
                enabled: false,
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: format!("Shortcut: {}", self.snapshot.config.shortcut),
                submenu: vec![
                    self.item("Record shortcut…", "shortcut-capture"),
                    self.item("Ctrl + Space", "shortcut:CTRL + SPACE"),
                    self.item("Ctrl + Alt + Space", "shortcut:CTRL + ALT + SPACE"),
                    self.item("Super + Alt + Space", "shortcut:SUPER + ALT + SPACE"),
                    self.item("F9", "shortcut:F9"),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: format!("Activation: {}", self.snapshot.config.mode.label()),
                submenu: vec![RadioGroup {
                    selected: modes
                        .iter()
                        .position(|m| *m == self.snapshot.config.mode)
                        .unwrap_or(0),
                    options: modes
                        .iter()
                        .map(|s| RadioItem {
                            label: s.label().to_string(),
                            ..Default::default()
                        })
                        .collect(),
                    select: Box::new(|this: &mut Self, n| {
                        let _ = this.sender.try_send(Event::Command(format!(
                            "mode:{}",
                            ["auto", "hold", "toggle"][n]
                        )));
                    }),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: format!("Wake-word listening ({})", self.snapshot.config.wake_phrase),
                checked: self.snapshot.wake,
                activate: Box::new(|this: &mut Self| {
                    let _ = this.sender.try_send(Event::Command(
                        if this.snapshot.wake {
                            "wake-off"
                        } else {
                            "wake-on"
                        }
                        .into(),
                    ));
                }),
                ..Default::default()
            }
            .into(),
            self.item("Record personal wake phrase…", "enroll"),
            self.item("Test personal wake phrase…", "wake-test"),
            ksni::MenuItem::Separator,
            self.item("Transcription prompt…", "prompt-edit"),
            CheckmarkItem {
                label: "Launch at startup".into(),
                checked: self.snapshot.startup,
                activate: Box::new(|this: &mut Self| {
                    let _ = this
                        .sender
                        .try_send(Event::Command("startup-toggle".into()));
                }),
                ..Default::default()
            }
            .into(),
            self.item("Check for updates", "check-updates"),
            self.item("Quit", "quit"),
        ];
        if let Some(view) = &self.snapshot.enrollment {
            let mut action = self.item(&view.action, "enroll-next");
            if let ksni::MenuItem::Standard(item) = &mut action {
                item.enabled = view.enabled;
            }
            items.insert(
                1,
                SubMenu {
                    label: "Personal phrase setup".into(),
                    submenu: vec![
                        StandardItem {
                            label: format!("{}: {}", view.title, view.text.replace('\n', " ")),
                            enabled: false,
                            ..Default::default()
                        }
                        .into(),
                        action,
                        self.item(
                            if view.done {
                                "Close setup"
                            } else {
                                "Cancel setup"
                            },
                            "enroll-cancel",
                        ),
                    ],
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
}

pub fn notify(message: &str) {
    feedback(message, "4000");
}
pub fn update_notification(message: &str) {
    notify(message);
}
fn feedback(message: &str, expiry: &str) {
    static ID: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
    let Ok(mut id) = ID.lock() else {
        return;
    };
    if let Ok(output) = Command::new("notify-send")
        .args([
            "--app-name=Whispertype",
            "--expire-time",
            expiry,
            "--print-id",
            "--replace-id",
            &id.to_string(),
            "Whispertype",
            message,
        ])
        .stdin(Stdio::null())
        .output()
    {
        if let Ok(value) = String::from_utf8_lossy(&output.stdout).trim().parse() {
            *id = value;
        }
    }
}

pub fn edit_prompt(current: &str) -> Result<Option<String>> {
    let mut child = Command::new("zenity").args(["--text-info", "--editable", "--title=Transcription prompt",
        "--width=560", "--height=320", "--ok-label=Save", "--cancel-label=Cancel"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()
        .context("Install zenity for the prompt editor, or edit prompt in config.toml and reload settings")?;
    child.stdin.take().unwrap().write_all(current.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let prompt = String::from_utf8(output.stdout)?;
    ensure!(
        prompt.len() <= 1024,
        "Keep the prompt under 1024 UTF-8 bytes"
    );
    Ok(Some(prompt))
}

pub fn open_releases() -> Result<()> {
    let status = Command::new("xdg-open")
        .arg(super::RELEASES_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    ensure!(status.success(), "Could not open the release download page");
    Ok(())
}

fn hypr(command: &str) -> Result<String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is missing")?;
    let signature =
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").context("Hyprland environment is missing")?;
    let path = PathBuf::from(runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(command.as_bytes())?;
    let mut data = String::new();
    stream.take(65536).read_to_string(&mut data)?;
    Ok(data)
}
pub fn type_text(text: &str) -> Result<()> {
    let mut child = Command::new("wtype")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("Install wtype for Wayland Unicode typing")?;
    child.stdin.take().unwrap().write_all(text.as_bytes())?;
    ensure!(
        child.wait()?.success(),
        "wtype failed; compositor must support virtual keyboards"
    );
    Ok(())
}
pub fn indicator(state: &str) -> Result<()> {
    overlay::run(state, false)
}
pub fn capture_shortcut() -> Result<()> {
    overlay::run("shortcut", true)
}

fn lua_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}
fn binding_text(config: &Config) -> Result<String> {
    let s = Shortcut::parse(&config.shortcut)?;
    let mods = [
        (s.ctrl, "CTRL"),
        (s.alt, "ALT"),
        (s.shift, "SHIFT"),
        (s.super_key, "SUPER"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .chain(Some(s.key.as_str()))
    .collect::<Vec<_>>()
    .join(" + ");
    let exe = std::env::current_exe()?;
    let shell = format!("'{}'", exe.to_string_lossy().replace('\'', "'\\''"));
    Ok(format!("-- Generated by Whispertype. Change shortcuts through the tray.\no.bind_hold({}, \"Voice typing\", {}, \"Finish voice typing\", {})\n",lua_quote(&mods),lua_quote(&format!("{shell} press")),lua_quote(&format!("{shell} release"))))
}
pub fn set_shortcut(paths: &Paths, config: &Config) -> Result<()> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let binds: Vec<serde_json::Value> = serde_json::from_str(&hypr("j/binds")?)?;
        let key = Shortcut::parse(&config.shortcut)?;
        let mask = if key.super_key { 64 } else { 0 }
            | if key.shift { 1 } else { 0 }
            | if key.ctrl { 4 } else { 0 }
            | if key.alt { 8 } else { 0 };
        ensure!(
            !binds.iter().any(|b| b["modmask"].as_u64() == Some(mask)
                && b["key"]
                    .as_str()
                    .is_some_and(|k| k.eq_ignore_ascii_case(&key.key))
                && !matches!(
                    b["description"].as_str(),
                    Some("Voice typing" | "Finish voice typing")
                )),
            "That shortcut is already assigned in Hyprland. Your previous shortcut is unchanged."
        );
    }
    crate::config::private_write(
        &paths.root.join("bindings.lua"),
        binding_text(config)?.as_bytes(),
    )?;
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let result = Command::new("hyprctl").arg("reload").output()?;
        ensure!(result.status.success(), "Hyprland reload failed");
        let errors = Command::new("hyprctl").arg("configerrors").output()?;
        ensure!(
            String::from_utf8_lossy(&errors.stdout).trim().is_empty(),
            "Hyprland config errors: {}",
            String::from_utf8_lossy(&errors.stdout)
        );
    }
    Ok(())
}

pub fn install(paths: &Paths, config: &Config) -> Result<()> {
    let home = directories::BaseDirs::new().context("No home directory")?;
    let hypr_bindings = home.config_dir().join("hypr/bindings.lua");
    if hypr_bindings.exists() {
        let mut content = std::fs::read_to_string(&hypr_bindings)?;
        let line = format!(
            "dofile({})",
            lua_quote(&paths.root.join("bindings.lua").to_string_lossy())
        );
        if !content.contains(&line) {
            let old = hypr("j/binds").unwrap_or_default();
            if let Ok(binds) = serde_json::from_str::<Vec<serde_json::Value>>(&old) {
                let key = Shortcut::parse(&config.shortcut)?;
                let mask = if key.super_key { 64 } else { 0 }
                    | if key.shift { 1 } else { 0 }
                    | if key.ctrl { 4 } else { 0 }
                    | if key.alt { 8 } else { 0 };
                ensure!(!binds.iter().any(|b|b["modmask"].as_u64()==Some(mask) && b["key"].as_str().is_some_and(|k|k.eq_ignore_ascii_case(&key.key))),"Shortcut is already bound in Hyprland. Choose another in config.toml before installing.");
            }
            let backup = hypr_bindings.with_extension(format!(
                "lua.whispertype-backup-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            ));
            std::fs::copy(&hypr_bindings, backup)?;
            content.push_str(&format!("\n-- Whispertype voice typing\n{line}\n"));
            // Write the generated source before its loader to avoid an intermediate invalid config.
            crate::config::private_write(
                &paths.root.join("bindings.lua"),
                binding_text(config)?.as_bytes(),
            )?;
            crate::config::private_write(&hypr_bindings, content.as_bytes())?;
        }
    }
    set_shortcut(paths, config)?;
    set_startup(true)?;
    ensure!(
        Command::new("systemctl")
            .args(["--user", "start", "whispertype.service"])
            .status()?
            .success(),
        "Service start failed"
    );
    Ok(())
}

pub fn startup_enabled() -> Result<bool> {
    let result = Command::new("systemctl")
        .args(["--user", "is-enabled", "whispertype.service"])
        .stdin(Stdio::null())
        .output()?;
    Ok(result.status.success() && String::from_utf8_lossy(&result.stdout).trim() == "enabled")
}

pub fn set_startup(enabled: bool) -> Result<()> {
    if !enabled {
        ensure!(
            Command::new("systemctl")
                .args(["--user", "disable", "whispertype.service"])
                .status()?
                .success(),
            "Could not disable sign-in startup"
        );
        return Ok(());
    }
    let home = directories::BaseDirs::new().context("No home directory")?;
    let units = home.config_dir().join("systemd/user");
    std::fs::create_dir_all(&units)?;
    let exe = std::env::current_exe()?
        .to_string_lossy()
        .replace('%', "%%")
        .replace('"', "\\\"");
    let unit=format!("[Unit]\nDescription=Whispertype voice typing\nAfter=graphical-session.target\nPartOf=graphical-session.target\n\n[Service]\nType=simple\nExecStart=\"{exe}\" run\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=graphical-session.target\n");
    crate::config::private_write(&units.join("whispertype.service"), unit.as_bytes())?;
    ensure!(
        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()?
            .success(),
        "systemd reload failed"
    );
    ensure!(
        Command::new("systemctl")
            .args(["--user", "enable", "whispertype.service"])
            .status()?
            .success(),
        "Service installation failed"
    );
    Ok(())
}
