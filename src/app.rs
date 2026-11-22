//! Shared application state: settings and commands exchanged between the render loop and the UI.

use std::sync::atomic::{AtomicBool, AtomicUsize};

// The update fields are only used by the Windows settings window so far.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct AppState {
    pub paused: AtomicBool,
    pub target_flakes: AtomicUsize,
    pub target_fps: AtomicUsize,
    pub clear_requested: AtomicBool,
    pub quit: AtomicBool,
    pub opacity: AtomicUsize,
    pub strength: AtomicUsize,
    pub height_limit: AtomicBool,
    pub acc_windows: AtomicBool,
    pub acc_dock: AtomicBool,
    pub acc_ground: AtomicBool,
    pub mouse_flakes: AtomicBool,
    pub update_state: AtomicUsize,
    pub update_tag: std::sync::Mutex<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            target_flakes: AtomicUsize::new(400),
            target_fps: AtomicUsize::new(30),
            clear_requested: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            opacity: AtomicUsize::new(100),
            strength: AtomicUsize::new(100),
            height_limit: AtomicBool::new(true),
            acc_windows: AtomicBool::new(true),
            acc_dock: AtomicBool::new(true),
            acc_ground: AtomicBool::new(false),
            mouse_flakes: AtomicBool::new(true),
            update_state: AtomicUsize::new(0),
            update_tag: std::sync::Mutex::new(String::new()),
        }
    }
}
