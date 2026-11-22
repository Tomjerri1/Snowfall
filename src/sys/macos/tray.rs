//! Status bar item with all settings in its menu.
//!
//! Built once with `tray-icon`; radio groups are check items whose state is re-synced from `AppState` after every
//! click and twice a second (which also picks up the result of a background update check).

use crate::app::AppState;
use crate::sys::tray_shared::{
    nearest, snowflake_rgba, Presets, FPS, INTENSITY, OPACITY, STRENGTH,
};
use crate::update;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const RELEASES_URL: &str = "https://github.com/Tomjerri1/Snowfall/releases/latest";
const SYNC_INTERVAL: Duration = Duration::from_millis(500);
/// Menu bar icons are about 22 points tall; this is the @2x size.
const ICON_SIZE: u32 = 44;

type NumberField = fn(&AppState) -> &AtomicUsize;
type FlagField = fn(&AppState) -> &AtomicBool;

#[derive(Clone, Copy)]
enum Action {
    Pause,
    Clear,
    Update,
    Quit,
    Set(NumberField, usize),
    Flip(FlagField),
}

struct Radio {
    presets: Presets,
    field: NumberField,
    items: Vec<CheckMenuItem>,
}

struct Flag {
    field: FlagField,
    item: CheckMenuItem,
}

pub struct TrayUi {
    _tray: TrayIcon,
    state: Arc<AppState>,
    actions: HashMap<MenuId, Action>,
    pause: CheckMenuItem,
    update: MenuItem,
    radios: Vec<Radio>,
    flags: Vec<Flag>,
    last_sync: Instant,
}

/// Collects menu items together with the action each one triggers.
struct Builder {
    actions: HashMap<MenuId, Action>,
}

impl Builder {
    fn button(&mut self, label: &str, action: Action) -> MenuItem {
        let item = MenuItem::new(label, true, None);
        self.actions.insert(item.id().clone(), action);
        item
    }

    fn check(&mut self, label: &str, action: Action) -> CheckMenuItem {
        let item = CheckMenuItem::new(label, true, false, None);
        self.actions.insert(item.id().clone(), action);
        item
    }

    fn radio(&mut self, label: &str, presets: Presets, field: NumberField) -> (Submenu, Radio) {
        let submenu = Submenu::new(label, true);
        let items: Vec<CheckMenuItem> = presets
            .iter()
            .map(|&(name, value)| {
                let item = self.check(name, Action::Set(field, value));
                let _ = submenu.append(&item);
                item
            })
            .collect();
        (
            submenu,
            Radio {
                presets,
                field,
                items,
            },
        )
    }
}

impl TrayUi {
    pub fn new(state: Arc<AppState>) -> Result<Self, String> {
        let mut b = Builder {
            actions: HashMap::new(),
        };
        let menu = Menu::new();
        let sep = PredefinedMenuItem::separator;
        let add = |item: &dyn IsMenuItem| menu.append(item).map_err(|e| e.to_string());

        let pause = b.check("Pause", Action::Pause);
        add(&pause)?;
        add(&b.button("Clear snow", Action::Clear))?;
        add(&sep())?;

        let mut radios = Vec::new();
        for (label, presets, field) in [
            (
                "Intensity",
                INTENSITY,
                (|s| &s.target_flakes) as NumberField,
            ),
            ("Opacity", OPACITY, |s| &s.opacity),
            ("Strength", STRENGTH, |s| &s.strength),
            ("Frame rate", FPS, |s| &s.target_fps),
        ] {
            let (submenu, radio) = b.radio(label, presets, field);
            add(&submenu)?;
            radios.push(radio);
        }

        let mut flags = Vec::new();
        let accumulate = Submenu::new("Accumulate on", true);
        let mut flag = |b: &mut Builder, label: &str, field: FlagField| {
            let item = b.check(label, Action::Flip(field));
            flags.push(Flag {
                field,
                item: item.clone(),
            });
            item
        };
        for (label, field) in [
            ("Windows", (|s| &s.acc_windows) as FlagField),
            ("Dock", |s| &s.acc_dock),
            ("Ground", |s| &s.acc_ground),
        ] {
            let item = flag(&mut b, label, field);
            accumulate.append(&item).map_err(|e| e.to_string())?;
        }
        add(&accumulate)?;
        add(&flag(&mut b, "Height limit", |s| &s.height_limit))?;
        add(&flag(&mut b, "Mouse pushes snow", |s| &s.mouse_flakes))?;
        add(&sep())?;

        let update = b.button("Check for updates", Action::Update);
        add(&update)?;
        add(&b.button("Quit", Action::Quit))?;

        let icon = Icon::from_rgba(snowflake_rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
            .map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Snowfall")
            .with_icon(icon)
            .build()
            .map_err(|e| format!("cannot create the status bar item: {e}"))?;

        let mut ui = Self {
            _tray: tray,
            state,
            actions: b.actions,
            pause,
            update,
            radios,
            flags,
            last_sync: Instant::now(),
        };
        ui.sync();
        Ok(ui)
    }

    /// Handles menu clicks and refreshes the check marks. Call on the main thread, regularly.
    pub fn poll(&mut self) {
        let mut clicked = false;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = self.actions.get(&event.id).copied() {
                self.run(action);
                clicked = true;
            }
        }
        if clicked || self.last_sync.elapsed() >= SYNC_INTERVAL {
            self.sync();
        }
    }

    fn run(&self, action: Action) {
        let state = &self.state;
        match action {
            Action::Pause => {
                let paused = state.paused.load(Ordering::Relaxed);
                state.paused.store(!paused, Ordering::Relaxed);
            }
            Action::Clear => state.clear_requested.store(true, Ordering::Relaxed),
            Action::Quit => state.quit.store(true, Ordering::Relaxed),
            Action::Set(field, value) => field(state).store(value, Ordering::Relaxed),
            Action::Flip(field) => {
                let flag = field(state);
                flag.store(!flag.load(Ordering::Relaxed), Ordering::Relaxed);
            }
            Action::Update => match state.update_state.load(Ordering::Relaxed) {
                update::STATE_AVAILABLE => {
                    let _ = std::process::Command::new("open").arg(RELEASES_URL).spawn();
                }
                update::STATE_CHECKING => {}
                _ => update::start_check(state, || {}),
            },
        }
    }

    fn sync(&mut self) {
        self.last_sync = Instant::now();
        let state = &self.state;

        self.pause.set_checked(state.paused.load(Ordering::Relaxed));
        for radio in &self.radios {
            let selected = nearest(radio.presets, (radio.field)(state).load(Ordering::Relaxed));
            for (index, item) in radio.items.iter().enumerate() {
                item.set_checked(index == selected);
            }
        }
        for flag in &self.flags {
            flag.item
                .set_checked((flag.field)(state).load(Ordering::Relaxed));
        }

        let label = match state.update_state.load(Ordering::Relaxed) {
            update::STATE_CHECKING => "Checking for updates...".to_string(),
            update::STATE_UP_TO_DATE => "Up to date (check again)".to_string(),
            update::STATE_FAILED => "Check failed (retry)".to_string(),
            update::STATE_AVAILABLE => {
                let tag = state
                    .update_tag
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                format!("Download update {tag}")
            }
            _ => "Check for updates".to_string(),
        };
        self.update.set_text(label);
    }
}
