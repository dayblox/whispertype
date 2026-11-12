use crate::{
    config::{Config, Paths, Shortcut},
    engine::{Event, Snapshot},
};
use anyhow::{ensure, Context, Result};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    io::{Read, Write},
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::SyncSender,
        Mutex, OnceLock,
    },
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};
pub mod http;
pub mod sha256;
mod tray_menu;
mod ui;
mod upgrade;
pub use upgrade::stop_for_upgrade;
pub fn diagnostic_prompt() -> bool {
    unsafe {
        MessageBoxW(null_mut(), wide("Click OK and start speaking straight away: say your saved wake phrase three times in your normal voice, with a pause between repeats. Recording lasts 10 seconds from that click. There is no beep to wait for. Audio is saved only on this computer for troubleshooting and is never uploaded.").as_ptr(), wide("Whispertype — local microphone test").as_ptr(), MB_OKCANCEL | MB_ICONINFORMATION | MB_SETFOREGROUND) == IDOK
    }
}

const CALLBACK: u32 = WM_APP + 1;
const UPDATE: u32 = WM_APP + 2;
const NOTICE: u32 = WM_APP + 3;
const CONTROL: u32 = WM_APP + 4;
const NOTIFICATION_TIMER: usize = 2;
#[cfg(not(test))]
const CLASS: &str = "Whispertype.Native.Tray.v1";
#[cfg(test)]
const CLASS: &str = "Whispertype.Test.Tray.v1";
#[cfg(not(test))]
const INDICATOR_CLASS: &str = "Whispertype.Indicator";
#[cfg(test)]
const INDICATOR_CLASS: &str = "Whispertype.Test.Indicator";
static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);
static SENDER: OnceLock<SyncSender<Event>> = OnceLock::new();
static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
static CAPTURE: AtomicBool = AtomicBool::new(false);
static DOWN: AtomicBool = AtomicBool::new(false);
static CANCELLABLE: AtomicBool = AtomicBool::new(false);
thread_local! {
    static ESCAPE_DOWN: Cell<bool> = const { Cell::new(false) };
}
static HOTKEY: AtomicU32 = AtomicU32::new(VK_SPACE as u32);
static MODIFIERS: AtomicU32 = AtomicU32::new(1);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
thread_local! {
    static ICON:Cell<HICON>=const{Cell::new(null_mut())};
    static OVERLAY:Cell<HWND>=const{Cell::new(null_mut())};
    static OVERLAY_STATE:RefCell<String>=const{RefCell::new(String::new())};
    static PENDING_NOTIFICATION: RefCell<Option<String>> = const { RefCell::new(None) };
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn event(command: &str) {
    if let Some(s) = SENDER.get() {
        let event = match command {
            "press" => Event::Key {
                down: true,
                at: std::time::Instant::now(),
            },
            "release" => Event::Key {
                down: false,
                at: std::time::Instant::now(),
            },
            "cancel" => Event::Cancel,
            _ => Event::Command(command.into()),
        };
        let _ = s.try_send(event);
    }
}
fn hwnd() -> HWND {
    HWND_VALUE.load(Ordering::Relaxed) as HWND
}

pub struct Desktop {
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Desktop {
    pub fn start(
        _paths: &Paths,
        config: &Config,
        sender: SyncSender<Event>,
        startup: bool,
    ) -> Result<Self> {
        set_shortcut(_paths, config)?;
        SENDER
            .set(sender)
            .map_err(|_| anyhow::anyhow!("Desktop already initialized"))?;
        SNAPSHOT
            .set(Mutex::new(Snapshot {
                config: config.clone(),
                state: "idle".into(),
                wake: false,
                startup,
                enrollment: None,
            }))
            .map_err(|_| anyhow::anyhow!("Desktop already initialized"))?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || unsafe {
            let result = (|| -> Result<()> {
                let window = create_window(CLASS, 0, 0, 0, 0, 0, 0)?;
                HWND_VALUE.store(window as isize, Ordering::Relaxed);
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_hook),
                    GetModuleHandleW(null()),
                    0,
                );
                ensure!(!hook.is_null(), "Could not install global keyboard hook");
                tray(NIM_ADD);
                let _ = tx.send(Ok(()));
                let mut message: MSG = zeroed();
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    if ui::dialog_message(&message) {
                        continue;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                clear_overlay();
                ui::close_all();
                UnhookWindowsHookEx(hook);
                tray(NIM_DELETE);
                HWND_VALUE.store(0, Ordering::Relaxed);
                Ok(())
            })();
            if let Err(err) = result {
                let _ = tx.send(Err(format!("{err:#}")));
            }
        });
        rx.recv_timeout(Duration::from_secs(10))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            thread: Some(thread),
        })
    }
    pub fn update(&self, snapshot: Snapshot) {
        CANCELLABLE.store(
            matches!(
                snapshot.state.as_str(),
                "recording" | "transcribing" | "enrolling"
            ),
            Ordering::Relaxed,
        );
        if let Some(s) = SNAPSHOT.get() {
            *s.lock().unwrap() = snapshot;
        }
        unsafe {
            PostMessageW(hwnd(), UPDATE, 0, 0);
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(hwnd(), WM_CLOSE, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe fn create_window(
    class: &str,
    ex: u32,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<HWND> {
    let name = wide(class);
    let instance = GetModuleHandleW(null());
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hIcon: LoadIconW(instance, std::ptr::without_provenance::<u16>(1)),
        hInstance: instance,
        lpszClassName: name.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let window = CreateWindowExW(
        ex,
        name.as_ptr(),
        name.as_ptr(),
        style,
        x,
        y,
        w,
        h,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    ensure!(
        !window.is_null(),
        "Could not create native window: {}",
        std::io::Error::last_os_error()
    );
    Ok(window)
}
unsafe extern "system" fn window_proc(window: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        CALLBACK => {
            if matches!(l as u32, WM_RBUTTONUP | WM_LBUTTONUP) {
                tray_menu::show(window);
            }
            0
        }
        UPDATE => {
            tray(NIM_MODIFY);
            if let Err(error) = sync_overlay() {
                eprintln!("Indicator unavailable: {error:#}");
            }
            ui::sync_enrollment();
            tray_menu::refresh();
            0
        }
        WM_MENUSELECT => {
            tray_menu::select(w, l);
            0
        }
        NOTICE => {
            let message = Box::from_raw(l as *mut String);
            queue_notification(window, *message);
            0
        }
        CONTROL => {
            if let Some(command) = CONTROL_COMMANDS.get(w) {
                event(command);
            }
            0
        }
        WM_TIMER if w == NOTIFICATION_TIMER => {
            KillTimer(window, NOTIFICATION_TIMER);
            if let Some(message) = PENDING_NOTIFICATION.with(|pending| pending.borrow_mut().take())
            {
                shell_notification(window, &message);
            }
            0
        }
        WM_TIMER => {
            PostQuitMessage(0);
            0
        }
        WM_CLOSE => {
            DestroyWindow(window);
            0
        }
        WM_DESTROY => {
            // The tray and its non-activating overlay share this message loop.
            if window == hwnd() || hwnd().is_null() {
                PostQuitMessage(0);
            }
            0
        }
        _ => {
            let taskbar = *TASKBAR_CREATED
                .get_or_init(|| RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
            if taskbar != 0
                && msg == taskbar
                && SNAPSHOT.get().is_some()
                && !CAPTURE.load(Ordering::Relaxed)
            {
                tray(NIM_ADD);
                return 0;
            }
            DefWindowProcW(window, msg, w, l)
        }
    }
}
unsafe fn copy_wide<const N: usize>(target: &mut [u16; N], s: &str) {
    for (to, from) in target.iter_mut().take(N - 1).zip(s.encode_utf16()) {
        *to = from;
    }
}
unsafe fn tray(action: NOTIFY_ICON_MESSAGE) {
    let snapshot = SNAPSHOT.get().unwrap().lock().unwrap().clone();
    if !snapshot.config.tray {
        return;
    }
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd();
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = CALLBACK;
    let icon = make_icon(snapshot.state == "recording");
    data.hIcon = icon;
    copy_wide(
        &mut data.szTip,
        &format!(
            "Whispertype · {} · {}",
            snapshot.state, snapshot.config.shortcut
        ),
    );
    Shell_NotifyIconW(action, &data);
    ICON.with(|old| {
        let previous = old.replace(icon);
        if !previous.is_null() {
            DestroyIcon(previous);
        }
    });
}
unsafe fn make_icon(active: bool) -> HICON {
    let mut bgra = [0u8; 32 * 32 * 4];
    let color = if active {
        [106, 206, 158]
    } else {
        [200, 190, 180]
    };
    for (bar, height) in [8, 16, 24, 16, 8].iter().enumerate() {
        for y in (16 - height / 2)..(16 + height / 2) {
            for x in (4 + bar * 5)..(7 + bar * 5) {
                let at = (y * 32 + x) * 4;
                bgra[at..at + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            }
        }
    }
    CreateIcon(
        GetModuleHandleW(null()),
        32,
        32,
        1,
        32,
        [0u8; 128].as_ptr(),
        bgra.as_ptr(),
    )
}

unsafe fn append(menu: HMENU, id: usize, label: &str, checked: bool) {
    AppendMenuW(
        menu,
        MF_STRING | if checked { MF_CHECKED } else { 0 },
        id,
        wide(label).as_ptr(),
    );
}
unsafe fn create_menu(s: &Snapshot) -> HMENU {
    let root = CreatePopupMenu();
    append(root, 1, &format!("Whispertype · {}", s.state), false);
    EnableMenuItem(root, 1, MF_BYCOMMAND | MF_GRAYED);
    let shortcuts = CreatePopupMenu();
    append(shortcuts, 10, "Record shortcut…", false);
    for (i, key) in [
        "CTRL + SPACE",
        "CTRL + ALT + SPACE",
        "SUPER + ALT + SPACE",
        "F9",
    ]
    .iter()
    .enumerate()
    {
        append(shortcuts, 11 + i, key, s.config.shortcut == *key);
    }
    AppendMenuW(
        root,
        MF_POPUP,
        shortcuts as usize,
        wide(&format!("Shortcut: {}", s.config.shortcut)).as_ptr(),
    );
    let modes = CreatePopupMenu();
    for (i, mode) in [
        crate::activation::Mode::Auto,
        crate::activation::Mode::Hold,
        crate::activation::Mode::Toggle,
    ]
    .iter()
    .enumerate()
    {
        append(modes, 20 + i, mode.label(), s.config.mode == *mode);
    }
    AppendMenuW(
        root,
        MF_POPUP,
        modes as usize,
        wide(&format!("Activation: {}", s.config.mode.label())).as_ptr(),
    );
    append(
        root,
        30,
        &format!("Wake-word listening ({})", s.config.wake_phrase),
        s.wake,
    );
    append(root, 31, "Record personal wake phrase…", false);
    append(root, 45, "Test personal wake phrase…", false);
    AppendMenuW(root, MF_SEPARATOR, 0, null());
    append(root, 44, "Transcription prompt…", false);
    append(root, 43, "Launch at startup", s.startup);
    append(root, 42, "Check for updates", false);
    append(root, 41, "Quit", false);
    root
}
fn vk(shortcut: &Shortcut) -> u32 {
    match shortcut.key.as_str() {
        "SPACE" => VK_SPACE as u32,
        k if k.starts_with('F') && k.len() > 1 => VK_F1 as u32 + k[1..].parse::<u32>().unwrap() - 1,
        k => k.as_bytes()[0] as u32,
    }
}
unsafe fn pressed(key: VIRTUAL_KEY) -> bool {
    GetAsyncKeyState(key as i32) < 0
}
unsafe fn modifiers() -> (bool, bool, bool, bool) {
    (
        pressed(VK_CONTROL),
        pressed(VK_MENU),
        pressed(VK_SHIFT),
        pressed(VK_LWIN) || pressed(VK_RWIN),
    )
}
unsafe extern "system" fn keyboard_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let key = &*(l as *const KBDLLHOOKSTRUCT);
    if key.flags & LLKHF_INJECTED != 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
    let up = w as u32 == WM_KEYUP || w as u32 == WM_SYSKEYUP;
    if CAPTURE.load(Ordering::Relaxed) {
        if down {
            if key.vkCode == VK_ESCAPE as u32 {
                PostQuitMessage(0);
                return 1;
            }
            let name = if key.vkCode == VK_SPACE as u32 {
                "SPACE".to_string()
            } else if (VK_F1 as u32..=VK_F24 as u32).contains(&key.vkCode) {
                format!("F{}", key.vkCode - VK_F1 as u32 + 1)
            } else if (0x30..=0x5a).contains(&key.vkCode) {
                char::from_u32(key.vkCode).unwrap().to_string()
            } else {
                return CallNextHookEx(null_mut(), code, w, l);
            };
            let (ctrl, alt, shift, win) = modifiers();
            let mut parts = Vec::new();
            for (on, name) in [
                (ctrl, "CTRL"),
                (alt, "ALT"),
                (shift, "SHIFT"),
                (win, "SUPER"),
            ] {
                if on {
                    parts.push(name.to_string());
                }
            }
            parts.push(name);
            println!("{}", parts.join(" + "));
            let _ = std::io::stdout().flush();
            PostQuitMessage(0);
            return 1;
        }
    } else {
        // The hook sees every key: unmatched keys must avoid locks and allocation.
        if key.vkCode == VK_ESCAPE as u32 {
            if up && ESCAPE_DOWN.with(|pressed| pressed.replace(false)) {
                return 1;
            }
            if down
                && (ESCAPE_DOWN.with(Cell::get)
                    || (CANCELLABLE.load(Ordering::Relaxed) && !ui::prompt_has_focus()))
            {
                if !ESCAPE_DOWN.with(|pressed| pressed.replace(true)) {
                    event("cancel");
                    // Escape also dismisses an open native menu. Subsequent
                    // key repeats are swallowed until the physical release.
                    if tray_menu::is_open() {
                        EndMenu();
                    }
                }
                return 1;
            }
        }
        if key.vkCode == HOTKEY.load(Ordering::Relaxed) {
            if up && DOWN.swap(false, Ordering::Relaxed) {
                event("release");
                return 1;
            }
            if down
                && (DOWN.load(Ordering::Relaxed)
                    || modifier_bits(modifiers()) == MODIFIERS.load(Ordering::Relaxed))
            {
                if !DOWN.swap(true, Ordering::Relaxed) {
                    event("press");
                }
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, w, l)
}

const CONTROL_COMMANDS: [&str; 22] = [
    "press",
    "release",
    "toggle",
    "start",
    "stop",
    "cancel",
    "wake-on",
    "wake-off",
    "enroll",
    "paste",
    "reload",
    "quit",
    "wake-default",
    "shortcut-capture",
    "check-updates",
    "startup-toggle",
    "enroll-next",
    "enroll-cancel",
    "prompt-edit",
    "wake-test",
    "_profile-wake-record",
    "_profile-stats",
];
pub fn send(command: &str) -> Result<()> {
    unsafe {
        let window = FindWindowW(wide(CLASS).as_ptr(), null());
        ensure!(!window.is_null(), "Whispertype is not running");
        let id = CONTROL_COMMANDS
            .iter()
            .position(|x| *x == command)
            .context("Unknown control command")?;
        ensure!(
            PostMessageW(window, CONTROL, id, 0) != 0,
            "Could not send command"
        );
        Ok(())
    }
}

pub fn edit_prompt(current: &str) -> Result<Option<String>> {
    unsafe { ui::edit_prompt(current) }
}
pub fn notify(message: &str) {
    unsafe {
        let window = hwnd();
        if !window.is_null() {
            let text = Box::into_raw(Box::new(message.to_string()));
            if PostMessageW(window, NOTICE, 0, text as isize) == 0 {
                drop(Box::from_raw(text));
            }
        }
    }
}
pub fn update_notification(message: &str) {
    notify(message);
}
unsafe fn shell_notification(window: HWND, message: &str) {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = 1;
    data.uFlags = NIF_INFO | NIF_REALTIME;
    data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
    copy_wide(&mut data.szInfoTitle, "Whispertype");
    copy_wide(&mut data.szInfo, message);
    Shell_NotifyIconW(NIM_MODIFY, &data);
}
unsafe fn queue_notification(window: HWND, message: String) {
    // An empty NIF_INFO removes our currently visible Windows notification.
    // Give Explorer time to dismiss it, coalesce bursts, then show only the
    // latest result. No long-lived timer or notification FIFO is maintained.
    shell_notification(window, "");
    PENDING_NOTIFICATION.with(|pending| *pending.borrow_mut() = Some(message));
    SetTimer(window, NOTIFICATION_TIMER, 200, None);
}
pub fn open_releases() -> Result<()> {
    unsafe {
        ensure!(
            ShellExecuteW(
                null_mut(),
                wide("open").as_ptr(),
                wide(super::RELEASES_URL).as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL
            ) as isize
                > 32,
            "Could not open the release download page"
        );
    }
    Ok(())
}
pub fn type_text(text: &str) -> Result<()> {
    send_unicode(text, |inputs| unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        ) as usize
    })
}

fn send_unicode(text: &str, mut send: impl FnMut(&[INPUT]) -> usize) -> Result<()> {
    // Bulk Unicode packets corrupted/dropped ordinary ASCII in modern Notepad
    // during the user's live test. One scalar's down/up pair(s) per call worked.
    // Reuse fixed storage: no clipboard access, allocation, sleeps or input guard.
    let mut inputs: [INPUT; 4] = unsafe { zeroed() };
    for ch in text.chars() {
        let mut encoded = [0; 2];
        let units = ch.encode_utf16(&mut encoded);
        let mut count = 0;
        for &unit in units.iter() {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                inputs[count].r#type = INPUT_KEYBOARD;
                inputs[count].Anonymous.ki = KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                };
                count += 1;
            }
        }
        let sent = send(&inputs[..count]);
        if sent != count {
            // Do not resend characters: part of the text may already be inserted.
            // Best-effort releases cannot add text if a down event was accepted.
            if sent > 0 {
                let releases = [inputs[1], inputs[3]];
                let _ = send(&releases[..units.len()]);
            }
            anyhow::bail!("Windows blocked typing; target may be elevated or on a secure desktop");
        }
    }
    Ok(())
}
pub fn set_shortcut(_paths: &Paths, config: &Config) -> Result<()> {
    let shortcut = Shortcut::parse(&config.shortcut)?;
    HOTKEY.store(vk(&shortcut), Ordering::Relaxed);
    MODIFIERS.store(
        modifier_bits((
            shortcut.ctrl,
            shortcut.alt,
            shortcut.shift,
            shortcut.super_key,
        )),
        Ordering::Relaxed,
    );
    DOWN.store(false, Ordering::Relaxed);
    Ok(())
}

fn modifier_bits((ctrl, alt, shift, win): (bool, bool, bool, bool)) -> u32 {
    ctrl as u32 | (alt as u32) << 1 | (shift as u32) << 2 | (win as u32) << 3
}

pub fn capture_shortcut() -> Result<()> {
    unsafe {
        CAPTURE.store(true, Ordering::Relaxed);
        let window = create_window("Whispertype.ShortcutCapture", 0, 0, 0, 0, 0, 0)?;
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            GetModuleHandleW(null()),
            0,
        );
        ensure!(!hook.is_null(), "Could not capture keyboard shortcut");
        SetTimer(window, 1, 20000, None);
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(hook);
        DestroyWindow(window);
        Ok(())
    }
}

unsafe fn clear_overlay() {
    OVERLAY.with(|slot| {
        let window = slot.replace(null_mut());
        if !window.is_null() {
            DestroyWindow(window);
        }
    });
    OVERLAY_STATE.with(|state| state.borrow_mut().clear());
}
unsafe fn sync_overlay() -> Result<()> {
    let snapshot = SNAPSHOT.get().unwrap().lock().unwrap().clone();
    let desired = if snapshot.config.indicator
        && matches!(
            snapshot.state.as_str(),
            "recording" | "transcribing" | "enrolling"
        ) {
        snapshot.state.as_str()
    } else {
        ""
    };
    if OVERLAY_STATE.with(|s| s.borrow().as_str() == desired) {
        return Ok(());
    }
    clear_overlay();
    if !desired.is_empty() {
        let window = create_overlay(desired)?;
        OVERLAY.with(|s| s.set(window));
        OVERLAY_STATE.with(|s| *s.borrow_mut() = desired.into());
    }
    Ok(())
}
unsafe fn create_overlay(kind: &str) -> Result<HWND> {
    let width = (super::INDICATOR_WIDTH * super::INDICATOR_SCALE) as i32;
    let height = (super::INDICATOR_HEIGHT * super::INDICATOR_SCALE) as i32;
    let foreground = GetForegroundWindow();
    let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    ensure!(
        GetMonitorInfoW(monitor, &mut info) != 0,
        "Could not identify the indicator monitor"
    );
    let x = (info.rcMonitor.left + info.rcMonitor.right - width) / 2;
    let y = info.rcMonitor.bottom - height;
    let window = create_window(
        INDICATOR_CLASS,
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        WS_POPUP,
        x,
        y,
        width,
        height,
    )?;
    let dc = GetDC(null_mut());
    let memory = CreateCompatibleDC(dc);
    let mut bitmap: BITMAPINFO = zeroed();
    bitmap.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(dc, &bitmap, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if dib.is_null() || bits.is_null() {
        if !dib.is_null() {
            DeleteObject(dib);
        }
        DeleteDC(memory);
        ReleaseDC(null_mut(), dc);
        DestroyWindow(window);
        anyhow::bail!("Could not create indicator bitmap");
    }
    let pixels = super::pixels(kind, width as usize, height as usize);
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
    let previous = SelectObject(memory, dib);
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let shown = UpdateLayeredWindow(
        window,
        dc,
        &POINT { x, y },
        &SIZE {
            cx: width,
            cy: height,
        },
        memory,
        &POINT { x: 0, y: 0 },
        0,
        &blend,
        ULW_ALPHA,
    );
    SelectObject(memory, previous);
    DeleteObject(dib);
    DeleteDC(memory);
    ReleaseDC(null_mut(), dc);
    if shown == 0 {
        DestroyWindow(window);
        anyhow::bail!("Could not show indicator");
    }
    ShowWindow(window, SW_SHOWNOACTIVATE);
    Ok(window)
}

// Kept for standalone diagnostics. Normal recording uses the tray's existing
// thread/window loop, with no child process or additional indicator thread.
pub fn indicator(kind: &str) -> Result<()> {
    unsafe {
        let window = create_overlay(kind)?;
        let raw = window as isize;
        std::thread::spawn(move || {
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            PostMessageW(raw as HWND, WM_CLOSE, 0, 0);
        });
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
        Ok(())
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn startup_enabled() -> Result<bool> {
    unsafe { startup_value_exists(RUN_KEY) }
}

unsafe fn startup_value_exists(path: &str) -> Result<bool> {
    let mut bytes = 0;
    let status = RegGetValueW(
        HKEY_CURRENT_USER,
        wide(path).as_ptr(),
        wide("Whispertype").as_ptr(),
        RRF_RT_REG_SZ,
        null_mut(),
        null_mut(),
        &mut bytes,
    );
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(false);
    }
    ensure!(
        status == ERROR_SUCCESS,
        "Could not read startup registration: {}",
        std::io::Error::from_raw_os_error(status as i32)
    );
    Ok(bytes > 2)
}

pub fn set_startup(enabled: bool) -> Result<()> {
    let value = format!("\"{}\" run", std::env::current_exe()?.display());
    unsafe { write_startup_value(RUN_KEY, enabled.then_some(value.as_str())) }
}

unsafe fn write_startup_value(path: &str, command: Option<&str>) -> Result<()> {
    let mut key = null_mut();
    let status = RegCreateKeyExW(
        HKEY_CURRENT_USER,
        wide(path).as_ptr(),
        0,
        null(),
        REG_OPTION_NON_VOLATILE,
        KEY_SET_VALUE,
        null(),
        &mut key,
        null_mut(),
    );
    ensure!(
        status == ERROR_SUCCESS,
        "Could not open startup registration: {}",
        std::io::Error::from_raw_os_error(status as i32)
    );
    let status = if let Some(command) = command {
        let value = wide(command);
        RegSetValueExW(
            key,
            wide("Whispertype").as_ptr(),
            0,
            REG_SZ,
            value.as_ptr() as *const u8,
            (value.len() * size_of::<u16>()) as u32,
        )
    } else {
        RegDeleteValueW(key, wide("Whispertype").as_ptr())
    };
    RegCloseKey(key);
    ensure!(
        status == ERROR_SUCCESS || command.is_none() && status == ERROR_FILE_NOT_FOUND,
        "Could not change startup registration: {}",
        std::io::Error::from_raw_os_error(status as i32)
    );
    Ok(())
}

pub fn install(paths: &Paths, _config: &Config) -> Result<()> {
    std::fs::create_dir_all(&paths.root)?;
    set_startup(true)
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
