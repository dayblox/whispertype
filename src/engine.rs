use crate::{
    activation::{Action, Activation, Mode},
    audio::{self, Capture, RATE},
    config::{self, Config, Paths},
    platform, wake,
};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Read,
    process::{Child, Command, Stdio},
    sync::{
        atomic::Ordering,
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

// Audio stays inline in the bounded queue to avoid allocations in the driver callback.
#[allow(clippy::large_enum_variant)]
pub enum Event {
    Command(String),
    Key { down: bool, at: Instant },
    Audio(audio::Frame),
    Fault(String),
    Result(u64, Transcript),
    Shortcut(String),
}
#[derive(Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct Snapshot {
    pub config: Config,
    pub state: String,
    pub wake: bool,
}
pub fn valid_command(s: &str) -> bool {
    matches!(
        s,
        "press"
            | "release"
            | "toggle"
            | "start"
            | "stop"
            | "cancel"
            | "wake-on"
            | "wake-off"
            | "enroll"
            | "paste"
            | "reload"
            | "quit"
    )
}

struct Recording {
    samples: Vec<i16>,
    started: Instant,
    silence: usize,
    voiced: bool,
    wake: bool,
    focus: Option<String>,
}
struct Enrollment {
    samples: HashMap<String, Vec<u8>>,
    pause_until: Instant,
    current: Vec<i16>,
    silence: usize,
    voiced: bool,
}
struct Job {
    child: Child,
    id: u64,
    focus: Option<String>,
}

struct Engine {
    paths: Paths,
    config: Config,
    sender: SyncSender<Event>,
    activation: Activation,
    capture: Option<Capture>,
    detector: Option<Box<wake::Detector>>,
    wake: bool,
    recording: Option<Recording>,
    job: Option<Job>,
    generation: u64,
    enrollment: Option<Enrollment>,
    desktop: platform::Desktop,
    state: String,
}

pub fn run(paths: Paths) -> Result<()> {
    let instance = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock)?;
    ensure!(
        instance.try_lock_exclusive().is_ok(),
        "Whispertype is already running"
    );
    let config = Config::load(&paths)?;
    if !paths.config.exists() {
        config.save(&paths)?;
    }
    let (sender, receiver) = mpsc::sync_channel(64);
    let desktop = platform::Desktop::start(&paths, &config, sender.clone())?;
    let wake = config.wake_enabled;
    let mut app = Engine {
        paths,
        config,
        sender,
        activation: Activation::default(),
        capture: None,
        detector: None,
        wake: false,
        recording: None,
        job: None,
        generation: 0,
        enrollment: None,
        desktop,
        state: "idle".into(),
    };
    if wake {
        if let Err(e) = app.set_wake(true) {
            app.error(&format!("{e:#}"));
        }
    }
    app.refresh();
    loop {
        let event = receiver.recv_timeout(if app.recording.is_some() || app.enrollment.is_some() {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(86400 * 365)
        });
        let result =
            match event {
                Ok(Event::Command(command)) if command == "quit" => break,
                Ok(Event::Command(command)) => app.command(&command),
                Ok(Event::Key { down, at }) => {
                    app.command_at(if down { "press" } else { "release" }, at)
                }
                Ok(Event::Audio(samples)) => app.audio(samples),
                Ok(Event::Fault(message)) => {
                    app.wake = false;
                    app.config.wake_enabled = false;
                    app.cancel();
                    app.error(&message);
                    Ok(())
                }
                Ok(Event::Result(id, result)) => app.completed(id, result),
                Ok(Event::Shortcut(shortcut)) => app.change_shortcut(&shortcut),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if app.recording.as_ref().is_some_and(|r| {
                        r.started.elapsed().as_secs() >= app.config.max_seconds as u64
                    }) {
                        app.finish()
                    } else {
                        Ok(())
                    }
                }
                Err(_) => break,
            };
        if let Err(err) = result {
            app.cancel();
            app.error(&format!("{err:#}"));
        }
    }
    app.wake = false;
    app.cancel();
    drop(instance);
    Ok(())
}

impl Engine {
    fn refresh(&mut self) {
        self.state = if self.enrollment.is_some() {
            "enrolling"
        } else if self.recording.is_some() {
            "recording"
        } else if self.job.is_some() {
            "transcribing"
        } else if self.wake {
            "listening"
        } else {
            "idle"
        }
        .into();
        self.desktop.update(Snapshot {
            config: self.config.clone(),
            state: self.state.clone(),
            wake: self.wake,
        });
    }
    fn error(&self, message: &str) {
        eprintln!("{message}");
        platform::notify(message);
    }
    fn ensure_capture(&mut self) -> Result<()> {
        if self.capture.is_none() {
            self.capture = Some(Capture::start(&self.config, self.sender.clone())?);
        }
        Ok(())
    }
    fn release_capture(&mut self) {
        self.capture = None;
        self.detector = None;
    }
    fn resume(&mut self) -> Result<()> {
        if self.wake {
            self.detector = Some(Box::new(wake::detector(&self.config, &self.paths)?));
            self.ensure_capture()?;
        } else {
            self.release_capture();
        }
        self.refresh();
        Ok(())
    }
    fn set_wake(&mut self, enabled: bool) -> Result<()> {
        if enabled {
            let _ = wake::detector(&self.config, &self.paths)?;
        }
        self.wake = enabled;
        self.config.wake_enabled = enabled;
        self.config.save(&self.paths)?;
        if self.job.is_none() && self.recording.is_none() && self.enrollment.is_none() {
            self.resume()?;
        }
        self.refresh();
        Ok(())
    }
    fn start(&mut self, wake: bool) -> Result<()> {
        ensure!(
            self.job.is_none() && self.enrollment.is_none(),
            "Finish or cancel the current operation first"
        );
        if self.recording.is_some() {
            return Ok(());
        }
        ensure!(
            std::env::var_os("GROQ_API_KEY").is_some() || self.paths.key.exists(),
            "Add your Groq API key first (whispertype set-key)"
        );
        let focus = platform::focused_window();
        if self.config.protect_focus {
            ensure!(
                focus.is_some(),
                "Cannot identify the focused window; check desktop integration"
            );
        }
        self.ensure_capture()?;
        self.recording = Some(Recording {
            samples: Vec::with_capacity(RATE * 2),
            started: Instant::now(),
            silence: 0,
            voiced: false,
            wake,
            focus,
        });
        self.refresh();
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        let Some(recording) = self.recording.take() else {
            return Ok(());
        };
        self.activation.reset();
        self.release_capture();
        let Some(range) = audio::speech_range(&recording.samples, self.config.speech_rms) else {
            return self.resume();
        };
        let Recording { samples, focus, .. } = recording;
        let mut child = platform::spawn_hidden(
            Command::new(std::env::current_exe()?)
                .arg("_worker")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        )?;
        let mut input = child.stdin.take().context("Missing worker input")?;
        let output = child.stdout.take().context("Missing worker output")?;
        self.generation += 1;
        let id = self.generation;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Transcript> {
                let write_result = audio::write_wav(&mut input, &samples[range]);
                // Release the recording as soon as the bounded pipe has accepted it.
                drop(samples);
                drop(input);
                let mut data = Vec::new();
                output.take(256_001).read_to_end(&mut data)?;
                ensure!(data.len() <= 256_000, "Worker response too large");
                // A worker can reject a key/header before consuming the whole
                // pipe. Preserve that error instead of hiding it behind EPIPE.
                let result: Transcript = match serde_json::from_slice(&data) {
                    Ok(result) => result,
                    Err(error) => {
                        write_result?;
                        return Err(error.into());
                    }
                };
                if result.error.is_none() {
                    write_result?;
                }
                Ok(result)
            })()
            .unwrap_or_else(|e| Transcript {
                text: String::new(),
                error: Some(format!("Transcription worker: {e:#}")),
            });
            let _ = sender.send(Event::Result(id, result));
        });
        self.job = Some(Job { child, id, focus });
        self.refresh();
        Ok(())
    }
    fn completed(&mut self, id: u64, result: Transcript) -> Result<()> {
        if self.job.as_ref().is_none_or(|j| j.id != id) {
            return Ok(());
        }
        let mut job = self.job.take().unwrap();
        let _ = job.child.wait();
        if let Some(err) = result.error {
            self.error(&err);
        } else if !result.text.is_empty() {
            config::private_write(&self.paths.pending, result.text.as_bytes())?;
            if self.config.protect_focus && job.focus != platform::focused_window() {
                self.error("Focus changed. Text retained locally; select Paste last transcript from the tray.");
            } else if let Err(e) = platform::type_text(&result.text) {
                self.error(&format!("Text retained locally. Insertion failed: {e:#}"));
            }
        }
        self.resume()
    }
    fn cancel(&mut self) {
        self.recording = None;
        self.enrollment = None;
        self.activation.reset();
        if let Some(mut job) = self.job.take() {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
        self.release_capture();
        if let Err(e) = self.resume() {
            self.wake = false;
            self.error(&format!("{e:#}"));
        }
    }
    fn change_shortcut(&mut self, shortcut: &str) -> Result<()> {
        ensure!(
            self.recording.is_none() && self.job.is_none() && self.enrollment.is_none(),
            "Finish or cancel the current operation before changing the shortcut"
        );
        let mut next = self.config.clone();
        next.shortcut = shortcut.into();
        next.validate()?;
        platform::set_shortcut(&self.paths, &next)?;
        next.save(&self.paths)?;
        self.config = next;
        self.activation.reset();
        self.refresh();
        Ok(())
    }
    fn command(&mut self, command: &str) -> Result<()> {
        self.command_at(command, Instant::now())
    }
    fn command_at(&mut self, command: &str, at: Instant) -> Result<()> {
        let action = match command {
            "press" => self
                .activation
                .press(at, self.config.mode, self.recording.is_some()),
            "release" => self.activation.release(
                at,
                self.config.mode,
                Duration::from_millis(self.config.hold_ms),
            ),
            "start" => Action::Start,
            "stop" => Action::Stop,
            "toggle" => {
                if self.recording.is_some() {
                    Action::Stop
                } else {
                    Action::Start
                }
            }
            "cancel" => {
                self.cancel();
                return Ok(());
            }
            "wake-on" => return self.set_wake(true),
            "wake-off" => return self.set_wake(false),
            "paste" => {
                return platform::type_text(
                    &std::fs::read_to_string(&self.paths.pending)
                        .context("No previous transcript")?,
                )
            }
            "enroll" => {
                ensure!(
                    self.recording.is_none() && self.job.is_none(),
                    "Finish dictation first"
                );
                self.ensure_capture()?;
                self.enrollment = Some(Enrollment {
                    samples: HashMap::new(),
                    pause_until: Instant::now() + Duration::from_secs(2),
                    current: Vec::new(),
                    silence: 0,
                    voiced: false,
                });
                platform::notify("After two seconds, say your wake phrase. Repeat it three times, pausing two seconds between repetitions. Use Cancel in the tray to abort.");
                self.refresh();
                return Ok(());
            }
            "wake-default" => {
                let _ = std::fs::remove_file(&self.paths.model);
                self.config.wake_phrase = "Hey Groq".into();
                self.config.save(&self.paths)?;
                return self.resume();
            }
            "shortcut-capture" => {
                platform::notify(
                    "Press a new shortcut (Space, letter, digit, or F key). Escape cancels.",
                );
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let result = platform::spawn_hidden(
                        Command::new(std::env::current_exe().unwrap())
                            .arg("_capture-shortcut")
                            .stdout(Stdio::piped()),
                    )
                    .and_then(|c| c.wait_with_output());
                    if let Ok(output) = result {
                        if output.status.success() {
                            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                            if !s.is_empty() {
                                let _ = sender.send(Event::Shortcut(s));
                            }
                        }
                    }
                });
                return Ok(());
            }
            "reload" => {
                let next = Config::load(&self.paths)?;
                platform::set_shortcut(&self.paths, &next)?;
                self.config = next;
                self.cancel();
                return self.set_wake(self.config.wake_enabled);
            }
            _ if command.starts_with("shortcut:") => return self.change_shortcut(&command[9..]),
            _ if command.starts_with("mode:") => {
                self.config.mode = match &command[5..] {
                    "auto" => Mode::Auto,
                    "hold" => Mode::Hold,
                    "toggle" => Mode::Toggle,
                    _ => anyhow::bail!("Invalid activation mode"),
                };
                self.config.save(&self.paths)?;
                self.activation.reset();
                self.refresh();
                return Ok(());
            }
            _ => return Ok(()),
        };
        match action {
            Action::Start => self.start(false),
            Action::Stop => self.finish(),
            Action::None => Ok(()),
        }
    }
    fn audio(&mut self, samples: audio::Frame) -> Result<()> {
        if self
            .capture
            .as_ref()
            .is_some_and(|c| c.overflow.load(Ordering::Relaxed))
        {
            anyhow::bail!(
                "Audio processing fell behind. Dictation canceled to avoid missing words."
            );
        }
        if self.enrollment.is_some() {
            return self.enroll_audio(&samples);
        }
        if let Some(recording) = &mut self.recording {
            if audio::is_speech(&samples, self.config.speech_rms) {
                recording.silence = 0;
                recording.voiced = true;
            } else {
                recording.silence += samples.len();
            }
            let room = self.config.max_seconds as usize * RATE - recording.samples.len();
            audio::append_recording(
                &mut recording.samples,
                &samples,
                self.config.max_seconds as usize * RATE,
            );
            let finish = room <= samples.len()
                || recording.wake
                    && ((recording.voiced
                        && recording.silence >= RATE * self.config.silence_ms as usize / 1000)
                        || !recording.voiced && recording.started.elapsed().as_secs() >= 5);
            if finish {
                self.finish()?;
            }
        } else if self.wake && self.job.is_none() {
            if let Some(detector) = &mut self.detector {
                if detector.process(&samples) {
                    self.start(true)?;
                }
            }
        }
        Ok(())
    }
    fn enroll_audio(&mut self, samples: &[i16]) -> Result<()> {
        let e = self.enrollment.as_mut().unwrap();
        if Instant::now() < e.pause_until {
            return Ok(());
        }
        let speech = audio::is_speech(samples, self.config.speech_rms);
        if speech {
            e.voiced = true;
            e.silence = 0;
        } else {
            e.silence += samples.len();
        }
        if e.voiced {
            e.current.extend_from_slice(samples);
        }
        if !e.voiced {
            return Ok(());
        }
        if e.current.len() > RATE * 4 {
            anyhow::bail!(
                "Wake phrase too long; use a short phrase and pause between repetitions."
            );
        }
        if e.silence >= RATE * 7 / 10 {
            if let Some(trimmed) = audio::trim_speech(&e.current, self.config.speech_rms) {
                let wav = audio::wav(trimmed)?;
                e.samples.insert(format!("{}.wav", e.samples.len()), wav);
            }
            e.current.clear();
            e.voiced = false;
            e.silence = 0;
            if e.samples.len() == 3 {
                let e = self.enrollment.take().unwrap();
                wake::train_buffers("Personal phrase", e.samples, &self.paths.model)?;
                self.config.wake_phrase = "Personal phrase".into();
                self.config.save(&self.paths)?;
                platform::notify(
                    "Personal wake phrase saved locally. Enable wake-word listening from the tray.",
                );
                return self.resume();
            }
            platform::notify(&format!(
                "Wake sample {}/3 saved. Wait two seconds, then repeat the same phrase.",
                e.samples.len()
            ));
            e.pause_until = Instant::now() + Duration::from_secs(2);
        }
        Ok(())
    }
}
