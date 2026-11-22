//! Tray icon and settings menu through the StatusNotifierItem D-Bus protocol (KDE, and GNOME with the AppIndicator extension).
//!
//! There is no settings window: every setting is a radio or check item in the tray menu, which is rebuilt from
//! `AppState` each time it opens.

use crate::app::AppState;
use crate::update;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const RELEASES_URL: &str = "https://github.com/Tomjerri1/Snowfall/releases/latest";

const INTENSITY: &[(&str, usize)] = &[
    ("Light", 150),
    ("Normal", 400),
    ("Medium", 800),
    ("Heavy", 1500),
    ("Blizzard", 3000),
];
const OPACITY: &[(&str, usize)] = &[("25%", 25), ("50%", 50), ("75%", 75), ("100%", 100)];
const STRENGTH: &[(&str, usize)] = &[("50%", 50), ("100%", 100), ("150%", 150), ("200%", 200)];
const FPS: &[(&str, usize)] = &[("20", 20), ("30", 30), ("60", 60)];

pub struct SnowTray {
    state: Arc<AppState>,
    icon: Vec<Icon>,
}

/// A submenu with one radio item per preset; the preset closest to the current value is selected.
fn radio(
    label: &str,
    options: &'static [(&'static str, usize)],
    field: fn(&AppState) -> &AtomicUsize,
    state: &AppState,
) -> MenuItem<SnowTray> {
    let current = field(state).load(Ordering::Relaxed);
    let selected = options
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, value))| value.abs_diff(current))
        .map_or(0, |(index, _)| index);
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

/// A white snowflake on a blue disc, as ARGB32 in network byte order.
fn snowflake_icon(size: u32) -> Icon {
    use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

    let mut pixmap = Pixmap::new(size, size).expect("icon size is non-zero");
    let center = size as f32 / 2.0;

    let mut paint = Paint::default();
    paint.set_color_rgba8(0x2E, 0x7D, 0xD7, 0xFF);
    if let Some(disc) = PathBuilder::from_circle(center, center, center - 0.5) {
        pixmap.fill_path(
            &disc,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    paint.set_color_rgba8(0xFF, 0xFF, 0xFF, 0xFF);
    let stroke = Stroke {
        width: size as f32 / 11.0,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    let arm = center * 0.68;
    let mut builder = PathBuilder::new();
    for step in 0..3 {
        let angle = step as f32 * std::f32::consts::PI / 3.0;
        let (dx, dy) = (angle.cos() * arm, angle.sin() * arm);
        builder.move_to(center - dx, center - dy);
        builder.line_to(center + dx, center + dy);
    }
    if let Some(arms) = builder.finish() {
        pixmap.stroke_path(&arms, &paint, &stroke, Transform::identity(), None);
    }

    // tiny-skia stores premultiplied RGBA; the protocol wants straight ARGB.
    let mut data = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.data().chunks_exact(4) {
        let alpha = px[3];
        let straight = |c: u8| {
            if alpha == 0 {
                0
            } else {
                (c as u32 * 255 / alpha as u32).min(255) as u8
            }
        };
        data.extend_from_slice(&[alpha, straight(px[0]), straight(px[1]), straight(px[2])]);
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
