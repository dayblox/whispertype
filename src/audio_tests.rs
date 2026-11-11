use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
    time::Instant,
};

#[test]
fn quiet_delivery_suppression_preserves_preroll_hangover_and_recording_pcm() {
    let mut delivery = FrameDelivery::new(0.003);
    let mut received = Vec::new();
    for _ in 0..1000 {
        delivery.process([0; FRAME], true, |frame| received.push(frame));
    }
    assert!(received.is_empty());
    delivery.process([9000; FRAME], true, |frame| received.push(frame));
    assert_eq!(received.len(), 9);
    assert!(received[..8].iter().all(|frame| *frame == [0; FRAME]));
    for _ in 0..68 {
        delivery.process([0; FRAME], true, |frame| received.push(frame));
    }
    assert_eq!(received.len(), 77);
    for _ in 0..1000 {
        delivery.process([0; FRAME], true, |frame| received.push(frame));
    }
    assert_eq!(received.len(), 77);
    received.clear();
    for value in [0, 1, 33, -42, 0, 9000] {
        delivery.process([value; FRAME], false, |frame| received.push(frame));
    }
    assert_eq!(
        received,
        [0, 1, 33, -42, 0, 9000].map(|value| [value; FRAME])
    );
}

#[test]
fn quiet_delivery_keeps_detection_frames_identical_across_long_pauses() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::config::Paths::at(temp.path().into()).unwrap();
    std::fs::write(&paths.model, include_bytes!("../assets/hey-groq.rpw")).unwrap();
    let config = Config {
        wake_threshold: 0.65,
        ..Config::default()
    };
    let mut direct = crate::wake::detector(&config, &paths).unwrap();
    let mut gated = crate::wake::detector(&config, &paths).unwrap();
    let mut delivery = FrameDelivery::new(config.speech_rms * 0.5);
    let mut input = vec![0; RATE * 5];
    for _ in 0..2 {
        input.extend(
            include_bytes!("../assets/test-positive.pcm")
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| i16::from_le_bytes(*v)),
        );
        input.extend(vec![0; RATE * 5]);
    }
    let mut direct_hits = Vec::new();
    let mut gated_hits = Vec::new();
    for (index, frame) in input.as_chunks::<FRAME>().0.iter().enumerate() {
        if direct.process(frame) {
            direct_hits.push(index);
        }
        delivery.process(*frame, true, |frame| {
            if gated.process(&frame) {
                gated_hits.push(index);
            }
        });
    }
    assert_eq!(direct_hits.len(), 2);
    assert_eq!(gated_hits, direct_hits);
}

#[test]
#[ignore = "Local personal voice replay; requires WHISPERTYPE_BENCH_MODEL and WHISPERTYPE_BENCH_WAV"]
fn personal_capture_delivery_equivalence() {
    let model = std::env::var_os("WHISPERTYPE_BENCH_MODEL").expect("Set WHISPERTYPE_BENCH_MODEL");
    let wav = std::env::var_os("WHISPERTYPE_BENCH_WAV").expect("Set WHISPERTYPE_BENCH_WAV");
    let mut reader = hound::WavReader::open(wav).unwrap();
    let mut pcm: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    pcm.extend(vec![0; RATE * 3]);
    let frames = pcm.as_chunks::<FRAME>().0;
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::config::Paths::at(temp.path().into()).unwrap();
    std::fs::copy(model, &paths.model).unwrap();
    let config = Config::default();
    let mut direct = crate::wake::detector(&config, &paths).unwrap();
    let expected: Vec<_> = frames
        .iter()
        .enumerate()
        .filter_map(|(i, frame)| direct.process(frame).then_some(i))
        .collect();
    assert_eq!(expected.len(), 3);
    let mut cached = crate::wake::detector(&config, &paths).unwrap();
    // A compiled detector must remain usable without re-reading its model.
    std::fs::remove_file(&paths.model).unwrap();
    for _ in 0..3 {
        // Reset with an unfinished phrase, as when dictation interrupts listening.
        for frame in &frames[..expected[0]] {
            cached.process(frame);
        }
        cached.reset();
        let mut delivery = FrameDelivery::new(config.speech_rms * 0.5);
        let mut actual = Vec::new();
        for (i, frame) in frames.iter().enumerate() {
            delivery.process(*frame, true, |frame| {
                if cached.process(&frame) {
                    actual.push(i);
                }
            });
        }
        assert_eq!(
            actual, expected,
            "Gate/cache changed personal detection timing"
        );
        cached.reset();
    }
    println!("All three real phrase detections retain identical frame positions through capture gating and repeated cached resets. No audio uploaded.");
}

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
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() && TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let live =
                LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed) + layout.size() as isize;
            PEAK.fetch_max(live.max(0) as usize, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if TRACK.load(Ordering::Relaxed) {
            LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
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
    for (name, gated) in [("shortcut_delivery", false), ("wake_delivery", true)] {
        let mut delivery = FrameDelivery::new(0.003);
        let (sender, receiver) = std::sync::mpsc::sync_channel(16);
        let (count, _) = allocations(|| {
            for i in 0..1000 {
                delivery.process(
                    if i % 100 < 10 {
                        [9000; FRAME]
                    } else {
                        [0; FRAME]
                    },
                    gated,
                    |frame| {
                        sender.try_send(crate::engine::Event::Audio(frame)).unwrap();
                        black_box(receiver.try_recv().unwrap());
                    },
                );
            }
        });
        assert_eq!(count, 0, "Delivery allocated after initialization");
        report.insert(
            name.into(),
            serde_json::json!({"allocations_per_1000_frames": count}),
        );
    }
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
    let config = Config {
        wake_threshold: 0.65,
        ..Config::default()
    };
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

#[test]
#[ignore = "Offline same-workload efficiency benchmark; run alone in release mode"]
fn efficiency_benchmark() {
    let model = std::env::var_os("WHISPERTYPE_BENCH_MODEL").expect("Set WHISPERTYPE_BENCH_MODEL");
    let positive = std::env::var_os("WHISPERTYPE_BENCH_WAV").expect("Set WHISPERTYPE_BENCH_WAV");
    let mut reader = hound::WavReader::open(positive).unwrap();
    let mut positive: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    positive.extend(vec![0; RATE * 3]);
    let mut negative: Vec<i16> = include_bytes!("../assets/test-negative.pcm")
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| i16::from_le_bytes(*v))
        .collect();
    negative.extend(vec![0; RATE * 3]);
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::config::Paths::at(temp.path().into()).unwrap();
    std::fs::copy(model, &paths.model).unwrap();
    let mut report = serde_json::Map::new();
    for (name, pcm, expected) in [
        ("digital_silence", vec![0; RATE * 30], 0),
        ("nonmatching_speech", negative, 0),
        ("personal_phrase", positive, 3),
    ] {
        let frames = pcm.as_chunks::<FRAME>().0;
        let mut detector = crate::wake::detector(&Config::default(), &paths).unwrap();
        assert_eq!(
            frames
                .iter()
                .filter(|frame| detector.process(frame))
                .count(),
            expected
        );
        let loops = 40;
        let milliseconds = timed(|| {
            let mut hits = 0;
            for _ in 0..loops {
                for frame in frames {
                    hits += detector.process(black_box(frame)) as usize;
                }
            }
            assert_eq!(hits, loops * expected);
        });
        let (allocs, peak) = allocations(|| {
            for frame in frames {
                black_box(detector.process(black_box(frame)));
            }
        });
        report.insert(
            name.into(),
            serde_json::json!({
                "median_ms": milliseconds, "passes": loops, "frames_per_pass": frames.len(),
                "audio_seconds_per_pass": frames.len() as f64 * 0.03,
                "microseconds_per_frame": milliseconds * 1000. / (loops * frames.len()) as f64,
            "allocations_per_pass": allocs, "peak_incremental_live_bytes": peak,
                "detections_per_pass": expected,
            }),
        );
    }
    println!("EFFICIENCY_JSON={}", serde_json::Value::Object(report));
}
