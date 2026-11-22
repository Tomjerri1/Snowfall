//! Monitor enumeration.

use crate::geometry::{sort_monitors, Monitor};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};

fn floor_y() -> f32 {
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

unsafe extern "system" fn monitor_enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        list.push(Monitor {
            left: mi.rcMonitor.left,
            top: mi.rcMonitor.top,
            right: mi.rcMonitor.right,
            bottom: mi.rcMonitor.bottom,
            work_top: mi.rcWork.top,
            work_bottom: mi.rcWork.bottom,
            primary: mi.dwFlags & 1 != 0,
        });
    }
    BOOL(1)
}

/// All monitors, primary first, then left-to-right. `virt` is (left, top, right, bottom) of the virtual screen and is used for a single fallback monitor when enumeration returns nothing.
pub fn monitors(virt: (i32, i32, i32, i32)) -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    if list.is_empty() {
        list.push(Monitor {
            left: virt.0,
            top: virt.1,
            right: virt.2,
            bottom: virt.3,
            work_top: virt.1,
            work_bottom: floor_y() as i32,
            primary: true,
        });
    }
    sort_monitors(&mut list);
    list
}
