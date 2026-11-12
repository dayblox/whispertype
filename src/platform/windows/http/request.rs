use super::*;
use std::{
    ffi::c_void,
    io::Read,
    marker::PhantomData,
    sync::atomic::{AtomicU32, Ordering},
    time::Instant,
};
use windows_sys::Win32::System::Threading::*;

// One completion event per request, plus the final close notification. No
// polling, watchdog thread or runtime: WinHTTP callbacks only signal events.
struct Completion {
    ready: HANDLE,
    closed: HANDLE,
    error: AtomicU32,
    bytes: AtomicU32,
}

impl Drop for Completion {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.ready);
            CloseHandle(self.closed);
        }
    }
}

unsafe extern "system" fn completed(
    _: *mut c_void,
    context: usize,
    status: u32,
    info: *mut c_void,
    length: u32,
) {
    if context == 0 {
        return;
    }
    let state = &*(context as *const Completion);
    if status == WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING {
        SetEvent(state.closed);
        return;
    }
    let bytes = match status {
        WINHTTP_CALLBACK_STATUS_REQUEST_ERROR => {
            state.error.store(
                (*(info as *const WINHTTP_ASYNC_RESULT)).dwError,
                Ordering::Relaxed,
            );
            0
        }
        WINHTTP_CALLBACK_STATUS_READ_COMPLETE => length,
        WINHTTP_CALLBACK_STATUS_WRITE_COMPLETE => *(info as *const u32),
        WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE
        | WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => 0,
        _ => return,
    };
    state.bytes.store(bytes, Ordering::Relaxed);
    SetEvent(state.ready);
}

pub struct Request<'a> {
    handle: Handle,
    completion: Box<Completion>,
    headers: Vec<u16>,
    deadline: Instant,
    _client: PhantomData<&'a mut Client>,
}

// Choose each integrated scheme at most once. Never select Basic, even if it
// is the first advertised scheme. WinHTTP handles the internal NTLM handshake.
fn proxy_scheme(supported: u32, tried: u32) -> Option<u32> {
    [WINHTTP_AUTH_SCHEME_NEGOTIATE, WINHTTP_AUTH_SCHEME_NTLM]
        .into_iter()
        .find(|s| supported & s != 0 && tried & s == 0)
}

impl<'a> Request<'a> {
    pub(super) fn new(
        handle: Handle,
        headers: &str,
        total: Duration,
        _: &'a mut Client,
    ) -> io::Result<Self> {
        let ready = unsafe { CreateEventW(null(), 0, 0, null()) };
        if ready.is_null() {
            return Err(io::Error::last_os_error());
        }
        let closed = unsafe { CreateEventW(null(), 0, 0, null()) };
        if closed.is_null() {
            unsafe {
                CloseHandle(ready);
            }
            return Err(io::Error::last_os_error());
        }
        let completion = Box::new(Completion {
            ready,
            closed,
            error: AtomicU32::new(0),
            bytes: AtomicU32::new(0),
        });
        let context = &*completion as *const Completion as usize;
        unsafe {
            check(WinHttpSetOption(
                handle.0,
                WINHTTP_OPTION_CONTEXT_VALUE,
                (&context as *const usize).cast(),
                size_of::<usize>() as u32,
            ))?;
            let previous = WinHttpSetStatusCallback(
                handle.0,
                Some(completed),
                WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
                0,
            );
            if previous.is_some_and(|f| f as usize == usize::MAX) {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            handle,
            completion,
            headers: wide(headers),
            deadline: Instant::now() + total,
            _client: PhantomData,
        })
    }

    fn wait(&mut self, ok: i32) -> io::Result<u32> {
        check(ok)?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let wait = unsafe {
            WaitForSingleObject(
                self.completion.ready,
                remaining.as_millis().min(u32::MAX as u128 - 1) as u32,
            )
        };
        if wait == WAIT_TIMEOUT || Instant::now() >= self.deadline {
            self.close();
            return Err(io::ErrorKind::TimedOut.into());
        }
        if wait != WAIT_OBJECT_0 {
            let error = io::Error::last_os_error();
            self.close();
            return Err(error);
        }
        let error = self.completion.error.swap(0, Ordering::Relaxed);
        if error == ERROR_WINHTTP_LOGIN_FAILURE {
            return Err(io::Error::other(PROXY_REFUSED));
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(self.completion.bytes.load(Ordering::Relaxed))
    }

    pub fn status(&self) -> io::Result<u32> {
        let mut status = 0u32;
        let mut size = 4;
        unsafe {
            check(WinHttpQueryHeaders(
                self.handle.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut size,
                null_mut(),
            ))?;
        }
        Ok(status)
    }

    pub fn header(&self, name: &str) -> Option<String> {
        let mut buffer = [0u16; 512];
        let mut size = size_of_val(&buffer) as u32;
        unsafe {
            check(WinHttpQueryHeaders(
                self.handle.0,
                WINHTTP_QUERY_CUSTOM,
                wide(name).as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                null_mut(),
            ))
            .ok()?;
        }
        Some(String::from_utf16_lossy(
            &buffer[..buffer.iter().position(|c| *c == 0)?],
        ))
    }

    fn authenticate(&self, tried: &mut u32) -> io::Result<bool> {
        let (mut supported, mut first, mut target) = (0, 0, 0);
        unsafe {
            if WinHttpQueryAuthSchemes(self.handle.0, &mut supported, &mut first, &mut target) == 0
            {
                return Ok(false);
            }
        }
        let Some(scheme) = proxy_scheme(supported, *tried) else {
            return Ok(false);
        };
        if target != WINHTTP_AUTH_TARGET_PROXY {
            return Ok(false);
        }
        *tried |= scheme;
        unsafe {
            check(WinHttpSetCredentials(
                self.handle.0,
                WINHTTP_AUTH_TARGET_PROXY,
                scheme,
                null(),
                null(),
                null_mut(),
            ))?;
        }
        Ok(true)
    }

    /// Auth may retry headers/tunnel setup. Once audio has been consumed, it
    /// cannot be replayed from a pipe; never buffer it or retry a transcription.
    pub fn send(&mut self, mut body: impl Read, length: u64) -> io::Result<()> {
        let length = u32::try_from(length).map_err(|_| io::ErrorKind::InvalidInput)?;
        let mut tried = 0;
        for attempt in 0..5 {
            let context = &*self.completion as *const Completion as usize;
            let sent = self.wait(unsafe {
                WinHttpSendRequest(
                    self.handle.0,
                    self.headers.as_ptr(),
                    (self.headers.len() - 1) as u32,
                    null(),
                    0,
                    length,
                    context,
                )
            });
            match sent {
                Err(e)
                    if e.raw_os_error() == Some(ERROR_WINHTTP_RESEND_REQUEST as i32)
                        && attempt < 4 =>
                {
                    continue
                }
                Err(e) => {
                    if self.status().ok() == Some(407) {
                        if self.authenticate(&mut tried)? {
                            continue;
                        }
                        return Err(io::Error::other(PROXY_REFUSED));
                    }
                    if e.raw_os_error() == Some(ERROR_WINHTTP_LOGIN_FAILURE as i32) {
                        return Err(io::Error::other(PROXY_REFUSED));
                    }
                    return Err(e);
                }
                Ok(_) => {}
            }
            let mut buffer = [0u8; 64 * 1024];
            let mut remaining = length as u64;
            while remaining != 0 {
                let limit = buffer.len().min(remaining as usize);
                let n = body.read(&mut buffer[..limit])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let mut written = 0;
                while written < n {
                    let count = self.wait(unsafe {
                        WinHttpWriteData(
                            self.handle.0,
                            buffer[written..n].as_ptr().cast(),
                            (n - written) as u32,
                            null_mut(),
                        )
                    })? as usize;
                    if count == 0 {
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                    written += count;
                }
                remaining -= n as u64;
            }
            match self.wait(unsafe { WinHttpReceiveResponse(self.handle.0, null_mut()) }) {
                Err(e)
                    if length == 0
                        && e.raw_os_error() == Some(ERROR_WINHTTP_RESEND_REQUEST as i32)
                        && attempt < 4 =>
                {
                    continue
                }
                Err(e) => return Err(e),
                Ok(_) => {}
            }
            if self.status()? != 407 || length != 0 || !self.authenticate(&mut tried)? {
                return Ok(());
            }
            let mut buffer = [0; 8192];
            let mut discarded = 0;
            loop {
                let n = self.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                discarded += n;
                if discarded > 128_000 {
                    return Err(io::Error::other(PROXY_REFUSED));
                }
            }
        }
        Err(io::Error::other(PROXY_REFUSED))
    }

    fn close(&mut self) {
        if !self.handle.0.is_null() {
            unsafe {
                WinHttpCloseHandle(self.handle.0);
                WaitForSingleObject(self.completion.closed, INFINITE);
            }
            self.handle.0 = null_mut();
        }
    }
}

impl Read for Request<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.wait(unsafe {
            WinHttpReadData(
                self.handle.0,
                buffer.as_mut_ptr().cast(),
                buffer.len().min(u32::MAX as usize) as u32,
                null_mut(),
            )
        })
        .map(|n| n as usize)
    }
}

impl Drop for Request<'_> {
    fn drop(&mut self) {
        // HANDLE_CLOSING is the final callback. Wait before releasing callback
        // state or caller-owned read/write buffers, including on timeout.
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_auth_prefers_negotiate_then_ntlm_and_never_basic() {
        let all =
            WINHTTP_AUTH_SCHEME_BASIC | WINHTTP_AUTH_SCHEME_NTLM | WINHTTP_AUTH_SCHEME_NEGOTIATE;
        assert_eq!(proxy_scheme(all, 0), Some(WINHTTP_AUTH_SCHEME_NEGOTIATE));
        assert_eq!(
            proxy_scheme(all, WINHTTP_AUTH_SCHEME_NEGOTIATE),
            Some(WINHTTP_AUTH_SCHEME_NTLM)
        );
        assert_eq!(proxy_scheme(all, all), None);
        assert_eq!(proxy_scheme(WINHTTP_AUTH_SCHEME_BASIC, 0), None);
    }
}
