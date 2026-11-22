//! Platform-independent geometry: window rects, monitors and the visible top-edge segments snow can land on.

#[derive(Debug, Clone, Copy)]
pub struct WinRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub rect: WinRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub x1: f32,
    pub x2: f32,
    pub y: f32,
    pub owner: Option<isize>,
    pub full_left: f32,
    pub full_right: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub work_top: i32,
    pub work_bottom: i32,
    pub primary: bool,
}

/// Orders monitors primary first, then left-to-right, top-to-bottom.
pub fn sort_monitors(list: &mut [Monitor]) {
    list.sort_by_key(|m| (!m.primary, m.left, m.top));
}

pub fn subtract_intervals(x1: f32, x2: f32, blockers: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if x1 >= x2 {
        return Vec::new();
    }
    let mut intervals = vec![(x1, x2)];

    for &(bx1, bx2) in blockers {
        if bx1 >= bx2 {
            continue;
        }
        let mut next_intervals = Vec::new();
        for (s_start, s_end) in intervals {
            if bx2 <= s_start || bx1 >= s_end {
                next_intervals.push((s_start, s_end));
            } else {
                if bx1 > s_start {
                    next_intervals.push((s_start, bx1));
                }
                if bx2 < s_end {
                    next_intervals.push((bx2, s_end));
                }
            }
        }
        intervals = next_intervals;
        if intervals.is_empty() {
            break;
        }
    }

    intervals
}

/// Which surfaces collect snow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accumulate {
    pub windows: bool,
    pub dock: bool,
    pub ground: bool,
}

impl Default for Accumulate {
    fn default() -> Self {
        Self {
            windows: true,
            dock: true,
            ground: false,
        }
    }
}

/// Builds the landing segments for `windows_list` (front-to-back z-order) and the per-monitor dock/ground floors.
/// `mons` must be primary first (see `sort_monitors`) and non-empty; `virt_top` is the top of the virtual screen.
pub fn build_segments(
    windows_list: &[WindowInfo],
    mons: &[Monitor],
    virt_top: i32,
    acc: Accumulate,
) -> Vec<Segment> {
    let Accumulate {
        windows,
        dock,
        ground,
    } = acc;
    let virt_top = virt_top as f32;

    let mut segments = Vec::new();

    // Each monitor gets its own dock floor (at its work-area bottom) and its own ground (at its bottom edge).
    for (i, m) in mons.iter().enumerate() {
        let left = m.left as f32;
        let right = m.right as f32;
        if dock {
            segments.push(Segment {
                x1: left,
                x2: right,
                y: m.work_bottom as f32,
                owner: crate::snow::dock_owner(i),
                full_left: left,
                full_right: right,
            });
        }
        if ground {
            segments.push(Segment {
                x1: left,
                x2: right,
                y: m.bottom as f32,
                owner: crate::snow::ground_owner(i),
                full_left: left,
                full_right: right,
            });
        }
    }

    // Window top-edge segments
    if windows {
        for (i, win) in windows_list.iter().enumerate() {
            let left = win.rect.left as f32;
            let right = win.rect.right as f32;
            let top = win.rect.top as f32;

            // Skip a window whose top is at the virtual top, or that fills the work area of the monitor it sits on (maximized/docked).
            // Pick the monitor under the window's top edge (slightly below it, to ignore invisible borders).
            let win_cx = (left + right) * 0.5;
            let win_cy = top + 16.0;
            let win_mon = mons
                .iter()
                .find(|m| {
                    win_cx >= m.left as f32
                        && win_cx < m.right as f32
                        && win_cy >= m.top as f32
                        && win_cy < m.bottom as f32
                })
                .or_else(|| mons.first());
            let covers_work_area = win_mon.is_some_and(|m| {
                left <= m.left as f32 + 2.0
                    && right >= m.right as f32 - 2.0
                    && top <= m.work_top as f32 + 2.0
            });
            if top <= virt_top + 2.0 || covers_work_area {
                continue;
            }

            // Occlusion: subtract from the top edge every x-interval of any window j with a smaller index (higher in z-order)
            // whose vertical range [top_j, bottom_j) contains the edge's y.
            let mut blockers = Vec::new();
            for blocker_win in windows_list.iter().take(i) {
                let top_j = blocker_win.rect.top as f32;
                let bottom_j = blocker_win.rect.bottom as f32;
                if top_j <= top && bottom_j > top {
                    blockers.push((blocker_win.rect.left as f32, blocker_win.rect.right as f32));
                }
            }

            let visible_parts = subtract_intervals(left, right, &blockers);
            if visible_parts.is_empty() {
                // Expose window full edge even if fully occluded
                segments.push(Segment {
                    x1: left,
                    x2: left,
                    y: top,
                    owner: Some(win.hwnd),
                    full_left: left,
                    full_right: right,
                });
            } else {
                for (sx1, sx2) in visible_parts {
                    segments.push(Segment {
                        x1: sx1,
                        x2: sx2,
                        y: top,
                        owner: Some(win.hwnd),
                        full_left: left,
                        full_right: right,
                    });
                }
            }
        }
    }

    segments
}

#[cfg_attr(not(windows), allow(dead_code))] // only the Windows --dump uses it
pub fn explain_occlusion(windows: &[WindowInfo], idx: usize) -> Vec<isize> {
    let win = &windows[idx];
    let left = win.rect.left as f32;
    let right = win.rect.right as f32;
    let top = win.rect.top as f32;

    let mut occluding = Vec::new();
    for blocker in windows.iter().take(idx) {
        let top_j = blocker.rect.top as f32;
        let bottom_j = blocker.rect.bottom as f32;
        let bx1 = blocker.rect.left as f32;
        let bx2 = blocker.rect.right as f32;

        if top_j <= top
            && bottom_j > top
            && bx1 < right
            && bx2 > left
            && !occluding.contains(&blocker.hwnd)
        {
            occluding.push(blocker.hwnd);
        }
    }
    occluding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtract_no_overlap() {
        let res = subtract_intervals(0.0, 200.0, &[(50.0, 100.0)]);
        assert_eq!(res, vec![(0.0, 50.0), (100.0, 200.0)]);
    }

    #[test]
    fn test_subtract_partial_overlap() {
        let res = subtract_intervals(100.0, 200.0, &[(150.0, 250.0)]);
        assert_eq!(res, vec![(100.0, 150.0)]);
    }

    #[test]
    fn test_subtract_fully_covered() {
        let res = subtract_intervals(100.0, 200.0, &[(50.0, 250.0)]);
        assert!(res.is_empty());
    }

    #[test]
    fn test_subtract_two_blockers() {
        let res = subtract_intervals(0.0, 200.0, &[(50.0, 80.0), (120.0, 150.0)]);
        assert_eq!(res, vec![(0.0, 50.0), (80.0, 120.0), (150.0, 200.0)]);
    }
}
