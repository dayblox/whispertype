use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsFd, AsRawFd, FromRawFd},
    time::{Duration, Instant},
};
use wayland_client::{
    delegate_noop,
    globals::registry_queue_init,
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_region, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};
use xkbcommon::xkb;

struct Overlay {
    surface: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    done: bool,
    capture: bool,
    keymap: Option<xkb::Keymap>,
    keys: Option<xkb::State>,
    context: xkb::Context,
}

pub fn run(kind: &str, capture: bool) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Overlay>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals
        .bind(&qh, 1..=4, ())
        .context("Compositor must support layer-shell for the indicator")?;
    let surface = compositor.create_surface(&qh, ());
    surface.set_buffer_scale(2);
    let region = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&region));
    region.destroy();
    let layer = shell.get_layer_surface(
        &surface,
        None,
        Layer::Overlay,
        "whispertype".into(),
        &qh,
        (),
    );
    layer.set_size(88, 36);
    layer.set_anchor(Anchor::Bottom);
    layer.set_margin(0, 0, 48, 0);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(if capture {
        KeyboardInteractivity::Exclusive
    } else {
        KeyboardInteractivity::None
    });
    let pixels = crate::platform::pixels(kind, 176, 72);
    let fd = unsafe { libc::memfd_create(c"whispertype-indicator".as_ptr(), libc::MFD_CLOEXEC) };
    anyhow::ensure!(fd >= 0, "memfd_create failed");
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&pixels)?;
    let pool = shm.create_pool(file.as_fd(), pixels.len() as i32, &qh, ());
    let buffer = pool.create_buffer(0, 176, 72, 176 * 4, wl_shm::Format::Argb8888, &qh, ());
    pool.destroy();
    if capture {
        let seat: wl_seat::WlSeat = globals
            .bind(&qh, 1..=7, ())
            .context("No Wayland keyboard seat")?;
        seat.get_keyboard(&qh, ());
    }
    let mut state = Overlay {
        surface,
        buffer,
        done: false,
        capture,
        keymap: None,
        keys: None,
        context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
    };
    state.surface.commit();
    conn.flush()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !state.done {
        queue.dispatch_pending(&mut state)?;
        conn.flush()?;
        if state.done {
            break;
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [
            libc::pollfd {
                fd: conn.backend().poll_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            },
        ];
        let timeout = if capture {
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32
        } else {
            -1
        };
        let result = unsafe { libc::poll(fds.as_mut_ptr(), if capture { 1 } else { 2 }, timeout) };
        if result < 0 {
            drop(guard);
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            anyhow::bail!("Indicator poll failed");
        }
        if result == 0 || !capture && fds[1].revents != 0 {
            drop(guard);
            break;
        }
        if fds[0].revents != 0 {
            guard.read()?;
        } else {
            drop(guard);
        }
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Overlay {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Overlay {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
                state.surface.attach(Some(&state.buffer), 0, 0);
                state.surface.damage_buffer(0, 0, 176, 72);
                state.surface.commit();
            }
            zwlr_layer_surface_v1::Event::Closed => state.done = true,
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Overlay {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { fd, size, .. } => {
                let mut text = String::new();
                let _ = File::from(fd).take(size as u64).read_to_string(&mut text);
                if let Some(map) = xkb::Keymap::new_from_string(
                    &state.context,
                    text.trim_end_matches('\0').to_string(),
                    xkb::KEYMAP_FORMAT_TEXT_V1,
                    xkb::COMPILE_NO_FLAGS,
                ) {
                    state.keys = Some(xkb::State::new(&map));
                    state.keymap = Some(map);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keys) = &mut state.keys {
                    keys.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } if state.capture
                && key_state == wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed) =>
            {
                if let Some(keys) = &state.keys {
                    let sym = keys.key_get_one_sym((key + 8).into());
                    let name = xkb::keysym_get_name(sym);
                    if name == "Escape" {
                        state.done = true;
                        return;
                    }
                    let name = name.to_uppercase();
                    let mut parts = Vec::new();
                    for (modifier, label) in [
                        (xkb::MOD_NAME_CTRL, "CTRL"),
                        (xkb::MOD_NAME_ALT, "ALT"),
                        (xkb::MOD_NAME_SHIFT, "SHIFT"),
                        (xkb::MOD_NAME_LOGO, "SUPER"),
                    ] {
                        if keys.mod_name_is_active(modifier, xkb::STATE_MODS_EFFECTIVE) {
                            parts.push(label.to_string());
                        }
                    }
                    parts.push(name);
                    let shortcut = parts.join(" + ");
                    if crate::config::Shortcut::parse(&shortcut).is_ok() {
                        println!("{shortcut}");
                        state.done = true;
                    }
                }
            }
            _ => {}
        }
    }
}
delegate_noop!(Overlay: ignore wl_compositor::WlCompositor);
delegate_noop!(Overlay: ignore wl_surface::WlSurface);
delegate_noop!(Overlay: ignore wl_shm::WlShm);
delegate_noop!(Overlay: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Overlay: ignore wl_buffer::WlBuffer);
delegate_noop!(Overlay: ignore wl_region::WlRegion);
delegate_noop!(Overlay: ignore wl_seat::WlSeat);
delegate_noop!(Overlay: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
