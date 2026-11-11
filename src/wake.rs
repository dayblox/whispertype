use crate::{
    audio,
    config::{Config, Paths},
};
use anyhow::{ensure, Context, Result};
use rustpotter::{
    Rustpotter, RustpotterConfig, WakewordLoad, WakewordRef, WakewordRefBuildFromBuffers,
    WakewordSave,
};
use std::{collections::HashMap, path::Path};
mod calibration;

#[cfg(test)]
pub(crate) fn reference(config: &Config, paths: &Paths) -> Result<Rustpotter> {
    reference_mode(config, paths, false).map(|(detector, _)| detector)
}
fn reference_mode(config: &Config, paths: &Paths, score_only: bool) -> Result<(Rustpotter, f32)> {
    let data = std::fs::read(&paths.model)
        .context("Record a personal wake phrase before enabling wake listening")?;
    let mut model = WakewordRef::load_from_buffer(&data).map_err(anyhow::Error::msg)?;
    let target = if config.wake_threshold == 0. {
        calibration::threshold(&model)?
    } else {
        config.wake_threshold
    };
    let mut settings = RustpotterConfig::default();
    settings.fmt.sample_format = rustpotter::SampleFormat::I16;
    settings.fmt.endianness = rustpotter::Endianness::Native;
    settings.detector.threshold = target;
    settings.detector.avg_threshold = 0.2;
    settings.detector.min_scores = 3;
    settings.detector.eager = true;
    // The outer energy gate already skips quiet audio. A second VAD can reject
    // short phrases after its baseline resets at the beginning of each burst.
    settings.detector.vad_mode = None;
    if score_only {
        settings.detector.threshold = 0.;
        settings.detector.avg_threshold = 0.;
        settings.detector.min_scores = usize::MAX;
        settings.detector.eager = false;
    }
    let mut detector = Rustpotter::new(&settings).map_err(anyhow::Error::msg)?;
    // Respect app/manual settings instead of an obsolete threshold embedded
    // in an older personal reference file. The user's model is never rewritten.
    model.threshold = None;
    model.avg_threshold = None;
    let data = model.save_to_buffer().map_err(anyhow::Error::msg)?;
    detector
        .add_wakeword_from_buffer("wake", &data)
        .map_err(anyhow::Error::msg)?;
    Ok((detector, target))
}

#[derive(Clone, serde::Serialize)]
pub struct ProbeReport {
    pub frames: usize,
    pub speech_frames: usize,
    pub peak_rms: f32,
    pub best_score: f32,
    pub target_score: f32,
    pub partial_frames: usize,
    pub recognized: bool,
    pub detections: usize,
}
pub struct Probe {
    detector: Detector,
    scorer: Rustpotter,
    report: ProbeReport,
    gate: f32,
}
impl Probe {
    pub fn new(config: &Config, paths: &Paths) -> Result<Self> {
        let detector = detector(config, paths)?;
        let target = detector.match_threshold;
        Ok(Self {
            detector,
            scorer: reference_mode(config, paths, true)?.0,
            report: ProbeReport {
                frames: 0,
                speech_frames: 0,
                peak_rms: 0.,
                best_score: 0.,
                target_score: target,
                partial_frames: 0,
                recognized: false,
                detections: 0,
            },
            gate: config.speech_rms * 0.5,
        })
    }
    pub fn report(&self) -> &ProbeReport {
        &self.report
    }
    pub fn process(&mut self, frame: &audio::Frame) -> bool {
        self.report.frames += 1;
        self.report.peak_rms = self.report.peak_rms.max(audio::rms(frame));
        self.report.speech_frames += audio::is_speech(frame, self.gate) as usize;
        let recognized = self.detector.process(frame);
        self.report.recognized |= recognized;
        self.report.detections += recognized as usize;
        if recognized {
            self.report.partial_frames = self.report.partial_frames.max(3);
        }
        if let Some(partial) = self.detector.inner.get_partial_detection() {
            self.report.partial_frames = self.report.partial_frames.max(partial.counter);
        }
        self.scorer.process_bytes(bytemuck::cast_slice(frame));
        if let Some(partial) = self.scorer.get_partial_detection() {
            self.report.best_score = self.report.best_score.max(partial.score);
        }
        recognized
    }
}

/// Explicit local troubleshooting: save one microphone sample and numeric
/// diagnostics. This path never initializes the engine, hotkeys or Groq.
#[cfg(windows)]
pub fn diagnose(paths: &Paths, report_path: &Path, input: Option<&Path>) -> Result<()> {
    use crate::engine::Event;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let config = Config::load(paths)?;
    let mut probe = Probe::new(&config, paths)?;
    let mut fixed = Probe::new(
        &Config {
            wake_threshold: 0.65,
            ..config.clone()
        },
        paths,
    )?;
    let samples = if let Some(input) = input {
        let mut wav = hound::WavReader::open(input)?;
        ensure!(
            wav.spec().channels == 1
                && wav.spec().sample_rate == audio::RATE as u32
                && wav.spec().bits_per_sample == 16,
            "Expected mono 16 kHz PCM16 audio"
        );
        wav.samples::<i16>().collect::<Result<Vec<_>, _>>()?
    } else {
        if !crate::platform::diagnostic_prompt() {
            return Ok(());
        }
        let (sender, receiver) = mpsc::sync_channel(64);
        let capture = audio::Capture::start(&config, sender)?;
        let until = Instant::now() + Duration::from_secs(10);
        let mut samples = Vec::with_capacity(audio::RATE * 11);
        while Instant::now() < until {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(Event::Audio(frame)) => samples.extend_from_slice(&frame),
                Ok(Event::Fault(error)) => anyhow::bail!(error),
                _ => (),
            }
        }
        drop(capture);
        crate::config::private_write(&report_path.with_extension("wav"), &audio::wav(&samples)?)?;
        samples
    };
    for frame in samples.as_chunks::<{ audio::FRAME }>().0 {
        probe.process(frame);
        fixed.process(frame);
    }
    let model =
        WakewordRef::load_from_file(&paths.model.to_string_lossy()).map_err(anyhow::Error::msg)?;
    let report = serde_json::json!({ "detection": probe.report(), "fixed_065_comparison": fixed.report(), "model": {
        "samples": model.samples_features.values().map(Vec::len).collect::<Vec<_>>(),
        "mfcc_size": model.mfcc_size, "rms": model.rms_level,
        "embedded_threshold": model.threshold,
    }});
    crate::config::private_write(report_path, &serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}

/// Keep a short pre-roll and two seconds of hangover so quiet syllables and
/// delayed detections survive gating. Long quiet periods never run FFT/MFCC/DTW.
pub struct Detector {
    inner: Rustpotter,
    pre_roll: [audio::Frame; 8],
    next: usize,
    buffered: usize,
    hangover: usize,
    active: bool,
    threshold: f32,
    match_threshold: f32,
}
pub fn detector(config: &Config, paths: &Paths) -> Result<Detector> {
    let (inner, match_threshold) = reference_mode(config, paths, false)?;
    ensure!(
        inner.get_samples_per_frame() == audio::FRAME,
        "Unexpected wake detector frame size"
    );
    Ok(Detector {
        inner,
        pre_roll: [[0; audio::FRAME]; 8],
        next: 0,
        buffered: 0,
        hangover: 0,
        active: false,
        threshold: config.speech_rms * 0.5,
        match_threshold,
    })
}
impl Detector {
    pub fn reset(&mut self) {
        self.inner.reset();
        self.next = 0;
        self.buffered = 0;
        self.hangover = 0;
        self.active = false;
    }
    pub fn process(&mut self, frame: &audio::Frame) -> bool {
        if audio::is_speech(frame, self.threshold) {
            self.hangover = 2000 / 30 + 1;
        } else if self.hangover > 0 {
            self.hangover -= 1;
        } else {
            if self.active {
                self.inner.reset();
                self.active = false;
            }
            self.pre_roll[self.next] = *frame;
            self.next = (self.next + 1) % self.pre_roll.len();
            self.buffered = (self.buffered + 1).min(self.pre_roll.len());
            return false;
        }
        if !self.active {
            for i in 0..self.buffered {
                let at =
                    (self.next + self.pre_roll.len() - self.buffered + i) % self.pre_roll.len();
                self.inner
                    .process_bytes(bytemuck::cast_slice(&self.pre_roll[at]));
            }
            self.buffered = 0;
            self.active = true;
        }
        self.inner
            .process_bytes(bytemuck::cast_slice(frame))
            .is_some()
    }
}

pub fn train_files(name: &str, files: &[String], target: &Path) -> Result<()> {
    let mut samples = HashMap::new();
    for (i, file) in files.iter().enumerate() {
        let data = std::fs::read(file)?;
        samples.insert(format!("sample-{i}.wav"), data);
    }
    train_buffers(name, samples, target)
}
pub fn train_buffers(name: &str, samples: HashMap<String, Vec<u8>>, target: &Path) -> Result<()> {
    ensure!(
        (3..=8).contains(&samples.len()),
        "Use three to eight wake-phrase samples"
    );
    for wav in samples.values() {
        let reader = hound::WavReader::new(std::io::Cursor::new(wav))?;
        let spec = reader.spec();
        ensure!(
            spec.channels == 1
                && spec.sample_rate == audio::RATE as u32
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int,
            "Wake samples must be mono 16 kHz PCM16 WAV"
        );
        ensure!(
            reader.duration() >= 3200 && reader.duration() <= 48000,
            "Each wake sample must be 0.2..3 seconds"
        );
    }
    let model = WakewordRef::new_from_sample_buffers(name.to_string(), None, None, samples, 16)
        .map_err(anyhow::Error::msg)?;
    calibration::threshold(&model)?;
    crate::config::private_write(target, &model.save_to_buffer().map_err(anyhow::Error::msg)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personally_trained_reference_recognizes_a_quieter_repeat() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        let config = Config::default();
        let pcm: Vec<i16> = include_bytes!("../assets/test-positive.pcm")
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| i16::from_le_bytes(*v))
            .collect();
        let clip = audio::trim_speech(&pcm, config.speech_rms).unwrap();
        let mut samples = HashMap::new();
        for (i, gain) in [0.85, 1.0, 1.15].iter().enumerate() {
            let scaled: Vec<i16> = clip
                .iter()
                .map(|s| (*s as f32 * gain).clamp(-32768.0, 32767.0) as i16)
                .collect();
            samples.insert(format!("{i}.wav"), audio::wav(&scaled).unwrap());
        }
        train_buffers("Test phrase", samples, &paths.model).unwrap();
        let mut detector = detector(&config, &paths).unwrap();
        let mut input = vec![0i16; audio::RATE];
        input.extend(pcm.iter().map(|s| (*s as f32 * 0.5) as i16));
        input.extend(vec![0i16; audio::RATE * 3]);
        assert!(input
            .as_chunks::<{ audio::FRAME }>()
            .0
            .iter()
            .any(|frame| detector.process(frame)));
        let mut reject = super::detector(&config, &paths).unwrap();
        let mut negative: Vec<i16> = include_bytes!("../assets/test-negative.pcm")
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| i16::from_le_bytes(*v))
            .collect();
        negative.extend(vec![0; audio::RATE * 3]);
        assert!(!negative
            .as_chunks::<{ audio::FRAME }>()
            .0
            .iter()
            .any(|frame| reject.process(frame)));
    }
    #[test]
    fn quiet_gate_rearms_and_preserves_phrase_after_long_silence() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        std::fs::write(&paths.model, include_bytes!("../assets/hey-groq.rpw")).unwrap();
        // This multi-voice synthetic legacy fixture predates personal calibration.
        let config = Config {
            wake_threshold: 0.65,
            ..Config::default()
        };
        let mut detector = detector(&config, &paths).unwrap();
        for _ in 0..1000 {
            assert!(!detector.process(&[0; audio::FRAME]));
        }
        assert!(!detector.active);
        let bytes = include_bytes!("../assets/test-positive.pcm");
        let mut samples = Vec::new();
        samples.extend(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| i16::from_le_bytes(*v)),
        );
        samples.extend(vec![0; audio::RATE * 3]);
        for _ in 0..2 {
            let mut detections = 0;
            for frame in samples.as_chunks::<{ audio::FRAME }>().0 {
                detections += detector.process(frame) as usize;
            }
            assert_eq!(detections, 1);
            assert!(!detector.active);
        }
    }
    #[test]
    fn synthetic_fixture_matches_held_out_synthetic_voice_and_rejects_other_speech() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        let config = Config {
            wake_threshold: 0.65,
            ..Config::default()
        };
        assert!(detector(&config, &paths).is_err());
        std::fs::write(&paths.model, include_bytes!("../assets/hey-groq.rpw")).unwrap();
        for (data, expected) in [
            (
                include_bytes!("../assets/test-positive.pcm").as_slice(),
                true,
            ),
            (
                include_bytes!("../assets/test-negative.pcm").as_slice(),
                false,
            ),
        ] {
            let mut detector = detector(&config, &paths).unwrap();
            let mut samples = vec![0i16; 16000];
            samples.extend(
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| i16::from_le_bytes(*v)),
            );
            samples.extend(vec![0i16; 32000]);
            let mut detected = false;
            for frame in samples.as_chunks::<{ audio::FRAME }>().0 {
                if detector.process(frame) {
                    detected = true;
                }
            }
            assert_eq!(detected, expected, "Unexpected wake detection");
        }
    }
}
