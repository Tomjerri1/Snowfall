//! Tracker module for enumerating real application windows.

use crate::platform::{WinRect, WindowInfo};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetWindowLongW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GWL_EXSTYLE, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW,
};

struct EnumData {
    own_hwnd: HWND,
    windows: Vec<WindowInfo>,
}

static DIRTY: AtomicBool = AtomicBool::new(true);
static MOVING_HWND: AtomicIsize = AtomicIsize::new(0);
pub static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
static MINIMIZED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static IGNORED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static mut HOOKS: [HWINEVENTHOOK; 10] = [HWINEVENTHOOK(std::ptr::null_mut()); 10];

pub fn get_overlay_hwnd() -> HWND {
    HWND(OVERLAY_HWND.load(Ordering::Acquire) as *mut c_void)
}

unsafe extern "system" fn winevent_callback(
    _h_win_event_hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    if id_object != OBJID_WINDOW.0 && id_object != 0 {
        return;
    }
    if id_child != (CHILDID_SELF as i32) && id_child != 0 {
        return;
    }
    let hwnd_isize = hwnd.0 as isize;
    if event == EVENT_SYSTEM_MOVESIZESTART {
        MOVING_HWND.store(hwnd_isize, Ordering::Relaxed);
    } else if event == EVENT_SYSTEM_MOVESIZEEND {
        MOVING_HWND.store(0, Ordering::Relaxed);
        DIRTY.store(true, Ordering::Relaxed);
    } else if event == EVENT_SYSTEM_MINIMIZESTART {
        if hwnd_isize != 0 {
            if let Ok(mut min) = MINIMIZED.lock() {
                min.push(hwnd_isize);
            }
            if let Ok(mut ign) = IGNORED.lock() {
                if !ign.contains(&hwnd_isize) {
                    ign.push(hwnd_isize);
                }
            }
        }
        DIRTY.store(true, Ordering::Relaxed);
    } else if event == EVENT_SYSTEM_MINIMIZEEND {
        if hwnd_isize != 0 {
            if let Ok(mut ign) = IGNORED.lock() {
                ign.retain(|&x| x != hwnd_isize);
            }
        }
        DIRTY.store(true, Ordering::Relaxed);
    } else {
        DIRTY.store(true, Ordering::Relaxed);
    }
}

pub fn init() {
    unsafe {
        let events = [
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE,
            EVENT_OBJECT_CLOAKED,
            EVENT_OBJECT_UNCLOAKED,
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
        ];
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for (i, &event) in events.iter().enumerate() {
            let hook = SetWinEventHook(event, event, None, Some(winevent_callback), 0, 0, flags);
            if !hook.is_invalid() {
                HOOKS[i] = hook;
            }
        }
    }
}

pub fn is_dirty() -> bool {
    DIRTY.load(Ordering::Relaxed)
}

pub fn reset_dirty() {
    DIRTY.store(false, Ordering::Relaxed);
}

pub fn take_minimized() -> Vec<isize> {
    if let Ok(mut min) = MINIMIZED.lock() {
        std::mem::take(&mut *min)
    } else {
        Vec::new()
    }
}

pub fn get_moving_hwnd() -> isize {
    MOVING_HWND.load(Ordering::Relaxed)
}

pub fn read_rect(hwnd_val: isize) -> Option<WinRect> {
    let hwnd = HWND(hwnd_val as *mut c_void);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return None;
        }
        let mut win_rect = RECT::default();
        let bounds_res = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut win_rect as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        );
        if bounds_res.is_err() {
            let _ = GetWindowRect(hwnd, &mut win_rect);
        }
        Some(WinRect {
            left: win_rect.left,
            top: win_rect.top,
            right: win_rect.right,
            bottom: win_rect.bottom,
        })
    }
}

static FULLSCREEN_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

unsafe fn fullscreen_active_inner() -> bool {
    let hwnd = GetForegroundWindow();
    if hwnd.is_invalid() {
        return false;
    }

    let own = get_overlay_hwnd();
    if !own.is_invalid() && hwnd == own {
        return false;
    }

    if !IsWindowVisible(hwnd).as_bool() {
        return false;
    }

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    if class_len > 0 {
        let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
        if matches!(
            class_name.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
    }

    let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if hmonitor.is_invalid() {
        return false;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !GetMonitorInfoW(hmonitor, &mut mi).as_bool() {
        return false;
    }

    let mut win_rect = RECT::default();
    let bounds_res = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut win_rect as *mut _ as *mut c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    if bounds_res.is_err() {
        let _ = GetWindowRect(hwnd, &mut win_rect);
    }

    win_rect.left <= mi.rcMonitor.left
        && win_rect.top <= mi.rcMonitor.top
        && win_rect.right >= mi.rcMonitor.right
        && win_rect.bottom >= mi.rcMonitor.bottom
}

pub fn fullscreen_active() -> bool {
    if let Ok(mut lock) = FULLSCREEN_CACHE.lock() {
        let now = Instant::now();
        if let Some((instant, res)) = *lock {
            if now.duration_since(instant) < Duration::from_millis(200) {
                return res;
            }
        }
        let res = unsafe { fullscreen_active_inner() };
        *lock = Some((now, res));
        res
    } else {
        unsafe { fullscreen_active_inner() }
    }
}

pub fn classify(hwnd: HWND, own: HWND) -> Result<WindowInfo, &'static str> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return Err("own process");
        }

        let hwnd_isize = hwnd.0 as isize;
        if let Ok(ign) = IGNORED.lock() {
            if ign.contains(&hwnd_isize) {
                return Err("minimizing");
            }
        }

        // 1. IsWindowVisible
        if !IsWindowVisible(hwnd).as_bool() {
            return Err("invisible");
        }

        // 2. !IsIconic
        if IsIconic(hwnd).as_bool() {
            return Err("iconic");
        }

        // 3. Not cloaked (DwmGetWindowAttribute DWMWA_CLOAKED == 0)
        let mut cloaked: u32 = 0;
        let cloaked_res = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if cloaked_res.is_ok() && cloaked != 0 {
            return Err("cloaked");
        }

        // 4. Not our own overlay hwnd
        if !own.is_invalid() && hwnd == own {
            return Err("own");
        }

        // 5. Has a non-empty title
        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return Err("no title");
        }
        let mut title_buf = vec![0u16; (title_len + 1) as usize];
        let actual_len = GetWindowTextW(hwnd, &mut title_buf);
        if actual_len <= 0 {
            return Err("no title");
        }
        let title = String::from_utf16_lossy(&title_buf[..actual_len as usize]);
        if title.trim().is_empty() {
            return Err("no title");
        }

        // 6. Is not WS_EX_TOOLWINDOW
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        if (ex_style & WS_EX_TOOLWINDOW.0 as i32) != 0 {
            return Err("toolwindow");
        }

        // 7. Class name is not "Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"
        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        if class_len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
            if matches!(
                class_name.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            ) {
                return Err("shell class");
            }
        }

        // 8. Rect from DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) (fallback to GetWindowRect)
        let mut win_rect = RECT::default();
        let bounds_res = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut win_rect as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        );
        if bounds_res.is_err() {
            let _ = GetWindowRect(hwnd, &mut win_rect);
        }

        let left = win_rect.left;
        let top = win_rect.top;
        let right = win_rect.right;
        let bottom = win_rect.bottom;

        // 9. Width >= 120 and height >= 60
        let width = right - left;
        let height = bottom - top;
        if width < 120 || height < 60 {
            return Err("too small");
        }

        Ok(WindowInfo {
            hwnd: hwnd.0 as isize,
            rect: WinRect {
                left,
                top,
                right,
                bottom,
            },
        })
    }
}

unsafe extern "system" fn enum_window_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = &mut *(lparam.0 as *mut EnumData);
    if let Ok(info) = classify(hwnd, data.own_hwnd) {
        data.windows.push(info);
    }
    BOOL(1)
}

pub fn snapshot(own_hwnd: HWND) -> Vec<WindowInfo> {
    let mut data = EnumData {
        own_hwnd,
        windows: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_window_callback),
            LPARAM(&mut data as *mut EnumData as isize),
        );
    }
    data.windows
}

pub fn refresh_rects(windows: &mut Vec<WindowInfo>) -> bool {
    let mut changed = false;
    windows.retain_mut(|win| {
        let hwnd = HWND(win.hwnd as *mut std::ffi::c_void);
        unsafe {
            if !IsWindow(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                changed = true;
                return false;
            }
            let mut cloaked: u32 = 0;
            let cloaked_res = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            );
            if cloaked_res.is_ok() && cloaked != 0 {
                changed = true;
                return false;
            }

            let mut win_rect = RECT::default();
            let bounds_res = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut win_rect as *mut _ as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            );
            if bounds_res.is_err() {
                let _ = GetWindowRect(hwnd, &mut win_rect);
            }

            let new_rect = WinRect {
                left: win_rect.left,
                top: win_rect.top,
                right: win_rect.right,
                bottom: win_rect.bottom,
            };

            if new_rect.left != win.rect.left
                || new_rect.top != win.rect.top
                || new_rect.right != win.rect.right
                || new_rect.bottom != win.rect.bottom
            {
                changed = true;
                win.rect = new_rect;
            }
        }
        true
    });
    changed
}

struct DumpRecord {
    hwnd: HWND,
    class_name: String,
    title: String,
    style: i32,
    ex_style: i32,
    dwm_rect: RECT,
    win_rect: RECT,
    res: Result<WindowInfo, &'static str>,
}

struct DumpEnumData {
    own_hwnd: HWND,
    records: Vec<DumpRecord>,
}

unsafe extern "system" fn dump_enum_window_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = &mut *(lparam.0 as *mut DumpEnumData);

    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == GetCurrentProcessId() {
        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let class_name = if class_len > 0 {
            String::from_utf16_lossy(&class_buf[..class_len as usize])
        } else {
            String::new()
        };
        let style = GetWindowLongW(hwnd, windows::Win32::UI::WindowsAndMessaging::GWL_STYLE);
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let mut win_rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut win_rect);
        data.records.push(DumpRecord {
            hwnd,
            class_name,
            title: String::new(),
            style,
            ex_style,
            dwm_rect: win_rect,
            win_rect,
            res: Err("own process"),
        });
        return BOOL(1);
    }

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class_name = if class_len > 0 {
        String::from_utf16_lossy(&class_buf[..class_len as usize])
    } else {
        String::new()
    };

    let title_len = GetWindowTextLengthW(hwnd);
    let title = if title_len > 0 {
        let mut title_buf = vec![0u16; (title_len + 1) as usize];
        let actual_len = GetWindowTextW(hwnd, &mut title_buf);
        if actual_len > 0 {
            let full_title = String::from_utf16_lossy(&title_buf[..actual_len as usize]);
            full_title.chars().take(60).collect()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let style = GetWindowLongW(hwnd, windows::Win32::UI::WindowsAndMessaging::GWL_STYLE);
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

    let mut dwm_rect = RECT::default();
    let bounds_res = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut dwm_rect as *mut _ as *mut c_void,
        std::mem::size_of::<RECT>() as u32,
    );

    let mut win_rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut win_rect);
    if bounds_res.is_err() {
        dwm_rect = win_rect;
    }

    let res = classify(hwnd, data.own_hwnd);

    data.records.push(DumpRecord {
        hwnd,
        class_name,
        title,
        style,
        ex_style,
        dwm_rect,
        win_rect,
        res,
    });

    BOOL(1)
}

pub fn dump_all(own: HWND) -> String {
    let mut data = DumpEnumData {
        own_hwnd: own,
        records: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(dump_enum_window_callback),
            LPARAM(&mut data as *mut DumpEnumData as isize),
        );
    }

    let accepted_windows: Vec<WindowInfo> = data
        .records
        .iter()
        .filter_map(|r| match r.res {
            Ok(ref info) => Some(info.clone()),
            Err(_) => None,
        })
        .collect();

    let width =
        unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXVIRTUALSCREEN) };
    let height =
        unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYVIRTUALSCREEN) };
    let origin_x =
        unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_XVIRTUALSCREEN) };
    let origin_y =
        unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_YVIRTUALSCREEN) };
    let virt = (origin_x, origin_y, origin_x + width, origin_y + height);
    let floor = crate::platform::floor_y();
    let segments = crate::platform::build_segments(&accepted_windows, virt, floor);

    let mut output = String::new();
    let mut accepted_idx = 0;

    for (z, rec) in data.records.iter().enumerate() {
        let status = match &rec.res {
            Ok(_) => "ACCEPTED".to_string(),
            Err(reason) => format!("REJECTED({})", reason),
        };

        let line = format!(
            "{}: hwnd=0x{:X}, class=\"{}\", title=\"{}\", style=0x{:08X}, exstyle=0x{:08X}, dwm=({},{} - {},{}), rect=({},{} - {},{}), {}",
            z,
            rec.hwnd.0 as isize,
            rec.class_name,
            rec.title,
            rec.style,
            rec.ex_style,
            rec.dwm_rect.left,
            rec.dwm_rect.top,
            rec.dwm_rect.right,
            rec.dwm_rect.bottom,
            rec.win_rect.left,
            rec.win_rect.top,
            rec.win_rect.right,
            rec.win_rect.bottom,
            status
        );
        output.push_str(&line);
        output.push('\n');

        if rec.res.is_ok() {
            let hwnd_isize = rec.hwnd.0 as isize;
            let win_segs: Vec<_> = segments
                .iter()
                .filter(|s| s.owner == Some(hwnd_isize))
                .collect();
            if !win_segs.is_empty() {
                let seg_strs: Vec<String> = win_segs
                    .iter()
                    .map(|s| {
                        format!(
                            "{:.1}..{:.1} @ {:.1}",
                            s.x1 + origin_x as f32,
                            s.x2 + origin_x as f32,
                            s.y + origin_y as f32
                        )
                    })
                    .collect();
                output.push_str(&format!("  segments: [{}]\n", seg_strs.join(", ")));
            } else {
                let occluders = crate::platform::explain_occlusion(&accepted_windows, accepted_idx);
                if !occluders.is_empty() {
                    let occ_details: Vec<String> = occluders
                        .iter()
                        .map(|&occ_hwnd| {
                            if let Some(occ_rec) =
                                data.records.iter().find(|r| r.hwnd.0 as isize == occ_hwnd)
                            {
                                format!(
                                    "0x{:X} ({}) \"{}\"",
                                    occ_hwnd, occ_rec.class_name, occ_rec.title
                                )
                            } else {
                                format!("0x{:X}", occ_hwnd)
                            }
                        })
                        .collect();
                    output.push_str(&format!("  occluded by: [{}]\n", occ_details.join(", ")));
                } else {
                    output.push_str("  segments: [] (no occlusion found)\n");
                }
            }
            accepted_idx += 1;
        }
    }

    output
}
