use super::{
    devices,
    probe::{Choice, Signal, Source},
    *,
};
use std::sync::atomic::{AtomicU32, AtomicU8};

pub(super) const WAKE: u8 = 1;
pub(super) const NORMAL: u8 = 2;
pub(super) const RETIRED: u8 = 4;
pub(super) const EPOCH: u8 = 8;
pub(super) const CANDIDATE: u8 = 16;
static TOKEN: AtomicU32 = AtomicU32::new(1);

pub struct Input {
    pub frame: Frame,
    pub token: u32,
    pub signal: Signal,
    pub epoch: u8,
}

/// Only stable identifiers survive release_capture. No stream/host is retained.
#[derive(Default)]
pub struct DeviceMemory {
    default: Option<String>,
    dead: Vec<String>,
}
impl DeviceMemory {
    fn observe_default(&mut self, id: Option<String>) {
        if self.default != id {
            self.dead = Vec::new();
            self.default = id;
        }
    }
}

struct Stream {
    _stream: NativeStream,
    mode: Arc<AtomicU8>,
    token: u32,
    id: String,
}
#[cfg(not(test))]
type NativeStream = cpal::Stream;
#[cfg(test)]
enum NativeStream {
    Real { _stream: cpal::Stream },
    Synthetic,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.mode.store(RETIRED, Ordering::Relaxed);
    }
}
impl Stream {
    fn rearm(&self, wake: bool) {
        // The engine owns all mode writes; callbacks only load it.
        let epoch = (self.mode.load(Ordering::Relaxed) ^ EPOCH) & EPOCH;
        self.mode
            .store(epoch | if wake { WAKE } else { 0 }, Ordering::Relaxed);
    }
}

struct Probe {
    devices: std::vec::IntoIter<cpal::Device>,
    tried: Vec<String>,
    extra: Option<Stream>,
    samples: Vec<i16>,
}

pub struct Capture {
    primary: Option<Stream>,
    primary_id: String,
    probe: Option<Probe>,
    choice: Choice,
    wake: bool,
    exhausted: bool,
    pub overflow: Arc<AtomicBool>,
}

pub enum Delivery {
    Ignore,
    PendingPrimary,
    Primary,
    ChosenPrimary,
    Candidate(Vec<i16>),
}

impl Capture {
    pub fn remembered(
        config: &Config,
        sender: SyncSender<Event>,
        memory: &mut DeviceMemory,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let default = if config.microphone.is_none() || !memory.dead.is_empty() {
            host.default_input_device()
        } else {
            None
        };
        if !memory.dead.is_empty() {
            memory.observe_default(devices::default_id(default.as_ref(), true));
        }
        let device = if let Some(name) = &config.microphone {
            host.input_devices()?
                .find(|d| devices::name(d).is_ok_and(|n| n.contains(name)))
                .context("Configured microphone not found")?
        } else {
            default.context("No default microphone")?
        };
        let primary_id = devices::identity(&device, memory.default.as_deref())?;
        let overflow = Arc::new(AtomicBool::new(false));
        let primary = open(
            &device,
            config,
            sender.clone(),
            overflow.clone(),
            primary_id.clone(),
            Source::Primary,
        )?;
        let remembered = memory.dead.contains(&primary_id);
        let mut capture = Self {
            primary: Some(primary),
            primary_id,
            probe: None,
            choice: Choice::default(),
            wake: false,
            exhausted: false,
            overflow,
        };
        if remembered {
            capture.next(config, sender);
        }
        Ok(capture)
    }
    pub fn set_wake_delivery(&mut self, enabled: bool) {
        if self.wake && !enabled && self.choice.chosen.is_some() {
            // A reused wake stream can have gone silent since it was opened.
            // Re-arm classification for the new recording without reopening
            // the working microphone or discarding its callback data.
            self.choice = Choice::default();
            self.exhausted = false;
            self.primary_id = self.primary.as_ref().unwrap().id.clone();
            self.primary.as_ref().unwrap().rearm(false);
        }
        self.wake = enabled;
        if let Some(stream) = &self.primary {
            if enabled {
                stream.mode.fetch_or(WAKE, Ordering::Relaxed);
            } else {
                stream.mode.fetch_and(!WAKE, Ordering::Relaxed);
            }
        }
    }
    pub fn prepare_recording(&mut self, config: &Config, sender: SyncSender<Event>) {
        if self.wake
            && self.choice.chosen.is_none()
            && self.exhausted
            && self.probe.as_ref().is_none_or(|p| p.extra.is_none())
        {
            self.probe = None;
            self.exhausted = false;
            self.next(config, sender);
        }
    }
    fn next(&mut self, config: &Config, sender: SyncSender<Event>) {
        if self.exhausted {
            return;
        }
        if self.probe.is_none() {
            let devices = match devices::inputs(&self.primary_id) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Microphone enumeration: {e}");
                    self.exhausted = true;
                    return;
                }
            };
            self.probe = Some(Probe {
                devices,
                tried: vec![self.primary_id.clone()],
                extra: None,
                samples: Vec::new(),
            });
        }
        let probe = self.probe.as_mut().unwrap();
        for device in probe.devices.by_ref() {
            let Ok(id) = devices::identity(&device, None) else {
                continue;
            };
            if probe.tried.contains(&id) || !devices::candidate(&device, &id) {
                continue;
            }
            match open(
                &device,
                config,
                sender.clone(),
                self.overflow.clone(),
                id.clone(),
                Source::Candidate,
            ) {
                Ok(stream) => {
                    probe.tried.push(id);
                    // Only retire the current candidate after its replacement
                    // has opened successfully. Reuse the bounded PCM storage.
                    probe.samples.clear();
                    probe.extra = Some(stream);
                    return;
                }
                Err(e) => eprintln!("Microphone candidate: {e:#}"),
            }
        }
        self.exhausted = true;
        eprintln!("No further microphone candidate opened; retaining the current inputs.");
    }
    pub fn input(
        &mut self,
        input: &Input,
        config: &Config,
        sender: SyncSender<Event>,
        memory: &mut DeviceMemory,
        recording: bool,
    ) -> Delivery {
        let stream = self
            .primary
            .as_ref()
            .filter(|s| s.token == input.token)
            .or_else(|| {
                self.probe
                    .as_ref()
                    .and_then(|p| p.extra.as_ref())
                    .filter(|s| s.token == input.token)
            });
        if stream.is_none_or(|s| s.mode.load(Ordering::Relaxed) & EPOCH != input.epoch) {
            return Delivery::Ignore;
        }
        let source = if self
            .primary
            .as_ref()
            .is_some_and(|s| s.token == input.token)
        {
            Source::Primary
        } else if self
            .probe
            .as_ref()
            .and_then(|p| p.extra.as_ref())
            .is_some_and(|s| s.token == input.token)
        {
            Source::Candidate
        } else {
            return Delivery::Ignore;
        };
        if self.choice.chosen.is_some() {
            // A driver callback can enqueue several frames before the engine
            // commits. Keep every already-queued frame from the winner.
            return if source == Source::Primary {
                Delivery::Primary
            } else {
                Delivery::Ignore
            };
        }
        if let Some(chosen) = self.choice.observe(source, input.signal) {
            let prefix = if chosen == Source::Candidate {
                let probe = self.probe.as_mut().unwrap();
                self.primary = None;
                self.primary = probe.extra.take();
                std::mem::take(&mut probe.samples)
            } else {
                Vec::new()
            };
            let winner = &self.primary.as_ref().unwrap().id;
            memory.dead.retain(|id| id != winner);
            if memory.dead.is_empty() {
                memory.default = None;
                memory.dead = Vec::new();
            }
            self.probe = None;
            let mode = NORMAL
                | if self.wake { WAKE } else { 0 }
                | (self.primary.as_ref().unwrap().mode.load(Ordering::Relaxed) & EPOCH);
            self.primary
                .as_ref()
                .unwrap()
                .mode
                .store(mode, Ordering::Relaxed);
            return if chosen == Source::Primary {
                Delivery::ChosenPrimary
            } else {
                Delivery::Candidate(prefix)
            };
        }
        if source == Source::Primary {
            if input.signal == Signal::Dead {
                if !memory.dead.contains(&self.primary_id) {
                    let default = cpal::default_host().default_input_device();
                    let default_id = devices::default_id(default.as_ref(), true);
                    if let Some(id) =
                        devices::resolve_alias(&self.primary_id, default_id.as_deref())
                    {
                        self.primary_id = id;
                    }
                    memory.observe_default(default_id);
                    if !memory.dead.contains(&self.primary_id) {
                        memory.dead.push(self.primary_id.clone());
                    }
                    self.primary.as_mut().unwrap().id = self.primary_id.clone();
                }
                if self.probe.is_none() {
                    self.next(config, sender);
                }
            }
            Delivery::PendingPrimary
        } else {
            if recording {
                super::append_recording(
                    &mut self.probe.as_mut().unwrap().samples,
                    &input.frame,
                    config.max_seconds as usize * RATE,
                );
            }
            if input.signal == Signal::Dead {
                // Candidate silence can be DSP warm-up, so never cache it as
                // dead. If no replacement opens, keep buffering this stream.
                self.next(config, sender);
            }
            Delivery::Ignore
        }
    }
    pub fn finish(&mut self, samples: &mut Vec<i16>) {
        if self.choice.chosen.is_none() {
            if let Some(probe) = &mut self.probe {
                // Exhausting the device list leaves the last candidate and
                // its buffer intact, including frames received after grace.
                if super::probe::prefer_candidate(samples, &probe.samples) {
                    std::mem::swap(samples, &mut probe.samples);
                }
            }
            eprintln!("No microphone delivered a live frame before recording finished.");
        }
    }
    pub fn selected(&self) -> bool {
        self.choice.chosen.is_some()
    }
    pub fn fault(&mut self, token: u32, message: &str, config: &Config, sender: SyncSender<Event>) {
        eprintln!("Microphone: {message}");
        if self.primary.as_ref().is_some_and(|s| s.token == token) {
            self.choice = Choice::default();
            self.primary.as_ref().unwrap().rearm(self.wake);
            self.exhausted = false;
            self.next(config, sender);
            return;
        }
        if self
            .probe
            .as_ref()
            .and_then(|p| p.extra.as_ref())
            .is_some_and(|s| s.token == token)
        {
            self.next(config, sender);
        }
    }
}

fn open(
    device: &cpal::Device,
    config: &Config,
    sender: SyncSender<Event>,
    overflow: Arc<AtomicBool>,
    id: String,
    source: Source,
) -> Result<Stream> {
    let default = device.default_input_config()?;
    let preferred = super::prefer_native(device, &default);
    let token = TOKEN.fetch_add(1, Ordering::Relaxed);
    let mode = Arc::new(AtomicU8::new(if source == Source::Candidate {
        CANDIDATE
    } else {
        0
    }));
    let build = |supported: cpal::SupportedStreamConfig| -> Result<cpal::Stream> {
        let format = supported.sample_format();
        let stream_config: cpal::StreamConfig = supported.into();
        macro_rules! stream {
            ($ty:ty) => {
                super::build::<$ty>(
                    device,
                    &stream_config,
                    sender.clone(),
                    overflow.clone(),
                    mode.clone(),
                    config.speech_rms * 0.5,
                    token,
                )
            };
        }
        match format {
            cpal::SampleFormat::F32 => stream!(f32),
            cpal::SampleFormat::I16 => stream!(i16),
            cpal::SampleFormat::U16 => stream!(u16),
            cpal::SampleFormat::I8 => stream!(i8),
            cpal::SampleFormat::U8 => stream!(u8),
            cpal::SampleFormat::I32 => stream!(i32),
            cpal::SampleFormat::U32 => stream!(u32),
            cpal::SampleFormat::I64 => stream!(i64),
            cpal::SampleFormat::U64 => stream!(u64),
            cpal::SampleFormat::F64 => stream!(f64),
            cpal::SampleFormat::I24 => stream!(cpal::I24),
            cpal::SampleFormat::U24 => stream!(cpal::U24),
            other => bail!("Microphone sample format {other} is unsupported"),
        }
    };
    let stream = if let Some(preferred) = preferred {
        build(preferred).or_else(|_| build(default))?
    } else {
        build(default)?
    };
    stream.play()?;
    Ok(Stream {
        #[cfg(not(test))]
        _stream: stream,
        #[cfg(test)]
        _stream: NativeStream::Real { _stream: stream },
        mode,
        token,
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (
        Capture,
        Arc<AtomicU8>,
        Arc<AtomicU8>,
        DeviceMemory,
        SyncSender<Event>,
    ) {
        let primary = Arc::new(AtomicU8::new(0));
        let extra = Arc::new(AtomicU8::new(CANDIDATE));
        let stream = |token, mode: Arc<AtomicU8>| Stream {
            _stream: NativeStream::Synthetic,
            mode,
            token,
            id: token.to_string(),
        };
        let capture = Capture {
            primary: Some(stream(1, primary.clone())),
            primary_id: "1".into(),
            probe: Some(Probe {
                devices: Vec::new().into_iter(),
                tried: vec!["1".into(), "2".into()],
                extra: Some(stream(2, extra.clone())),
                samples: Vec::new(),
            }),
            choice: Choice::default(),
            wake: false,
            exhausted: false,
            overflow: Arc::new(AtomicBool::new(false)),
        };
        let memory = DeviceMemory {
            default: Some("default".into()),
            dead: vec!["1".into()],
        };
        let (sender, _) = std::sync::mpsc::sync_channel(8);
        (capture, primary, extra, memory, sender)
    }
    #[test]
    fn live_candidate_preserves_prefix_closes_primary_and_keeps_queued_frames() {
        let (mut capture, primary, extra, mut memory, sender) = setup();
        let config = Config::default();
        memory.dead.push("2".into());
        let pending = Input {
            frame: [2; FRAME],
            token: 2,
            signal: Signal::Pending,
            epoch: 0,
        };
        assert!(matches!(
            capture.input(&pending, &config, sender.clone(), &mut memory, true),
            Delivery::Ignore
        ));
        let alive = Input {
            frame: [50; FRAME],
            token: 2,
            signal: Signal::Alive,
            epoch: 0,
        };
        let Delivery::Candidate(prefix) =
            capture.input(&alive, &config, sender.clone(), &mut memory, true)
        else {
            panic!("Candidate was not chosen");
        };
        assert_eq!(prefix, [2; FRAME]);
        assert_eq!(primary.load(Ordering::Relaxed), RETIRED);
        assert_eq!(extra.load(Ordering::Relaxed), NORMAL);
        assert!(capture.probe.is_none());
        assert_eq!(memory.dead, ["1"]);
        assert!(matches!(
            capture.input(&alive, &config, sender.clone(), &mut memory, true),
            Delivery::Primary
        ));
        let stale = Input {
            frame: [300; FRAME],
            token: 1,
            signal: Signal::Alive,
            epoch: 0,
        };
        assert!(matches!(
            capture.input(&stale, &config, sender, &mut memory, true),
            Delivery::Ignore
        ));
    }
    #[test]
    fn recovered_primary_wins_closes_candidate_and_forgets_dead_memory() {
        let (mut capture, primary, extra, mut memory, sender) = setup();
        let config = Config::default();
        let alive = Input {
            frame: [3; FRAME],
            token: 1,
            signal: Signal::Alive,
            epoch: 0,
        };
        assert!(matches!(
            capture.input(&alive, &config, sender, &mut memory, true),
            Delivery::ChosenPrimary
        ));
        assert_eq!(primary.load(Ordering::Relaxed), NORMAL);
        assert_eq!(extra.load(Ordering::Relaxed), RETIRED);
        assert!(memory.dead.is_empty() && memory.default.is_none());
    }
    #[test]
    fn last_silent_candidate_stays_open_buffers_and_is_not_remembered_dead() {
        let (mut capture, primary, extra, mut memory, sender) = setup();
        let config = Config::default();
        let dead = Input {
            frame: [2; FRAME],
            token: 2,
            signal: Signal::Dead,
            epoch: 0,
        };
        capture.input(&dead, &config, sender.clone(), &mut memory, true);
        assert!(capture.exhausted);
        assert_eq!(extra.load(Ordering::Relaxed), CANDIDATE);
        assert_eq!(primary.load(Ordering::Relaxed), 0);
        assert_eq!(memory.dead, ["1"]);
        capture.input(&dead, &config, sender, &mut memory, true);
        let mut samples = vec![0; FRAME];
        capture.finish(&mut samples);
        assert_eq!(samples, [2; FRAME * 2]);
        assert_eq!(extra.load(Ordering::Relaxed), CANDIDATE);
        drop(capture);
        assert_eq!(extra.load(Ordering::Relaxed), RETIRED);
        assert_eq!(primary.load(Ordering::Relaxed), RETIRED);
    }
    #[test]
    fn candidate_with_1100_ms_dsp_warmup_wins_and_keeps_its_silent_prefix() {
        use std::time::{Duration, Instant};

        let (mut capture, primary, extra, mut memory, sender) = setup();
        let config = Config::default();
        let first_callback = Instant::now();
        let mut health = super::super::probe::Health::new();
        health.callback(first_callback);
        let dead = Input {
            frame: [1; FRAME],
            token: 1,
            signal: Signal::Dead,
            epoch: 0,
        };
        assert!(matches!(
            capture.input(&dead, &config, sender.clone(), &mut memory, true),
            Delivery::PendingPrimary
        ));
        for n in 0..37 {
            let frame = [0; FRAME];
            let signal = health.frame(
                &frame,
                first_callback + Duration::from_millis(n * 30),
                Source::Candidate,
            );
            assert_eq!(signal, Signal::Pending);
            capture.input(
                &Input {
                    frame,
                    token: 2,
                    signal,
                    epoch: 0,
                },
                &config,
                sender.clone(),
                &mut memory,
                true,
            );
            assert!(!capture.exhausted);
            assert_eq!(extra.load(Ordering::Relaxed), CANDIDATE);
        }
        let frame = [50; FRAME];
        let alive = Input {
            frame,
            token: 2,
            signal: health.frame(
                &frame,
                first_callback + Duration::from_millis(1100),
                Source::Candidate,
            ),
            epoch: 0,
        };
        let Delivery::Candidate(prefix) =
            capture.input(&alive, &config, sender.clone(), &mut memory, true)
        else {
            panic!("Warmed-up candidate was not chosen");
        };
        assert_eq!(prefix, vec![0; FRAME * 37]);
        assert_eq!(primary.load(Ordering::Relaxed), RETIRED);
        assert_eq!(extra.load(Ordering::Relaxed), NORMAL);
        assert_eq!(memory.dead, ["1"]);
        assert!(matches!(
            capture.input(&alive, &config, sender, &mut memory, true),
            Delivery::Primary
        ));
    }
    #[test]
    fn last_candidate_can_become_alive_after_exhaustion_and_recording_start() {
        let (mut capture, primary, extra, mut memory, sender) = setup();
        let config = Config::default();
        capture.wake = true;
        capture.input(
            &Input {
                frame: [0; FRAME],
                token: 2,
                signal: Signal::Dead,
                epoch: 0,
            },
            &config,
            sender.clone(),
            &mut memory,
            false,
        );
        assert!(capture.exhausted);
        capture.prepare_recording(&config, sender.clone());
        capture.set_wake_delivery(false);
        assert_eq!(extra.load(Ordering::Relaxed), CANDIDATE);
        assert!(matches!(
            capture.input(
                &Input {
                    frame: [50; FRAME],
                    token: 2,
                    signal: Signal::Alive,
                    epoch: 0,
                },
                &config,
                sender,
                &mut memory,
                true,
            ),
            Delivery::Candidate(_)
        ));
        assert_eq!(primary.load(Ordering::Relaxed), RETIRED);
        assert_eq!(extra.load(Ordering::Relaxed), NORMAL);
    }
    #[test]
    fn candidate_buffer_is_bounded_and_default_changes_clear_the_cache() {
        let (mut capture, _, _, mut memory, sender) = setup();
        let config = Config {
            max_seconds: 2,
            ..Config::default()
        };
        for _ in 0..1000 {
            capture.input(
                &Input {
                    frame: [1; FRAME],
                    token: 2,
                    signal: Signal::Pending,
                    epoch: 0,
                },
                &config,
                sender.clone(),
                &mut memory,
                true,
            );
        }
        let samples = &capture.probe.as_ref().unwrap().samples;
        assert_eq!(samples.len(), RATE * 2);
        assert!(samples.capacity() <= RATE * 2);
        memory.observe_default(Some("default".into()));
        assert!(!memory.dead.is_empty());
        memory.observe_default(Some("another-default".into()));
        assert!(memory.dead.is_empty());
    }
    #[test]
    fn recording_rearms_a_reused_wake_stream_without_reopening_it() {
        let (mut capture, primary, _, mut memory, sender) = setup();
        capture.wake = true;
        capture.input(
            &Input {
                frame: [100; FRAME],
                token: 1,
                signal: Signal::Alive,
                epoch: 0,
            },
            &Config::default(),
            sender,
            &mut memory,
            false,
        );
        assert_eq!(primary.load(Ordering::Relaxed), NORMAL | WAKE);
        capture.set_wake_delivery(false);
        assert_eq!(primary.load(Ordering::Relaxed), EPOCH);
        assert!(capture.choice.chosen.is_none() && capture.primary.is_some());
        let stale = Input {
            frame: [100; FRAME],
            token: 1,
            signal: Signal::Alive,
            epoch: 0,
        };
        let config = Config::default();
        let (sender, _) = std::sync::mpsc::sync_channel(8);
        assert!(matches!(
            capture.input(&stale, &config, sender.clone(), &mut memory, true),
            Delivery::Ignore
        ));
        let current = Input {
            epoch: EPOCH,
            ..stale
        };
        assert!(matches!(
            capture.input(&current, &config, sender, &mut memory, true),
            Delivery::ChosenPrimary
        ));
    }
}
