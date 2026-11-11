use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

#[test]
#[ignore = "Requires a real microphone; captures three seconds locally without storing or uploading audio"]
fn microphone_smoke() {
    let (sender, receiver) = mpsc::sync_channel(64);
    let capture = Capture::start(&Config::default(), sender).unwrap();
    let until = Instant::now() + std::time::Duration::from_secs(3);
    let mut frames = 0;
    while Instant::now() < until {
        match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(Event::Audio(_)) => frames += 1,
            Ok(Event::Fault(error)) => panic!("{error}"),
            _ => (),
        }
    }
    assert!(
        frames >= 30,
        "Expected at least one second of audio, got {frames} frames"
    );
    assert!(!capture.overflow.load(Ordering::Relaxed));
    drop(capture);
    println!("Microphone opened, delivered {frames} PCM frames, and closed without overflow; no audio saved or uploaded.");
}

struct Allocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() && TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACK.load(Ordering::Relaxed) {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        System.dealloc(pointer, layout);
    }
}
// Used only by the explicitly selected, single-threaded benchmark test below.
fn allocations(action: impl FnOnce()) -> (usize, usize) {
    ALLOCS.store(0, Ordering::Relaxed);
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    action();
    TRACK.store(false, Ordering::Relaxed);
    (ALLOCS.load(Ordering::Relaxed), PEAK.load(Ordering::Relaxed))
}

#[test]
fn native_capture_preserves_pcm_across_driver_buffer_boundaries() {
    let samples: Vec<i16> = (0..FRAME * 8).map(|i| i as i16).collect();
    let mut converter = Converter::new(RATE, 1).unwrap();
    assert!(converter.fft.is_none());
    let mut received = Vec::new();
    for input in samples.chunks(137) {
        converter.process(input, |frame| received.extend_from_slice(&frame));
    }
    assert_eq!(received, samples);
    let extremes = [i16::MIN, 0, i16::MAX].repeat(FRAME / 3);
    converter.process(&extremes, |frame| assert_eq!(frame.as_slice(), extremes));
}

#[test]
fn resampling_keeps_voice_band_and_rejects_aliasing() {
    fn tone(rate: usize, frequency: f32) -> Vec<i16> {
        let mut converter = Converter::new(rate, 2).unwrap();
        let mut received = Vec::new();
        let input: Vec<f32> = (0..rate)
            .flat_map(|i| {
                let sample =
                    (std::f32::consts::TAU * frequency * i as f32 / rate as f32).sin() * 0.5;
                [sample, sample]
            })
            .collect();
        for chunk in input.chunks(274) {
            converter.process(chunk, |frame| received.extend_from_slice(&frame));
        }
        assert!(received.len() >= RATE - FRAME * 2 && received.len() <= RATE);
        received
    }
    for rate in [44100, 48000] {
        let speech = tone(rate, 1000.0);
        let high = tone(rate, 12000.0);
        assert!(rms(&speech[FRAME * 2..]) > 0.3);
        assert!(rms(&high[FRAME * 2..]) < 0.01, "Aliasing at {rate} Hz");
    }
}

#[test]
fn streamed_wav_is_identical_and_recording_capacity_is_bounded() {
    let samples: Vec<_> = (0..RATE).map(|i| i as i16).collect();
    let old = wav(&samples).unwrap();
    let mut streamed = Vec::new();
    write_wav(&mut streamed, &samples).unwrap();
    assert_eq!(old, streamed);
    let mut recording = Vec::new();
    let limit = RATE * 3 + 17;
    for _ in 0..200 {
        append_recording(&mut recording, &[123; FRAME], limit);
        assert!(recording.capacity() <= limit);
    }
    assert_eq!(recording.len(), limit);
    assert!(recording.iter().all(|s| *s == 123));
}

#[test]
fn energy_gate_and_silence_trimming_preserve_quiet_speech() {
    assert!(!is_speech(&[], 0.006));
    assert!(!is_speech(&[0; FRAME], 0.006));
    let mut samples = vec![0; RATE];
    samples.extend(vec![9000; RATE / 2]);
    samples.extend(vec![0; RATE]);
    let trimmed = trim_speech(&samples, 0.006).unwrap();
    assert!(trimmed.len() < samples.len());
    let encoded = wav(trimmed).unwrap();
    let reader = hound::WavReader::new(Cursor::new(encoded)).unwrap();
    assert_eq!(reader.len() as usize, trimmed.len());
    assert_eq!(reader.spec().sample_rate, 16000);
    for level in [5i16, 100, 1000, 10000] {
        let frame = [level; FRAME];
        assert_eq!(is_speech(&frame, 0.006), rms(&frame) >= 0.006);
    }
}

/// The original capture algorithm, kept only as a benchmark reference.
struct Before {
    channels: usize,
    resampler: rubato::FftFixedIn<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    used: usize,
}
impl Before {
    fn new(rate: usize, channels: usize) -> Self {
        let resampler = rubato::FftFixedIn::new(rate, RATE, rate * 3 / 100, 1, 1).unwrap();
        let input = resampler.input_buffer_allocate(true);
        let output = resampler.output_buffer_allocate(true);
        Self {
            channels,
            resampler,
            input,
            output,
            used: 0,
        }
    }
    fn process(&mut self, data: &[f32]) {
        for frame in data.chunks_exact(self.channels) {
            self.input[0][self.used] = frame.iter().sum::<f32>() / self.channels as f32;
            self.used += 1;
            if self.used == self.input[0].len() {
                let (_, n) = self
                    .resampler
                    .process_into_buffer(&self.input, &mut self.output, None)
                    .unwrap();
                black_box(
                    self.output[0][..n]
                        .iter()
                        .map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16)
                        .collect::<Vec<_>>(),
                );
                self.used = 0;
            }
        }
    }
}

fn timed(mut action: impl FnMut()) -> f64 {
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        action();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[2]
}

#[test]
#[ignore = "Offline release benchmark; run alone with --test-threads=1"]
fn optimization_benchmark() {
    let mut report = serde_json::Map::new();
    for (name, rate, channels) in [("native_16khz", RATE, 1), ("resampled_48khz", 48000, 2)] {
        let input: Vec<f32> = (0..rate * 3 / 100 * channels)
            .map(|i| (i as f32 * 0.1).sin() * 0.4)
            .collect();
        let mut before = Before::new(rate, channels);
        let mut after = Converter::new(rate, channels).unwrap();
        let before_ms = timed(|| {
            for _ in 0..5000 {
                before.process(black_box(&input));
            }
        });
        let after_ms = timed(|| {
            for _ in 0..5000 {
                after.process(black_box(&input), |frame| {
                    black_box(frame);
                });
            }
        });
        let old_allocs = allocations(|| {
            for _ in 0..500 {
                before.process(black_box(&input));
            }
        })
        .0;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let new_allocs = allocations(|| {
            for _ in 0..500 {
                after.process(black_box(&input), |frame| {
                    assert!(sender.try_send(crate::engine::Event::Audio(frame)).is_ok());
                    black_box(receiver.try_recv().unwrap());
                });
            }
        })
        .0;
        assert_eq!(new_allocs, 0, "Capture allocated after initialization");
        report.insert(name.into(),serde_json::json!({"before_ms":before_ms,"after_ms":after_ms,"simulated_seconds":150,"before_allocations_per_500_frames":old_allocs,"after_allocations_per_500_frames":new_allocs}));
    }
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::config::Paths::at(temp.path().to_path_buf()).unwrap();
    std::fs::write(&paths.model, include_bytes!("../assets/hey-groq.rpw")).unwrap();
    let config = Config::default();
    let mut before = crate::wake::reference(&config, &paths).unwrap();
    let mut after = crate::wake::detector(&config, &paths).unwrap();
    let before_ms = timed(|| {
        for _ in 0..1000 {
            black_box(before.process_samples(vec![0i16; FRAME]));
        }
    });
    let after_ms = timed(|| {
        for _ in 0..1000 {
            black_box(after.process(black_box(&[0; FRAME])));
        }
    });
    report.insert("quiet_wake".into(),serde_json::json!({"before_ms":before_ms,"after_ms":after_ms,"simulated_seconds":30,"note":"digital silence; speech/noisy rooms still run the reference detector"}));
    let (_, old_peak) = allocations(|| {
        let pcm = vec![1234; RATE * 120];
        let encoded = wav(&pcm).unwrap();
        let mut worker_copy = Vec::new();
        std::io::Read::read_to_end(&mut Cursor::new(&encoded), &mut worker_copy).unwrap();
        black_box((&pcm, &encoded, &worker_copy));
    });
    let (_, new_peak) = allocations(|| {
        let pcm = vec![1234; RATE * 120];
        write_wav(std::io::sink(), black_box(&pcm)).unwrap();
        black_box(&pcm);
    });
    assert!(new_peak * 2 < old_peak);
    report.insert("audio_buffers_120s".into(),serde_json::json!({"before_peak_bytes":old_peak,"after_peak_bytes":new_peak,"note":"allocator-tracked audio buffers, excludes OS/driver/TLS memory"}));
    println!("OPTIMIZATION_JSON={}", serde_json::Value::Object(report));
}
