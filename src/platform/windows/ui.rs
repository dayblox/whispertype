use super::*;

const NOTE_ROLE: isize = 1;
const ENROLL_ROLE: isize = 2;
const PROMPT_ROLE: isize = 3;
const ACTION: usize = 100;
const CANCEL: usize = 101;
const TEXT: i32 = 102;
// Standard STATIC/EDIT control messages (winuser.h).
const SS_NOTIFY: u32 = 0x0100;
const EM_SETLIMITTEXT: u32 = 0x00c5;
#[cfg(not(test))]
const UI_CLASS: &str = "Whispertype.Feedback.v2";
#[cfg(test)]
const UI_CLASS: &str = "Whispertype.Test.Feedback.v2";
thread_local! {
    static NOTE: Cell<HWND> = const { Cell::new(null_mut()) };
    static ENROLL: Cell<HWND> = const { Cell::new(null_mut()) };
    static PROMPT_REPLY: RefCell<Option<String>> = const { RefCell::new(None) };
}

unsafe fn shell(role: isize, title: &str, width: i32, height: i32) -> Result<HWND> {
    let instance = GetModuleHandleW(null());
    let name = wide(UI_CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        lpszClassName: name.as_ptr(),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
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
    let (x, y) = if role == NOTE_ROLE {
        (
            info.rcWork.right - width - 12,
            info.rcWork.bottom - height - 12,
        )
    } else {
        (
            (info.rcWork.left + info.rcWork.right - width) / 2,
            (info.rcWork.top + info.rcWork.bottom - height) / 2,
        )
    };
    let (ex, style) = if role == NOTE_ROLE {
        (
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WS_POPUP | WS_BORDER,
        )
    } else {
        (WS_EX_TOOLWINDOW, WS_CAPTION | WS_SYSMENU | WS_OVERLAPPED)
    };
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
        GetStockObject(DEFAULT_GUI_FONT) as usize,
        1,
    );
    child
}

unsafe extern "system" fn proc(window: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let role = GetWindowLongPtrW(window, GWLP_USERDATA);
    match msg {
        WM_TIMER if role == NOTE_ROLE => {
            KillTimer(window, 1);
            DestroyWindow(window);
            0
        }
        WM_LBUTTONUP if role == NOTE_ROLE => {
            DestroyWindow(window);
            0
        }
        WM_COMMAND if role == NOTE_ROLE => {
            DestroyWindow(window);
            0
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
            if role == NOTE_ROLE {
                NOTE.with(|slot| {
                    if slot.get() == window {
                        slot.set(null_mut());
                    }
                });
            }
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

// A single app-owned card is replaced immediately and never queued by Explorer.
pub(super) unsafe fn notice(text: &str, persistent: bool) {
    notice_visible(text, persistent, true);
}
unsafe fn notice_visible(text: &str, persistent: bool, visible: bool) {
    let current = NOTE.with(Cell::get);
    let window = if current.is_null() {
        let Ok(window) = shell(NOTE_ROLE, "Whispertype", 410, 128) else {
            return;
        };
        control(window, "STATIC", "Whispertype", 0, [14, 10, 370, 20], 0);
        control(
            window,
            "STATIC",
            "",
            SS_NOTIFY,
            [14, 34, 375, 76],
            TEXT as usize,
        );
        NOTE.with(|slot| slot.set(window));
        window
    } else {
        current
    };
    let text: String = text.chars().take(500).collect();
    SetWindowTextW(GetDlgItem(window, TEXT), wide(&text).as_ptr());
    if persistent {
        KillTimer(window, 1);
    } else {
        SetTimer(window, 1, 4000, None);
    }
    if visible {
        ShowWindow(window, SW_SHOWNOACTIVATE);
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
        let Ok(window) = shell(ENROLL_ROLE, "Personal wake phrase", 480, 245) else {
            return;
        };
        control(window, "STATIC", "", 0, [20, 18, 430, 126], TEXT as usize);
        control(
            window,
            "BUTTON",
            "Record sample",
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            [20, 158, 225, 32],
            ACTION,
        );
        control(
            window,
            "BUTTON",
            "Cancel",
            WS_TABSTOP,
            [332, 158, 110, 32],
            CANCEL,
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
        GetDlgItem(window, ACTION as i32),
        wide(&view.action).as_ptr(),
    );
    EnableWindow(GetDlgItem(window, ACTION as i32), view.enabled as i32);
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
        WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        [278, 268, 110, 30],
        ACTION,
    );
    control(
        window,
        "BUTTON",
        "Cancel",
        WS_TABSTOP,
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
    for window in [NOTE.with(Cell::get), ENROLL.with(Cell::get)] {
        if !window.is_null() {
            DestroyWindow(window);
        }
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
    #[test]
    fn feedback_replaces_in_place_and_expires_without_quitting_tray() {
        unsafe {
            notice_visible("Checking for updates", true, false);
            let first = NOTE.with(Cell::get);
            assert!(!first.is_null());
            notice_visible("Up to date", false, false);
            assert_eq!(first, NOTE.with(Cell::get));
            let mut text = [0u16; 100];
            let n = GetWindowTextW(GetDlgItem(first, TEXT), text.as_mut_ptr(), 100);
            assert_eq!(String::from_utf16_lossy(&text[..n as usize]), "Up to date");
            SendMessageW(first, WM_TIMER, 1, 0);
            assert!(NOTE.with(Cell::get).is_null());
            let mut msg: MSG = zeroed();
            assert_eq!(
                PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE),
                0
            );
        }
    }
}
