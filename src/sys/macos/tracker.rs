//! Window tracking through `CGWindowListCopyWindowInfo`.
//!
//! macOS has no notifications for windows of other apps, so the tracker polls the on-screen window list (front to back)
//! at most every [`POLL`], and reports a change when the sequence of window ids differs. Window titles are not needed,
//! so no Screen Recording permission is required. Dragging is guessed from the left mouse button being held while the
//! window under the cursor changes its rect.

use crate::geometry::{build_segments, Accumulate, Monitor, WinRect, WindowInfo};
use crate::sys::WindowId;
use objc2_app_kit::NSEvent;
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
};
use objc2_core_graphics::{
    kCGNullWindowID, kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowName,
    kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID, CGRectMakeWithDictionaryRepresentation,
    CGWindowListCopyWindowInfo, CGWindowListCreateDescriptionFromArray, CGWindowListOption,
};
use std::cell::RefCell;
use std::time::{Duration, Instant};

const MIN_WIDTH: i32 = 120;
const MIN_HEIGHT: i32 = 60;
/// Minimum time between two queries of the window list.
const POLL: Duration = Duration::from_millis(50);
/// A snapshot requested right after a poll reuses it instead of querying again.
const SNAPSHOT_REUSE: Duration = Duration::from_millis(10);
/// Tolerance when comparing a window with a monitor frame.
const FULLSCREEN_SLACK: i32 = 1;

type Dict = CFDictionary<CFString, CFType>;

struct Raw {
    id: u32,
    rect: WinRect,
    layer: i32,
    alpha: f64,
    pid: i32,
    owner: String,
    name: String,
}

#[derive(Default)]
struct Inner {
    polled_at: Option<Instant>,
    /// Accepted windows from the latest poll, front to back.
    cached: Vec<WindowInfo>,
    /// Window ids the app was last told about (`snapshot`).
    reported: Vec<WindowId>,
    dirty: bool,
    minimized: Vec<WindowId>,
    moving: Option<WindowId>,
}

pub struct Tracker {
    own_pid: i32,
    inner: RefCell<Inner>,
}

fn string_of(dict: &Dict, key: &CFString) -> String {
    dict.get(key)
        .and_then(|value| value.downcast_ref::<CFString>().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn number_of(dict: &Dict, key: &CFString) -> Option<CFRetained<CFNumber>> {
    dict.get(key)?.downcast::<CFNumber>().ok()
}

fn parse(dict: &Dict) -> Option<Raw> {
    // SAFETY: reading CoreGraphics constants.
    let (number, layer, alpha, pid, bounds, owner, name) = unsafe {
        (
            kCGWindowNumber,
            kCGWindowLayer,
            kCGWindowAlpha,
            kCGWindowOwnerPID,
            kCGWindowBounds,
            kCGWindowOwnerName,
            kCGWindowName,
        )
    };
    let id = number_of(dict, number)?.as_i64()? as u32;
    let bounds = dict.get(bounds)?;
    let bounds = bounds.downcast_ref::<CFDictionary>()?;
    let mut rect = CGRect::default();
    // SAFETY: `bounds` is a CFDictionary and `rect` a valid pointer.
    if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect) } {
        return None;
    }
    Some(Raw {
        id,
        rect: WinRect {
            left: rect.origin.x.round() as i32,
            top: rect.origin.y.round() as i32,
            right: (rect.origin.x + rect.size.width).round() as i32,
            bottom: (rect.origin.y + rect.size.height).round() as i32,
        },
        layer: number_of(dict, layer).and_then(|n| n.as_i32()).unwrap_or(0),
        alpha: number_of(dict, alpha)
            .and_then(|n| n.as_f64())
            .unwrap_or(1.0),
        pid: number_of(dict, pid).and_then(|n| n.as_i32()).unwrap_or(0),
        owner: string_of(dict, owner),
        name: string_of(dict, name),
    })
}

fn raws(array: Option<CFRetained<CFArray>>) -> Vec<Raw> {
    let Some(array) = array else {
        return Vec::new();
    };
    // SAFETY: the window list is an array of window description dictionaries.
    let array: CFRetained<CFArray<Dict>> = unsafe { CFRetained::cast_unchecked(array) };
    array.iter().filter_map(|dict| parse(&dict)).collect()
}

/// Every window the window server reports as on screen, front to back, with all layers.
fn query_on_screen() -> Vec<Raw> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    raws(CGWindowListCopyWindowInfo(options, kCGNullWindowID))
}

/// Windows by id, on screen or not (minimized and hidden ones are included).
fn query_ids(ids: &[WindowId]) -> Vec<Raw> {
    if ids.is_empty() {
        return Vec::new();
    }
    let numbers: Vec<CFRetained<CFNumber>> =
        ids.iter().map(|&id| CFNumber::new_i64(id as i64)).collect();
    let array = CFArray::from_retained_objects(&numbers);
    // SAFETY: an array of window id numbers, as the function requires.
    let described = unsafe {
        CGWindowListCreateDescriptionFromArray(Some(&*CFRetained::cast_unchecked::<CFArray>(array)))
    };
    raws(described)
}

impl Raw {
    fn reject_reason(&self, own_pid: i32) -> Option<&'static str> {
        if self.pid == own_pid {
            Some("own process")
        } else if self.layer != 0 {
            Some("not a normal window layer")
        } else if self.alpha <= 0.0 {
            Some("transparent")
        } else if self.rect.right - self.rect.left < MIN_WIDTH
            || self.rect.bottom - self.rect.top < MIN_HEIGHT
        {
            Some("too small")
        } else {
            None
        }
    }
}

fn same_rect(a: &WinRect, b: &WinRect) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            own_pid: std::process::id() as i32,
            inner: RefCell::new(Inner {
                dirty: true,
                ..Default::default()
            }),
        }
    }

    /// Re-reads the window list unless it was read less than `max_age` ago.
    fn poll(&self, max_age: Duration) {
        let mut inner = self.inner.borrow_mut();
        if inner.polled_at.is_some_and(|at| at.elapsed() < max_age) {
            return;
        }
        let accepted: Vec<WindowInfo> = query_on_screen()
            .into_iter()
            .filter(|raw| raw.reject_reason(self.own_pid).is_none())
            .map(|raw| WindowInfo {
                hwnd: raw.id as WindowId,
                rect: raw.rect,
            })
            .collect();
        inner.polled_at = Some(Instant::now());
        if accepted
            .iter()
            .map(|w| w.hwnd)
            .ne(inner.reported.iter().copied())
        {
            inner.dirty = true;
        }
        inner.cached = accepted;
    }

    pub fn is_dirty(&self) -> bool {
        self.poll(POLL);
        self.inner.borrow().dirty
    }

    pub fn reset_dirty(&self) {
        self.inner.borrow_mut().dirty = false;
    }

    /// Visible application windows in front-to-back order.
    pub fn snapshot(&self) -> Vec<WindowInfo> {
        self.poll(SNAPSHOT_REUSE);
        let mut inner = self.inner.borrow_mut();
        let now: Vec<WindowId> = inner.cached.iter().map(|w| w.hwnd).collect();

        // A window that left the screen but still exists was minimized or hidden (a closed one is gone for good).
        let vanished: Vec<WindowId> = inner
            .reported
            .iter()
            .copied()
            .filter(|id| !now.contains(id))
            .collect();
        if !vanished.is_empty() {
            let existing = query_ids(&vanished);
            for id in vanished {
                if existing.iter().any(|raw| raw.id as WindowId == id) {
                    inner.minimized.push(id);
                }
            }
        }

        inner.reported = now;
        inner.cached.clone()
    }

    /// Re-reads the rects of `windows`, dropping those that are gone. Returns whether anything changed.
    pub fn refresh_rects(&self, windows: &mut Vec<WindowInfo>) -> bool {
        self.poll(POLL);
        let inner = self.inner.borrow();
        let mut changed = false;
        windows.retain_mut(
            |win| match inner.cached.iter().find(|w| w.hwnd == win.hwnd) {
                None => {
                    changed = true;
                    false
                }
                Some(fresh) => {
                    if !same_rect(&fresh.rect, &win.rect) {
                        win.rect = fresh.rect;
                        changed = true;
                    }
                    true
                }
            },
        );
        changed
    }

    pub fn read_rect(&self, id: WindowId) -> Option<WinRect> {
        let rect = query_ids(&[id]).into_iter().next()?.rect;
        if let Some(win) = self
            .inner
            .borrow_mut()
            .cached
            .iter_mut()
            .find(|w| w.hwnd == id)
        {
            win.rect = rect;
        }
        Some(rect)
    }

    pub fn take_minimized(&self) -> Vec<WindowId> {
        std::mem::take(&mut self.inner.borrow_mut().minimized)
    }

    /// The window being dragged: the one under the cursor whose rect changes while the left button is held.
    pub fn moving_window(&self, cursor: (i32, i32)) -> Option<WindowId> {
        let pressed = NSEvent::pressedMouseButtons() & 1 != 0;
        if !pressed {
            let mut inner = self.inner.borrow_mut();
            if inner.moving.take().is_some() {
                inner.dirty = true;
            }
            return None;
        }
        if let Some(id) = self.inner.borrow().moving {
            return Some(id);
        }

        let candidate = self
            .inner
            .borrow()
            .cached
            .iter()
            .find(|w| {
                (w.rect.left..w.rect.right).contains(&cursor.0)
                    && (w.rect.top..w.rect.bottom).contains(&cursor.1)
            })
            .map(|w| (w.hwnd, w.rect))?;
        let fresh = query_ids(&[candidate.0]).into_iter().next()?.rect;
        if same_rect(&fresh, &candidate.1) {
            return None;
        }
        self.inner.borrow_mut().moving = Some(candidate.0);
        Some(candidate.0)
    }

    /// One of the front-most windows covers a whole monitor, like a full-screen app or video.
    pub fn fullscreen_active(&self, monitors: &[Monitor]) -> bool {
        self.poll(POLL);
        self.inner.borrow().cached.iter().take(3).any(|w| {
            monitors.iter().any(|m| {
                (w.rect.left - m.left).abs() <= FULLSCREEN_SLACK
                    && (w.rect.top - m.top).abs() <= FULLSCREEN_SLACK
                    && (w.rect.right - m.right).abs() <= FULLSCREEN_SLACK
                    && (w.rect.bottom - m.bottom).abs() <= FULLSCREEN_SLACK
            })
        })
    }

    /// Lists every on-screen window with the reason it was accepted or rejected (`--dump`).
    pub fn dump(&self, monitors: &[Monitor]) -> String {
        let mut out = String::new();
        let mut accepted = Vec::new();
        for (z, raw) in query_on_screen().iter().enumerate() {
            let status = match raw.reject_reason(self.own_pid) {
                None => "ACCEPTED".to_string(),
                Some(reason) => format!("REJECTED({reason})"),
            };
            out.push_str(&format!(
                "{z}: id={}, owner=\"{}\", title=\"{}\", layer={}, rect=({},{} - {},{}), {status}\n",
                raw.id,
                raw.owner,
                raw.name.chars().take(60).collect::<String>(),
                raw.layer,
                raw.rect.left,
                raw.rect.top,
                raw.rect.right,
                raw.rect.bottom,
            ));
            if raw.reject_reason(self.own_pid).is_none() {
                accepted.push(WindowInfo {
                    hwnd: raw.id as WindowId,
                    rect: raw.rect,
                });
            }
        }

        let virt_top = monitors.iter().map(|m| m.top).min().unwrap_or(0);
        for seg in build_segments(&accepted, monitors, virt_top, Accumulate::default()) {
            out.push_str(&format!(
                "segment owner={:?}: {:.1}..{:.1} @ {:.1}\n",
                seg.owner, seg.x1, seg.x2, seg.y
            ));
        }
        out
    }
}
