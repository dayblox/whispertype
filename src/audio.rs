use crate::{config::Config, engine::Event};
use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::Resampler;
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc,
    },
};

#[cfg(test)]
use std::sync::mpsc;

pub const RATE: usize = 16000;

mod capture;
mod devices;
mod probe;
pub use capture::{Capture, Delivery, DeviceMemory, Input};

/// Exactly one 30 ms detector frame. Inline storage keeps the bounded event
/// channel allocation-free during capture, including when the main thread is busy.
pub const FRAME: usize = RATE * 3 / 100;
pub type Frame = [i16; FRAME];

/// Suppress long quiet stretches only while waiting for a wake phrase.
/// Keep the same pre-roll/hangover as Detector, including its reset frame.
struct FrameDelivery {
    pre_roll: [Frame; 8],
    next: usize,
    buffered: usize,
    hangover: usize,
    gated: bool,
    threshold: f32,
}
impl FrameDelivery {
    fn new(threshold: f32) -> Self {
        Self {
            pre_roll: [[0; FRAME]; 8],
            next: 0,
            buffered: 0,
            hangover: 0,
            gated: false,
            threshold,
        }
    }
    fn process(&mut self, frame: Frame, gated: bool, mut emit: impl FnMut(Frame)) {
        if gated != self.gated {
            self.next = 0;
            self.buffered = 0;
            self.hangover = 0;
            self.gated = gated;
        }
        if !gated {
            emit(frame);
            return;
        }
        if is_speech(&frame, self.threshold) {
            for i in 0..self.buffered {
                emit(self.pre_roll[(self.next + 8 - self.buffered + i) % 8]);
            }
            self.buffered = 0;
            self.hangover = 2000 / 30 + 2;
            emit(frame);
        } else if self.hangover > 0 {
            self.hangover -= 1;
            emit(frame);
        } else {
            self.pre_roll[self.next] = frame;
            self.next = (self.next + 1) % 8;
            self.buffered = (self.buffered + 1).min(8);
        }
    }
}

fn prefer_native(
    device: &cpal::Device,
    default: &cpal::SupportedStreamConfig,
) -> Option<cpal::SupportedStreamConfig> {
    if default.channels() == 1 && default.sample_rate() == RATE as u32 {
        #[cfg(windows)]
        return Some(default.clone());
        #[cfg(target_os = "linux")]
        return Some(*default);
    }
    device
        .supported_input_configs()
        .ok()?
        .filter(|c| {
            c.channels() == 1
                && c.min_sample_rate() <= RATE as u32
                && c.max_sample_rate() >= RATE as u32
                && matches!(
                    c.sample_format(),
                    cpal::SampleFormat::I16 | cpal::SampleFormat::F32 | cpal::SampleFormat::U16
                )
        })
        .max_by_key(|c| match c.sample_format() {
            cpal::SampleFormat::I16 => 3,
            cpal::SampleFormat::F32 => 2,
            _ => 1,
        })
        .map(|c| c.with_sample_rate(RATE as u32))
}

struct FftConverter {
    resampler: rubato::FftFixedIn<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    used: usize,
}
/// Buffers are allocated only at initialization. Driver callbacks may have
/// arbitrary lengths; the converter emits contiguous 480-sample frames.
struct Converter {
    channels: usize,
    fft: Option<FftConverter>,
    frame: Frame,
    used: usize,
}
impl Converter {
    fn new(rate: usize, channels: usize) -> Result<Self> {
        anyhow::ensure!(channels > 0, "Microphone has no channels");
        let fft = if rate == RATE {
            None
        } else {
            let resampler = rubato::FftFixedIn::<f32>::new(rate, RATE, rate * 3 / 100, 1, 1)?;
            let input = resampler.input_buffer_allocate(true);
            let output = resampler.output_buffer_allocate(true);
            Some(FftConverter {
                resampler,
                input,
                output,
                used: 0,
            })
        };
        Ok(Self {
            channels,
            fft,
            frame: [0; FRAME],
            used: 0,
        })
    }
    #[inline]
    fn push(&mut self, value: i16, emit: &mut impl FnMut(Frame)) {
        self.frame[self.used] = value;
        self.used += 1;
        if self.used == FRAME {
            emit(self.frame);
            self.used = 0;
        }
    }
    fn process<T>(&mut self, data: &[T], mut emit: impl FnMut(Frame))
    where
        T: cpal::Sample,
        f32: cpal::FromSample<T>,
        i16: cpal::FromSample<T>,
    {
        if self.fft.is_none() {
            if self.channels == 1 {
                pack_mono(&mut self.frame, &mut self.used, data, &mut emit);
            } else {
                let gain = 1.0 / self.channels as f32;
                for channel_frame in data.chunks_exact(self.channels) {
                    let mono = channel_frame
                        .iter()
                        .map(|s| cpal::Sample::to_sample::<f32>(*s))
                        .sum::<f32>()
                        * gain;
                    self.push(cpal::Sample::to_sample::<i16>(mono), &mut emit);
                }
            }
            return;
        }
        let gain = 1.0 / self.channels as f32;
        let mut data = data;
        while data.len() >= self.channels {
            let fft = self.fft.as_mut().unwrap();
            let count = (fft.input[0].len() - fft.used).min(data.len() / self.channels);
            let target = &mut fft.input[0][fft.used..fft.used + count];
            match self.channels {
                1 => {
                    for (target, sample) in target.iter_mut().zip(data) {
                        *target = cpal::Sample::to_sample::<f32>(*sample);
                    }
                }
                2 => {
                    for (target, frame) in target.iter_mut().zip(data.as_chunks::<2>().0) {
                        *target = (cpal::Sample::to_sample::<f32>(frame[0])
                            + cpal::Sample::to_sample::<f32>(frame[1]))
                            * 0.5;
                    }
                }
                _ => {
                    for (target, frame) in target.iter_mut().zip(data.chunks_exact(self.channels)) {
                        *target = frame
                            .iter()
                            .map(|s| cpal::Sample::to_sample::<f32>(*s))
                            .sum::<f32>()
                            * gain;
                    }
                }
            }
            fft.used += count;
            data = &data[count * self.channels..];
            if fft.used == fft.input[0].len() {
                let (_, count) = fft
                    .resampler
                    .process_into_buffer(&fft.input, &mut fft.output, None)
                    .expect("Preallocated resampler buffers match the initialized configuration");
                fft.used = 0;
                pack_mono::<f32>(
                    &mut self.frame,
                    &mut self.used,
                    &fft.output[0][..count],
                    &mut emit,
                );
            }
        }
    }
}

#[inline]
fn pack_mono<T: cpal::Sample>(
    frame: &mut Frame,
    used: &mut usize,
    mut data: &[T],
    emit: &mut impl FnMut(Frame),
) where
    i16: cpal::FromSample<T>,
{
    while !data.is_empty() {
        let count = (FRAME - *used).min(data.len());
        for (target, source) in frame[*used..*used + count].iter_mut().zip(&data[..count]) {
            *target = cpal::Sample::to_sample::<i16>(*source);
        }
        *used += count;
        data = &data[count..];
        if *used == FRAME {
            emit(*frame);
            *used = 0;
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: SyncSender<Event>,
    overflow: Arc<AtomicBool>,
    mode: Arc<std::sync::atomic::AtomicU8>,
    threshold: f32,
    token: u32,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
    i16: cpal::FromSample<T>,
{
    if std::env::var_os("WHISPERTYPE_TRACE_AUDIO").is_some() {
        eprintln!(
            "Audio capture: {} Hz, {} channel(s), {}",
            config.sample_rate,
            config.channels,
            if config.sample_rate == RATE as u32 {
                "direct"
            } else {
                "FFT resampling"
            }
        );
    }
    let mut converter = Converter::new(config.sample_rate as usize, config.channels as usize)?;
    let mut delivery = FrameDelivery::new(threshold);
    let error_sender = sender.clone();
    let mut health = probe::Health::new();
    let mut epoch = 0;
    #[cfg(target_os = "linux")]
    let config = *config;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            let state = mode.load(Ordering::Relaxed);
            if state & capture::RETIRED != 0 {
                return;
            }
            let now = if state & capture::NORMAL == 0 {
                if state & capture::EPOCH != epoch {
                    health = probe::Health::new();
                    epoch = state & capture::EPOCH;
                }
                let now = std::time::Instant::now();
                health.callback(now);
                Some(now)
            } else {
                None
            };
            converter.process(data, |frame| {
                if let Some(now) = now {
                    let source = if state & capture::CANDIDATE != 0 {
                        probe::Source::Candidate
                    } else {
                        probe::Source::Primary
                    };
                    let signal = health.frame(&frame, now, source);
                    if sender
                        .try_send(Event::Input(Input {
                            frame,
                            token,
                            signal,
                            epoch,
                        }))
                        .is_err()
                    {
                        overflow.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                let emit = |frame| {
                    if sender.try_send(Event::Audio(frame)).is_err() {
                        overflow.store(true, Ordering::Relaxed);
                    }
                };
                if state & capture::WAKE != 0 {
                    delivery.process(frame, true, emit);
                } else {
                    // Keep recording on the original direct-delivery path.
                    // The quiet gate and its larger stack frame are wake-only.
                    delivery.gated = false;
                    emit(frame);
                }
            });
        },
        move |err| {
            let _ = error_sender.try_send(Event::InputFault {
                token,
                message: err.to_string(),
            });
        },
        None,
    )?)
}

/// Compare squared energy instead of taking a square root for every frame.
pub fn is_speech(samples: &[i16], threshold: f32) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy = samples
        .iter()
        .map(|&x| {
            let x = x as f32;
            x * x
        })
        .sum::<f32>();
    energy >= threshold * threshold * 32768.0 * 32768.0 * samples.len() as f32
}

pub fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples
        .iter()
        .map(|&x| (x as f32 / 32768.0).powi(2))
        .sum::<f32>()
        / samples.len() as f32)
        .sqrt()
}

pub fn speech_range(samples: &[i16], threshold: f32) -> Option<std::ops::Range<usize>> {
    let mut first = None;
    let mut last = 0;
    let mut speech = 0;
    for (i, chunk) in samples.chunks(320).enumerate() {
        if is_speech(chunk, threshold) {
            first.get_or_insert(i * 320);
            last = (i * 320 + chunk.len()).min(samples.len());
            speech += chunk.len();
        }
    }
    if speech < RATE * 12 / 100 {
        return None;
    }
    let pad = RATE / 5;
    Some(first?.saturating_sub(pad)..(last + pad).min(samples.len()))
}
pub fn trim_speech(samples: &[i16], threshold: f32) -> Option<&[i16]> {
    Some(&samples[speech_range(samples, threshold)?])
}

/// Grow in bounded steps so a 120-second recording cannot acquire the old
/// 128-second power-of-two capacity (or exceed the configured duration).
pub fn append_recording(buffer: &mut Vec<i16>, samples: &[i16], limit: usize) {
    let count = samples.len().min(limit.saturating_sub(buffer.len()));
    let needed = buffer.len() + count;
    if needed > buffer.capacity() {
        let capacity = needed
            .max(buffer.capacity() + (buffer.capacity() / 2).max(RATE * 2))
            .min(limit);
        buffer.reserve_exact(capacity - buffer.len());
    }
    buffer.extend_from_slice(&samples[..count]);
}

pub fn wav_header(samples: usize) -> [u8; 44] {
    let bytes = (samples * 2) as u32;
    let mut header = [0; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&1u16.to_le_bytes());
    header[24..28].copy_from_slice(&(RATE as u32).to_le_bytes());
    header[28..32].copy_from_slice(&(RATE as u32 * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&bytes.to_le_bytes());
    header
}

pub fn write_wav(mut output: impl std::io::Write, samples: &[i16]) -> Result<()> {
    output.write_all(&wav_header(samples.len()))?;
    #[cfg(target_endian = "little")]
    output.write_all(bytemuck::cast_slice(samples))?;
    #[cfg(target_endian = "big")]
    for sample in samples {
        output.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

pub fn wav(samples: &[i16]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    let mut writer = hound::WavWriter::new(
        &mut cursor,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
pub(crate) fn test_allocations(action: impl FnOnce()) -> (usize, usize) {
    tests::allocations(action)
}
