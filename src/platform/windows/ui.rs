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
struct Theme {
    brush: HBRUSH,
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
            normal: font(-16, FW_NORMAL as i32),
            heading: font(-17, FW_SEMIBOLD as i32),
        }
    }
}
impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.brush);
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
    let (x, y) = (
        (info.rcWork.left + info.rcWork.right - width) / 2,
        (info.rcWork.top + info.rcWork.bottom - height) / 2,
    );
    let (ex, style) = (WS_EX_TOOLWINDOW, WS_CAPTION | WS_SYSMENU | WS_OVERLAPPED);
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
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            SetTextColor(w as HDC, FOREGROUND);
            SetBkColor(w as HDC, SURFACE);
            THEME.with(|theme| theme.brush as isize)
        }
        WM_DRAWITEM => {
            paint_button(&*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_COMMAND => {
            let command = w & 0xffff;
            if role == ENROLL_ROLE {
                if command == ACTION {
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
        let Ok(window) = shell(ENROLL_ROLE, "Personal wake phrase", 560, 270) else {
            return;
        };
        let title = control(window, "STATIC", "", 0, [24, 22, 500, 28], HEADING);
        SendMessageW(
            title,
            WM_SETFONT,
            THEME.with(|theme| theme.heading as usize),
            1,
        );
        control(window, "STATIC", "", 0, [24, 62, 500, 92], TEXT as usize);
        control(
            window,
            "BUTTON",
            "Record sample",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            [24, 170, 188, 38],
            ACTION,
        );
        control(
            window,
            "BUTTON",
            "Cancel",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            [430, 170, 100, 38],
            CANCEL,
        );
        control(
            window,
            "BUTTON",
            "Test phrase",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            [228, 170, 184, 38],
            TEST,
        );
        ENROLL.with(|slot| slot.set(window));
        ShowWindow(window, SW_SHOWNORMAL);
        SetForegroundWindow(window);
        window
    } else {
        current
    };
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
    let window = shell(PROMPT_ROLE, "Transcription prompt", 540, 350)?;
    control(window, "STATIC", "Optional spelling, vocabulary and context hints sent to Groq. This is not a chat instruction. Keep it concise (Groq uses up to 224 tokens). Clear the field to disable it.", 0, [18, 15, 490, 65], 0);
    let edit = control(
        window,
        "EDIT",
        current,
        WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_WANTRETURN as u32,
        [18, 87, 490, 160],
        TEXT as usize,
    );
    SendMessageW(edit, EM_SETLIMITTEXT, 1024, 0);
    control(
        window,
        "BUTTON",
        "Save",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [278, 268, 110, 30],
        ACTION,
    );
    control(
        window,
        "BUTTON",
        "Cancel",
        WS_TABSTOP | BS_OWNERDRAW as u32,
        [398, 268, 110, 30],
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
        if IsDialogMessageW(window, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(PROMPT_REPLY.with(|reply| reply.borrow_mut().take()))
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
}
