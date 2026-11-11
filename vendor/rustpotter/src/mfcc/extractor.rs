// Modified by Whispertype: cache FFT/DCT plans and reuse extraction buffers.
use std::{f32::consts::PI, sync::Arc};

use rustfft::{num_complex::Complex32, Fft, FftPlanner};

pub struct MfccExtractor {
    num_coefficients: usize,
    pre_emphasis_coefficient: f32,
    samples_per_frame: usize,
    samples_per_shift: usize,
    magnitude_spectrum_size: usize,
    sample_rate: usize,
    // state
    filter_bank: Vec<Vec<f32>>,
    hamming_window: Vec<f32>,
    samples: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    fft_buffer: Vec<Complex32>,
    fft_scratch: Vec<Complex32>,
    magnitude_spectrum: Vec<f32>,
    mel_cepstrum: Vec<f32>,
    dct_weights: Vec<Vec<f32>>,
}

#[cfg(test)]
#[path = "extractor_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_mfcc_matches_upstream_bit_for_bit_across_resets_and_sizes() {
        for frame_size in [400, 480, 512] {
            let mut optimized = MfccExtractor::new(16000, frame_size, 160, 17, 0.97);
            let mut original = reference::MfccExtractor::new(16000, frame_size, 160, 17, 0.97);
            for output_size in [16, 8, 32, 16] {
                optimized.set_out_size(output_size);
                original.set_out_size(output_size);
                for pass in 0..2 {
                    for index in 0..40 {
                        let audio: Vec<f32> = (0..480)
                            .map(|i| {
                                if index < 4 {
                                    0.
                                } else {
                                    ((index * 480 + i + pass * 17) as f32 * 0.137).sin() * 0.4
                                }
                            })
                            .collect();
                        let expected = original.compute(&audio);
                        let actual = optimized.compute(&audio);
                        assert_eq!(actual.len(), expected.len());
                        for (a, b) in actual.iter().flatten().zip(expected.iter().flatten()) {
                            assert_eq!(
                                a.to_bits(),
                                b.to_bits(),
                                "frame={frame_size} size={output_size} part={index}"
                            );
                        }
                    }
                    optimized.reset();
                    original.reset();
                }
            }
        }
    }
}

impl MfccExtractor {
    pub fn new(
        sample_rate: usize,
        samples_per_frame: usize,
        samples_per_shift: usize,
        num_coefficients: u16,
        pre_emphasis_coefficient: f32,
    ) -> MfccExtractor {
        let min_frequency = 0;
        let max_frequency = sample_rate / 2;
        let magnitude_spectrum_size = samples_per_frame / 2;
        let fft = FftPlanner::new().plan_fft_forward(samples_per_frame);
        let fft_scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        MfccExtractor {
            samples: Vec::with_capacity(samples_per_frame + samples_per_shift),
            samples_per_shift,
            samples_per_frame,
            pre_emphasis_coefficient,
            num_coefficients: num_coefficients as usize,
            magnitude_spectrum_size,
            filter_bank: Self::new_mel_filter_bank(
                sample_rate,
                magnitude_spectrum_size,
                num_coefficients as usize,
                min_frequency,
                max_frequency,
            ),
            hamming_window: Self::new_hamming_window(samples_per_frame),
            sample_rate,
            fft,
            fft_buffer: vec![Complex32::default(); samples_per_frame],
            fft_scratch,
            magnitude_spectrum: vec![0.; magnitude_spectrum_size],
            mel_cepstrum: vec![0.; num_coefficients as usize],
            dct_weights: Self::new_dct_weights(num_coefficients as usize),
        }
    }
    pub fn set_out_size(&mut self, out_size: u16) {
        self.num_coefficients = (out_size + 1) as usize; // first coefficient is dropped
        let min_frequency = 0;
        let max_frequency = self.sample_rate / 2;
        self.filter_bank = Self::new_mel_filter_bank(
            self.sample_rate,
            self.magnitude_spectrum_size,
            self.num_coefficients,
            min_frequency,
            max_frequency,
        );
        self.mel_cepstrum.resize(self.num_coefficients, 0.);
        self.dct_weights = Self::new_dct_weights(self.num_coefficients);
        self.reset();
    }
    pub fn compute(&mut self, audio_samples: &[f32]) -> Vec<Vec<f32>> {
        audio_samples
            .chunks_exact(self.samples_per_shift)
            .filter_map(|audio_part| self.process_audio_part(audio_part))
            .collect()
    }
    pub fn reset(&mut self) {
        self.samples.clear();
    }
    fn process_audio_part(&mut self, audio_buffer: &[f32]) -> Option<Vec<f32>> {
        let ready = self.samples.len() >= self.samples_per_frame;
        if ready {
            self.samples.drain(0..audio_buffer.len());
        }
        let mut tmp_sample = 0.;
        self.samples.extend(audio_buffer.iter().map(|current| {
            let previous = tmp_sample;
            tmp_sample = *current;
            tmp_sample - self.pre_emphasis_coefficient * previous
        }));
        ready.then(|| self.extract_mfccs())
    }

    //==================================================================
    // Feature extraction utils
    fn extract_mfccs(&mut self) -> Vec<f32> {
        for (i, value) in self.fft_buffer.iter_mut().enumerate() {
            *value = Complex32 {
                re: self.samples[i] * self.hamming_window[i],
                im: 0.,
            };
        }
        self.fft
            .process_with_scratch(&mut self.fft_buffer, &mut self.fft_scratch);
        for (value, spectrum) in self.magnitude_spectrum.iter_mut().zip(&self.fft_buffer) {
            *value = (spectrum.re * spectrum.re + spectrum.im * spectrum.im).sqrt();
        }
        for (value, filter) in self.mel_cepstrum.iter_mut().zip(&self.filter_bank) {
            let energy: f32 = self
                .magnitude_spectrum
                .iter()
                .zip(filter)
                .map(|(ms, coefficient)| ms * ms * coefficient)
                .sum();
            *value = (energy + f32::MIN_POSITIVE).ln();
        }
        // Keep coefficient accumulation order and drop the same first output.
        self.dct_weights
            .iter()
            .skip(1)
            .map(|weights| {
                2. * self
                    .mel_cepstrum
                    .iter()
                    .zip(weights)
                    .map(|(sample, weight)| sample * weight)
                    .sum::<f32>()
            })
            .collect()
    }
    fn new_hamming_window(samples_per_frame: usize) -> Vec<f32> {
        let ns_minus_1 = samples_per_frame - 1;
        (0..samples_per_frame)
            .map(|s| 0.54 - (0.46 * (2. * PI * (s as f32 / ns_minus_1 as f32)).cos()))
            .collect()
    }
    fn frequency_to_mel(frequency: usize) -> f32 {
        1127. * (1. + (frequency as f32 / 700.0)).ln()
    }
    fn new_dct_weights(size: usize) -> Vec<Vec<f32>> {
        let pi_over_n = PI / size as f32;
        (0..size)
            .map(|k| {
                (0..size)
                    .map(|n| (pi_over_n * (n as f32 + 0.5) * k as f32).cos())
                    .collect()
            })
            .collect()
    }
    fn new_mel_filter_bank(
        sample_rate: usize,
        magnitude_spectrum_size: usize,
        num_coefficients: usize,
        min_frequency: usize,
        max_frequency: usize,
    ) -> Vec<Vec<f32>> {
        let max_mel = Self::frequency_to_mel(max_frequency).floor();
        let min_mel = Self::frequency_to_mel(min_frequency).floor();
        let mut filter_bank = vec![vec![0.; magnitude_spectrum_size]; num_coefficients];
        let centre_indices: Vec<usize> = (0..num_coefficients + 2)
            .map(|i| {
                let f = i as f32 * (max_mel - min_mel) / (num_coefficients + 1) as f32 + min_mel;
                let mut tmp = (1_f32 + 1000.0 / 700.0).ln() / 1000.0;
                tmp = ((f * tmp).exp() - 1.) / (sample_rate as f32 / 2.);
                (0.5 + 700. * magnitude_spectrum_size as f32 * tmp).floor() as usize
            })
            .collect();
        for i in 0..num_coefficients {
            let filter_begin_index = centre_indices[i];
            let filter_center_index = centre_indices[i + 1];
            let filter_end_index = centre_indices[i + 2];
            let triangle_range_up = filter_center_index - filter_begin_index;
            let triangle_range_down = filter_end_index - filter_center_index;
            // upward slope
            for (k, value) in filter_bank[i]
                .iter_mut()
                .enumerate()
                .take(filter_center_index)
                .skip(filter_begin_index)
            {
                *value = (k - filter_begin_index) as f32 / triangle_range_up as f32;
            }
            // downwards slope
            for (k, value) in filter_bank[i]
                .iter_mut()
                .enumerate()
                .take(filter_end_index)
                .skip(filter_center_index)
            {
                *value = (filter_end_index - k) as f32 / triangle_range_down as f32;
            }
        }
        filter_bank
    }
}
