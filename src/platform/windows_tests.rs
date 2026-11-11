use super::*;
use std::{sync::mpsc, time::Instant};

#[test]
fn shortcut_virtual_keys_and_modifiers() {
    for (name, key, bits) in [
        ("CTRL + SPACE", VK_SPACE as u32, 1),
        ("CTRL + ALT + F24", VK_F24 as u32, 3),
        ("SUPER + SHIFT + A", 0x41, 12),
        ("F9", VK_F9 as u32, 0),
        ("ALT + 9", 0x39, 2),
    ] {
        let shortcut = Shortcut::parse(name).unwrap();
        assert_eq!(vk(&shortcut), key);
        assert_eq!(
            modifier_bits((
                shortcut.ctrl,
                shortcut.alt,
                shortcut.shift,
                shortcut.super_key
            )),
            bits
        );
    }
}

#[test]
fn unicode_batches_preserve_surrogate_pairs_and_key_releases() {
    for prefix in 0..128 {
        let text = format!("{}🎙é你好{}", "a".repeat(prefix), "🙂".repeat(80));
        let mut units = text.encode_utf16();
        let mut received = Vec::new();
        loop {
            let mut inputs: [INPUT; 128] = unsafe { zeroed() };
            let count = unicode_batch(&mut units, &mut inputs);
            if count == 0 {
                break;
            }
            assert!(count <= inputs.len());
            let mut batch = Vec::new();
            for pair in inputs[..count].as_chunks::<2>().0 {
                unsafe {
                    assert_eq!(pair[0].r#type, INPUT_KEYBOARD);
                    assert_eq!(pair[0].Anonymous.ki.dwFlags, KEYEVENTF_UNICODE);
                    assert_eq!(
                        pair[1].Anonymous.ki.dwFlags,
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                    );
                    assert_eq!(pair[0].Anonymous.ki.wScan, pair[1].Anonymous.ki.wScan);
                    batch.push(pair[0].Anonymous.ki.wScan);
                }
            }
            assert!(
                String::from_utf16(&batch).is_ok(),
                "surrogate split between batches"
            );
            received.extend(batch);
        }
        assert_eq!(String::from_utf16(&received).unwrap(), text);
    }
}

unsafe fn pump() {
    let mut msg: MSG = zeroed();
    while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

#[test]
#[ignore = "Requires an interactive Windows desktop; creates and types only into its own test window"]
fn windows_desktop_smoke() {
    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::at(temp.path().to_path_buf()).unwrap();
        let config = Config {
            shortcut: "F24".into(),
            ..Config::default()
        };
        let (tx, rx) = mpsc::sync_channel(64);
        let desktop = Desktop::start(&paths, &config, tx).unwrap();
        let window = TestWindow(CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("Whispertype desktop smoke test").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | 4,
            100,
            100,
            500,
            200,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        ));
        assert!(!window.0.is_null());
        SetWindowTextW(window.0, wide("").as_ptr());
        SetForegroundWindow(window.0);
        SetFocus(window.0);
        pump();
        assert_eq!(
            GetForegroundWindow(),
            window.0,
            "Test window must have focus before typing"
        );
        let text = format!("{}🎙 Français — 日本語 🙂", "x".repeat(62));
        type_text(&text).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        let actual = loop {
            pump();
            let mut buffer = [0u16; 256];
            let len = GetWindowTextW(window.0, buffer.as_mut_ptr(), buffer.len() as i32);
            let actual = String::from_utf16_lossy(&buffer[..len as usize]);
            if actual == text || Instant::now() >= until {
                break actual;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(actual, text);
        for state in ["recording", "transcribing", "enrolling", "idle"] {
            desktop.update(Snapshot {
                config: config.clone(),
                state: state.into(),
                wake: false,
            });
            // Execute the current snapshot synchronously so every state is
            // checked even when updates arrive faster than the tray loop.
            SendMessageW(hwnd(), UPDATE, 0, 0);
            let until = Instant::now() + Duration::from_secs(2);
            let overlay = loop {
                let overlay = FindWindowW(wide("Whispertype.Indicator").as_ptr(), null());
                if (!overlay.is_null() == (state != "idle"))
                    && (overlay.is_null() || IsWindowVisible(overlay) != 0)
                    || Instant::now() >= until
                {
                    break overlay;
                }
                pump();
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(!overlay.is_null(), state != "idle");
            if !overlay.is_null() {
                assert_ne!(IsWindowVisible(overlay), 0);
            }
            assert_eq!(GetForegroundWindow(), window.0, "Indicator stole focus");
        }
        // Exercise the hook's repeat suppression and release handling with the
        // same unflagged structure supplied by Windows for physical keystrokes.
        let key = KBDLLHOOKSTRUCT {
            vkCode: VK_F24 as u32,
            ..zeroed()
        };
        for msg in [WM_KEYDOWN, WM_KEYDOWN, WM_KEYUP] {
            assert_eq!(
                keyboard_hook(HC_ACTION as i32, msg as usize, &key as *const _ as isize),
                1
            );
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Event::Key { down: true, .. }
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Event::Key { down: false, .. }
        ));
        assert!(rx.try_recv().is_err());
        send("cancel").unwrap();
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), Event::Command(s) if s == "cancel")
        );
        assert!(send("invalid").is_err());
        drop(desktop);
        assert!(FindWindowW(wide(CLASS).as_ptr(), null()).is_null());
        println!("Native tray lifecycle, hook tap/repeat/release, IPC, Unicode SendInput, and non-activating indicator passed.");
    }
}
