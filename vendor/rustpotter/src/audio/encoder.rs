// Modified by Whispertype; see UPSTREAM.md for changes.
use crate::{config::AudioFmt, Endianness, Sample, SampleFormat};

/// Encode and convert audio to the supported format.
pub struct AudioEncoder {
    source_sample_format: SampleFormat,
    source_channels: u16,
    source_endianness: Endianness,
    input_samples_per_frame: usize,
    output_samples_per_frame: usize,
}
impl AudioEncoder {
    pub fn get_input_frame_length(&self) -> usize {
        self.input_samples_per_frame
    }
    pub fn get_output_frame_length(&self) -> usize {
        self.output_samples_per_frame
    }
    pub fn get_input_byte_length(&self) -> usize {
        self.get_input_frame_length() * self.source_sample_format.get_bytes_per_sample() as usize
    }
    pub fn encode_into(&self, buffer: &[u8], out: &mut Vec<f32>) {
        out.clear();
        let stride = self.source_channels.max(1) as usize;
        match &self.source_sample_format {
            SampleFormat::I8 => {
                encode_audio_bytes_into::<i8>(buffer, &self.source_endianness, stride, out)
            }
            SampleFormat::I16 => {
                encode_audio_bytes_into::<i16>(buffer, &self.source_endianness, stride, out)
            }
            SampleFormat::I32 => {
                encode_audio_bytes_into::<i32>(buffer, &self.source_endianness, stride, out)
            }
            SampleFormat::F32 => {
                encode_audio_bytes_into::<f32>(buffer, &self.source_endianness, stride, out)
            }
        }
    }
    pub fn encode_samples_into<T: Sample>(&self, samples: &[T], out: &mut Vec<f32>) {
        out.clear();
        let stride = self.source_channels.max(1) as usize;
        if stride == 1 {
            out.extend(samples.iter().copied().map(T::into_f32));
        } else {
            out.extend(
                samples
                    .chunks_exact(stride)
                    .map(|chunk| T::into_f32(chunk[0])),
            );
        }
    }
    pub fn rencode_and_resample<T: Sample>(&mut self, buffer: Vec<T>) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.output_samples_per_frame);
        self.encode_samples_into(&buffer, &mut out);
        out
    }
    pub fn new(
        input_spec: &AudioFmt,
        frame_length_ms: usize,
        target_sample_rate: usize,
    ) -> Result<AudioEncoder, &'static str> {
        if input_spec.sample_rate != target_sample_rate {
            return Err("Unsupported sample rate, expected 16000 Hz");
        }
        let input_samples_per_frame =
            (input_spec.sample_rate * frame_length_ms / 1000) * input_spec.channels as usize;
        let output_samples_per_frame = target_sample_rate * frame_length_ms / 1000;
        Ok(AudioEncoder {
            input_samples_per_frame,
            output_samples_per_frame,
            source_sample_format: input_spec.sample_format.clone(),
            source_channels: input_spec.channels,
            source_endianness: input_spec.endianness.clone(),
        })
    }
}

fn encode_audio_bytes_into<T: Sample>(
    audio_buffer: &[u8],
    endianness: &Endianness,
    stride: usize,
    out: &mut Vec<f32>,
) {
    let step = T::get_byte_size() * stride;
    let byte_size = T::get_byte_size();
    let chunks = audio_buffer.chunks_exact(step);
    match endianness {
        Endianness::Little => {
            out.extend(chunks.map(|c| T::from_le_bytes(&c[..byte_size]).into_f32()))
        }
        Endianness::Big => out.extend(chunks.map(|c| T::from_be_bytes(&c[..byte_size]).into_f32())),
        Endianness::Native => {
            out.extend(chunks.map(|c| T::from_ne_bytes(&c[..byte_size]).into_f32()))
        }
    }
}

#[test]
fn it_returns_correct_samples_per_frame() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let file = std::fs::File::open(dir.to_owned() + "/tests/resources/oye_casa_g_1.wav").unwrap();
    let wav_reader = hound::WavReader::new(std::io::BufReader::new(file)).unwrap();
    let encoder = AudioEncoder::new(
        &wav_reader.spec().try_into().unwrap(),
        crate::constants::MFCCS_EXTRACTOR_FRAME_LENGTH_MS,
        crate::constants::DETECTOR_INTERNAL_SAMPLE_RATE,
    )
    .unwrap();
    let input_length = encoder.get_input_frame_length();
    let output_length = encoder.get_output_frame_length();
    assert_eq!(
        input_length, output_length,
        "input and output have same length"
    );
    assert_eq!(input_length, 480, "input size is correct");
    assert_eq!(output_length, 480, "output size is correct");
}

#[test]
fn reencode_wav_with_different_format() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let i16_samples_file =
        std::fs::File::open(dir.to_owned() + "/tests/resources/oye_casa_g_1.wav").unwrap();
    let wav_reader = hound::WavReader::new(std::io::BufReader::new(i16_samples_file)).unwrap();
    let mut encoder = AudioEncoder::new(
        &wav_reader.spec().try_into().unwrap(),
        crate::constants::MFCCS_EXTRACTOR_FRAME_LENGTH_MS,
        crate::constants::DETECTOR_INTERNAL_SAMPLE_RATE,
    )
    .unwrap();
    let input_length = encoder.get_input_frame_length();
    let output_length = encoder.get_output_frame_length();
    assert_eq!(
        input_length, output_length,
        "input and output have same length"
    );
    assert_eq!(input_length, 480, "input size is correct");
    assert_eq!(output_length, 480, "output size is correct");
    let samples = wav_reader
        .into_samples::<i16>()
        .map(|chunk| *chunk.as_ref().unwrap())
        .collect::<Vec<_>>();
    let internal_spec = hound::WavSpec {
        sample_rate: crate::constants::DETECTOR_INTERNAL_SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
        channels: 1,
    };
    let mut writer = hound::WavWriter::create(
        dir.to_owned() + "/tests/resources/oye_casa_g_1_f32.wav",
        internal_spec,
    )
    .unwrap();
    samples
        .chunks_exact(encoder.get_input_frame_length())
        .map(|chuck| encoder.rencode_and_resample::<i16>(chuck.to_vec()))
        .for_each(|reencoded_chunk| {
            for sample in reencoded_chunk {
                writer.write_sample(sample).ok();
            }
        });
    writer.finalize().expect("Unable to save file");
}
