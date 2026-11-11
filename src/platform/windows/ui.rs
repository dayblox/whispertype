use super::*;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS};

const ENROLL_ROLE: isize = 2;
const PROMPT_ROLE: isize = 3;
const ACTION: usize = 100;
const CANCEL: usize = 101;
const TEXT: i32 = 102;
const TEST: usize = 103;
const HEADING: usize = 104;
// Standard STATIC/EDIT control messages (winuser.h).
const EM_SETLIMITTEXT: u32 = 0x00c5;
const EM_SETSEL: u32 = 0x00b1;
#[cfg(not(test))]
const UI_CLASS: &str = "Whispertype.Feedback.v2";
#[cfg(test)]
const UI_CLASS: &str = "Whispertype.Test.Feedback.v2";
thread_local! {
    static ENROLL: Cell<HWND> = const { Cell::new(null_mut()) };
    static PROMPT_REPLY: RefCell<Option<String>> = const { RefCell::new(None) };
    static THEME: Theme = unsafe { Theme::new() };
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
const SURFACE: u32 = rgb(23, 27, 36);
const FOREGROUND: u32 = rgb(235, 240, 246);
const ACCENT: u32 = rgb(158, 206, 106);
const EDIT_SURFACE: u32 = rgb(31, 37, 48);
struct Theme {
    brush: HBRUSH,
    edit_brush: HBRUSH,
    normal: HFONT,
    heading: HFONT,
}
impl Theme {
    unsafe fn new() -> Self {
        let font = |height, weight| {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        Self {
            brush: CreateSolidBrush(SURFACE),
            edit_brush: CreateSolidBrush(EDIT_SURFACE),
            normal: font(-16, FW_NORMAL as i32),
            heading: font(-17, FW_SEMIBOLD as i32),
        }
    }
}
impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.brush);
            DeleteObject(self.edit_brush);
            DeleteObject(self.normal);
            DeleteObject(self.heading);
        }
    }
}
unsafe fn paint_button(data: &DRAWITEMSTRUCT) {
    THEME.with(|theme| {
        let primary = data.CtlID == ACTION as u32;
        let color = if data.itemState & ODS_DISABLED != 0 {
            rgb(43, 50, 62)
        } else if primary {
            ACCENT
        } else {
            rgb(49, 57, 73)
        };
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(data.hDC, brush);
        let old_pen = SelectObject(data.hDC, GetStockObject(NULL_PEN));
        let r = data.rcItem;
        // Owner-drawn controls start with the system button background. Paint
        // the entire rectangle before rounding so its corners cannot stay white.
        FillRect(data.hDC, &r, theme.brush);
        RoundRect(data.hDC, r.left, r.top, r.right, r.bottom, 12, 12);
        SelectObject(data.hDC, old_pen);
        SelectObject(data.hDC, old_brush);
        DeleteObject(brush);
        let old = SelectObject(data.hDC, theme.normal);
        SetBkMode(data.hDC, TRANSPARENT as i32);
        SetTextColor(
            data.hDC,
            if primary && data.itemState & ODS_DISABLED == 0 {
                SURFACE
            } else {
                FOREGROUND
            },
        );
        let mut text = [0u16; 100];
        let n = GetWindowTextW(data.hwndItem, text.as_mut_ptr(), 100);
        let mut rect = r;
        DrawTextW(
            data.hDC,
            text.as_ptr(),
            n,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        if data.itemState & ODS_FOCUS != 0 {
            InflateRect(&mut rect, -4, -4);
            DrawFocusRect(data.hDC, &rect);
        }
        SelectObject(data.hDC, old);
    });
}

unsafe fn shell(role: isize, title: &str, width: i32, height: i32) -> Result<HWND> {
    let instance = GetModuleHandleW(null());
    let name = wide(UI_CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        lpszClassName: name.as_ptr(),
        hbrBackground: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hIcon: LoadIconW(instance, std::ptr::without_provenance::<u16>(1)),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    ensure!(
        GetMonitorInfoW(monitor, &mut info) != 0,
        "Could not locate display"
    );
    let (ex, style) = (WS_EX_TOOLWINDOW, WS_CAPTION | WS_SYSMENU | WS_OVERLAPPED);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    AdjustWindowRectEx(&mut rect, style, 0, ex);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let (x, y) = (
        (info.rcWork.left + info.rcWork.right - width) / 2,
        (info.rcWork.top + info.rcWork.bottom - height) / 2,
    );
    let window = CreateWindowExW(
        ex,
        name.as_ptr(),
        wide(title).as_ptr(),
        style,
        x,
        y,
        width,
        height,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    ensure!(!window.is_null(), "Could not create feedback window");
    SetWindowLongPtrW(window, GWLP_USERDATA, role);
    let dark = 1i32;
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        window,
        20,
        &dark as *const _ as *const c_void,
        4,
    );
    for (attribute, value) in [(33, 2u32), (34, SURFACE), (35, SURFACE), (36, FOREGROUND)] {
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            window,
            attribute,
            &value as *const _ as *const c_void,
            4,
        );
    }
    Ok(window)
}

unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    rect: [i32; 4],
    id: usize,
) -> HWND {
    let child = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        rect[0],
        rect[1],
        rect[2],
        rect[3],
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(
        child,
        WM_SETFONT,
        THEME.with(|theme| theme.normal as usize),
        1,
    );
    child
}

unsafe extern "system" fn proc(window: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let role = GetWindowLongPtrW(window, GWLP_USERDATA);
    match msg {
        WM_ERASEBKGND => {
            let mut rect = zeroed();
            GetClientRect(window, &mut rect);
            THEME.with(|theme| FillRect(w as HDC, &rect, theme.brush));
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
            SetTextColor(w as HDC, FOREGROUND);
            let edit = msg == WM_CTLCOLOREDIT;
            SetBkColor(w as HDC, if edit { EDIT_SURFACE } else { SURFACE });
            THEME.with(|theme| if edit { theme.edit_brush } else { theme.brush } as isize)
        }
        WM_DRAWITEM => {
            paint_button(&*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_NEXTDLGCTL => {
            let next = if l != 0 {
                w as HWND
            } else {
                GetNextDlgTabItem(window, GetFocus(), (w != 0) as i32)
            };
            if !next.is_null() {
                SetFocus(next);
            }
            0
        }
        WM_COMMAND => {
            let command = w & 0xffff;
            if role == ENROLL_ROLE {
                if command == ACTION || command == IDOK as usize {
                    event("enroll-next");
                }
                if command == CANCEL || command == IDCANCEL as usize {
                    event("enroll-cancel");
                }
                if command == TEST {
                    event("wake-test");
                }
            } else if role == PROMPT_ROLE {
                if command == ACTION || command == IDOK as usize {
                    let edit = GetDlgItem(window, TEXT);
                    let len = GetWindowTextLengthW(edit).max(0) as usize;
                    let mut text = vec![0u16; len + 1];
                    GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32);
                    let text = String::from_utf16_lossy(&text[..len]);
                    if text.len() > 1024 {
                        MessageBoxW(window, wide("Keep the prompt under 1024 UTF-8 bytes. Short spelling and context hints work best.").as_ptr(),
                            wide("Prompt too long").as_ptr(), MB_OK | MB_ICONINFORMATION);
                        return 0;
                    }
                    PROMPT_REPLY.with(|reply| *reply.borrow_mut() = Some(text));
                    DestroyWindow(window);
                } else if command == CANCEL || command == IDCANCEL as usize {
                    DestroyWindow(window);
                }
            }
            0
        }
        WM_CLOSE => {
            if role == ENROLL_ROLE {
                event("enroll-cancel");
            }
            DestroyWindow(window);
            0
        }
        WM_DESTROY => {
            if role == ENROLL_ROLE {
                ENROLL.with(|slot| {
                    if slot.get() == window {
                        slot.set(null_mut());
                    }
                });
            }
            if role == PROMPT_ROLE {
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(window, msg, w, l),
    }
}

unsafe fn enrollment_window() -> Result<HWND> {
    let window = shell(ENROLL_ROLE, "Personal wake phrase", 560, 232)?;
    let title = control(window, "STATIC", "", 0, [24, 22, 500, 28], HEADING);
    SendMessageW(
        title,
        WM_SETFONT,
        THEME.with(|theme| theme.heading as usize),
        1,
    );
    control(window, "STATIC", "", 0, [24, 62, 512, 92], TEXT as usize);
    control(
        window,
        "BUTTON",
        "Record sample",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [24, 170, 180, 38],
        ACTION,
    );
    control(
        window,
        "BUTTON",
        "Cancel",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [416, 170, 120, 38],
        CANCEL,
    );
    control(
        window,
        "BUTTON",
        "Test phrase",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [220, 170, 180, 38],
        TEST,
    );
    Ok(window)
}

pub(super) unsafe fn sync_enrollment() {
    let view = SNAPSHOT
        .get()
        .and_then(|snapshot| snapshot.lock().ok().and_then(|s| s.enrollment.clone()));
    let current = ENROLL.with(Cell::get);
    let Some(view) = view else {
        if !current.is_null() {
            DestroyWindow(current);
        }
        return;
    };
    let window = if current.is_null() {
        let Ok(window) = enrollment_window() else {
            return;
        };
        ENROLL.with(|slot| slot.set(window));
        ShowWindow(window, SW_SHOWNORMAL);
        SetForegroundWindow(window);
        window
    } else {
        current
    };
    apply_enrollment_view(window, &view);
}
pub(super) unsafe fn dialog_message(message: &MSG) -> bool {
    let window = ENROLL.with(Cell::get);
    !window.is_null() && IsDialogMessageW(window, message) != 0
}

unsafe fn apply_enrollment_view(window: HWND, view: &crate::enrollment::View) {
    SetWindowTextW(GetDlgItem(window, TEXT), wide(&view.text).as_ptr());
    SetWindowTextW(
        GetDlgItem(window, HEADING as i32),
        wide(&view.title).as_ptr(),
    );
    SetWindowTextW(
        GetDlgItem(window, ACTION as i32),
        wide(&view.action).as_ptr(),
    );
    EnableWindow(GetDlgItem(window, ACTION as i32), view.enabled as i32);
    ShowWindow(
        GetDlgItem(window, TEST as i32),
        if view.done { SW_SHOW } else { SW_HIDE },
    );
    SetWindowTextW(
        GetDlgItem(window, CANCEL as i32),
        wide(if view.done { "Close" } else { "Cancel" }).as_ptr(),
    );
}

unsafe fn prompt_window(current: &str) -> Result<HWND> {
    let window = shell(PROMPT_ROLE, "Transcription prompt", 540, 360)?;
    let heading = control(
        window,
        "STATIC",
        "Context and spellings",
        0,
        [24, 22, 492, 28],
        HEADING,
    );
    SendMessageW(
        heading,
        WM_SETFONT,
        THEME.with(|theme| theme.heading as usize),
        1,
    );
    control(window, "STATIC", "Optional vocabulary and context hints for Groq, not chat instructions. Keep it concise (up to 224 tokens). Clear the field to disable it.", 0, [24, 62, 492, 58], 0);
    let edit = control(
        window,
        "EDIT",
        current,
        WS_TABSTOP | WS_BORDER | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
        [24, 132, 492, 140],
        TEXT as usize,
    );
    SendMessageW(edit, EM_SETLIMITTEXT, 1024, 0);
    control(
        window,
        "BUTTON",
        "Save",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [280, 298, 112, 38],
        ACTION,
    );
    control(
        window,
        "BUTTON",
        "Cancel",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [404, 298, 112, 38],
        CANCEL,
    );
    Ok(window)
}

pub(super) unsafe fn edit_prompt(current: &str) -> Result<Option<String>> {
    PROMPT_REPLY.with(|reply| *reply.borrow_mut() = None);
    let window = prompt_window(current)?;
    ShowWindow(window, SW_SHOWNORMAL);
    SetForegroundWindow(window);
    SetFocus(GetDlgItem(window, TEXT));
    let mut message: MSG = zeroed();
    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
        prompt_message(window, &message);
    }
    Ok(PROMPT_REPLY.with(|reply| reply.borrow_mut().take()))
}

unsafe fn prompt_message(window: HWND, message: &MSG) {
    // Multiline EDIT requests all keys, including Escape, from IsDialogMessage.
    if message.message == WM_KEYDOWN && message.wParam == VK_ESCAPE as usize {
        SendMessageW(window, WM_COMMAND, CANCEL, 0);
        return;
    }
    // Process edit accelerators before dialog navigation/TranslateMessage.
    // The classic multiline EDIT does not implement Ctrl+A itself.
    if message.message == WM_KEYDOWN
        && message.hwnd == GetDlgItem(window, TEXT)
        && GetKeyState(VK_CONTROL as i32) < 0
        && GetKeyState(VK_MENU as i32) >= 0
        && edit_shortcut(message.hwnd, message.wParam as u32)
    {
        return;
    }
    if IsDialogMessageW(window, message) == 0 {
        TranslateMessage(message);
        DispatchMessageW(message);
    }
}

unsafe fn edit_shortcut(edit: HWND, key: u32) -> bool {
    match key {
        0x41 => {
            SendMessageW(edit, EM_SETSEL, 0, -1);
        }
        0x43 => {
            SendMessageW(edit, WM_COPY, 0, 0);
        }
        0x58 => {
            SendMessageW(edit, WM_CUT, 0, 0);
        }
        0x56 => {
            SendMessageW(edit, WM_PASTE, 0, 0);
        }
        0x5a => {
            SendMessageW(edit, WM_UNDO, 0, 0);
        }
        _ => return false,
    }
    true
}

pub(super) unsafe fn close_all() {
    let window = ENROLL.with(Cell::get);
    if !window.is_null() {
        DestroyWindow(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe fn capture_client(window: HWND) -> (i32, i32, Vec<u8>) {
        // Render a real native window off-screen without activating it or
        // interrupting the user's current window. Hidden windows may print black.
        SetWindowPos(
            window,
            null_mut(),
            -20000,
            -20000,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
        );
        ShowWindow(window, SW_SHOWNOACTIVATE);
        RedrawWindow(
            window,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
        let mut rect = zeroed();
        GetClientRect(window, &mut rect);
        let (width, height) = (rect.right, rect.bottom);
        let screen = GetDC(null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        assert!(!bitmap.is_null());
        let old = SelectObject(dc, bitmap);
        assert_ne!(
            windows_sys::Win32::Storage::Xps::PrintWindow(window, dc, 3),
            0
        );
        SendMessageW(window, WM_ERASEBKGND, dc as usize, 0);
        // Off-screen DWM snapshots can omit child controls. Ask each real
        // native control to render synchronously into the same device context.
        let mut child = GetWindow(window, GW_CHILD);
        while !child.is_null() {
            if GetWindowLongPtrW(child, GWL_STYLE) as u32 & WS_VISIBLE != 0 {
                let mut bounds: RECT = zeroed();
                GetWindowRect(child, &mut bounds);
                MapWindowPoints(null_mut(), window, &mut bounds as *mut _ as *mut POINT, 2);
                let saved = SaveDC(dc);
                SetViewportOrgEx(dc, bounds.left, bounds.top, null_mut());
                SendMessageW(
                    child,
                    WM_PRINT,
                    dc as usize,
                    (PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND) as isize,
                );
                RestoreDC(dc, saved);
            }
            child = GetWindow(child, GW_HWNDNEXT);
        }
        GdiFlush();
        let bytes =
            std::slice::from_raw_parts(pixels as *const u8, (width * height * 4) as usize).to_vec();
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(null_mut(), screen);
        (width, height, bytes)
    }
    unsafe fn inspect_form(window: HWND, name: &str, buttons: &[i32]) {
        let (width, height, bytes) = capture_client(window);
        if let Some(folder) = std::env::var_os("WHISPERTYPE_UI_CAPTURE") {
            let folder = std::path::PathBuf::from(folder);
            std::fs::create_dir_all(&folder).unwrap();
            let mut bmp = Vec::with_capacity(bytes.len() + 54);
            bmp.extend_from_slice(b"BM");
            bmp.extend_from_slice(&((bytes.len() + 54) as u32).to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&54u32.to_le_bytes());
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&width.to_le_bytes());
            bmp.extend_from_slice(&(-height).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&[0; 24]);
            bmp.extend_from_slice(&bytes);
            std::fs::write(folder.join(format!("{name}.bmp")), bmp).unwrap();
        }
        let pixel = |x: i32, y: i32| {
            let at = ((y * width + x) * 4) as usize;
            rgb(bytes[at + 2] as u32, bytes[at + 1] as u32, bytes[at] as u32)
        };
        assert_eq!(pixel(8, 8), SURFACE, "Window background was not rendered");
        for id in buttons {
            let child = GetDlgItem(window, *id);
            let mut rect: RECT = zeroed();
            GetWindowRect(child, &mut rect);
            MapWindowPoints(null_mut(), window, &mut rect as *mut _ as *mut POINT, 2);
            assert!(
                rect.left >= 0 && rect.top >= 0 && rect.right <= width && rect.bottom <= height
            );
            for (x, y) in [
                (rect.left, rect.top),
                (rect.right - 1, rect.top),
                (rect.left, rect.bottom - 1),
                (rect.right - 1, rect.bottom - 1),
            ] {
                assert_eq!(
                    pixel(x, y),
                    SURFACE,
                    "Button {id} left a light corner in {name}"
                );
            }
        }
    }
    #[test]
    fn native_form_pixels_cover_button_corners_and_controls_fit_client_area() {
        unsafe {
            let window = enrollment_window().unwrap();
            let mut e = crate::enrollment::Enrollment::new();
            apply_enrollment_view(window, &e.view(std::time::Instant::now()));
            inspect_form(window, "enrollment-ready", &[ACTION as i32, CANCEL as i32]);
            let now = std::time::Instant::now();
            e.arm_test(now);
            e.tick(now + std::time::Duration::from_secs(2));
            e.probe(&crate::wake::ProbeReport {
                frames: 8,
                speech_frames: 3,
                peak_rms: 0.02,
                best_score: 0.61,
                target_score: 0.52,
                partial_frames: 2,
                recognized: false,
                detections: 0,
            });
            apply_enrollment_view(window, &e.view(now));
            inspect_form(
                window,
                "enrollment-testing",
                &[ACTION as i32, CANCEL as i32],
            );
            e.finish(
                "Your personal phrase is saved. Test it with your microphone, or enable listening."
                    .into(),
            );
            apply_enrollment_view(window, &e.view(std::time::Instant::now()));
            inspect_form(
                window,
                "enrollment-saved",
                &[ACTION as i32, TEST as i32, CANCEL as i32],
            );
            DestroyWindow(window);
            let window = prompt_window("Whispertype, Groq, Dayblox").unwrap();
            assert_eq!(
                GetWindowLongPtrW(GetDlgItem(window, TEXT), GWL_STYLE) as u32 & WS_VSCROLL,
                0
            );
            inspect_form(window, "prompt", &[ACTION as i32, CANCEL as i32]);
            DestroyWindow(window);
            let mut msg: MSG = zeroed();
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
    }
    #[test]
    fn prompt_editor_preserves_unicode_and_saves_only_on_save() {
        unsafe {
            PROMPT_REPLY.with(|reply| *reply.borrow_mut() = None);
            let window = prompt_window("Old prompt").unwrap();
            let text = "Whispertype, Groq — français 日本語";
            SetWindowTextW(GetDlgItem(window, TEXT), wide(text).as_ptr());
            SendMessageW(window, WM_COMMAND, ACTION, 0);
            assert_eq!(
                PROMPT_REPLY.with(|reply| reply.borrow_mut().take()),
                Some(text.to_string())
            );
            let mut msg: MSG = zeroed();
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
            let window = prompt_window("Keep existing prompt").unwrap();
            SendMessageW(window, WM_COMMAND, CANCEL, 0);
            assert!(PROMPT_REPLY.with(|reply| reply.borrow().is_none()));
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
    }

    #[test]
    fn prompt_shortcuts_select_all_and_undo_without_inserting_control_characters() {
        unsafe {
            let original = "Groq, français 日本語";
            let window = prompt_window(original).unwrap();
            let edit = GetDlgItem(window, TEXT);
            let mut keys = [0u8; 256];
            assert_ne!(GetKeyboardState(keys.as_mut_ptr()), 0);
            let mut ctrl = keys;
            ctrl[VK_CONTROL as usize] = 0x80;
            ctrl[VK_MENU as usize] = 0;
            assert_ne!(SetKeyboardState(ctrl.as_ptr()), 0);
            let message = MSG {
                hwnd: edit,
                message: WM_KEYDOWN,
                wParam: 0x41,
                ..zeroed()
            };
            prompt_message(window, &message);
            SetKeyboardState(keys.as_ptr());
            let (mut start, mut end) = (0u32, 0u32);
            SendMessageW(
                edit,
                0x00b0, /* EM_GETSEL */
                &mut start as *mut _ as usize,
                &mut end as *mut _ as isize,
            );
            assert_eq!(start, 0);
            assert_eq!(end as usize, original.encode_utf16().count());
            SendMessageW(edit, WM_CHAR, 'X' as usize, 0);
            assert_eq!(GetWindowTextLengthW(edit), 1);
            assert!(edit_shortcut(edit, 0x5a));
            let mut text = vec![0u16; original.encode_utf16().count() + 1];
            let n = GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32);
            assert_eq!(String::from_utf16_lossy(&text[..n as usize]), original);
            assert!(!edit_shortcut(edit, VK_F1 as u32));
            DestroyWindow(window);
            let mut msg: MSG = zeroed();
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
    }

    #[test]
    fn prompt_dialog_navigation_keeps_enter_in_the_editor_and_escape_cancels() {
        unsafe {
            PROMPT_REPLY.with(|reply| *reply.borrow_mut() = None);
            let window = prompt_window("Context").unwrap();
            let edit = GetDlgItem(window, TEXT);
            SetFocus(edit);
            assert_eq!(GetFocus(), edit);
            let mut message = MSG {
                hwnd: edit,
                message: WM_KEYDOWN,
                wParam: VK_TAB as usize,
                ..zeroed()
            };
            prompt_message(window, &message);
            assert_eq!(GetFocus(), GetDlgItem(window, ACTION as i32));
            // Return in a multiline prompt must not submit the form.
            message.hwnd = edit;
            message.wParam = VK_RETURN as usize;
            SetFocus(edit);
            prompt_message(window, &message);
            assert_ne!(IsWindow(window), 0);
            assert!(PROMPT_REPLY.with(|reply| reply.borrow().is_none()));
            // Discard the translated Return before testing Escape.
            let mut translated: MSG = zeroed();
            while PeekMessageW(&mut translated, window, WM_CHAR, WM_CHAR, PM_REMOVE) != 0 {}
            message.wParam = VK_ESCAPE as usize;
            prompt_message(window, &message);
            assert_eq!(IsWindow(window), 0);
            assert!(PROMPT_REPLY.with(|reply| reply.borrow().is_none()));
            PeekMessageW(&mut translated, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
            let window = prompt_window("Save with Enter").unwrap();
            let save = GetDlgItem(window, ACTION as i32);
            SetFocus(save);
            message.hwnd = save;
            message.wParam = VK_RETURN as usize;
            prompt_message(window, &message);
            assert_eq!(IsWindow(window), 0);
            assert_eq!(
                PROMPT_REPLY.with(|reply| reply.borrow_mut().take()),
                Some("Save with Enter".into())
            );
            PeekMessageW(&mut translated, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
    }
}
