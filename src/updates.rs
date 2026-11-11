use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const ENDPOINT: &str = "https://api.github.com/repos/dayblox/whisper/releases/latest";
const LIMIT: u64 = 64_000;
static CHECKING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub newer_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn fetch(endpoint: &str, current: &str) -> Result<Option<String>> {
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build()
        .get(endpoint)
        .set("Accept", "application/vnd.github+json")
        .set(
            "User-Agent",
            concat!("Whispertype/", env!("CARGO_PKG_VERSION")),
        )
        .call();
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(403 | 429, _)) => {
            bail!("GitHub rate limit reached. Try again later.")
        }
        Err(ureq::Error::Status(404, _)) => bail!("No stable release is available yet."),
        Err(error) => return Err(error).context("Could not check GitHub for updates"),
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut body)?;
    ensure!(body.len() as u64 <= LIMIT, "Update response is too large");
    let release: Release = serde_json::from_slice(&body).context("Invalid update response")?;
    ensure!(
        !release.draft && !release.prerelease,
        "No stable release is available"
    );
    let latest = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("Unrecognized release version")?;
    ensure!(latest.pre.is_empty(), "No stable release is available");
    Ok((latest > semver::Version::parse(current)?).then(|| latest.to_string()))
}

// TLS and HTTP allocations live in this short-lived worker, not the idle tray.
pub fn check() -> Reply {
    match fetch(ENDPOINT, env!("CARGO_PKG_VERSION")) {
        Ok(newer_version) => Reply {
            newer_version,
            error: None,
        },
        Err(error) => Reply {
            newer_version: None,
            error: Some(format!("{error:#}")),
        },
    }
}

pub fn start() -> Result<()> {
    let exe = std::env::current_exe()?;
    if CHECKING.swap(true, Ordering::Relaxed) {
        return Ok(());
    }
    let child = crate::platform::spawn_hidden(
        Command::new(exe)
            .arg("_check-updates")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            CHECKING.store(false, Ordering::Relaxed);
            return Err(error.into());
        }
    };
    crate::platform::notify("Checking for updates…");
    std::thread::spawn(move || {
        let result = (|| -> Result<Reply> {
            let mut data = Vec::new();
            child
                .stdout
                .take()
                .context("Missing update worker output")?
                .take(LIMIT + 1)
                .read_to_end(&mut data)?;
            ensure!(
                child.wait()?.success(),
                "Update checker exited unexpectedly"
            );
            ensure!(
                data.len() as u64 <= LIMIT,
                "Update worker response is too large"
            );
            Ok(serde_json::from_slice(&data)?)
        })();
        let message = match result {
            Ok(Reply { error: Some(error), .. }) => format!("Update check failed: {error}"),
            Ok(Reply { newer_version: Some(version), .. }) => {
                match crate::platform::open_releases() {
                    Ok(()) => format!("Whispertype {version} is available. Opening the download page."),
                    Err(_) => format!("Whispertype {version} is available at github.com/dayblox/whisper/releases/latest"),
                }
            }
            Ok(_) => format!("Whispertype {} is up to date.", env!("CARGO_PKG_VERSION")),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                format!("Update check failed: {error:#}")
            }
        };
        CHECKING.store(false, Ordering::Relaxed);
        crate::platform::notify(&message);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    fn mock(body: &str, status: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 4096]);
            let _ = stream.write_all(response.as_bytes());
        });
        url
    }

    #[test]
    fn stable_versions_are_compared_numerically_and_prereleases_are_rejected() {
        for (tag, newer) in [
            ("v0.1.0", false),
            ("v0.1.1", false),
            ("v0.1.10", true),
            ("v1.0.0", true),
        ] {
            let body = format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false}}"#);
            assert_eq!(
                fetch(&mock(&body, "200 OK"), "0.1.1").unwrap().is_some(),
                newer
            );
        }
        for body in [
            r#"{"tag_name":"v0.2.0-beta.1","draft":false,"prerelease":false}"#,
            r#"{"tag_name":"v0.2.0","draft":false,"prerelease":true}"#,
            r#"{"tag_name":"v0.2.0","draft":true,"prerelease":false}"#,
            r#"{"tag_name":"bad","draft":false,"prerelease":false}"#,
            "not json",
        ] {
            assert!(fetch(&mock(body, "200 OK"), "0.1.1").is_err());
        }
    }

    #[test]
    fn rate_limits_missing_releases_and_oversized_responses_fail_cleanly() {
        for status in [
            "403 Forbidden",
            "429 Too Many Requests",
            "404 Not Found",
            "500 Server Error",
        ] {
            assert!(fetch(&mock("{}", status), "0.1.1").is_err());
        }
        assert!(fetch(&mock(&" ".repeat(LIMIT as usize + 1), "200 OK"), "0.1.1").is_err());
    }
}
