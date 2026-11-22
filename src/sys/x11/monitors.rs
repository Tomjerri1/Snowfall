//! Monitor layout from RandR, with work areas taken from `_NET_WORKAREA`.

use super::conn::Conn;
use crate::geometry::{sort_monitors, Monitor};
use x11rb::protocol::randr::ConnectionExt as _;

/// The work area (x, y, w, h) of the current desktop, i.e. the screen minus panels and docks.
fn work_area(c: &Conn) -> Option<(i32, i32, i32, i32)> {
    let areas = c.property_cardinal(c.root, c.atoms._NET_WORKAREA);
    let desktop = c
        .property_cardinal(c.root, c.atoms._NET_CURRENT_DESKTOP)
        .first()
        .copied()
        .unwrap_or(0) as usize;
    let start = if areas.len() >= (desktop + 1) * 4 {
        desktop * 4
    } else {
        0
    };
    let a = areas.get(start..start + 4)?;
    Some((a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32))
}

/// All monitors, primary first. Falls back to one monitor covering the whole root window if RandR is unavailable.
pub fn monitors(c: &Conn) -> Vec<Monitor> {
    let screen = c.screen();
    let wa = work_area(c);

    let mut list: Vec<Monitor> = c
        .conn
        .randr_get_monitors(c.root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| {
            reply
                .monitors
                .iter()
                .map(|m| {
                    let (left, top) = (m.x as i32, m.y as i32);
                    let (right, bottom) = (left + m.width as i32, top + m.height as i32);
                    let (mut work_top, mut work_bottom) = (top, bottom);
                    if let Some((_, wy, _, wh)) = wa {
                        work_top = work_top.max(wy).min(bottom);
                        work_bottom = work_bottom.min(wy + wh).max(work_top);
                    }
                    Monitor {
                        left,
                        top,
                        right,
                        bottom,
                        work_top,
                        work_bottom,
                        primary: m.primary,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if list.is_empty() {
        let (right, bottom) = (
            screen.width_in_pixels as i32,
            screen.height_in_pixels as i32,
        );
        let (work_top, work_bottom) = wa.map_or((0, bottom), |(_, y, _, h)| {
            (y.clamp(0, bottom), (y + h).clamp(0, bottom))
        });
        list.push(Monitor {
            left: 0,
            top: 0,
            right,
            bottom,
            work_top,
            work_bottom,
            primary: true,
        });
    }

    // RandR marks at most one monitor primary; if none is, the first one acts as primary.
    if !list.iter().any(|m| m.primary) {
        list[0].primary = true;
    }
    sort_monitors(&mut list);
    list
}
