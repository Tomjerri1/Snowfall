//! macOS backend: a borderless NSWindow overlay, CGWindowList window tracking and a status bar item.
//!
//! Everything AppKit-related runs on the main thread, which is where the render loop lives, so the loop itself
//! pumps the NSApplication event queue (see `pump_events`).
//!
//! The tray menu carries all settings; `Ctrl+Option+Q` quits. Window titles are never read, so no Screen Recording
//! or Accessibility permission is needed. Only windows on the current Space are tracked.

mod overlay;
mod screens;
mod tracker;
mod tray;

use crate::app::AppState;
use crate::geometry::{Monitor, WinRect, WindowInfo};
use crate::sys::{Backend, ScreenBounds, WindowId};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2::MainThreadMarker;
use objc2_app_kit::NSEvent;
use overlay::Overlay;
use screens::Layout;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracker::Tracker;
use tray::TrayUi;

const LAYOUT_CACHE: Duration = Duration::from_secs(1);

pub struct MacBackend {
    mtm: MainThreadMarker,
    state: Arc<AppState>,
    overlay: Overlay,
    tracker: Tracker,
    tray: Option<TrayUi>,
    /// Keeps the hotkey registered for as long as the backend lives.
    hotkey: Option<(GlobalHotKeyManager, HotKey)>,
    layout: RefCell<(Instant, Layout)>,
}

fn fatal(message: String) -> ! {
    eprintln!("snowfall: {message}");
    std::process::exit(1);
}

fn register_quit_hotkey() -> Option<(GlobalHotKeyManager, HotKey)> {
    let manager = GlobalHotKeyManager::new().ok()?;
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyQ);
    manager.register(hotkey).ok()?;
    Some((manager, hotkey))
}

impl MacBackend {
    /// The screen layout, re-read at most once a second.
    fn layout(&self) -> Layout {
        let mut cache = self.layout.borrow_mut();
        if cache.0.elapsed() >= LAYOUT_CACHE {
            *cache = (Instant::now(), Layout::read(self.mtm));
        }
        cache.1.clone()
    }
}

impl Backend for MacBackend {
    fn start(state: Arc<AppState>) -> Self {
        let mtm = MainThreadMarker::new()
            .unwrap_or_else(|| fatal("the render loop must run on the main thread".into()));
        let layout = Layout::read(mtm);
        let overlay = Overlay::new(mtm, &layout).unwrap_or_else(|e| fatal(e));

        let tray = match TrayUi::new(state.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                eprintln!("snowfall: {e}");
                None
            }
        };
        let hotkey = register_quit_hotkey();
        if hotkey.is_none() {
            eprintln!("snowfall: cannot register the Ctrl+Option+Q hotkey");
        }

        Self {
            mtm,
            state,
            overlay,
            tracker: Tracker::new(),
            tray,
            hotkey,
            layout: RefCell::new((Instant::now(), layout)),
        }
    }

    fn bounds(&self) -> ScreenBounds {
        self.overlay.bounds()
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.layout().monitors
    }

    fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        self.overlay.pixmap_mut()
    }

    fn present(&mut self) {
        self.overlay.present();
    }

    fn pump_events(&mut self) -> bool {
        self.overlay.pump_events();
        if let Some(tray) = &mut self.tray {
            tray.poll();
        }
        if let Some((_, hotkey)) = &self.hotkey {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.id == hotkey.id() && event.state == HotKeyState::Pressed {
                    self.state
                        .quit
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        true
    }

    fn set_overlay_visible(&mut self, visible: bool) {
        self.overlay.set_visible(visible);
    }

    fn snapshot(&self) -> Vec<WindowInfo> {
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
        self.tracker.moving_window(self.cursor_pos())
    }

    fn fullscreen_active(&self) -> bool {
        self.tracker.fullscreen_active(&self.monitors())
    }

    fn cursor_pos(&self) -> (i32, i32) {
        // Cocoa reports the mouse with `y` up from the bottom of the primary screen.
        let location = NSEvent::mouseLocation();
        let y = self.layout().quartz_y(location.y);
        (location.x.round() as i32, y.round() as i32)
    }

    fn set_high_priority(&mut self, _on: bool) {}

    fn wait_vsync(&self) -> bool {
        false
    }

    fn dump_windows(&self) -> String {
        self.tracker.dump(&self.monitors())
    }
}
