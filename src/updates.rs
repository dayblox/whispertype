use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::SyncSender,
    time::Duration,
};

const ENDPOINT: &str = "https://api.github.com/repos/dayblox/whispertype/releases/latest";
const LIMIT: u64 = 64_000;
static CHECKING: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{apply, cleanup, Prepared};

#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub newer_version: Option<String>,
    pub error: Option<String>,
    #[cfg(windows)]
    pub staged: Option<std::path::PathBuf>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[cfg(windows)]
    #[serde(default)]
    assets: Vec<Asset>,
}

#[cfg(windows)]
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn fetch(
    endpoint: &str,
    current: &str,
    #[cfg(windows)] client: &mut crate::platform::http::Client,
) -> Result<Option<Release>> {
    #[cfg(target_os = "linux")]
    let response = {
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
        match response {
            Ok(response) => response,
            Err(ureq::Error::Status(403 | 429, _)) => {
                bail!("GitHub rate limit reached. Try again later.")
            }
            Err(ureq::Error::Status(404, _)) => bail!("No stable release is available yet."),
            Err(error) => return Err(error).context("Could not check GitHub for updates"),
        }
    };
    #[cfg(windows)]
    let response = {
        let mut response = client
            .request(
                "GET",
                endpoint,
                "Accept: application/vnd.github+json\r\n",
                Duration::from_secs(5),
                Duration::from_secs(10),
                0,
            )
            .context("Could not check GitHub for updates")?;
        response
            .send(std::io::empty(), 0)
            .context("Could not check GitHub for updates")?;
        match response.status()? {
            200..300 => {}
            403 | 429 => bail!("GitHub rate limit reached. Try again later."),
            404 => bail!("No stable release is available yet."),
            407 => bail!(crate::platform::http::PROXY_REFUSED),
            status => bail!("Could not check GitHub for updates: HTTP {status}"),
        }
        response
    };
    let mut body = Vec::new();
    #[cfg(target_os = "linux")]
    let response = response.into_reader();
    response.take(LIMIT + 1).read_to_end(&mut body)?;
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
    Ok((latest > semver::Version::parse(current)?).then_some(release))
}

// TLS and HTTP allocations live in this short-lived worker, not the idle tray.
pub fn check() -> Reply {
    let result = (|| -> Result<_> {
        #[cfg(windows)]
        let mut client =
            crate::platform::http::Client::new().context("Could not check GitHub for updates")?;
        let release = fetch(
            ENDPOINT,
            env!("CARGO_PKG_VERSION"),
            #[cfg(windows)]
            &mut client,
        )?;
        #[cfg(not(windows))]
        let client = ();
        Ok((release, client))
    })();
    match result {
        Ok((release, _client)) => {
            #[cfg(windows)]
            let mut client = _client;
            let reply = Reply {
                newer_version: release
                    .as_ref()
                    .map(|r| r.tag_name.trim_start_matches('v').to_string()),
                error: None,
                #[cfg(windows)]
                staged: None,
            };
            #[cfg(windows)]
            let reply = {
                let mut reply = reply;
                if let Some(release) = release {
                    match windows::prepare(&release, &mut client) {
                        Ok(staged) => reply.staged = Some(staged),
                        Err(error) => reply.error = Some(format!("{error:#}")),
                    }
                }
                reply
            };
            reply
        }
        Err(error) => Reply {
            newer_version: None,
            error: Some(format!("{error:#}")),
            #[cfg(windows)]
            staged: None,
        },
    }
}

pub fn start(sender: SyncSender<crate::engine::Event>) -> Result<()> {
    #[cfg(not(windows))]
    let _ = sender;
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
        #[cfg(windows)]
        if let Ok(reply) = &result {
            if let Some(staged) = &reply.staged {
                let update = Prepared::new(staged.clone());
                crate::platform::update_notification("Update downloaded and verified. Whispertype will restart when dictation is idle.");
                // Keep CHECKING set until the engine installs or rejects this update.
                let _ = sender.send(crate::engine::Event::UpdateReady(update));
                return;
            }
        }
        let message = match result {
            Ok(Reply { newer_version: Some(version), error: Some(error), .. }) => {
                let _ = crate::platform::open_releases();
                format!("Whispertype {version} is available. Automatic update failed: {error}. Download it from the release page.")
            }
            Ok(Reply { error: Some(error), .. }) => format!("Update check failed: {error}"),
            Ok(Reply { newer_version: Some(version), .. }) => {
                match crate::platform::open_releases() {
                    Ok(()) => format!("Whispertype {version} is available. Opening the download page."),
                    Err(_) => format!("Whispertype {version} is available at github.com/dayblox/whispertype/releases/latest"),
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
        crate::platform::update_notification(&message);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fetch(endpoint: &str, current: &str) -> Result<Option<Release>> {
        super::fetch(
            endpoint,
            current,
            #[cfg(windows)]
            &mut crate::platform::http::Client::new()?,
        )
    }
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
