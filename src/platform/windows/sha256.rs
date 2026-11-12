//! Streaming SHA-256 and staging entropy from Windows CNG (no bundled crypto).
use anyhow::{ensure, Result};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::*;

pub struct Sha256(BCRYPT_HASH_HANDLE);

fn check(status: i32) -> Result<()> {
    ensure!(
        status >= 0,
        "Windows cryptography failed (NTSTATUS {status:#x})"
    );
    Ok(())
}

impl Sha256 {
    pub fn new() -> Result<Self> {
        let mut handle = null_mut();
        unsafe {
            check(BCryptCreateHash(
                BCRYPT_SHA256_ALG_HANDLE,
                &mut handle,
                null_mut(),
                0,
                null(),
                0,
                0,
            ))?;
        }
        Ok(Self(handle))
    }
    pub fn update(&mut self, bytes: &[u8]) -> Result<()> {
        unsafe {
            check(BCryptHashData(
                self.0,
                bytes.as_ptr(),
                u32::try_from(bytes.len())?,
                0,
            ))
        }
    }
    pub fn finish(self) -> Result<[u8; 32]> {
        let mut bytes = [0; 32];
        unsafe {
            check(BCryptFinishHash(
                self.0,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                0,
            ))?;
        }
        Ok(bytes)
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe {
            BCryptDestroyHash(self.0);
        }
    }
}

pub fn random(bytes: &mut [u8]) -> Result<()> {
    unsafe {
        check(BCryptGenRandom(
            null_mut(),
            bytes.as_mut_ptr(),
            u32::try_from(bytes.len())?,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_sha256_vector_in_chunks() {
        let mut hash = Sha256::new().unwrap();
        hash.update(b"a").unwrap();
        hash.update(b"bc").unwrap();
        let text: String = hash
            .finish()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            text,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
