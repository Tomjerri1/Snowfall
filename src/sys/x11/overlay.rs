//! Transparent, click-through overlay window covering the whole root window.
//!
//! The window uses a 32-bit ARGB visual, so a compositing window manager blends it with the desktop.
//! It is override-redirect (the window manager never decorates or moves it) and its input shape is empty,
//! which lets all mouse events pass through to the windows below.

use super::conn::Conn;
use super::shm::ShmBuffer;
use std::sync::Arc;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{
    ClipOrdering, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, Gcontext, ImageFormat, ImageOrder, PropMode, StackMode,
    VisualClass, Visualid, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

pub struct Overlay {
    c: Arc<Conn>,
    window: Window,
    gc: Gcontext,
    width: u16,
    height: u16,
    pixels: Pixels,
    /// A `ShmPutImage` was sent and its completion event has not arrived, so the server may still read `pixels`.
    shm_pending: bool,
    visible: bool,
}

/// Premultiplied BGRA, top-down, `width * height * 4` bytes.
enum Pixels {
    /// Uploaded with `PutImage`.
    Heap(Vec<u8>),
    /// Shared with the X server, uploaded with `ShmPutImage`.
    Shared(ShmBuffer),
}

impl Pixels {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Pixels::Heap(buf) => buf,
            Pixels::Shared(shm) => shm.as_mut_slice(),
        }
    }
}

/// Finds a 32-bit TrueColor visual with the usual ARGB layout.
fn argb_visual(c: &Conn) -> Option<Visualid> {
    c.screen()
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR && v.red_mask == 0x00ff_0000)
        .map(|v| v.visual_id)
}

impl Overlay {
    pub fn new(c: Arc<Conn>) -> Result<Self, String> {
        let screen = c.screen();
        let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);

        if c.conn.setup().image_byte_order != ImageOrder::LSB_FIRST {
            eprintln!("snowfall: big-endian X servers are not supported, colors may be wrong");
        }

        let visual = argb_visual(&c).ok_or(
            "the X server has no 32-bit ARGB visual, so a transparent overlay is impossible",
        )?;
        shape::query_version(&c.conn)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("the X server lacks the SHAPE extension: {e}"))?;

        let map_err = |e: x11rb::errors::ConnectionError| e.to_string();
        let colormap = c.conn.generate_id().map_err(|e| e.to_string())?;
        c.conn
            .create_colormap(ColormapAlloc::NONE, colormap, c.root, visual)
            .map_err(map_err)?;

        let window = c.conn.generate_id().map_err(|e| e.to_string())?;
        c.conn
            .create_window(
                32,
                window,
                c.root,
                0,
                0,
                width,
                height,
                0,
                WindowClass::INPUT_OUTPUT,
                visual,
                &CreateWindowAux::new()
                    .background_pixel(0)
                    .border_pixel(0)
                    .override_redirect(1)
                    .colormap(colormap)
                    .event_mask(EventMask::NO_EVENT),
            )
            .map_err(map_err)?;

        c.conn
            .change_property8(
                PropMode::REPLACE,
                window,
                x11rb::protocol::xproto::AtomEnum::WM_NAME,
                x11rb::protocol::xproto::AtomEnum::STRING,
                b"SnowfallOverlay",
            )
            .map_err(map_err)?;

        // An empty input region makes the window invisible to the mouse.
        c.conn
            .shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )
            .map_err(map_err)?;

        let gc = c.conn.generate_id().map_err(|e| e.to_string())?;
        c.conn
            .create_gc(gc, window, &CreateGCAux::new())
            .map_err(map_err)?;

        c.conn.map_window(window).map_err(map_err)?;
        c.conn
            .configure_window(
                window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )
            .map_err(map_err)?;
        c.conn.flush().map_err(map_err)?;

        let len = width as usize * height as usize * 4;
        let pixels = match ShmBuffer::create(&c, len) {
            Ok(shm) => Pixels::Shared(shm),
            Err(reason) => {
                eprintln!(
                    "snowfall: MIT-SHM unavailable ({reason}), uploading frames with PutImage"
                );
                Pixels::Heap(vec![0; len])
            }
        };

        Ok(Self {
            c,
            window,
            gc,
            width,
            height,
            pixels,
            shm_pending: false,
            visible: true,
        })
    }

    pub fn window(&self) -> Window {
        self.window
    }

    /// Virtual screen as (left, top, right, bottom).
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        (0, 0, self.width as i32, self.height as i32)
    }

    pub fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        tiny_skia::PixmapMut::from_bytes(
            self.pixels.as_mut_slice(),
            self.width as u32,
            self.height as u32,
        )
        .expect("overlay buffer matches its size")
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        let _ = if visible {
            self.c.conn.map_window(self.window).map(drop)
        } else {
            self.c.conn.unmap_window(self.window).map(drop)
        };
        if visible {
            let _ = self.c.conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        }
        let _ = self.c.conn.flush();
    }

    /// Raises the overlay above windows that were mapped after it.
    pub fn raise(&self) {
        let _ = self.c.conn.configure_window(
            self.window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
    }

    /// True while the X server may still be reading the shared buffer; drawing must wait until
    /// [`Overlay::shm_completed`] is called for the `ShmCompletion` event.
    pub fn shm_pending(&self) -> bool {
        self.shm_pending
    }

    pub fn shm_completed(&mut self) {
        self.shm_pending = false;
    }

    /// Shows the pixel buffer in the window.
    pub fn present(&mut self) {
        if !self.visible {
            return;
        }
        match &mut self.pixels {
            Pixels::Shared(shm) => {
                let seg = shm.seg();
                let sent = self.c.conn.shm_put_image(
                    self.window,
                    self.gc,
                    self.width,
                    self.height,
                    0,
                    0,
                    self.width,
                    self.height,
                    0,
                    0,
                    32,
                    ImageFormat::Z_PIXMAP.into(),
                    true,
                    seg,
                    0,
                );
                self.shm_pending = sent.is_ok();
                let _ = self.c.conn.flush();
            }
            Pixels::Heap(buf) => {
                put_image_in_bands(&self.c, self.window, self.gc, self.width, self.height, buf)
            }
        }
    }
}

/// Uploads `buf` with `PutImage`, in horizontal bands that fit into one X request.
fn put_image_in_bands(c: &Conn, window: Window, gc: Gcontext, width: u16, height: u16, buf: &[u8]) {
    {
        let row_bytes = width as usize * 4;
        // A PutImage request has a 24-byte header; leave some more slack.
        let max_bytes = c.conn.maximum_request_bytes().saturating_sub(64);
        let rows_per_band = (max_bytes / row_bytes).clamp(1, height as usize);

        let mut y = 0usize;
        while y < height as usize {
            let rows = rows_per_band.min(height as usize - y);
            let data = &buf[y * row_bytes..(y + rows) * row_bytes];
            if c.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    window,
                    gc,
                    width,
                    rows as u16,
                    0,
                    y as i16,
                    0,
                    32,
                    data,
                )
                .is_err()
            {
                return;
            }
            y += rows;
        }
        let _ = c.conn.flush();
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.c.conn.free_gc(self.gc);
        let _ = self.c.conn.destroy_window(self.window);
        let _ = self.c.conn.flush();
    }
}
