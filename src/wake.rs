use crate::{
    audio,
    config::{Config, Paths},
};
use anyhow::{ensure, Result};
use rustpotter::{
    Rustpotter, RustpotterConfig, WakewordRef, WakewordRefBuildFromBuffers, WakewordSave,
};
use std::{collections::HashMap, path::Path};

pub(crate) fn reference(config: &Config, paths: &Paths) -> Result<Rustpotter> {
    let mut settings = RustpotterConfig::default();
    settings.fmt.sample_format = rustpotter::SampleFormat::I16;
    settings.fmt.endianness = rustpotter::Endianness::Native;
    settings.detector.threshold = config.wake_threshold;
    settings.detector.avg_threshold = 0.2;
    settings.detector.min_scores = 3;
    settings.detector.vad_mode = Some(rustpotter::VADMode::Easy);
    let mut detector = Rustpotter::new(&settings).map_err(anyhow::Error::msg)?;
    let data = if paths.model.exists() {
        std::fs::read(&paths.model)?
    } else {
        include_bytes!("../assets/hey-groq.rpw").to_vec()
    };
    detector
        .add_wakeword_from_buffer("wake", &data)
        .map_err(anyhow::Error::msg)?;
    Ok(detector)
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
}
pub fn detector(config: &Config, paths: &Paths) -> Result<Detector> {
    let inner = reference(config, paths)?;
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
    })
}
impl Detector {
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
    crate::config::private_write(target, &model.save_to_buffer().map_err(anyhow::Error::msg)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_gate_rearms_and_preserves_phrase_after_long_silence() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        let mut detector = detector(&Config::default(), &paths).unwrap();
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
    fn bundled_phrase_matches_held_out_voice_and_rejects_other_speech() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
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
            let mut detector = detector(&Config::default(), &paths).unwrap();
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
