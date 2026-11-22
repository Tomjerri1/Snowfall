//! Shared X server connection and the atoms the backend needs.

use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST_STACKING,
        _NET_CURRENT_DESKTOP,
        _NET_FRAME_EXTENTS,
        _NET_WM_DESKTOP,
        _NET_WM_NAME,
        _NET_WM_STATE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_HIDDEN,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WORKAREA,
        _GTK_FRAME_EXTENTS,
        UTF8_STRING,
    }
}

pub struct Conn {
    pub conn: RustConnection,
    pub screen_num: usize,
    pub root: Window,
    pub atoms: Atoms,
}

impl Conn {
    pub fn connect() -> Result<Arc<Self>, String> {
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|e| format!("cannot connect to the X server (is DISPLAY set?): {e}"))?;
        let root = conn.setup().roots[screen_num].root;
        let atoms = Atoms::new(&conn)
            .map_err(|e| format!("cannot intern atoms: {e}"))?
            .reply()
            .map_err(|e| format!("cannot intern atoms: {e}"))?;
        Ok(Arc::new(Self {
            conn,
            screen_num,
            root,
            atoms,
        }))
    }

    pub fn screen(&self) -> &x11rb::protocol::xproto::Screen {
        &self.conn.setup().roots[self.screen_num]
    }

    /// Reads a 32-bit property as a list of values. Missing or mistyped properties give an empty list.
    pub fn property_u32(&self, window: Window, property: u32, ty: impl Into<u32>) -> Vec<u32> {
        self.conn
            .get_property(false, window, property, ty, 0, 4096)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|values| values.collect()))
            .unwrap_or_default()
    }

    pub fn property_cardinal(&self, window: Window, property: u32) -> Vec<u32> {
        self.property_u32(window, property, AtomEnum::CARDINAL)
    }
}
