//! Persistent settings stored in %APPDATA%\Snowfall\settings.txt (simple key=value lines).

use crate::tray::AppState;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    let mut p = PathBuf::from(base);
    p.push("Snowfall");
    p.push("settings.txt");
    Some(p)
}

/// Current values of all persisted settings, used to detect changes.
pub fn snapshot(state: &AppState) -> [usize; 9] {
    [
        state.target_flakes.load(Ordering::Relaxed),
        state.target_fps.load(Ordering::Relaxed),
        state.opacity.load(Ordering::Relaxed),
        state.height_limit.load(Ordering::Relaxed) as usize,
        state.acc_windows.load(Ordering::Relaxed) as usize,
        state.acc_dock.load(Ordering::Relaxed) as usize,
        state.acc_ground.load(Ordering::Relaxed) as usize,
        state.mouse_flakes.load(Ordering::Relaxed) as usize,
        state.strength.load(Ordering::Relaxed),
    ]
}

/// Parses and applies settings from text to `state`.
pub fn apply_settings(state: &AppState, text: &str) {
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Ok(v) = value.trim().parse::<usize>() else {
            continue;
        };
        match key.trim() {
            "flakes" => state
                .target_flakes
                .store(v.clamp(50, 3000), Ordering::Relaxed),
            "fps" => {
                if matches!(v, 20 | 30 | 60) {
                    state.target_fps.store(v, Ordering::Relaxed);
                }
            }
            "opacity" => state.opacity.store(v.clamp(10, 100), Ordering::Relaxed),
            "strength" => state.strength.store(v.clamp(25, 200), Ordering::Relaxed),
            "height_limit" => state.height_limit.store(v != 0, Ordering::Relaxed),
            "acc_windows" => state.acc_windows.store(v != 0, Ordering::Relaxed),
            "acc_dock" => state.acc_dock.store(v != 0, Ordering::Relaxed),
            "acc_ground" => state.acc_ground.store(v != 0, Ordering::Relaxed),
            "mouse_flakes" => state.mouse_flakes.store(v != 0, Ordering::Relaxed),
            _ => {}
        }
    }
}

/// Loads saved settings into `state`. Missing file or bad values are ignored.
pub fn load(state: &AppState) {
    let Some(path) = config_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    apply_settings(state, &text);
}

/// Formats current settings into string representation.
pub fn format_settings(state: &AppState) -> String {
    let s = snapshot(state);
    format!(
        "flakes={}\nfps={}\nopacity={}\nheight_limit={}\nacc_windows={}\nacc_dock={}\nacc_ground={}\nmouse_flakes={}\nstrength={}\n",
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8]
    )
}

/// Saves current settings. Errors are ignored (settings are best-effort).
pub fn save(state: &AppState) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = format_settings(state);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
