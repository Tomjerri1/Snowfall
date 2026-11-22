//! X11 backend: ARGB override-redirect overlay, EWMH window tracking and an XGrabKey hotkey.
//!
//! Requirements: an X server (or XWayland) with RandR and SHAPE, a compositing window manager so the
//! overlay can be transparent, and an EWMH-compliant window manager (`_NET_CLIENT_LIST_STACKING`).
//! Only X11 clients are tracked: native Wayland windows are invisible to this backend.
//!
//! The tray icon (StatusNotifierItem over D-Bus) carries all settings in its menu; `Ctrl+Alt+Q` quits.

mod conn;
mod monitors;
mod overlay;
mod shm;
mod tracker;
mod tray;

use crate::app::AppState;
use crate::geometry::{Monitor, WinRect, WindowInfo};
use crate::sys::{Backend, ScreenBounds, WindowId};
use conn::Conn;
use overlay::Overlay;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracker::Tracker;
use tray::SnowTray;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, Keycode, ModMask};
use x11rb::protocol::Event;

/// Keysym of the lowercase `q`; the uppercase one is 0x51.
const KEYSYM_Q: u32 = 0x71;
const KEYSYM_UPPER_Q: u32 = 0x51;
const MONITOR_CACHE: Duration = Duration::from_secs(1);

pub struct X11Backend {
    c: Arc<Conn>,
    overlay: Overlay,
    tracker: Tracker,
    quit_keycode: Option<Keycode>,
    quit: bool,
    tray: Option<ksni::blocking::Handle<SnowTray>>,
    monitors: Mutex<Option<(Instant, Vec<Monitor>)>>,
}

fn fatal(message: String) -> ! {
    eprintln!("snowfall: {message}");
    std::process::exit(1);
}

fn keycode_for(c: &Conn, keysyms: &[u32]) -> Option<Keycode> {
    let setup = c.conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = c
        .conn
        .get_keyboard_mapping(min, max - min + 1)
        .ok()?
        .reply()
        .ok()?;
    let per_key = mapping.keysyms_per_keycode as usize;
    if per_key == 0 {
        return None;
    }
    let index = mapping
        .keysyms
        .chunks(per_key)
        .position(|syms| syms.iter().any(|s| keysyms.contains(s)))?;
    Some(min + index as u8)
}

/// Grabs Ctrl+Alt+<keycode> on the root window, also with Caps Lock and Num Lock active.
fn grab_ctrl_alt(c: &Conn, keycode: Keycode) {
    const CONTROL: u16 = 0x04;
    const ALT: u16 = 0x08; // Mod1
    const CAPS_LOCK: u16 = 0x02;
    const NUM_LOCK: u16 = 0x10; // Mod2

    for extra in [0, CAPS_LOCK, NUM_LOCK, CAPS_LOCK | NUM_LOCK] {
        let modifiers = ModMask::from(CONTROL | ALT | extra);
        let _ = c.conn.grab_key(
            true,
            c.root,
            modifiers,
            keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
    }
    let _ = c.conn.flush();
}

impl X11Backend {
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::KeyPress(ev) if Some(ev.detail) == self.quit_keycode => self.quit = true,
            Event::ShmCompletion(_) => self.overlay.shm_completed(),
            event => self.tracker.on_event(&event),
        }
    }
}

impl Backend for X11Backend {
    fn start(state: Arc<AppState>) -> Self {
        let c = Conn::connect().unwrap_or_else(|e| fatal(e));
        let overlay = Overlay::new(c.clone()).unwrap_or_else(|e| fatal(e));
        let tracker = Tracker::new(c.clone(), overlay.window()).unwrap_or_else(|e| fatal(e));

        let quit_keycode = keycode_for(&c, &[KEYSYM_Q, KEYSYM_UPPER_Q]);
        match quit_keycode {
            Some(keycode) => grab_ctrl_alt(&c, keycode),
            None => eprintln!("snowfall: no key for Q found, the Ctrl+Alt+Q hotkey is disabled"),
        }

        let tray = tray::spawn(state);

        Self {
            c,
            overlay,
            tracker,
            quit_keycode,
            quit: false,
            tray,
            monitors: Mutex::new(None),
        }
    }

    fn bounds(&self) -> ScreenBounds {
        self.overlay.bounds()
    }

    fn monitors(&self) -> Vec<Monitor> {
        let mut cache = self.monitors.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, list)) = cache.as_ref() {
            if at.elapsed() < MONITOR_CACHE {
                return list.clone();
            }
        }
        let list = monitors::monitors(&self.c);
        *cache = Some((Instant::now(), list.clone()));
        list
    }

    fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        // The server may still be reading the shared buffer; it announces that it is done with an event.
        while self.overlay.shm_pending() {
            match self.c.conn.wait_for_event() {
                Ok(event) => self.handle_event(event),
                Err(_) => self.overlay.shm_completed(), // connection lost: nothing left to wait for
            }
        }
        self.overlay.pixmap_mut()
    }

    fn present(&mut self) {
        self.overlay.present();
    }

    fn pump_events(&mut self) -> bool {
        loop {
            match self.c.conn.poll_for_event() {
                Ok(Some(event)) => self.handle_event(event),
                Ok(None) => break,
                // The connection to the X server is gone.
                Err(_) => return false,
            }
        }
        !self.quit
    }

    fn set_overlay_visible(&mut self, visible: bool) {
        self.overlay.set_visible(visible);
    }

    fn snapshot(&self) -> Vec<WindowInfo> {
        // Window managers may raise a freshly mapped window above the overlay, so restack it on every refresh.
        self.overlay.raise();
        let _ = self.c.conn.flush();
        self.tracker.snapshot()
    }

    fn refresh_rects(&self, windows: &mut Vec<WindowInfo>) -> bool {
        self.tracker.refresh_rects(windows)
    }

    fn read_rect(&self, id: WindowId) -> Option<WinRect> {
        self.tracker.read_rect(id)
    }

    fn is_dirty(&self) -> bool {
        self.tracker.is_dirty()
    }

    fn reset_dirty(&self) {
        self.tracker.reset_dirty();
    }

    fn take_minimized(&self) -> Vec<WindowId> {
        self.tracker.take_minimized()
    }

    fn moving_window(&self) -> Option<WindowId> {
        self.tracker.moving_window()
    }

    fn fullscreen_active(&self) -> bool {
        self.tracker.fullscreen_active()
    }

    fn cursor_pos(&self) -> (i32, i32) {
        self.c
            .conn
            .query_pointer(self.c.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map_or((0, 0), |reply| (reply.root_x as i32, reply.root_y as i32))
    }

    fn set_high_priority(&mut self, _on: bool) {}

    fn wait_vsync(&self) -> bool {
        false
    }

    fn dump_windows(&self) -> String {
        self.tracker.dump(&self.monitors())
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        if let Some(tray) = self.tray.take() {
            tray.shutdown().wait();
        }
    }
}
