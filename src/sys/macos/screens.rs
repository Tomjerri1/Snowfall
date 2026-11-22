//! Screen layout in Quartz coordinates.
//!
//! Cocoa measures `y` upwards from the bottom of the primary screen, while Quartz (window lists, the overlay buffer)
//! measures it downwards from the top. The rest of the app uses the Quartz convention, so everything is converted here.

use crate::geometry::{sort_monitors, Monitor};
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

#[derive(Clone)]
pub struct Layout {
    /// Virtual screen as (left, top, right, bottom).
    pub bounds: (i32, i32, i32, i32),
    /// Height of the primary screen, the pivot of the Cocoa <-> Quartz `y` conversion.
    pub primary_height: f64,
    /// Primary first. `work_*` exclude the menu bar and the Dock.
    pub monitors: Vec<Monitor>,
}

impl Layout {
    pub fn read(mtm: MainThreadMarker) -> Self {
        let screens = NSScreen::screens(mtm);
        // The first screen is the one with the menu bar and the origin of the Cocoa coordinate space.
        let primary_height = screens
            .iter()
            .next()
            .map_or(1080.0, |s| s.frame().size.height);

        let mut monitors: Vec<Monitor> = screens
            .iter()
            .enumerate()
            .map(|(index, screen)| {
                let frame = screen.frame();
                let work = screen.visibleFrame();
                Monitor {
                    left: frame.origin.x.round() as i32,
                    top: (primary_height - frame.origin.y - frame.size.height).round() as i32,
                    right: (frame.origin.x + frame.size.width).round() as i32,
                    bottom: (primary_height - frame.origin.y).round() as i32,
                    work_top: (primary_height - work.origin.y - work.size.height).round() as i32,
                    work_bottom: (primary_height - work.origin.y).round() as i32,
                    primary: index == 0,
                }
            })
            .collect();

        if monitors.is_empty() {
            monitors.push(Monitor {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
                work_top: 0,
                work_bottom: 1080,
                primary: true,
            });
        }

        let bounds = monitors.iter().fold(
            (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
            |(l, t, r, b), m| (l.min(m.left), t.min(m.top), r.max(m.right), b.max(m.bottom)),
        );
        sort_monitors(&mut monitors);
        Self {
            bounds,
            primary_height,
            monitors,
        }
    }

    /// Converts a Cocoa `y` (up from the primary screen bottom) to Quartz (down from its top).
    pub fn quartz_y(&self, cocoa_y: f64) -> f64 {
        self.primary_height - cocoa_y
    }
}
