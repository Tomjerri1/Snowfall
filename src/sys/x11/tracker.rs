//! Window tracking through EWMH root-window properties and X events.
//!
//! X11 has no "window moved" hook for other clients, so the tracker listens to structure events on the root window
//! (map, unmap, configure, ...) plus a few EWMH properties, and flags the window list dirty when any of them changes.
//! The window being dragged is guessed from configure events of a frame while the left mouse button is held.

use super::conn::Conn;
use crate::geometry::{build_segments, Accumulate, Monitor, WinRect, WindowInfo};
use crate::sys::WindowId;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, MapState, Window,
};
use x11rb::protocol::Event;

const MIN_WIDTH: i32 = 120;
const MIN_HEIGHT: i32 = 60;
/// How long after the last configure event of a frame it still counts as being dragged.
const DRAG_IDLE: Duration = Duration::from_millis(150);
const FULLSCREEN_CACHE: Duration = Duration::from_millis(200);
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;
const BUTTON1_MASK: u16 = 0x0100;

/// How far the visible window outline extends beyond the client area (negative for client-side shadows).
#[derive(Clone, Copy, Default)]
struct Outset {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Default)]
struct Inner {
    dirty: bool,
    minimized: Vec<WindowId>,
    /// Windows accepted by the previous snapshot.
    accepted: HashSet<Window>,
    outsets: HashMap<Window, Outset>,
    /// Window manager frame (child of the root) -> client window.
    frames: HashMap<Window, Window>,
    last_configure: Option<(Window, Instant)>,
    was_moving: bool,
    fullscreen: Option<(Instant, bool)>,
}

pub struct Tracker {
    c: Arc<Conn>,
    own: Window,
    inner: Mutex<Inner>,
}

impl Tracker {
    pub fn new(c: Arc<Conn>, own: Window) -> Result<Self, String> {
        c.conn
            .change_window_attributes(
                c.root,
                &ChangeWindowAttributesAux::new()
                    .event_mask(EventMask::PROPERTY_CHANGE | EventMask::SUBSTRUCTURE_NOTIFY),
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("cannot select root window events: {e}"))?;
        Ok(Self {
            c,
            own,
            inner: Mutex::new(Inner {
                dirty: true,
                ..Default::default()
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Feeds an X event to the tracker.
    pub fn on_event(&self, event: &Event) {
        let a = &self.c.atoms;
        let mut inner = self.lock();
        match event {
            Event::ConfigureNotify(ev) if ev.window != self.own => {
                inner.dirty = true;
                if let Some(&client) = inner.frames.get(&ev.window) {
                    inner.last_configure = Some((client, Instant::now()));
                }
            }
            Event::MapNotify(ev) if ev.window != self.own => inner.dirty = true,
            Event::UnmapNotify(ev) if ev.window != self.own => inner.dirty = true,
            Event::DestroyNotify(_) | Event::ReparentNotify(_) | Event::CirculateNotify(_) => {
                inner.dirty = true
            }
            Event::PropertyNotify(ev)
                if [
                    a._NET_CLIENT_LIST_STACKING,
                    a._NET_ACTIVE_WINDOW,
                    a._NET_CURRENT_DESKTOP,
                    a._NET_WORKAREA,
                ]
                .contains(&ev.atom) =>
            {
                inner.dirty = true
            }
            _ => {}
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.lock().dirty
    }

    pub fn reset_dirty(&self) {
        self.lock().dirty = false;
    }

    pub fn take_minimized(&self) -> Vec<WindowId> {
        std::mem::take(&mut self.lock().minimized)
    }

    /// The client window the user is dragging: its frame was reconfigured a moment ago and the left button is down.
    pub fn moving_window(&self) -> Option<WindowId> {
        let pressed = self
            .c
            .conn
            .query_pointer(self.c.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| u16::from(reply.mask) & BUTTON1_MASK != 0);

        let mut inner = self.lock();
        let moving = match inner.last_configure {
            Some((client, at)) if pressed && at.elapsed() < DRAG_IDLE => Some(client),
            _ => None,
        };
        if inner.was_moving && moving.is_none() {
            inner.dirty = true;
        }
        inner.was_moving = moving.is_some();
        moving.map(|w| w as WindowId)
    }

    fn atoms_of(&self, window: Window, property: u32) -> Vec<u32> {
        self.c.property_u32(window, property, AtomEnum::ATOM)
    }

    fn current_desktop(&self) -> Option<u32> {
        self.c
            .property_cardinal(self.c.root, self.c.atoms._NET_CURRENT_DESKTOP)
            .first()
            .copied()
    }

    fn outset_of(&self, window: Window) -> Outset {
        let a = &self.c.atoms;
        // `_GTK_FRAME_EXTENTS` is the invisible shadow around a client-side decorated window: shrink the rect by it.
        if let [l, r, t, b, ..] = self.c.property_cardinal(window, a._GTK_FRAME_EXTENTS)[..] {
            return Outset {
                left: -(l as i32),
                right: -(r as i32),
                top: -(t as i32),
                bottom: -(b as i32),
            };
        }
        if let [l, r, t, b, ..] = self.c.property_cardinal(window, a._NET_FRAME_EXTENTS)[..] {
            return Outset {
                left: l as i32,
                right: r as i32,
                top: t as i32,
                bottom: b as i32,
            };
        }
        Outset::default()
    }

    /// Why `window` is not a snow-bearing window, or its outline when it is.
    fn classify(
        &self,
        window: Window,
        current_desktop: Option<u32>,
    ) -> Result<(WinRect, Outset), &'static str> {
        let a = &self.c.atoms;
        if window == self.own {
            return Err("own");
        }

        if self
            .atoms_of(window, a._NET_WM_STATE)
            .contains(&a._NET_WM_STATE_HIDDEN)
        {
            return Err("hidden");
        }

        let types = self.atoms_of(window, a._NET_WM_WINDOW_TYPE);
        if !types.is_empty()
            && !types.contains(&a._NET_WM_WINDOW_TYPE_NORMAL)
            && !types.contains(&a._NET_WM_WINDOW_TYPE_DIALOG)
        {
            return Err("not a normal window");
        }

        if let (Some(&desktop), Some(current)) = (
            self.c.property_cardinal(window, a._NET_WM_DESKTOP).first(),
            current_desktop,
        ) {
            if desktop != ALL_DESKTOPS && desktop != current {
                return Err("other desktop");
            }
        }

        let viewable = self
            .c
            .conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|attrs| attrs.map_state == MapState::VIEWABLE);
        if !viewable {
            return Err("unmapped");
        }

        let outset = self.outset_of(window);
        let rect = self.rect_with(window, outset).ok_or("gone")?;
        if rect.right - rect.left < MIN_WIDTH || rect.bottom - rect.top < MIN_HEIGHT {
            return Err("too small");
        }
        Ok((rect, outset))
    }

    fn rect_with(&self, window: Window, outset: Outset) -> Option<WinRect> {
        let origin = self
            .c
            .conn
            .translate_coordinates(window, self.c.root, 0, 0)
            .ok()?;
        let geometry = self.c.conn.get_geometry(window).ok()?;
        let origin = origin.reply().ok()?;
        let geometry = geometry.reply().ok()?;
        Some(outline(
            origin.dst_x,
            origin.dst_y,
            geometry.width,
            geometry.height,
            outset,
        ))
    }

    /// Visible application windows in front-to-back order.
    pub fn snapshot(&self) -> Vec<WindowInfo> {
        let a = &self.c.atoms;
        let mut stack =
            self.c
                .property_u32(self.c.root, a._NET_CLIENT_LIST_STACKING, AtomEnum::WINDOW);
        stack.reverse(); // the property lists windows bottom-to-top

        let desktop = self.current_desktop();
        let mut accepted = Vec::new();
        let mut outsets = HashMap::new();
        let mut hidden = Vec::new();
        for &window in &stack {
            match self.classify(window, desktop) {
                Ok((rect, outset)) => {
                    outsets.insert(window, outset);
                    accepted.push(WindowInfo {
                        hwnd: window as WindowId,
                        rect,
                    });
                }
                Err("hidden") => hidden.push(window),
                Err(_) => {}
            }
        }

        // Map each window manager frame back to its client, so configure events of the frame can be attributed.
        let trees: Vec<_> = accepted
            .iter()
            .map(|info| {
                let client = info.hwnd as Window;
                (client, self.c.conn.query_tree(client).ok())
            })
            .collect();
        let mut frames = HashMap::new();
        for (client, cookie) in trees {
            let parent = cookie
                .and_then(|cookie| cookie.reply().ok())
                .map_or(client, |reply| reply.parent);
            let frame = if parent == self.c.root || parent == 0 {
                client
            } else {
                parent
            };
            frames.insert(frame, client);
        }

        let mut inner = self.lock();
        // A window that was shown before and is now iconified has been minimized.
        for window in hidden {
            if inner.accepted.contains(&window) {
                inner.minimized.push(window as WindowId);
            }
        }
        inner.accepted = accepted.iter().map(|w| w.hwnd as Window).collect();
        inner.outsets = outsets;
        inner.frames = frames;
        accepted
    }

    /// Re-reads the rects of `windows`, dropping those that no longer exist. Requests are pipelined.
    pub fn refresh_rects(&self, windows: &mut Vec<WindowInfo>) -> bool {
        let outsets = self.lock().outsets.clone();
        let requests: Vec<_> = windows
            .iter()
            .map(|win| {
                let id = win.hwnd as Window;
                (
                    self.c
                        .conn
                        .translate_coordinates(id, self.c.root, 0, 0)
                        .ok(),
                    self.c.conn.get_geometry(id).ok(),
                )
            })
            .collect();

        let mut replies = requests.into_iter();
        let mut changed = false;
        windows.retain_mut(|win| {
            let (origin, geometry) = replies.next().unwrap_or((None, None));
            let origin = origin.and_then(|cookie| cookie.reply().ok());
            let geometry = geometry.and_then(|cookie| cookie.reply().ok());
            let (Some(origin), Some(geometry)) = (origin, geometry) else {
                changed = true;
                return false;
            };
            let outset = outsets
                .get(&(win.hwnd as Window))
                .copied()
                .unwrap_or_default();
            let rect = outline(
                origin.dst_x,
                origin.dst_y,
                geometry.width,
                geometry.height,
                outset,
            );
            let old = win.rect;
            if (rect.left, rect.top, rect.right, rect.bottom)
                != (old.left, old.top, old.right, old.bottom)
            {
                win.rect = rect;
                changed = true;
            }
            true
        });
        changed
    }

    pub fn read_rect(&self, id: WindowId) -> Option<WinRect> {
        let window = id as Window;
        let outset = self.lock().outsets.get(&window).copied();
        let outset = outset.unwrap_or_else(|| self.outset_of(window));
        self.rect_with(window, outset)
    }

    /// The focused window is in the full-screen state.
    pub fn fullscreen_active(&self) -> bool {
        if let Some((at, value)) = self.lock().fullscreen {
            if at.elapsed() < FULLSCREEN_CACHE {
                return value;
            }
        }
        let a = &self.c.atoms;
        let active = self
            .c
            .property_u32(self.c.root, a._NET_ACTIVE_WINDOW, AtomEnum::WINDOW)
            .first()
            .copied()
            .unwrap_or(0);
        let value = active != 0
            && active != self.own
            && self
                .atoms_of(active, a._NET_WM_STATE)
                .contains(&a._NET_WM_STATE_FULLSCREEN);
        self.lock().fullscreen = Some((Instant::now(), value));
        value
    }

    fn text_property(&self, window: Window, property: u32, ty: u32) -> String {
        self.c
            .conn
            .get_property(false, window, property, ty, 0, 256)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
            .unwrap_or_default()
    }

    fn title(&self, window: Window) -> String {
        let a = &self.c.atoms;
        let title = self.text_property(window, a._NET_WM_NAME, a.UTF8_STRING);
        let title = if title.is_empty() {
            self.text_property(window, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into())
        } else {
            title
        };
        title.chars().take(60).collect()
    }

    fn class(&self, window: Window) -> String {
        // WM_CLASS is "instance\0class\0"; the class part is the useful one.
        self.text_property(window, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
            .split('\0')
            .rfind(|part| !part.is_empty())
            .unwrap_or("")
            .to_string()
    }

    /// Lists every client window with the reason it was accepted or rejected (`--dump`).
    pub fn dump(&self, monitors: &[Monitor]) -> String {
        let a = &self.c.atoms;
        let mut stack =
            self.c
                .property_u32(self.c.root, a._NET_CLIENT_LIST_STACKING, AtomEnum::WINDOW);
        stack.reverse();
        let desktop = self.current_desktop();

        let mut out = String::new();
        let mut accepted = Vec::new();
        for (z, &window) in stack.iter().enumerate() {
            let result = self.classify(window, desktop);
            let status = match &result {
                Ok(_) => "ACCEPTED".to_string(),
                Err(reason) => format!("REJECTED({reason})"),
            };
            let rect = match &result {
                Ok((rect, _)) => Some(*rect),
                Err(_) => self.rect_with(window, self.outset_of(window)),
            };
            let rect = rect.map_or("?".to_string(), |r| {
                format!("({},{} - {},{})", r.left, r.top, r.right, r.bottom)
            });
            out.push_str(&format!(
                "{z}: id=0x{window:X}, class=\"{}\", title=\"{}\", rect={rect}, {status}\n",
                self.class(window),
                self.title(window),
            ));
            if let Ok((rect, _)) = result {
                accepted.push(WindowInfo {
                    hwnd: window as WindowId,
                    rect,
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

fn outline(x: i16, y: i16, width: u16, height: u16, outset: Outset) -> WinRect {
    let (x, y) = (x as i32, y as i32);
    WinRect {
        left: x - outset.left,
        top: y - outset.top,
        right: x + width as i32 + outset.right,
        bottom: y + height as i32 + outset.bottom,
    }
}
