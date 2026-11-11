use crate::{
    activation::{Action, Activation, Mode},
    audio::{self, Capture, RATE},
    config::{self, Config, Paths},
    enrollment::{Enrollment, Outcome},
    platform, wake,
};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
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
    Key {
        down: bool,
        at: Instant,
    },
    Audio(audio::Frame),
    Fault(String),
    Result(u64, Transcript),
    Shortcut(String),
    Startup(Result<bool, String>),
    Prompt(Result<Option<String>, String>),
    #[cfg(windows)]
    UpdateReady(crate::updates::Prepared),
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
    pub startup: bool,
    pub enrollment: Option<crate::enrollment::View>,
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
            | "check-updates"
            | "startup-toggle"
            | "enroll-next"
            | "enroll-cancel"
            | "prompt-edit"
            | "wake-test"
            | "_profile-wake-record"
            | "_profile-stats"
            | "quit"
    )
}

struct Recording {
    samples: Vec<i16>,
    started: Instant,
    silence: usize,
    voiced: bool,
    wake: bool,
}
struct Job {
    child: Child,
    id: u64,
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
    startup: bool,
    startup_changing: bool,
    enrollment_view: Option<crate::enrollment::View>,
    test_detector: Option<Box<wake::Probe>>,
    profiling: bool,
    audio_events: u64,
    prompt_editing: bool,
    #[cfg(windows)]
    pending_update: Option<crate::updates::Prepared>,
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
    let mut config = Config::load(&paths)?;
    // Wake listening is opt-in for each session, including upgrades from old profiles.
    config.wake_enabled = false;
    config.save(&paths)?;
    let (sender, receiver) = mpsc::sync_channel(64);
    let startup = platform::startup_enabled().unwrap_or(false);
    let desktop = platform::Desktop::start(&paths, &config, sender.clone(), startup)?;
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
        startup,
        startup_changing: false,
        enrollment_view: None,
        test_detector: None,
        profiling: std::env::var_os("WHISPERTYPE_PROFILE").is_some(),
        audio_events: 0,
        prompt_editing: false,
        #[cfg(windows)]
        pending_update: None,
    };
    app.refresh();
    loop {
        let event = receiver.recv_timeout(
            if app.recording.is_some() || app.enrollment.as_ref().is_some_and(Enrollment::active) {
                Duration::from_millis(200)
            } else {
                Duration::from_secs(86400 * 365)
            },
        );
        let result = match event {
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
            Ok(Event::Prompt(result)) => {
                app.prompt_editing = false;
                match result {
                    Ok(Some(prompt)) => {
                        let mut next = app.config.clone();
                        next.prompt = prompt;
                        match next.save(&app.paths) {
                            Ok(()) => {
                                app.config = next;
                                platform::notify(
                                    "Transcription prompt saved. It will be used for new uploads.",
                                );
                            }
                            Err(error) => app.error(&format!("Prompt not saved: {error:#}")),
                        }
                    }
                    Ok(None) => (),
                    Err(error) => app.error(&format!("Prompt editor: {error}")),
                }
                Ok(())
            }
            #[cfg(windows)]
            Ok(Event::UpdateReady(update)) => {
                app.pending_update = Some(update);
                Ok(())
            }
            Ok(Event::Startup(result)) => {
                app.startup_changing = false;
                match result {
                    Ok(enabled) => {
                        app.startup = enabled;
                        app.refresh();
                    }
                    Err(error) => app.error(&format!("Could not change sign-in startup: {error}")),
                }
                Ok(())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(enrollment) = &mut app.enrollment {
                    let outcome = enrollment.tick(Instant::now());
                    let changed =
                        app.enrollment_view.as_ref() != Some(&enrollment.view(Instant::now()));
                    if outcome.is_some() {
                        app.release_capture();
                    }
                    if changed {
                        app.refresh();
                    }
                }
                if app
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.started.elapsed().as_secs() >= app.config.max_seconds as u64)
                {
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
        #[cfg(windows)]
        if app.pending_update.is_some()
            && app.recording.is_none()
            && app.job.is_none()
            && !app.enrollment.as_ref().is_some_and(Enrollment::active)
            && !app.startup_changing
            && !app.prompt_editing
        {
            if let Some(update) = app.pending_update.take() {
                match update.launch() {
                    Ok(()) => break,
                    Err(error) => app.error(&format!("Could not install update: {error:#}")),
                }
            }
        }
    }
    app.wake = false;
    app.cancel();
    drop(instance);
    Ok(())
}

impl Engine {
    fn refresh(&mut self) {
        let state = if self.enrollment.as_ref().is_some_and(Enrollment::active) {
            "enrolling"
        } else if self.recording.is_some() {
            "recording"
        } else if self.job.is_some() {
            "transcribing"
        } else if self.wake {
            "listening"
        } else {
            "idle"
        };
        if self.profiling && self.state != state {
            eprintln!("Profile state: {state}");
        }
        self.state = state.into();
        self.enrollment_view = self.enrollment.as_ref().map(|e| e.view(Instant::now()));
        self.desktop.update(Snapshot {
            config: self.config.clone(),
            state: self.state.clone(),
            wake: self.wake,
            startup: self.startup,
            enrollment: self.enrollment_view.clone(),
        });
    }
    fn error(&self, message: &str) {
        eprintln!("{message}");
        platform::notify(message);
    }
    fn ensure_capture(&mut self, wake_only: bool) -> Result<()> {
        if self.capture.is_none() {
            self.capture = Some(Capture::start(&self.config, self.sender.clone())?);
        }
        self.capture.as_ref().unwrap().set_wake_delivery(wake_only);
        Ok(())
    }
    fn release_capture(&mut self) {
        self.capture = None;
        if self.wake {
            if let Some(detector) = &mut self.detector {
                detector.reset();
            }
        } else {
            self.detector = None;
        }
        self.test_detector = None;
    }
    fn resume(&mut self) -> Result<()> {
        if self.wake {
            if let Some(detector) = &mut self.detector {
                detector.reset();
            } else {
                self.detector = Some(Box::new(wake::detector(&self.config, &self.paths)?));
            }
            self.ensure_capture(true)?;
        } else {
            self.release_capture();
        }
        self.refresh();
        Ok(())
    }
    fn set_wake(&mut self, enabled: bool) -> Result<()> {
        if enabled && self.enrollment.as_ref().is_some_and(|e| !e.done()) {
            platform::notify("Finish personal phrase setup before enabling wake listening.");
            return Ok(());
        }
        if self.enrollment.as_ref().is_some_and(Enrollment::done) {
            self.enrollment = None;
        }
        if enabled && !self.paths.model.exists() {
            return self.begin_enrollment();
        }
        if enabled {
            if self.detector.is_none() {
                self.detector = Some(Box::new(wake::detector(&self.config, &self.paths)?));
            }
        } else {
            self.detector = None;
        }
        self.wake = enabled;
        self.config.wake_enabled = false;
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
            self.profiling || std::env::var_os("GROQ_API_KEY").is_some() || self.paths.key.exists(),
            "Add your Groq API key first (whispertype set-key)"
        );
        self.ensure_capture(false)?;
        self.recording = Some(Recording {
            samples: Vec::with_capacity(RATE * 2),
            started: Instant::now(),
            silence: 0,
            voiced: false,
            wake,
        });
        self.refresh();
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        // Explicit profiling sessions can capture but can NEVER launch an
        // upload worker, even if a timer or wake event finishes a recording.
        if self.profiling {
            self.recording = None;
            self.activation.reset();
            self.release_capture();
            return self.resume();
        }
        let Some(recording) = self.recording.take() else {
            return Ok(());
        };
        self.activation.reset();
        self.release_capture();
        let Some(range) = audio::speech_range(&recording.samples, self.config.speech_rms) else {
            return self.resume();
        };
        let Recording { samples, .. } = recording;
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
        self.job = Some(Job { child, id });
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
            if let Err(e) = platform::type_text(&result.text) {
                self.error(&format!("Text retained locally. Insertion failed: {e:#}"));
            }
        }
        self.resume()
    }
    fn cancel(&mut self) {
        self.test_detector = None;
        self.recording = None;
        self.enrollment = None;
        self.activation.reset();
        if let Some(mut job) = self.job.take() {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
        if !self.wake
            || self
                .capture
                .as_ref()
                .is_some_and(|c| c.overflow.load(Ordering::Relaxed))
        {
            self.release_capture();
        }
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
            "_profile-wake-record" if self.profiling => return self.start(true),
            "_profile-stats" if self.profiling => {
                eprintln!(
                    "Profile stats: {}",
                    serde_json::json!({
                        "state": self.state, "audio_events": self.audio_events,
                        "recording_samples": self.recording.as_ref().map_or(0, |r| r.samples.len()),
                        "recording_capacity": self.recording.as_ref().map_or(0, |r| r.samples.capacity()),
                        "wake_recording": self.recording.as_ref().is_some_and(|r| r.wake)
                    })
                );
                return Ok(());
            }
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
            "check-updates" => return crate::updates::start(self.sender.clone()),
            "startup-toggle" => {
                if !self.startup_changing {
                    self.startup_changing = true;
                    let next = !self.startup;
                    let sender = self.sender.clone();
                    std::thread::spawn(move || {
                        let result = platform::set_startup(next)
                            .map(|()| next)
                            .map_err(|error| format!("{error:#}"));
                        let _ = sender.send(Event::Startup(result));
                    });
                }
                return Ok(());
            }
            "paste" => {
                return platform::type_text(
                    &std::fs::read_to_string(&self.paths.pending)
                        .context("No previous transcript")?,
                )
            }
            "enroll" => {
                return self.begin_enrollment();
            }
            "enroll-next" => {
                if self.enrollment.as_ref().is_some_and(Enrollment::done) {
                    return self.set_wake(true);
                }
                if self.enrollment.is_some() {
                    self.ensure_capture(false)?;
                    self.enrollment.as_mut().unwrap().arm(Instant::now());
                    self.refresh();
                }
                return Ok(());
            }
            "enroll-cancel" => {
                self.test_detector = None;
                self.enrollment = None;
                self.release_capture();
                self.refresh();
                return Ok(());
            }
            "wake-test" => {
                if self.recording.is_some() || self.job.is_some() {
                    return Ok(());
                }
                if self.enrollment.as_ref().is_some_and(|e| !e.done()) {
                    platform::notify("Finish recording your personal phrase before testing it.");
                    return Ok(());
                }
                let detector = wake::Probe::new(&self.config, &self.paths)?;
                self.wake = false;
                self.release_capture();
                self.test_detector = Some(Box::new(detector));
                let enrollment = self.enrollment.get_or_insert_with(Enrollment::new);
                enrollment.arm_test(Instant::now());
                self.ensure_capture(false)?;
                self.refresh();
                return Ok(());
            }
            "prompt-edit" => {
                if self.prompt_editing {
                    return Ok(());
                }
                self.prompt_editing = true;
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let result = (|| -> Result<Option<String>> {
                        let child = platform::spawn_hidden(
                            Command::new(std::env::current_exe()?)
                                .arg("_edit-prompt")
                                .stdout(Stdio::piped())
                                .stdin(Stdio::null()),
                        )?;
                        let output = child.wait_with_output()?;
                        ensure!(output.status.success(), "Could not open the prompt editor");
                        Ok(serde_json::from_slice(&output.stdout)?)
                    })();
                    let _ =
                        sender.send(Event::Prompt(result.map_err(|error| format!("{error:#}"))));
                });
                return Ok(());
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
                if self.recording.is_some() || self.job.is_some() || self.enrollment.is_some() {
                    platform::notify("Finish dictation or phrase setup before reloading settings.");
                    return Ok(());
                }
                self.startup = platform::startup_enabled().unwrap_or(self.startup);
                let mut next = Config::load(&self.paths)?;
                next.wake_enabled = false;
                platform::set_shortcut(&self.paths, &next)?;
                self.config = next;
                self.config.save(&self.paths)?;
                self.activation.reset();
                self.detector = None;
                self.release_capture();
                platform::notify("Settings reloaded from file.");
                return self.resume();
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
        if self.profiling {
            self.audio_events += 1;
        }
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
            if recording.wake {
                if audio::is_speech(&samples, self.config.speech_rms) {
                    recording.silence = 0;
                    recording.voiced = true;
                } else {
                    recording.silence += samples.len();
                }
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
            if finish && !self.profiling {
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
    fn begin_enrollment(&mut self) -> Result<()> {
        if self.recording.is_some() || self.job.is_some() {
            platform::notify("Finish dictation before setting up your wake phrase.");
            return Ok(());
        }
        self.wake = false;
        self.config.wake_enabled = false;
        self.config.save(&self.paths)?;
        self.release_capture();
        self.enrollment = Some(Enrollment::new());
        self.refresh();
        Ok(())
    }
    fn enroll_audio(&mut self, samples: &audio::Frame) -> Result<()> {
        let now = Instant::now();
        let e = self.enrollment.as_mut().unwrap();
        let before = self.enrollment_view.clone();
        let outcome = if e.testing() {
            let outcome = e.tick(now);
            let recognized = if e.test_listening() {
                self.test_detector.as_mut().is_some_and(|d| {
                    let recognized = d.process(samples);
                    e.probe(d.report());
                    recognized
                })
            } else {
                false
            };
            if recognized {
                e.finish("Phrase recognized. Your microphone and saved phrase work together. Enable listening when you are ready.".into());
                Some(Outcome::TestFinished)
            } else {
                outcome
            }
        } else {
            e.audio(samples, now, self.config.speech_rms)
        };
        let changed = before.as_ref() != Some(&e.view(now));
        if outcome.is_some() {
            self.test_detector = None;
            self.release_capture();
        }
        if matches!(outcome, Some(Outcome::Complete)) {
            let samples = std::mem::take(&mut self.enrollment.as_mut().unwrap().samples);
            let result = wake::train_buffers("Personal phrase", samples, &self.paths.model);
            let saved = result.is_ok();
            let message = match result {
                Ok(()) => {
                    self.config.wake_phrase = "Personal phrase".into();
                    self.config.save(&self.paths)?;
                    "Your personal phrase is saved. Test it with your microphone, or enable wake listening now. Listening starts off each time you launch Whispertype.".to_string()
                }
                Err(error) => {
                    let reason: String = error
                        .to_string()
                        .split('.')
                        .next()
                        .unwrap_or("Try recording again")
                        .chars()
                        .take(85)
                        .collect();
                    format!("Samples were not saved. {reason}.")
                }
            };
            let enrollment = self.enrollment.as_mut().unwrap();
            if saved {
                enrollment.finish(message);
            } else {
                enrollment.restart(message);
            }
        }
        if changed || outcome.is_some() {
            self.refresh();
        }
        Ok(())
    }
}
