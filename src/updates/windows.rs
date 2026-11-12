use super::*;
use crate::platform::{
    http,
    sha256::{self, Sha256},
};
use std::{fs::File, io::Write, path::Path, path::PathBuf};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

const BINARY: &str = "whispertype-windows-x64.exe";
const BINARY_LIMIT: u64 = 100 * 1024 * 1024;

pub struct Prepared {
    path: PathBuf,
    retained: bool,
}

impl Prepared {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            retained: false,
        }
    }

    pub fn launch(mut self) -> Result<()> {
        crate::platform::spawn_hidden(
            Command::new(self.path.join("updater.exe"))
                .arg("_apply-update")
                .arg(std::env::current_exe()?)
                .arg(std::process::id().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .context("Could not start the update helper")?;
        self.retained = true;
        Ok(())
    }

    fn keep(mut self) -> PathBuf {
        self.retained = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.retained {
            let _ = remove_stage(&self.path);
        }
        CHECKING.store(false, Ordering::Relaxed);
    }
}

fn asset(release: &Release) -> Result<&Asset> {
    ensure!(
        cfg!(target_arch = "x86_64"),
        "Automatic updates require Windows x64"
    );
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == BINARY)
        .context("The release has no Windows x64 binary")?;
    // Do not execute arbitrary URLs supplied by unexpected release metadata.
    ensure!(
        asset.browser_download_url
            == format!(
                "https://github.com/dayblox/whisper/releases/download/{}/{BINARY}",
                release.tag_name
            ),
        "Unexpected update download URL"
    );
    ensure!(
        asset.size > 0 && asset.size <= BINARY_LIMIT,
        "Invalid update size"
    );
    expected_digest(asset.digest.as_deref())?;
    Ok(asset)
}

fn expected_digest(digest: Option<&str>) -> Result<[u8; 32]> {
    let digest = digest
        .and_then(|d| d.strip_prefix("sha256:"))
        .context("The release has no SHA-256 digest")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid update digest"
    );
    let mut bytes = [0; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digest[i * 2..i * 2 + 2], 16)?;
    }
    Ok(bytes)
}

fn download(mut reader: impl Read, asset: &Asset, destination: &Path) -> Result<()> {
    let mut file = File::create(destination)?;
    let mut hash = Sha256::new()?;
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(
            count <= asset.size && count <= BINARY_LIMIT,
            "Update download is too large"
        );
        hash.update(&buffer[..n])?;
        file.write_all(&buffer[..n])?;
    }
    ensure!(count == asset.size, "Update download is incomplete");
    ensure!(
        hash.finish()? == expected_digest(asset.digest.as_deref())?,
        "Update checksum does not match; no files were replaced"
    );
    file.sync_all()?;
    Ok(())
}

pub(super) fn prepare(release: &Release, client: &mut http::Client) -> Result<PathBuf> {
    let asset = asset(release)?;
    let target = std::env::current_exe()?;
    // Staging beside the EXE checks write access before downloading and keeps
    // replacement/rollback renames on the same filesystem for portable copies.
    let stage = staging(target.parent().context("Missing application directory")?)?;
    let mut response = client.request(
        "GET",
        &asset.browser_download_url,
        "",
        Duration::from_secs(10),
        Duration::from_secs(180),
        5,
    )?;
    response
        .send(std::io::empty(), 0)
        .context("Could not download the update")?;
    let status = response.status()?;
    ensure!(status != 407, http::PROXY_REFUSED);
    ensure!(
        (200..300).contains(&status),
        "Could not download the update: HTTP {status}"
    );
    download(response, asset, &stage.path.join("new.exe"))?;
    std::fs::copy(target, stage.path.join("updater.exe"))?;
    Ok(stage.keep())
}

fn staging(parent: &Path) -> Result<Prepared> {
    let mut random = [0u8; 16];
    sha256::random(&mut random).context("Could not create a random update directory name")?;
    let path = parent.join(format!(
        ".whispertype-update-{:032x}",
        u128::from_le_bytes(random)
    ));
    // Atomic creation refuses existing directories/symlinks. Use the OS RNG.
    std::fs::create_dir(&path).context(
        "Could not create update staging; the application directory may not be writable",
    )?;
    Ok(Prepared::new(path))
}

fn replace(
    target: &Path,
    next: &Path,
    backup: &Path,
    restart: impl Fn(&Path, bool) -> Result<()>,
) -> Result<()> {
    std::fs::rename(target, backup).context("Could not back up the current executable")?;
    let result = (|| {
        std::fs::rename(next, target).context("Could not replace the executable")?;
        restart(target, true).context("Could not restart Whispertype")
    })();
    if let Err(error) = result {
        if target.exists() {
            std::fs::rename(target, next).context("Could not remove the failed update")?;
        }
        std::fs::rename(backup, target).context("Could not restore the previous executable")?;
        let _ = restart(target, false);
        return Err(error);
    }
    Ok(())
}

pub fn apply(target: &Path, parent: u32) -> Result<()> {
    // This helper is a copy of the old EXE, outside the file being replaced.
    // Wait for the engine to finish dropping its capture, tray and instance lock.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, parent);
        if !handle.is_null() {
            let result = WaitForSingleObject(handle, 30_000);
            CloseHandle(handle);
            ensure!(
                result == WAIT_OBJECT_0,
                "Whispertype did not exit; update was not installed"
            );
        }
    }
    let helper = std::env::current_exe()?;
    let stage = helper
        .parent()
        .context("Missing update directory")?
        .canonicalize()?;
    let target = target.canonicalize()?;
    ensure!(
        stage.parent() == target.parent(),
        "Update must be staged beside the application"
    );
    crate::platform::stop_for_upgrade(&target)?;
    replace(
        &target,
        &stage.join("new.exe"),
        &stage.join("previous.exe"),
        |exe, updated| {
            let mut command = Command::new(exe);
            command.arg("run");
            if updated {
                command
                    .arg("--updated-from")
                    .arg(&stage)
                    .arg(std::process::id().to_string());
            }
            crate::platform::spawn_hidden(
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
            Ok(())
        },
    )
}

pub fn cleanup(stage: &Path, helper: u32) -> Result<()> {
    let target = std::env::current_exe()?.canonicalize()?;
    let stage = stage.canonicalize()?;
    ensure!(
        stage.parent() == target.parent()
            && stage
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".whispertype-update-")),
        "Unexpected update staging directory"
    );
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, helper);
        if !handle.is_null() {
            let result = WaitForSingleObject(handle, 30_000);
            CloseHandle(handle);
            ensure!(result == WAIT_OBJECT_0, "Update helper did not exit");
        }
    }
    remove_stage(&stage)
}

fn remove_stage(stage: &Path) -> Result<()> {
    // Remove only our known staging files and the empty directory.
    for name in ["updater.exe", "previous.exe", "new.exe"] {
        let _ = std::fs::remove_file(stage.join(name));
    }
    std::fs::remove_dir(stage)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(hash: [u8; 32]) -> String {
        use std::fmt::Write;
        let mut text = String::with_capacity(64);
        for byte in hash.as_ref() {
            write!(&mut text, "{byte:02x}").unwrap();
        }
        text
    }

    fn fixture(bytes: &[u8]) -> Asset {
        Asset {
            name: BINARY.into(),
            browser_download_url: format!(
                "https://github.com/dayblox/whisper/releases/download/v0.2.0/{BINARY}"
            ),
            size: bytes.len() as u64,
            digest: Some(format!("sha256:{}", {
                let mut hash = Sha256::new().unwrap();
                hash.update(bytes).unwrap();
                hex(hash.finish().unwrap())
            })),
        }
    }

    #[test]
    fn download_rejects_corruption_truncation_and_extra_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new.exe");
        let asset = fixture(b"verified binary");
        download(&b"verified binary"[..], &asset, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"verified binary");
        for bytes in [
            &b"corrupt! binary"[..],
            &b"short"[..],
            &b"verified binary extra"[..],
        ] {
            assert!(download(bytes, &asset, &target).is_err());
        }
    }

    #[test]
    fn only_expected_release_assets_with_valid_digests_are_used() {
        let mut release = Release {
            tag_name: "v0.2.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![fixture(b"test")],
        };
        assert!(asset(&release).is_ok());
        release.assets[0].browser_download_url = "https://example.com/malware.exe".into();
        assert!(asset(&release).is_err());
        for digest in [None, Some("md5:abc"), Some("sha256:bad")] {
            assert!(expected_digest(digest).is_err());
        }
    }

    #[test]
    fn staging_is_unique_and_abandoned_downloads_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let first = staging(root.path()).unwrap();
        let second = staging(root.path()).unwrap();
        assert_ne!(first.path, second.path);
        let path = first.path.clone();
        std::fs::write(path.join("new.exe"), b"incomplete").unwrap();
        drop(first);
        assert!(!path.exists());
        assert!(second.path.exists());
    }

    #[test]
    fn replacement_rolls_back_when_install_or_restart_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe");
        let next = dir.path().join("new.exe");
        let backup = dir.path().join("previous.exe");
        std::fs::write(&target, b"old").unwrap();
        assert!(replace(&target, &next, &backup, |_, _| Ok(())).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        std::fs::write(&next, b"new").unwrap();
        let starts = std::cell::RefCell::new(Vec::new());
        assert!(replace(&target, &next, &backup, |_, updated| {
            starts.borrow_mut().push(updated);
            bail!("restart failed")
        })
        .is_err());
        assert_eq!(*starts.borrow(), [true, false]);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        replace(&target, &next, &backup, |_, _| Ok(())).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
    }

    #[test]
    #[ignore = "Offline allocation benchmark; run alone with --test-threads=1"]
    fn update_download_allocations_stay_bounded_as_the_binary_grows() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new.exe");
        for size in [64 * 1024u64, 8 * 1024 * 1024] {
            let block = [42u8; 8192];
            let mut hash = Sha256::new().unwrap();
            for _ in 0..size / block.len() as u64 {
                hash.update(&block).unwrap();
            }
            let mut asset = fixture(b"");
            asset.size = size;
            asset.digest = Some(format!("sha256:{}", hex(hash.finish().unwrap())));
            let (allocations, peak) = crate::audio::test_allocations(|| {
                download(std::io::repeat(42).take(size), &asset, &target).unwrap();
            });
            assert!(
                peak <= 4096,
                "Download buffered the executable in RAM: {peak}"
            );
            println!("Update bytes: {size}, heap allocations: {allocations}, tracked peak heap bytes: {peak}; fixed 64 KiB stack buffer, excludes HTTP/TLS.");
        }
    }
}
