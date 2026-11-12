//! Platform module for visible top-edge segments and floor.

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};

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

pub fn floor_y() -> f32 {
    let mut rect = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut std::ffi::c_void),
            Default::default(),
        );
    }
    if rect.bottom > 0 {
        rect.bottom as f32
    } else {
        1080.0
    }
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

pub trait BuildSegmentsArg3 {
    fn build(self, windows_list: &[WindowInfo], virt: (i32, i32, i32, i32)) -> Vec<Segment>;
}

impl BuildSegmentsArg3 for f32 {
    fn build(self, windows_list: &[WindowInfo], virt: (i32, i32, i32, i32)) -> Vec<Segment> {
        build_segments_inner(windows_list, virt, self, true, true, false)
    }
}

impl BuildSegmentsArg3 for (f32, bool, bool, bool) {
    fn build(self, windows_list: &[WindowInfo], virt: (i32, i32, i32, i32)) -> Vec<Segment> {
        build_segments_inner(windows_list, virt, self.0, self.1, self.2, self.3)
    }
}

pub fn build_segments<A: BuildSegmentsArg3>(
    windows_list: &[WindowInfo],
    virt: (i32, i32, i32, i32),
    arg3: A,
) -> Vec<Segment> {
    arg3.build(windows_list, virt)
}

fn build_segments_inner(
    windows_list: &[WindowInfo],
    virt: (i32, i32, i32, i32),
    floor: f32,
    windows: bool,
    dock: bool,
    ground: bool,
) -> Vec<Segment> {
    let virt_left = virt.0 as f32;
    let virt_top = virt.1 as f32;
    let virt_right = virt.2 as f32;
    let virt_bottom = virt.3 as f32;

    let mut work_rect = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work_rect as *mut RECT as *mut std::ffi::c_void),
            Default::default(),
        );
    }
    let work_area_top = work_rect.top as f32;

    let mut segments = Vec::new();

    // Floor segment spans the whole virtual screen width at y = floor, never reduced by occlusion.
    if dock {
        segments.push(Segment {
            x1: virt_left,
            x2: virt_right,
            y: floor,
            owner: None,
            full_left: virt_left,
            full_right: virt_right,
        });
    }

    if ground {
        segments.push(Segment {
            x1: virt_left,
            x2: virt_right,
            y: virt_bottom,
            owner: Some(crate::snow::GROUND_ID),
            full_left: virt_left,
            full_right: virt_right,
        });
    }

    // Window top-edge segments
    if windows {
        for (i, win) in windows_list.iter().enumerate() {
            let left = win.rect.left as f32;
            let right = win.rect.right as f32;
            let top = win.rect.top as f32;

            // Skip a window only if its top <= virtual top + 2, or it is truly docked/maximized:
            // left <= virt left + 2 && right >= virt right - 2 && top <= work-area top + 2.
            if top <= virt_top + 2.0
                || (left <= virt_left + 2.0
                    && right >= virt_right - 2.0
                    && top <= work_area_top + 2.0)
            {
                continue;
            }

            // Occlusion: subtract from the top edge every x-interval of any window j with a smaller index (higher in z-order)
            // whose vertical range [top_j, bottom_j) contains the edge's y.
            let mut blockers = Vec::new();
            for j in 0..i {
                let blocker_win = &windows_list[j];
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

pub fn explain_occlusion(windows: &[WindowInfo], idx: usize) -> Vec<isize> {
    let win = &windows[idx];
    let left = win.rect.left as f32;
    let right = win.rect.right as f32;
    let top = win.rect.top as f32;

    let mut occluding = Vec::new();
    for j in 0..idx {
        let blocker = &windows[j];
        let top_j = blocker.rect.top as f32;
        let bottom_j = blocker.rect.bottom as f32;
        let bx1 = blocker.rect.left as f32;
        let bx2 = blocker.rect.right as f32;

        if top_j <= top && bottom_j > top && bx1 < right && bx2 > left {
            if !occluding.contains(&blocker.hwnd) {
                occluding.push(blocker.hwnd);
            }
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
