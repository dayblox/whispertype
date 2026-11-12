//! Windows owns TLS, certificates and proxy discovery. No environment proxy lookup.
//! Each short-lived network helper owns one session and reuses its connection.
mod handle;
mod request;
#[cfg(test)]
mod tests;
use handle::Handle;
pub use request::Request;
use std::{
    io,
    mem::size_of,
    ptr::{null, null_mut},
    time::Duration,
};
use windows_sys::Win32::{Foundation::*, Networking::WinHttp::*};

pub const PROXY_REFUSED: &str =
    "Proxy login refused (HTTP 407). Check your Windows account and system proxy settings.";

pub struct Client {
    // Drop connections before their session.
    connection: Option<(String, u16, Handle)>,
    session: Handle,
}

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn option(handle: *mut std::ffi::c_void, name: u32, value: u32) -> io::Result<()> {
    unsafe {
        check(WinHttpSetOption(
            handle,
            name,
            (&value as *const u32).cast(),
            size_of::<u32>() as u32,
        ))
    }
}

impl Client {
    pub fn new() -> io::Result<Self> {
        let agent = wide(concat!("Whispertype/", env!("CARGO_PKG_VERSION")));
        let mut raw = unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                WINHTTP_FLAG_ASYNC,
            )
        };
        if raw.is_null()
            && matches!(
                unsafe { GetLastError() },
                ERROR_INVALID_PARAMETER | ERROR_WINHTTP_INVALID_OPTION | ERROR_NOT_SUPPORTED
            )
        {
            raw = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                    null(),
                    null(),
                    WINHTTP_FLAG_ASYNC,
                )
            };
        }
        let session = Handle::new(raw)?;
        // Only current-account Negotiate/NTLM is allowed, including automatic
        // proxy auth on modern Windows. Older WinHTTP has no stored credentials
        // supplied by this client and auto-logon itself only uses these schemes.
        if let Err(error) = option(
            session.0,
            WINHTTP_OPTION_DISABLE_PROXY_AUTH_SCHEMES,
            WINHTTP_PROXY_DISABLE_SCHEME_BASIC | WINHTTP_PROXY_DISABLE_SCHEME_DIGEST,
        ) {
            if error.raw_os_error() != Some(ERROR_WINHTTP_INVALID_OPTION as i32) {
                return Err(error);
            }
        }
        Ok(Self {
            connection: None,
            session,
        })
    }

    pub fn request(
        &mut self,
        method: &str,
        url: &str,
        headers: &str,
        connect: Duration,
        total: Duration,
        redirects: u32,
    ) -> io::Result<Request<'_>> {
        let url = wide(url);
        let mut parts = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            dwUserNameLength: u32::MAX,
            dwPasswordLength: u32::MAX,
            ..Default::default()
        };
        unsafe {
            check(WinHttpCrackUrl(
                url.as_ptr(),
                (url.len() - 1) as u32,
                0,
                &mut parts,
            ))?;
        }
        let secure = parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        // Plain HTTP exists only for loopback tests; production is always TLS.
        if (!secure && !cfg!(test))
            || (!secure && parts.nScheme != WINHTTP_INTERNET_SCHEME_HTTP)
            || parts.dwUserNameLength != 0
            || parts.dwPasswordLength != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Expected an HTTPS URL without credentials",
            ));
        }
        let host = unsafe {
            String::from_utf16_lossy(std::slice::from_raw_parts(
                parts.lpszHostName,
                parts.dwHostNameLength as usize,
            ))
        };
        if !secure && host != "127.0.0.1" {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if self
            .connection
            .as_ref()
            .is_none_or(|(h, p, _)| *h != host || *p != parts.nPort)
        {
            let handle = Handle::new(unsafe {
                WinHttpConnect(self.session.0, wide(&host).as_ptr(), parts.nPort, 0)
            })?;
            self.connection = Some((host, parts.nPort, handle));
        }
        let mut path = Vec::new();
        if parts.dwUrlPathLength == 0 {
            path.push(b'/' as u16);
        } else {
            unsafe {
                path.extend_from_slice(std::slice::from_raw_parts(
                    parts.lpszUrlPath,
                    parts.dwUrlPathLength as usize,
                ));
            }
        }
        if parts.dwExtraInfoLength != 0 {
            unsafe {
                path.extend_from_slice(std::slice::from_raw_parts(
                    parts.lpszExtraInfo,
                    parts.dwExtraInfoLength as usize,
                ));
            }
        }
        path.push(0);
        let connection = &self.connection.as_ref().unwrap().2;
        let handle = Handle::new(unsafe {
            WinHttpOpenRequest(
                connection.0,
                wide(method).as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                if secure { WINHTTP_FLAG_SECURE } else { 0 },
            )
        })?;
        option(
            handle.0,
            WINHTTP_OPTION_AUTOLOGON_POLICY,
            WINHTTP_AUTOLOGON_SECURITY_LEVEL_LOW,
        )?;
        // Do not retain HTTP cookies between requests.
        option(
            handle.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES,
        )?;
        option(
            handle.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            if redirects == 0 {
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER
            } else {
                WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP
            },
        )?;
        if redirects != 0 {
            option(
                handle.0,
                WINHTTP_OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS,
                redirects,
            )?;
        }
        unsafe {
            check(WinHttpSetTimeouts(
                handle.0,
                connect.as_millis() as i32,
                connect.as_millis() as i32,
                total.as_millis() as i32,
                total.as_millis() as i32,
            ))?;
        }
        Request::new(handle, headers, total, self)
    }
}
