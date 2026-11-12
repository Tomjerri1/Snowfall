//! Tray module for system tray icon and menu handling.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, UnregisterClassW,
    GWLP_USERDATA, HICON, HWND_MESSAGE, ICONINFO, IDI_APPLICATION, MF_BYCOMMAND, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

pub const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
pub const TRAY_ID: u32 = 1;

pub struct AppState {
    pub paused: AtomicBool,
    pub target_flakes: AtomicUsize,
    pub target_fps: AtomicUsize,
    pub clear_requested: AtomicBool,
    pub quit: AtomicBool,
    pub tray_hwnd: AtomicUsize,
    pub settings_hwnd: AtomicUsize,
    pub opacity: AtomicUsize,
    pub strength: AtomicUsize,
    pub height_limit: AtomicBool,
    pub acc_windows: AtomicBool,
    pub acc_dock: AtomicBool,
    pub acc_ground: AtomicBool,
    pub mouse_flakes: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            target_flakes: AtomicUsize::new(400),
            target_fps: AtomicUsize::new(30),
            clear_requested: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            tray_hwnd: AtomicUsize::new(0),
            settings_hwnd: AtomicUsize::new(0),
            opacity: AtomicUsize::new(100),
            strength: AtomicUsize::new(100),
            height_limit: AtomicBool::new(true),
            acc_windows: AtomicBool::new(true),
            acc_dock: AtomicBool::new(true),
            acc_ground: AtomicBool::new(false),
            mouse_flakes: AtomicBool::new(true),
        }
    }
}

pub struct TrayIcon {
    hwnd: HWND,
    hicon: HICON,
}

impl TrayIcon {
    pub fn new(hwnd: HWND) -> Self {
        let hicon = unsafe { create_tray_icon() };
        unsafe {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY_CALLBACK;
            nid.hIcon = hicon;

            let tip = windows::core::w!("Snowfall");
            let dest = &mut nid.szTip;
            for (i, &c) in tip.as_wide().iter().enumerate() {
                if i < dest.len() - 1 {
                    dest[i] = c;
                } else {
                    break;
                }
            }

            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }

        Self { hwnd, hicon }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = TRAY_ID;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);

            if !self.hicon.is_invalid() {
                let _ = DestroyIcon(self.hicon);
            }
        }
    }
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_CALLBACK => {
            let l = lparam.0 as u32;
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state_arc = &*(ptr as *const Arc<AppState>);
                if l == WM_RBUTTONUP || l == 0x0400 {
                    show_context_menu(hwnd, state_arc);
                } else if l == WM_LBUTTONUP {
                    crate::settings_ui::toggle(state_arc);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE | WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn spawn_tray_thread(state: Arc<AppState>) -> JoinHandle<()> {
    thread::spawn(move || unsafe {
        let hinstance = HINSTANCE(std::ptr::null_mut());
        let class_name = windows::core::w!("SnowfallTray");

        let wnd_class = WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: Default::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: class_name,
        };

        let _ = RegisterClassW(&wnd_class);

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            windows::core::w!("SnowfallTrayWindow"),
            Default::default(),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
        .expect("Failed to create tray window");

        state.tray_hwnd.store(hwnd.0 as usize, Ordering::Release);

        let state_box = Box::new(state.clone());
        let state_ptr = Box::into_raw(state_box) as isize;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);

        let tray_icon = TrayIcon::new(hwnd);

        crate::settings_ui::create_hidden(&state);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        drop(tray_icon);

        let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        if ptr != 0 {
            let _ = Box::from_raw(ptr as *mut Arc<AppState>);
        }

        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name, hinstance);
    })
}

unsafe fn create_tray_icon() -> HICON {
    if let Ok(hinst) = GetModuleHandleW(None) {
        if let Ok(hicon) = LoadIconW(hinst, windows::core::PCWSTR(1 as *const u16)) {
            if !hicon.is_invalid() {
                return hicon;
            }
        }
    }

    let hdc_screen = GetDC(None);
    let hdc_compat = windows::Win32::Graphics::Gdi::CreateCompatibleDC(hdc_screen);
    ReleaseDC(None, hdc_screen);

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 16,
            biHeight: -16,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [Default::default(); 1],
    };

    let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbmp_color =
        match CreateDIBSection(hdc_compat, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) {
            Ok(h) => h,
            Err(_) => {
                let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc_compat);
                return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
            }
        };

    if !bits_ptr.is_null() {
        let pixels = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, 16 * 16 * 4);
        for i in (0..pixels.len()).step_by(4) {
            pixels[i] = 255;
            pixels[i + 1] = 128;
            pixels[i + 2] = 0;
            pixels[i + 3] = 255;
        }

        let draw_pixel = |px: usize, py: usize, pxls: &mut [u8]| {
            if px < 16 && py < 16 {
                let idx = (py * 16 + px) * 4;
                pxls[idx] = 255;
                pxls[idx + 1] = 255;
                pxls[idx + 2] = 255;
                pxls[idx + 3] = 255;
            }
        };

        for i in 2..14 {
            draw_pixel(i, 7, pixels);
            draw_pixel(i, 8, pixels);
            draw_pixel(7, i, pixels);
            draw_pixel(8, i, pixels);
        }
        for i in 4..12 {
            let offset = i - 4;
            draw_pixel(4 + offset, 4 + offset, pixels);
            draw_pixel(11 - offset, 4 + offset, pixels);
        }
    }

    let mask_bits = [0u8; 32];
    let hbmp_mask = CreateBitmap(16, 16, 1, 1, Some(mask_bits.as_ptr() as *const _));

    let icon_info = ICONINFO {
        fIcon: TRUE,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbmp_mask,
        hbmColor: hbmp_color,
    };

    let hicon = CreateIconIndirect(&icon_info)
        .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());

    let _ = DeleteObject(HGDIOBJ(hbmp_color.0));
    let _ = DeleteObject(HGDIOBJ(hbmp_mask.0));
    let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc_compat);

    if hicon.is_invalid() {
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
    } else {
        hicon
    }
}

pub unsafe fn show_context_menu(hwnd: HWND, state: &Arc<AppState>) {
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);

    loop {
        let _ = SetForegroundWindow(hwnd);

        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => break,
        };

        let _ = AppendMenuW(hmenu, MF_STRING, 1007, windows::core::w!("Settings..."));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());

        let is_paused = state.paused.load(Ordering::Relaxed);
        let pause_label = if is_paused {
            windows::core::w!("Resume")
        } else {
            windows::core::w!("Pause")
        };

        let _ = AppendMenuW(hmenu, MF_STRING, 1001, pause_label);
        if is_paused {
            let _ = CheckMenuItem(hmenu, 1001, (MF_BYCOMMAND | MF_CHECKED).0);
        }

        let _ = AppendMenuW(hmenu, MF_STRING, 1005, windows::core::w!("Clear snow"));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, 1006, windows::core::w!("Exit"));

        let cmd = TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );

        let _ = PostMessageW(
            hwnd,
            WM_NULL,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );

        let _ = DestroyMenu(hmenu);

        let cmd_id = cmd.0 as u32;
        if cmd_id == 0 {
            break;
        }

        match cmd_id {
            1007 => {
                crate::settings_ui::toggle(state);
            }
            1001 => {
                let current = state.paused.load(Ordering::Relaxed);
                state.paused.store(!current, Ordering::Relaxed);
            }
            1005 => {
                state.clear_requested.store(true, Ordering::Relaxed);
            }
            1006 => {
                state.quit.store(true, Ordering::Relaxed);
                break;
            }
            _ => {}
        }
    }
}
