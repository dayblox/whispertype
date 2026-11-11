use crate::audio::{self, Frame, FRAME, RATE};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub text: String,
    pub action: String,
    pub done: bool,
    pub enabled: bool,
}

enum Phase {
    Ready,
    Countdown(Instant),
    Listening(Instant),
    Done(String),
}
pub enum Outcome {
    Retry,
    Saved,
    Complete,
    TestFinished,
}

pub struct Enrollment {
    pub samples: HashMap<String, Vec<u8>>,
    phase: Phase,
    current: Vec<i16>,
    pre_roll: [Frame; 8],
    next: usize,
    buffered: usize,
    silence: usize,
    voiced_samples: usize,
    hint: String,
    testing: bool,
    probe: Option<crate::wake::ProbeReport>,
}
impl Enrollment {
    pub fn new() -> Self {
        Self { samples: HashMap::new(), phase: Phase::Ready, current: Vec::new(),
            pre_roll: [[0; FRAME]; 8], next: 0, buffered: 0, silence: 0,
            voiced_samples: 0, testing: false, probe: None, hint: "Choose a short, distinctive phrase. Record it three times in your normal voice. Audio stays on this device.".into() }
    }
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Countdown(_) | Phase::Listening(_))
    }
    pub fn done(&self) -> bool {
        matches!(self.phase, Phase::Done(_))
    }
    pub fn arm(&mut self, now: Instant) {
        if !matches!(self.phase, Phase::Ready) {
            return;
        }
        self.phase = Phase::Countdown(now + Duration::from_secs(2));
        self.current = Vec::with_capacity(RATE * 4);
        self.next = 0;
        self.buffered = 0;
        self.silence = 0;
        self.voiced_samples = 0;
    }
    pub fn finish(&mut self, message: String) {
        self.testing = false;
        self.phase = Phase::Done(message);
    }
    pub fn restart(&mut self, message: String) {
        *self = Self::new();
        self.hint = message;
    }
    pub fn arm_test(&mut self, now: Instant) {
        self.phase = Phase::Ready;
        self.arm(now);
        self.testing = true;
        self.probe = None;
    }
    pub fn probe(&mut self, report: &crate::wake::ProbeReport) {
        // Limit live status updates to about four per second while testing.
        if report.frames.is_multiple_of(8) {
            self.probe = Some(report.clone());
        }
    }
    pub fn testing(&self) -> bool {
        self.testing
    }
    pub fn test_listening(&self) -> bool {
        self.testing && matches!(self.phase, Phase::Listening(_))
    }
    pub fn view(&self, now: Instant) -> View {
        let n = self.samples.len() + 1;
        let label = if self.testing {
            "Test your phrase".into()
        } else {
            format!("Sample {n} of 3")
        };
        let text = match &self.phase {
            Phase::Ready => format!(
                "{}\n\nSample {n} of 3. Click Record when you are ready.",
                self.hint
            ),
            Phase::Countdown(until) => format!(
                "{label} — get ready… {}",
                until
                    .saturating_duration_since(now)
                    .as_millis()
                    .div_ceil(1000)
                    .max(1)
            ),
            Phase::Listening(_) if self.testing && self.probe.is_some() => {
                let p = self.probe.as_ref().unwrap();
                format!("Say your phrase naturally. Nothing is uploaded.\nMicrophone: {} · peak level {:.4}\nMatch score: {:.3} · target {:.3}",
                    if p.speech_frames > 0 { "audio heard" } else { "too quiet / muted" }, p.peak_rms, p.best_score, p.target_score)
            }
            Phase::Listening(_) if self.voiced_samples == 0 => {
                format!(
                    "{label} — say your phrase now.{}",
                    if self.testing {
                        " This only tests recognition; nothing is sent to Groq."
                    } else {
                        ""
                    }
                )
            }
            Phase::Listening(_) => {
                format!("Sample {n} of 3 — recording. Finish the phrase, then stay quiet.")
            }
            Phase::Done(text) => text.clone(),
        };
        View {
            title: match &self.phase {
                Phase::Ready => "Record your personal phrase".into(),
                Phase::Countdown(_) => "Get ready".into(),
                Phase::Listening(_) => label,
                Phase::Done(text) => text
                    .split_once(". ")
                    .map_or(text.as_str(), |(title, _)| title)
                    .into(),
            },
            text: if self.done() {
                text.split_once(". ")
                    .map_or(text.clone(), |(_, body)| body.into())
            } else {
                text
            },
            action: if self.done() {
                "Enable listening".into()
            } else {
                if self.testing {
                    "Testing phrase…".into()
                } else {
                    format!("Record sample {n} / 3")
                }
            },
            done: self.done(),
            enabled: !self.active(),
        }
    }
    fn retry(&mut self, hint: &str) -> Outcome {
        self.current.clear();
        self.phase = Phase::Ready;
        self.hint = hint.into();
        Outcome::Retry
    }
    pub fn audio(&mut self, frame: &Frame, now: Instant, threshold: f32) -> Option<Outcome> {
        if let Some(outcome) = self.tick(now) {
            return Some(outcome);
        }
        let Phase::Listening(_) = self.phase else {
            return None;
        };
        let speech = audio::is_speech(frame, threshold);
        if speech {
            if self.voiced_samples == 0 {
                for i in 0..self.buffered {
                    let at = (self.next + 8 - self.buffered + i) % 8;
                    self.current.extend_from_slice(&self.pre_roll[at]);
                }
            }
            self.voiced_samples += FRAME;
            self.silence = 0;
        } else {
            self.silence += FRAME;
        }
        if self.voiced_samples == 0 {
            self.pre_roll[self.next] = *frame;
            self.next = (self.next + 1) % 8;
            self.buffered = (self.buffered + 1).min(8);
            return None;
        }
        self.current.extend_from_slice(frame);
        if self.current.len().saturating_sub(self.silence) > RATE * 3 {
            return Some(
                self.retry("That phrase was too long. Use a short phrase under three seconds."),
            );
        }
        if self.silence < RATE * 7 / 10 {
            return None;
        }
        if self.voiced_samples < RATE / 5 {
            return Some(
                self.retry(
                    "That sample was too short. Say the whole phrase clearly and try again.",
                ),
            );
        }
        let Some(trimmed) = audio::trim_speech(&self.current, threshold) else {
            return Some(
                self.retry(
                    "That sample was too quiet. Move closer to the microphone and try again.",
                ),
            );
        };
        if !(RATE / 5..=RATE * 3).contains(&trimmed.len()) {
            return Some(
                self.retry("Keep each phrase between 0.2 and 3 seconds. Try this sample again."),
            );
        }
        let Ok(wav) = audio::wav(trimmed) else {
            return Some(self.retry("Could not encode the sample. Please retry."));
        };
        self.samples
            .insert(format!("{}.wav", self.samples.len()), wav);
        self.current.clear();
        self.phase = Phase::Ready;
        self.hint = format!(
            "Sample {} saved. Repeat the same phrase in the same voice.",
            self.samples.len()
        );
        Some(if self.samples.len() == 3 {
            Outcome::Complete
        } else {
            Outcome::Saved
        })
    }
    pub fn tick(&mut self, now: Instant) -> Option<Outcome> {
        if let Phase::Countdown(until) = self.phase {
            if now >= until {
                self.phase = Phase::Listening(now);
            }
        }
        if let Phase::Listening(started) = self.phase {
            if self.testing && now.duration_since(started) >= Duration::from_secs(8) {
                let message = match &self.probe {
                    Some(p) if p.speech_frames == 0 => format!("Microphone is too quiet. No sound reached the voice gate (peak level {:.4}). Check the selected input, microphone gain, or mute switch.", p.peak_rms),
                    Some(p) => format!("Voice heard, phrase not recognized. Best match {:.3}, target {:.3}, confirmed matching frames {} / 3. Try the same speaking distance; the score helps diagnose sensitivity instead of guessing.", p.best_score, p.target_score, p.partial_frames),
                    None => "No microphone audio received. Check the input device and microphone permissions, then retry.".into(),
                };
                self.finish(message);
                return Some(Outcome::TestFinished);
            }
            if self.voiced_samples == 0 && now.duration_since(started) >= Duration::from_secs(10) {
                return Some(self.retry(
                    "No speech detected. Check your microphone and try this sample again.",
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrase_test_counts_down_expires_and_can_be_retried_without_samples() {
        let now = Instant::now();
        let mut e = Enrollment::new();
        e.finish("Your personal phrase is saved. Test it before enabling.".into());
        assert_eq!(e.view(now).action, "Enable listening");
        e.arm_test(now);
        assert!(e.active() && e.testing());
        assert!(!e.test_listening());
        e.tick(now + Duration::from_secs(2));
        assert!(e.test_listening());
        assert!(matches!(
            e.tick(now + Duration::from_secs(10)),
            Some(Outcome::TestFinished)
        ));
        assert!(e.done() && !e.testing() && !e.active());
        assert!(e.samples.is_empty());
        assert_eq!(e.view(now).title, "No microphone audio received");
        e.arm_test(now);
        e.finish("Phrase recognized. Enable listening when ready.".into());
        assert!(e.done() && !e.active());
        assert_eq!(e.view(now).title, "Phrase recognized");
    }
    #[test]
    fn short_and_long_samples_retry_and_countdown_advances_without_audio() {
        let now = Instant::now();
        let mut e = Enrollment::new();
        e.arm(now);
        assert!(e.view(now).text.ends_with('2'));
        e.tick(now + Duration::from_secs(2));
        assert!(e
            .view(now + Duration::from_secs(2))
            .text
            .contains("say your phrase"));
        e.audio(&[9000; FRAME], now + Duration::from_secs(3), 0.006);
        let mut retry = false;
        for _ in 0..25 {
            retry |= matches!(
                e.audio(&[0; FRAME], now + Duration::from_secs(4), 0.006),
                Some(Outcome::Retry)
            );
        }
        assert!(retry);
        assert!(e.samples.is_empty());
        e.arm(now);
        let mut retry = false;
        for _ in 0..110 {
            retry |= matches!(
                e.audio(&[9000; FRAME], now + Duration::from_secs(3), 0.006),
                Some(Outcome::Retry)
            );
        }
        assert!(retry);
        assert!(e.samples.is_empty());
        assert!(!e.active());
    }
    #[test]
    fn guided_enrollment_waits_for_each_click_and_retries_silence() {
        let mut e = Enrollment::new();
        let now = Instant::now();
        assert!(!e.active());
        assert!(e.audio(&[9000; FRAME], now, 0.006).is_none());
        e.arm(now);
        assert!(e.active());
        assert!(e.audio(&[9000; FRAME], now, 0.006).is_none());
        e.audio(&[0; FRAME], now + Duration::from_secs(2), 0.006);
        assert!(matches!(
            e.audio(&[0; FRAME], now + Duration::from_secs(12), 0.006),
            Some(Outcome::Retry)
        ));
        assert!(e.samples.is_empty());
        assert!(!e.active());
        for n in 1..=3 {
            e.arm(now);
            for _ in 0..15 {
                e.audio(&[9000; FRAME], now + Duration::from_secs(3), 0.006);
            }
            let mut result = None;
            for _ in 0..25 {
                if let Some(outcome) = e.audio(&[0; FRAME], now + Duration::from_secs(4), 0.006) {
                    result = Some(outcome);
                }
            }
            assert_eq!(e.samples.len(), n);
            assert!(!e.active());
            assert!(matches!(result, Some(Outcome::Saved | Outcome::Complete)));
        }
    }
}
