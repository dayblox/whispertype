//! On-demand PulseAudio / pipewire-pulse introspection. This never captures
//! audio, creates a worker, subscribes to changes, or survives the operation.
//! Only dead/remembered devices use it; ordinary capture remains CPAL/ALSA.
use libc::{c_char, c_int, c_void};
use std::{
    ffi::CStr,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};

pub(super) struct Snapshot {
    pub default: String,
    pub sources: Vec<String>,
}
#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}
#[repr(C)]
struct ServerInfo {
    user: *const c_char,
    host: *const c_char,
    version: *const c_char,
    name: *const c_char,
    spec: SampleSpec,
    sink: *const c_char,
    source: *const c_char,
}
#[repr(C)]
struct SourceInfo {
    name: *const c_char,
    index: u32,
    description: *const c_char,
}
type ServerCallback = unsafe extern "C" fn(*mut c_void, *const ServerInfo, *mut c_void);
type SourceCallback = unsafe extern "C" fn(*mut c_void, *const SourceInfo, c_int, *mut c_void);

macro_rules! api {
    ($($name:ident: $ty:ty),* $(,)?) => {
        struct Api { $($name: $ty,)* }
        impl Api {
            unsafe fn load(library: *mut c_void) -> Option<Self> {
                $(let symbol = libc::dlsym(library, concat!(stringify!($name), "\0").as_ptr().cast());
                  if symbol.is_null() { return None; }
                  let $name = std::mem::transmute::<*mut c_void, $ty>(symbol);)*
                Some(Self { $($name,)* })
            }
        }
    }
}
api! {
    pa_mainloop_new: unsafe extern "C" fn() -> *mut c_void,
    pa_mainloop_free: unsafe extern "C" fn(*mut c_void),
    pa_mainloop_get_api: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pa_mainloop_prepare: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    pa_mainloop_poll: unsafe extern "C" fn(*mut c_void) -> c_int,
    pa_mainloop_dispatch: unsafe extern "C" fn(*mut c_void) -> c_int,
    pa_context_new: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    pa_context_connect: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *const c_void) -> c_int,
    pa_context_get_state: unsafe extern "C" fn(*mut c_void) -> c_int,
    pa_context_disconnect: unsafe extern "C" fn(*mut c_void),
    pa_context_unref: unsafe extern "C" fn(*mut c_void),
    pa_context_get_server_info: unsafe extern "C" fn(*mut c_void, ServerCallback, *mut c_void) -> *mut c_void,
    pa_context_get_source_info_list: unsafe extern "C" fn(*mut c_void, SourceCallback, *mut c_void) -> *mut c_void,
    pa_operation_get_state: unsafe extern "C" fn(*mut c_void) -> c_int,
    pa_operation_unref: unsafe extern "C" fn(*mut c_void),
}

struct Library(*mut c_void);
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.0);
        }
    }
}
struct Client {
    api: Api,
    mainloop: *mut c_void,
    context: *mut c_void,
}
impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                (self.api.pa_context_disconnect)(self.context);
                (self.api.pa_context_unref)(self.context);
            }
            (self.api.pa_mainloop_free)(self.mainloop);
        }
    }
}
impl Client {
    unsafe fn wait(&self, mut done: impl FnMut() -> bool) -> Option<()> {
        let deadline = Instant::now() + Duration::from_millis(250);
        while !done() {
            if matches!((self.api.pa_context_get_state)(self.context), 5 | 6) {
                return None;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            if (self.api.pa_mainloop_prepare)(self.mainloop, remaining.as_millis().max(1) as c_int)
                < 0
                || (self.api.pa_mainloop_poll)(self.mainloop) < 0
                || (self.api.pa_mainloop_dispatch)(self.mainloop) < 0
            {
                return None;
            }
        }
        Some(())
    }
    unsafe fn operation(&self, operation: *mut c_void) -> Option<()> {
        if operation.is_null() {
            return None;
        }
        let result = self.wait(|| (self.api.pa_operation_get_state)(operation) != 0);
        (self.api.pa_operation_unref)(operation);
        result
    }
}

unsafe extern "C" fn server(_: *mut c_void, info: *const ServerInfo, data: *mut c_void) {
    if let Some(info) = info.as_ref() {
        if !info.source.is_null() {
            (*(data as *mut Snapshot)).default =
                CStr::from_ptr(info.source).to_string_lossy().into_owned();
        }
    }
}
unsafe extern "C" fn source(
    _: *mut c_void,
    info: *const SourceInfo,
    end: c_int,
    data: *mut c_void,
) {
    if end != 0 {
        return;
    }
    if let Some(info) = info.as_ref() {
        if !info.name.is_null() {
            (*(data as *mut Snapshot))
                .sources
                .push(CStr::from_ptr(info.name).to_string_lossy().into_owned());
        }
    }
}

pub(super) fn snapshot(list_sources: bool) -> Option<Snapshot> {
    unsafe {
        let raw = libc::dlopen(c"libpulse.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if raw.is_null() {
            return None;
        }
        let library = Library(raw);
        let api = Api::load(library.0)?;
        let mainloop = (api.pa_mainloop_new)();
        if mainloop.is_null() {
            return None;
        }
        let mut client = Client {
            api,
            mainloop,
            context: null_mut(),
        };
        client.context = (client.api.pa_context_new)(
            (client.api.pa_mainloop_get_api)(mainloop),
            c"Whispertype input probe".as_ptr(),
        );
        if client.context.is_null() {
            return None;
        }
        // PA_CONTEXT_NOAUTOSPAWN: never launch a server as a side effect.
        if (client.api.pa_context_connect)(client.context, null(), 1, null()) < 0 {
            return None;
        }
        client.wait(|| (client.api.pa_context_get_state)(client.context) == 4)?;
        // Keep callback data alive until after the context is destroyed, even
        // if a query times out with a pending callback.
        let mut result = Snapshot {
            default: String::new(),
            sources: Vec::new(),
        };
        let data = (&mut result as *mut Snapshot).cast();
        let query = (client.api.pa_context_get_server_info)(client.context, server, data);
        let mut ok = client.operation(query).is_some();
        if ok && list_sources {
            let query = (client.api.pa_context_get_source_info_list)(client.context, source, data);
            ok = client.operation(query).is_some();
        }
        drop(client);
        (ok && !result.default.is_empty()).then_some(result)
    }
}
