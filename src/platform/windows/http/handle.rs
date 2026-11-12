use std::{ffi::c_void, io};
use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

pub(super) struct Handle(pub *mut c_void);

impl Handle {
    pub fn new(raw: *mut c_void) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}
