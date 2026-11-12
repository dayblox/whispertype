use super::*;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

thread_local! {
    static ROOT: Cell<HMENU> = const { Cell::new(null_mut()) };
    static OWNER: Cell<HWND> = const { Cell::new(null_mut()) };
    static CURRENT: Cell<HMENU> = const { Cell::new(null_mut()) };
    static SELECTED: Cell<u32> = const { Cell::new(u32::MAX) };
}

pub(super) fn is_open() -> bool {
    !ROOT.with(Cell::get).is_null()
}

struct Session {
    root: HMENU,
    hook: HHOOK,
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.hook.is_null() {
                UnhookWindowsHookEx(self.hook);
            }
            ROOT.with(|slot| slot.set(null_mut()));
            OWNER.with(|slot| slot.set(null_mut()));
            CURRENT.with(|slot| slot.set(null_mut()));
            SELECTED.with(|slot| slot.set(u32::MAX));
            DestroyMenu(self.root);
        }
    }
}

fn command(id: u32, wake: bool) -> Option<&'static str> {
    Some(match id {
        10 => "shortcut-capture",
        11 => "shortcut:CTRL + SPACE",
        12 => "shortcut:CTRL + ALT + SPACE",
        13 => "shortcut:SUPER + ALT + SPACE",
        14 => "shortcut:F9",
        20 => "mode:auto",
        21 => "mode:hold",
        22 => "mode:toggle",
        30 if wake => "wake-off",
        30 => "wake-on",
        31 => "enroll",
        41 => "quit",
        42 => "check-updates",
        43 => "startup-toggle",
        44 => "prompt-edit",
        45 => "wake-test",
        _ => return None,
    })
}

fn stays_open(id: u32) -> bool {
    matches!(id, 11..=14 | 20..=22 | 30 | 42 | 43)
}

fn dispatch(id: u32) {
    // Only the wake toggle needs current state to choose its command.
    let wake = id == 30
        && SNAPSHOT
            .get()
            .is_some_and(|snapshot| snapshot.lock().unwrap().wake);
    if let Some(command) = command(id, wake) {
        event(command);
    }
}

pub(super) unsafe fn show(window: HWND) {
    if is_open() {
        EndMenu();
        return;
    }
    let s = SNAPSHOT.get().unwrap().lock().unwrap().clone();
    let root = create_menu(&s);
    if root.is_null() {
        return;
    }
    ROOT.with(|slot| slot.set(root));
    OWNER.with(|slot| slot.set(window));
    let session = Session {
        root,
        // This filter exists only for this thread's open native menu.
        hook: SetWindowsHookExW(WH_MSGFILTER, Some(filter), null_mut(), GetCurrentThreadId()),
    };
    let mut point = zeroed();
    GetCursorPos(&mut point);
    SetForegroundWindow(window);
    let selected = TrackPopupMenu(
        root,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        window,
        null(),
    );
    drop(session);
    PostMessageW(window, WM_NULL, 0, 0);
    dispatch(selected as u32);
}

pub(super) fn select(w: WPARAM, l: LPARAM) {
    if !is_open() {
        return;
    }
    CURRENT.with(|slot| slot.set(l as HMENU));
    let flags = (w >> 16) as u32;
    SELECTED.with(|slot| {
        slot.set(
            if flags & (MF_POPUP | MF_DISABLED | MF_GRAYED | MF_SEPARATOR) != 0 || l == 0 {
                u32::MAX
            } else {
                (w & 0xffff) as u32
            },
        )
    });
}

unsafe extern "system" fn filter(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == MSGF_MENU as i32 && is_open() {
        let message = &*(l as *const MSG);
        if let Some(id) = selection(message) {
            if stays_open(id) {
                // Do not dispatch auto-repeat from a held Enter/Space key.
                if message.message != WM_KEYDOWN || message.lParam & (1 << 30) == 0 {
                    dispatch(id);
                }
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, w, l)
}

unsafe fn selection(message: &MSG) -> Option<u32> {
    match message.message {
        WM_KEYDOWN
            if message.wParam == VK_RETURN as usize || message.wParam == VK_SPACE as usize =>
        {
            Some(SELECTED.with(Cell::get))
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let current = CURRENT.with(Cell::get);
            if current.is_null() {
                return None;
            }
            let index = MenuItemFromPoint(OWNER.with(Cell::get), current, message.pt);
            (index >= 0).then(|| GetMenuItemID(current, index))
        }
        _ => None,
    }
}

pub(super) unsafe fn refresh() {
    if is_open() {
        let snapshot = SNAPSHOT.get().unwrap().lock().unwrap().clone();
        sync(&snapshot);
    }
}

unsafe fn label(menu: HMENU, id: u32, text: &str) {
    let mut text = wide(text);
    let item = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: text.as_mut_ptr(),
        ..zeroed()
    };
    SetMenuItemInfoW(menu, id, 0, &item);
}

unsafe fn sync(s: &Snapshot) {
    let root = ROOT.with(Cell::get);
    if root.is_null() {
        return;
    }
    label(root, 1, &format!("Whispertype · {}", s.state));
    // Submenu headers have no command ID: address them by position.
    for (index, text) in [
        (1, format!("Shortcut: {}", s.config.shortcut)),
        (2, format!("Activation: {}", s.config.mode.label())),
    ] {
        let mut text = wide(&text);
        let item = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING,
            dwTypeData: text.as_mut_ptr(),
            ..zeroed()
        };
        SetMenuItemInfoW(root, index, 1, &item);
    }
    for (id, checked) in [(30, s.wake), (43, s.startup)] {
        CheckMenuItem(
            root,
            id,
            MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED },
        );
    }
    let shortcut = GetSubMenu(root, 1);
    for (id, key) in [
        (11, "CTRL + SPACE"),
        (12, "CTRL + ALT + SPACE"),
        (13, "SUPER + ALT + SPACE"),
        (14, "F9"),
    ] {
        CheckMenuItem(
            shortcut,
            id,
            MF_BYCOMMAND
                | if s.config.shortcut == key {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                },
        );
    }
    let mode = match s.config.mode {
        crate::activation::Mode::Auto => 20,
        crate::activation::Mode::Hold => 21,
        crate::activation::Mode::Toggle => 22,
    };
    CheckMenuRadioItem(GetSubMenu(root, 2), 20, 22, mode, MF_BYCOMMAND);
    EnumThreadWindows(GetCurrentThreadId(), Some(redraw), 0);
}

unsafe extern "system" fn redraw(window: HWND, _: LPARAM) -> i32 {
    let mut class = [0u16; 8];
    let n = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
    if class[..n.max(0) as usize]
        .iter()
        .copied()
        .eq("#32768".encode_utf16())
    {
        InvalidateRect(window, null(), 0);
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    const STEP: u32 = WM_APP + 120;

    #[derive(Default)]
    struct Probe {
        mouse_kept_open: bool,
        keyboard_kept_open: bool,
        startup_checked: bool,
        mode_checked: bool,
        outside_ignored: bool,
        escape_ignored: bool,
    }

    unsafe extern "system" fn owner(window: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_MENUSELECT => {
                select(w, l);
                0
            }
            STEP => {
                let probe = &mut *(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Probe);
                let root = ROOT.with(Cell::get);
                if w == 0 {
                    let index = (0..GetMenuItemCount(root))
                        .find(|i| GetMenuItemID(root, *i) == 43)
                        .unwrap_or(-1);
                    let mut rect: RECT = zeroed();
                    if index >= 0 && GetMenuItemRect(window, root, index as u32, &mut rect) != 0 {
                        select(43 | ((MF_HILITE as usize) << 16), root as isize);
                        let mouse = MSG {
                            message: WM_LBUTTONUP,
                            pt: POINT {
                                x: (rect.left + rect.right) / 2,
                                y: (rect.top + rect.bottom) / 2,
                            },
                            ..zeroed()
                        };
                        probe.mouse_kept_open = selection(&mouse) == Some(43)
                            && filter(MSGF_MENU as i32, 0, &mouse as *const _ as isize) == 1;
                        let outside = MSG {
                            pt: POINT {
                                x: -100000,
                                y: -100000,
                            },
                            ..mouse
                        };
                        probe.outside_ignored = selection(&outside).is_none();
                    }
                    let snapshot = Snapshot {
                        config: Config {
                            mode: crate::activation::Mode::Hold,
                            shortcut: "F9".into(),
                            ..Config::default()
                        },
                        state: "idle".into(),
                        wake: false,
                        startup: true,
                        enrollment: None,
                    };
                    sync(&snapshot);
                    probe.startup_checked = GetMenuState(root, 43, MF_BYCOMMAND) & MF_CHECKED != 0;
                    probe.mode_checked = GetMenuState(GetSubMenu(root, 2), 21, MF_BYCOMMAND)
                        & MF_CHECKED
                        != 0
                        && GetMenuState(GetSubMenu(root, 1), 14, MF_BYCOMMAND) & MF_CHECKED != 0;
                    select(
                        21 | ((MF_HILITE as usize) << 16),
                        GetSubMenu(root, 2) as isize,
                    );
                    let key = MSG {
                        message: WM_KEYDOWN,
                        wParam: VK_RETURN as usize,
                        ..zeroed()
                    };
                    probe.keyboard_kept_open =
                        filter(MSGF_MENU as i32, 0, &key as *const _ as isize) == 1;
                    let escape = MSG {
                        wParam: VK_ESCAPE as usize,
                        ..key
                    };
                    probe.escape_ignored = selection(&escape).is_none();
                    // An actual menu-loop Escape must end tracking. It is not
                    // intercepted as a persistent setting selection.
                    PostMessageW(window, WM_KEYDOWN, VK_ESCAPE as usize, 0);
                }
                0
            }
            _ => DefWindowProcW(window, msg, w, l),
        }
    }

    #[test]
    fn native_menu_keeps_mouse_and_keyboard_settings_open_and_updates_in_place() {
        unsafe {
            let name = wide("Whispertype.Test.PersistentMenu");
            let instance = GetModuleHandleW(null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(owner),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..zeroed()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!window.is_null());
            let mut probe = Probe::default();
            SetWindowLongPtrW(window, GWLP_USERDATA, &mut probe as *mut _ as isize);
            let snapshot = Snapshot {
                config: Config::default(),
                state: "idle".into(),
                wake: false,
                startup: false,
                enrollment: None,
            };
            let root = create_menu(&snapshot);
            ROOT.with(|slot| slot.set(root));
            OWNER.with(|slot| slot.set(window));
            let session = Session {
                root,
                hook: SetWindowsHookExW(
                    WH_MSGFILTER,
                    Some(filter),
                    null_mut(),
                    GetCurrentThreadId(),
                ),
            };
            assert!(!session.hook.is_null());
            PostMessageW(window, STEP, 0, 0);
            let selected = TrackPopupMenu(root, TPM_RETURNCMD, 10, 10, 0, window, null());
            drop(session);
            DestroyWindow(window);
            assert_eq!(selected, 0);
            assert!(
                probe.mouse_kept_open,
                "Native mouse activation would dismiss the menu"
            );
            assert!(
                probe.keyboard_kept_open,
                "Native Enter activation would dismiss the menu"
            );
            assert!(
                probe.startup_checked && probe.mode_checked,
                "Checkmarks were not updated in the existing menu"
            );
            assert!(probe.outside_ignored && probe.escape_ignored);
            assert!(!is_open());
            assert_eq!(
                GetMenuItemCount(root),
                -1,
                "Menu was retained after dismissal"
            );
        }
    }

    #[test]
    fn editors_and_quit_dismiss_but_settings_remain_open() {
        for id in [10, 31, 41, 44, 45] {
            assert!(!stays_open(id));
        }
        for id in [11, 12, 13, 14, 20, 21, 22, 30, 42, 43] {
            assert!(stays_open(id));
        }
        assert_eq!(command(30, true), Some("wake-off"));
        assert_eq!(command(30, false), Some("wake-on"));
        assert_eq!(command(0, false), None);
    }
}
