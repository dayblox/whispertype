// Modified by Whispertype: borrow input and reuse normalization workspaces.
use std::{cell::RefCell, collections::HashMap};

use crate::{
    mfcc::MfccComparator, wakewords::WakewordDetector, RustpotterDetection, ScoreMode, WakewordRef,
};

pub(crate) struct WakewordComparator {
    name: String,
    avg_features: Option<Vec<Vec<f32>>>,
    samples_features: HashMap<String, Vec<Vec<f32>>>,
    threshold: Option<f32>,
    avg_threshold: Option<f32>,
    rms_level: f32,
    // state
    score_mode: ScoreMode,
    mfcc_comparator: MfccComparator,
    scratch: RefCell<Scratch>,
}

#[derive(Default)]
struct Scratch {
    windows: Vec<Normalized>,
    scores: Vec<f32>,
}
struct Normalized {
    len: usize,
    frames: Vec<Vec<f32>>,
    sums: Vec<f32>,
    valid: bool,
}
impl Scratch {
    fn window(&mut self, input: &[Vec<f32>], length: usize) -> &[Vec<f32>] {
        let len = input.len().min(length);
        let position = self
            .windows
            .iter()
            .position(|entry| entry.len == len)
            .unwrap_or_else(|| {
                self.windows.push(Normalized {
                    len,
                    frames: vec![vec![0.; input[0].len()]; len],
                    sums: vec![0.; input[0].len()],
                    valid: false,
                });
                self.windows.len() - 1
            });
        let entry = &mut self.windows[position];
        if !entry.valid {
            entry.sums.fill(0.);
            // Preserve upstream summation/subtraction order exactly.
            for (row, source) in entry.frames.iter_mut().zip(input) {
                for ((target, sum), value) in row.iter_mut().zip(&mut entry.sums).zip(source) {
                    *target = *value;
                    *sum += *value;
                }
            }
            for row in &mut entry.frames {
                for (value, sum) in row.iter_mut().zip(&entry.sums) {
                    *value -= *sum / len as f32;
                }
            }
            entry.valid = true;
        }
        &entry.frames
    }
}

impl WakewordComparator {
    fn score_frame(&self, frame_features: &[Vec<f32>], template: &[Vec<f32>]) -> f32 {
        self.mfcc_comparator.compare(template, frame_features)
    }
    fn get_percentile(&self, sorted_values: &[f32], percentile: f32) -> f32 {
        let n = sorted_values.len();
        let index = percentile / 100.0 * (n - 1) as f32;
        let index_floor = index.floor();
        if index_floor == index {
            sorted_values[index as usize]
        } else {
            let i = index_floor as usize;
            let d = index - index_floor;
            sorted_values[i] * (1.0 - d) + sorted_values[i + 1] * d
        }
    }
    pub fn new(
        wakeword: &WakewordRef,
        mfcc_comparator: MfccComparator,
        score_mode: ScoreMode,
    ) -> Self {
        WakewordComparator {
            name: wakeword.name.clone(),
            avg_features: wakeword.avg_features.clone(),
            samples_features: wakeword.samples_features.clone(),
            threshold: wakeword.threshold,
            avg_threshold: wakeword.avg_threshold,
            rms_level: wakeword.rms_level,
            score_mode,
            scratch: RefCell::new(Scratch::default()),
            mfcc_comparator,
        }
    }
}

impl WakewordDetector for WakewordComparator {
    fn get_mfcc_frame_size(&self) -> usize {
        self.samples_features
            .values()
            .map(Vec::len)
            .max()
            .unwrap_or(usize::MIN)
    }

    fn run_detection(
        &self,
        mfcc_frame: &[Vec<f32>],
        avg_threshold: f32,
        threshold: f32,
    ) -> Option<RustpotterDetection> {
        let avg_threshold = self.avg_threshold.unwrap_or(avg_threshold);
        let mut scratch = self.scratch.borrow_mut();
        for entry in &mut scratch.windows {
            entry.valid = false;
        }
        let mut avg_score = 0.;
        if let Some(wakeword_samples_avg_features) =
            self.avg_features.as_ref().filter(|_| avg_threshold != 0.)
        {
            // discard detections against the wakeword averaged features
            let normalized = scratch.window(mfcc_frame, wakeword_samples_avg_features.len());
            avg_score = self.score_frame(normalized, wakeword_samples_avg_features);
            if avg_score < avg_threshold {
                return None;
            }
        }
        let threshold = self.threshold.unwrap_or(threshold);
        if matches!(self.score_mode, ScoreMode::Max) {
            scratch.scores.clear();
            let mut best = f32::NEG_INFINITY;
            for template in self.samples_features.values() {
                let normalized = scratch.window(mfcc_frame, template.len());
                let score = self.score_frame(normalized, template);
                scratch.scores.push(score);
                if score.total_cmp(&best).is_gt() {
                    best = score;
                }
            }
            if best.partial_cmp(&threshold) != Some(std::cmp::Ordering::Greater) {
                return None;
            }
            let scores = self
                .samples_features
                .keys()
                .zip(&scratch.scores)
                .map(|(name, score)| (name.clone(), *score))
                .collect();
            return Some(RustpotterDetection {
                name: self.name.clone(),
                avg_score,
                score: best,
                scores,
                counter: usize::MIN,
                gain: f32::NAN,
            });
        }
        let scores = self.samples_features.iter().fold(
            HashMap::new(),
            |mut acc: HashMap<String, f32>, (name, wakeword_sample_features)| {
                let frame_features_normalized =
                    scratch.window(mfcc_frame, wakeword_sample_features.len());
                acc.insert(
                    name.to_string(),
                    self.score_frame(frame_features_normalized, wakeword_sample_features),
                );
                acc
            },
        );
        let mut sorted_scores = scores.values().copied().collect::<Vec<f32>>();
        let score = match self.score_mode {
            ScoreMode::Average => sorted_scores.iter().sum::<f32>() / sorted_scores.len() as f32,
            ScoreMode::Max => {
                sorted_scores.sort_by(|a, b| b.total_cmp(a));
                sorted_scores[0]
            }
            ScoreMode::Median | ScoreMode::P50 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 50.)
            }
            ScoreMode::P25 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 25.)
            }
            ScoreMode::P75 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 75.)
            }
            ScoreMode::P80 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 80.)
            }
            ScoreMode::P90 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 90.)
            }
            ScoreMode::P95 => {
                sorted_scores.sort_by(|a, b| a.total_cmp(b));
                self.get_percentile(&sorted_scores, 95.)
            }
        };
        if score > threshold {
            Some(RustpotterDetection {
                name: self.name.clone(),
                avg_score,
                score,
                scores,
                counter: usize::MIN, // added by the detector
                gain: f32::NAN,      // added by the detector
            })
        } else {
            None
        }
    }

    fn get_rms_level(&self) -> f32 {
        self.rms_level
    }

    fn get_mfcc_size(&self) -> u16 {
        self.samples_features.values().next().unwrap()[0].len() as u16
    }

    fn update_config(&mut self, score_ref: f32, band_size: u16, score_mode: ScoreMode) {
        self.score_mode = score_mode;
        self.mfcc_comparator = MfccComparator::new(score_ref, band_size);
    }
}
