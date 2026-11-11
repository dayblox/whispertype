use crate::{
    audio::RATE,
    config::{self, Config, Paths},
};
use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Usage {
    at: u64,
    seconds: f64,
}

fn check_budget(entries: &[Usage], now: u64, seconds: f64) -> Result<()> {
    let recent = |age| {
        entries
            .iter()
            .filter(move |x| now.saturating_sub(x.at) < age)
    };
    ensure!(
        recent(60).count() < 20,
        "Local free-tier limit: 20 requests/minute. Try later."
    );
    ensure!(
        recent(86400).count() < 2000,
        "Local free-tier limit: 2,000 requests/day."
    );
    ensure!(
        recent(3600).map(|x| x.seconds).sum::<f64>() + seconds <= 7200.0,
        "Local free-tier limit: two audio hours/hour."
    );
    ensure!(
        recent(86400).map(|x| x.seconds).sum::<f64>() + seconds <= 28800.0,
        "Local free-tier limit: eight audio hours/day."
    );
    Ok(())
}

fn reserve(paths: &Paths, seconds: f64) -> Result<()> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&paths.state)?;
    file.lock_exclusive()?;
    let mut data = String::new();
    file.read_to_string(&mut data)?;
    let mut entries: Vec<Usage> = if data.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&data)
            .context("Usage ledger is damaged; inspect usage.json before resetting it")?
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    entries.retain(|x| now.saturating_sub(x.at) < 86400);
    // Conservatively reserve at least ten seconds per request, including failures.
    let seconds = seconds.max(10.0);
    check_budget(&entries, now, seconds)?;
    entries.push(Usage { at: now, seconds });
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    serde_json::to_writer(&mut file, &entries)?;
    file.flush()?;
    file.sync_data()?;
    Ok(())
}

const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MAX_WAV_BYTES: u64 = 20_000_000;

/// Make short/truncated inputs an I/O error instead of silently declaring a
/// larger Content-Length than the body we actually transmit.
struct ExactRead<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for ExactRead<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let limit = output.len().min(self.remaining as usize);
        if limit == 0 {
            return Ok(0);
        }
        let count = self.inner.read(&mut output[..limit])?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Truncated audio stream",
            ));
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

pub fn worker(paths: &Paths) -> Result<String> {
    let input = std::io::stdin();
    let mut input = input.lock();
    let mut header = [0; 44];
    input
        .read_exact(&mut header)
        .context("Missing worker WAV header")?;
    let bytes = u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64;
    ensure!(
        bytes > 0 && bytes.is_multiple_of(2) && bytes + 44 <= MAX_WAV_BYTES,
        "Invalid worker audio length"
    );
    ensure!(
        header == crate::audio::wav_header(bytes as usize / 2),
        "Expected canonical mono 16 kHz PCM16 WAV"
    );
    let body = Cursor::new(header).chain(input);
    send_audio(
        paths,
        &Config::load(paths)?,
        body,
        bytes + 44,
        bytes as f64 / 2.0 / RATE as f64,
        ENDPOINT,
    )
}

pub fn transcribe_file(paths: &Paths, config: &Config, path: &std::path::Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    ensure!(
        bytes <= MAX_WAV_BYTES,
        "Audio exceeds the configured size bound"
    );
    let seconds = wav_duration(&mut file)?;
    file.rewind()?;
    send_audio(paths, config, file, bytes, seconds, ENDPOINT)
}

fn wav_duration(reader: impl Read + Seek) -> Result<f64> {
    let reader = hound::WavReader::new(reader).context("Expected a WAV file")?;
    ensure!(
        reader.spec().channels == 1
            && reader.spec().sample_rate == RATE as u32
            && reader.spec().bits_per_sample == 16
            && reader.spec().sample_format == hound::SampleFormat::Int,
        "Expected 16 kHz mono PCM16 WAV; convert your file before uploading"
    );
    Ok(reader.duration() as f64 / RATE as f64)
}

#[cfg(test)]
fn transcribe_at(paths: &Paths, config: &Config, wav: Vec<u8>, endpoint: &str) -> Result<String> {
    let seconds = wav_duration(Cursor::new(&wav))?;
    let bytes = wav.len() as u64;
    send_audio(paths, config, Cursor::new(wav), bytes, seconds, endpoint)
}

fn send_audio(
    paths: &Paths,
    config: &Config,
    audio: impl Read,
    bytes: u64,
    seconds: f64,
    endpoint: &str,
) -> Result<String> {
    ensure!(
        bytes <= MAX_WAV_BYTES,
        "Audio exceeds the configured size bound"
    );
    ensure!(
        seconds > 0.0 && seconds <= 600.0,
        "Audio duration must be between 0 and 600 seconds"
    );
    let key = std::env::var("GROQ_API_KEY")
        .or_else(|_| std::fs::read_to_string(&paths.key))
        .context("Add a Groq API key with whispertype set-key (or GROQ_API_KEY)")?;
    let key = key.trim();
    config::validate_key(key)?;
    reserve(paths, seconds)?;
    let boundary = "whispertype-6f8c7ba92089497b";
    let mut header = String::new();
    for (name, value) in [
        ("model", "whisper-large-v3"),
        ("response_format", "json"),
        ("temperature", "0"),
    ] {
        header.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    for (name, value) in [
        ("language", config.language.as_deref().unwrap_or("")),
        ("prompt", config.prompt.as_str()),
    ] {
        if !value.is_empty() {
            header.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"));
        }
    }
    header.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"));
    let footer = format!("\r\n--{boundary}--\r\n");
    let length = header.len() as u64 + bytes + footer.len() as u64;
    let body = Cursor::new(header)
        .chain(ExactRead {
            inner: audio,
            remaining: bytes,
        })
        .chain(Cursor::new(footer));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .redirects(0)
        .build();
    let response = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {key}"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .set("Content-Length", &length.to_string())
        .send(body);
    match response {
        Ok(response) => {
            let mut body = String::new(); response.into_reader().take(128_001).read_to_string(&mut body)?;
            ensure!(body.len() <= 128_000, "Unexpectedly large Groq response");
            let result: serde_json::Value = serde_json::from_str(&body)?;
            Ok(result["text"].as_str().context("Groq response has no text")?.trim().to_string())
        }
        Err(ureq::Error::Status(429,response)) => bail!("Groq rate limit reached. Retry after {} seconds. Limits are shared across your organization.",response.header("retry-after").unwrap_or("a few")),
        Err(ureq::Error::Status(401|403,_)) => bail!("Groq rejected the API key or model permission. Check your Groq account."),
        Err(ureq::Error::Status(status,_)) => bail!("Groq returned HTTP {status}. No automatic retry was made."),
        Err(ureq::Error::Transport(_)) => bail!("Could not reach Groq (network, TLS, or timeout). No automatic retry was made."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_reader_rejects_short_audio_and_does_not_consume_trailing_data() {
        let mut short = ExactRead {
            inner: Cursor::new(b"short"),
            remaining: 10,
        };
        let mut received = Vec::new();
        assert_eq!(
            short.read_to_end(&mut received).unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        let mut exact = ExactRead {
            inner: Cursor::new(b"audioTRAILER"),
            remaining: 5,
        };
        let mut received = Vec::new();
        exact.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"audio");
        assert_eq!(exact.inner.position(), 5);
        assert_eq!(exact.read(&mut []).unwrap(), 0);
    }
    #[test]
    fn multipart_upload_and_rate_limit_response() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        config::private_write(&paths.key, b"gsk_testkey00000000000000000000000").unwrap();
        for (status, response, headers) in [
            (200, r#"{"text":"Hello, world."}"#, ""),
            (429, "{}", "Retry-After: 17\r\n"),
            (401, "{}", ""),
        ] {
            let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = server.local_addr().unwrap();
            let handle = std::thread::spawn(move || {
                let (mut client, _) = server.accept().unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let mut chunk = [0; 4096];
                let (end, length) = loop {
                    let n = client.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]);
                        let length = header
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while data.len() < end + length {
                    let n = client.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                }
                let data = String::from_utf8_lossy(&data);
                assert!(data.contains("whisper-large-v3"));
                assert!(data.contains("filename=\"speech.wav\""));
                assert!(data.contains("RIFF"));
                assert!(data.contains("name=\"prompt\""));
                assert!(data.contains("Whispertype, Groq, Dayblox"));
                write!(client,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{response}",response.len()).unwrap();
            });
            let result = transcribe_at(
                &paths,
                &Config {
                    prompt: "Whispertype, Groq, Dayblox".into(),
                    ..Config::default()
                },
                crate::audio::wav(&vec![1000; 16000]).unwrap(),
                &format!("http://{address}/audio/transcriptions"),
            );
            match status {
                200 => assert_eq!(result.unwrap(), "Hello, world."),
                429 => assert!(result.unwrap_err().to_string().contains("17 seconds")),
                _ => assert!(result.unwrap_err().to_string().contains("API key")),
            };
            handle.join().unwrap();
        }
        let entries: Vec<Usage> =
            serde_json::from_slice(&std::fs::read(&paths.state).unwrap()).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|u| u.seconds == 10.0));
    }
    #[test]
    fn all_free_tier_bounds_and_expiration() {
        let now = 100000;
        let requests: Vec<_> = (0..20)
            .map(|_| Usage {
                at: now,
                seconds: 10.0,
            })
            .collect();
        assert!(check_budget(&requests, now, 10.0).is_err());
        assert!(check_budget(&requests, now + 60, 10.0).is_ok());
        assert!(check_budget(
            &[Usage {
                at: now - 100,
                seconds: 7195.0
            }],
            now,
            10.0
        )
        .is_err());
        assert!(check_budget(
            &[Usage {
                at: now - 3700,
                seconds: 28795.0
            }],
            now,
            10.0
        )
        .is_err());
        assert!(check_budget(
            &[Usage {
                at: now - 86400,
                seconds: 28800.0
            }],
            now,
            10.0
        )
        .is_ok());
    }
}
