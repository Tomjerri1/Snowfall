//! Tray icon and settings menu through the StatusNotifierItem D-Bus protocol (KDE, and GNOME with the AppIndicator extension).
//!
//! There is no settings window: every setting is a radio or check item in the tray menu, which is rebuilt from
//! `AppState` each time it opens.

use crate::app::AppState;
use crate::sys::tray_shared::{
    nearest, snowflake_rgba, Presets, FPS, INTENSITY, OPACITY, STRENGTH,
};
use crate::update;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const RELEASES_URL: &str = "https://github.com/Tomjerri1/Snowfall/releases/latest";

pub struct SnowTray {
    state: Arc<AppState>,
    icon: Vec<Icon>,
}

/// A submenu with one radio item per preset; the preset closest to the current value is selected.
fn radio(
    label: &str,
    options: Presets,
    field: fn(&AppState) -> &AtomicUsize,
    state: &AppState,
) -> MenuItem<SnowTray> {
    let current = field(state).load(Ordering::Relaxed);
    let selected = nearest(options, current);
    SubMenu {
        label: label.into(),
        submenu: vec![RadioGroup {
            selected,
            select: Box::new(move |tray: &mut SnowTray, index| {
                if let Some((_, value)) = options.get(index) {
                    field(&tray.state).store(*value, Ordering::Relaxed);
                }
            }),
            options: options
                .iter()
                .map(|(name, _)| RadioItem {
                    label: (*name).into(),
                    ..Default::default()
                })
                .collect(),
        }
        .into()],
        ..Default::default()
    }
    .into()
}

fn check(label: &str, field: fn(&AppState) -> &AtomicBool, state: &AppState) -> MenuItem<SnowTray> {
    CheckmarkItem {
        label: label.into(),
        checked: field(state).load(Ordering::Relaxed),
        activate: Box::new(move |tray: &mut SnowTray| {
            let flag = field(&tray.state);
            flag.store(!flag.load(Ordering::Relaxed), Ordering::Relaxed);
        }),
        ..Default::default()
    }
    .into()
}

fn button(label: String, activate: impl Fn(&mut SnowTray) + Send + 'static) -> MenuItem<SnowTray> {
    StandardItem {
        label,
        activate: Box::new(activate),
        ..Default::default()
    }
    .into()
}

fn open_releases_page() {
    let _ = std::process::Command::new("xdg-open")
        .arg(RELEASES_URL)
        .spawn();
}

impl ksni::Tray for SnowTray {
    // A left click opens the menu, like a right click.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "snowfall".into()
    }

    fn title(&self) -> String {
        "Snowfall".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    // Overriding the default lets the host rebuild the menu from AppState every time it opens.
    fn menu_about_to_show(&mut self) {}

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = &*self.state;
        let paused = state.paused.load(Ordering::Relaxed);

        let update_item = match state.update_state.load(Ordering::Relaxed) {
            update::STATE_CHECKING => {
                button("Checking for updates...".into(), |_: &mut SnowTray| {})
            }
            update::STATE_AVAILABLE => {
                let tag = state
                    .update_tag
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                button(format!("Download update {tag}"), |_: &mut SnowTray| {
                    open_releases_page()
                })
            }
            other => {
                let label = match other {
                    update::STATE_UP_TO_DATE => "Up to date (check again)",
                    update::STATE_FAILED => "Check failed (retry)",
                    _ => "Check for updates",
                };
                button(label.into(), |tray: &mut SnowTray| {
                    update::start_check(&tray.state, || {});
                })
            }
        };

        vec![
            CheckmarkItem {
                label: "Pause".into(),
                checked: paused,
                activate: Box::new(|tray: &mut SnowTray| {
                    let paused = tray.state.paused.load(Ordering::Relaxed);
                    tray.state.paused.store(!paused, Ordering::Relaxed);
                }),
                ..Default::default()
            }
            .into(),
            button("Clear snow".into(), |tray: &mut SnowTray| {
                tray.state.clear_requested.store(true, Ordering::Relaxed);
            }),
            MenuItem::Separator,
            radio("Intensity", INTENSITY, |s| &s.target_flakes, state),
            radio("Opacity", OPACITY, |s| &s.opacity, state),
            radio("Strength", STRENGTH, |s| &s.strength, state),
            radio("Frame rate", FPS, |s| &s.target_fps, state),
            SubMenu {
                label: "Accumulate on".into(),
                submenu: vec![
                    check("Windows", |s| &s.acc_windows, state),
                    check("Panel", |s| &s.acc_dock, state),
                    check("Ground", |s| &s.acc_ground, state),
                ],
                ..Default::default()
            }
            .into(),
            check("Height limit", |s| &s.height_limit, state),
            check("Mouse pushes snow", |s| &s.mouse_flakes, state),
            MenuItem::Separator,
            update_item,
            button("Quit".into(), |tray: &mut SnowTray| {
                tray.state.quit.store(true, Ordering::Relaxed);
            }),
        ]
    }
}

/// The shared snowflake as ARGB32 in network byte order, which is what the protocol wants.
fn snowflake_icon(size: u32) -> Icon {
    let rgba = snowflake_rgba(size);
    let mut data = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    Icon {
        width: size as i32,
        height: size as i32,
        data,
    }
}

/// Starts the tray service in the background. Returns `None` (after printing why) if D-Bus is unavailable.
pub fn spawn(state: Arc<AppState>) -> Option<Handle<SnowTray>> {
    let tray = SnowTray {
        state,
        icon: vec![snowflake_icon(32), snowflake_icon(22)],
    };
    // Spawn even when no StatusNotifierWatcher is running yet: the icon then appears once one starts.
    match tray.assume_sni_available(true).spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("snowfall: tray icon unavailable: {e}");
            None
        }
    }
}
