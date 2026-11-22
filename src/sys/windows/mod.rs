//! Windows backend: layered overlay window, WinEvent hooks, notification-area icon and a GDI settings window.

mod http;
mod monitors;
mod overlay;
mod settings_ui;
mod tracker;
mod tray;

pub use http::http_get;

use crate::app::AppState;
use crate::geometry::{Monitor, WinRect, WindowInfo};
use crate::sys::{Backend, ScreenBounds, WindowId};
use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use ::windows::Win32::Graphics::Dwm::DwmFlush;
use ::windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use ::windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL,
};
use ::windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetCursorPos, PostMessageW, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE, WM_CLOSE,
};
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct WindowsBackend {
    state: Arc<AppState>,
    overlay: overlay::Overlay,
    tray_thread: Option<JoinHandle<()>>,
    high_priority: bool,
}

impl Backend for WindowsBackend {
    fn start(state: Arc<AppState>) -> Self {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = timeBeginPeriod(1);
        }
        tracker::init();
        let tray_thread = Some(tray::spawn_tray_thread(state.clone()));
        let overlay = overlay::Overlay::new();
        Self {
            state,
            overlay,
            tray_thread,
            high_priority: false,
        }
    }

    fn bounds(&self) -> ScreenBounds {
        self.overlay.bounds()
    }

    fn monitors(&self) -> Vec<Monitor> {
        monitors::monitors(self.overlay.bounds())
    }

    fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        self.overlay.pixmap_mut()
    }

    fn present(&mut self) {
        self.overlay.present();
    }

    fn pump_events(&mut self) -> bool {
        self.overlay.pump_messages()
    }

    fn set_overlay_visible(&mut self, visible: bool) {
        let cmd = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        unsafe {
            let _ = ShowWindow(self.overlay.hwnd(), cmd);
        }
    }

    fn snapshot(&self) -> Vec<WindowInfo> {
        tracker::snapshot(HWND::default())
    }

    fn refresh_rects(&self, windows: &mut Vec<WindowInfo>) -> bool {
        tracker::refresh_rects(windows)
    }

    fn read_rect(&self, id: WindowId) -> Option<WinRect> {
        tracker::read_rect(id)
    }

    fn is_dirty(&self) -> bool {
        tracker::is_dirty()
    }

    fn reset_dirty(&self) {
        tracker::reset_dirty();
    }

    fn take_minimized(&self) -> Vec<WindowId> {
        tracker::take_minimized()
    }

    fn moving_window(&self) -> Option<WindowId> {
        match tracker::get_moving_hwnd() {
            0 => None,
            id => Some(id),
        }
    }

    fn fullscreen_active(&self) -> bool {
        tracker::fullscreen_active()
    }

    fn cursor_pos(&self) -> (i32, i32) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        (pt.x, pt.y)
    }

    fn set_high_priority(&mut self, on: bool) {
        if on == self.high_priority {
            return;
        }
        self.high_priority = on;
        let level = if on {
            THREAD_PRIORITY_ABOVE_NORMAL
        } else {
            THREAD_PRIORITY_NORMAL
        };
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), level);
        }
    }

    fn wait_vsync(&self) -> bool {
        unsafe { DwmFlush().is_ok() }
    }

    fn dump_windows(&self) -> String {
        tracker::dump_all(tracker::get_overlay_hwnd())
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        if self.high_priority {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL);
            }
        }

        // Ask the tray thread to close its message window and wait for it.
        self.state.quit.store(true, Ordering::Relaxed);
        let mut tray_hwnd = HWND(tray::TRAY_HWND.load(Ordering::Acquire) as *mut c_void);
        if tray_hwnd.is_invalid() {
            tray_hwnd = unsafe {
                FindWindowW(::windows::core::w!("SnowfallTray"), None).unwrap_or_default()
            };
        }
        if !tray_hwnd.is_invalid() {
            let _ = unsafe { PostMessageW(tray_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        if let Some(handle) = self.tray_thread.take() {
            let _ = handle.join();
        }

        unsafe {
            let _ = timeEndPeriod(1);
        }
    }
}
