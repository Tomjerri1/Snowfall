//! Platform layer. Everything OS-specific lives behind the [`Backend`] trait; the rest of the app is platform-independent.
//!
//! A backend owns the overlay surface, the tray/settings UI, window tracking and a few system services.
//! Exactly one backend is compiled in; it is exported as [`PlatformBackend`].

use crate::app::AppState;
use crate::geometry::{Monitor, WinRect, WindowInfo};
use std::sync::Arc;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{http_get, WindowsBackend as PlatformBackend};

#[cfg(unix)]
mod curl_http;
#[cfg(unix)]
pub use curl_http::http_get;

#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

#[cfg(not(windows))]
compile_error!("Snowfall has no platform backend for this OS yet");

/// Opaque id of a top-level window. Positive values are real windows, negative ones are reserved for synthetic owners (see `snow::GROUND_ID`).
pub type WindowId = isize;

/// Virtual screen as (left, top, right, bottom) in pixels.
pub type ScreenBounds = (i32, i32, i32, i32);

pub trait Backend: Sized {
    /// Initialises the platform (DPI awareness, timers, window hooks), creates the overlay and starts the tray UI.
    fn start(state: Arc<AppState>) -> Self;

    // ---- Overlay surface ----

    /// Virtual screen covered by the overlay.
    fn bounds(&self) -> ScreenBounds;
    /// All monitors, primary first.
    fn monitors(&self) -> Vec<Monitor>;
    /// Pixels of the overlay as premultiplied BGRA, `bounds()` wide and high.
    fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_>;
    /// Shows the current pixmap on screen.
    fn present(&mut self);
    /// Handles pending OS events. Returns `false` when the app should quit.
    fn pump_events(&mut self) -> bool;
    fn set_overlay_visible(&mut self, visible: bool);

    // ---- Window tracking ----

    /// Visible application windows in front-to-back order.
    fn snapshot(&self) -> Vec<WindowInfo>;
    /// Re-reads the rects of `windows`, dropping closed/hidden ones. Returns whether anything changed.
    fn refresh_rects(&self, windows: &mut Vec<WindowInfo>) -> bool;
    fn read_rect(&self, id: WindowId) -> Option<WinRect>;
    /// True if the window set changed since the last `reset_dirty` (event-driven backends only; polling ones may always return true).
    fn is_dirty(&self) -> bool;
    fn reset_dirty(&self);
    /// Windows minimized since the previous call.
    fn take_minimized(&self) -> Vec<WindowId>;
    /// The window the user is currently dragging or resizing.
    fn moving_window(&self) -> Option<WindowId>;
    /// A full-screen app is in the foreground and the overlay should hide.
    fn fullscreen_active(&self) -> bool;

    // ---- System services ----

    /// Mouse position in screen coordinates.
    fn cursor_pos(&self) -> (i32, i32);
    /// Raises or restores the priority of the render thread while a window is dragged.
    fn set_high_priority(&mut self, on: bool);
    /// Blocks until the next display refresh. Returns `false` if unsupported, so the caller should sleep instead.
    fn wait_vsync(&self) -> bool;
    /// Human-readable list of all windows and why each one was accepted or rejected (`--dump`).
    fn dump_windows(&self) -> String;
}
