//! X11 backend: ARGB override-redirect overlay, EWMH window tracking and an XGrabKey hotkey.
#![allow(dead_code)] // the pieces are connected by the backend in a later commit

mod conn;
mod monitors;
mod overlay;
mod tracker;
