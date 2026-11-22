//! macOS backend: a borderless NSWindow overlay, CGWindowList window tracking and a status bar item.
#![allow(dead_code)] // the pieces are connected by the backend in a later commit

mod overlay;
mod screens;
