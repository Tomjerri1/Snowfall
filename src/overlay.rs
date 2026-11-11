//! Overlay module for layered window and presentation.

use std::ffi::c_void;
use std::sync::Arc;
use windows::Win32::Foundation::{HWND, WPARAM, LPARAM, LRESULT, HINSTANCE, POINT, SIZE, COLORREF};
use windows::Win32::Graphics::Gdi::{
    HDC, HBITMAP, HGDIOBJ, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    BLENDFUNCTION, AC_SRC_OVER, AC_SRC_ALPHA
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetSystemMetrics, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    TranslateMessage, UnregisterClassW, ShowWindow, SW_SHOW, UpdateLayeredWindow,
    ULW_ALPHA,
    IDC_ARROW, MSG, PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WM_CLOSE, WM_DESTROY, WM_HOTKEY,
    WM_QUIT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_ALT};
use crate::tray::AppState;

pub struct Overlay {
    hwnd: HWND,
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
    hdc: HDC,
    hmem_dc: HDC,
    hbmp: HBITMAP,
    old_bmp: HGDIOBJ,
    bits: *mut c_void,
}

unsafe impl Send for Overlay {}
unsafe impl Sync for Overlay {}

static mut QUIT_REQUESTED: bool = false;

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if wparam.0 == 1 {
                unsafe { QUIT_REQUESTED = true; }
            }
            LRESULT(0)
        }
        WM_DESTROY | WM_CLOSE => {
            unsafe {
                QUIT_REQUESTED = true;
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Overlay {
    pub fn new(_state: Arc<AppState>) -> Self {
        unsafe {
            let hinstance = HINSTANCE(std::ptr::null_mut());
            let class_name = windows::core::w!("SnowfallOverlayClass");

            let wnd_class = WNDCLASSW {
                style: Default::default(),
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: Default::default(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: Default::default(),
                lpszMenuName: windows::core::PCWSTR::null(),
                lpszClassName: class_name,
            };

            let _ = RegisterClassW(&wnd_class);

            let origin_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let origin_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);

            let ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let style = WS_POPUP;

            let hwnd = CreateWindowExW(
                ex_style,
                class_name,
                windows::core::w!("SnowfallOverlay"),
                style,
                origin_x,
                origin_y,
                width,
                height,
                None,
                None,
                hinstance,
                None,
            ).expect("Failed to create overlay window");

            crate::tracker::OVERLAY_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::Release);

            // Register global hotkey Ctrl+Alt+Q (ID 1)
            let _ = RegisterHotKey(hwnd, 1, MOD_CONTROL | MOD_ALT, 0x51); // 0x51 is 'Q'

            let hdc = GetDC(hwnd);
            let hmem_dc = CreateCompatibleDC(hdc);

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down
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

            let mut bits_ptr: *mut c_void = std::ptr::null_mut();
            let hbmp = CreateDIBSection(
                hmem_dc,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits_ptr,
                None,
                0,
            ).expect("Failed to create DIB section");

            let old_bmp = SelectObject(hmem_dc, HGDIOBJ(hbmp.0));

            let _ = ShowWindow(hwnd, SW_SHOW);

            Self {
                hwnd,
                origin_x,
                origin_y,
                width,
                height,
                hdc,
                hmem_dc,
                hbmp,
                old_bmp,
                bits: bits_ptr,
            }
        }
    }

    pub fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        let len = (self.width * self.height * 4) as usize;
        let slice = unsafe { std::slice::from_raw_parts_mut(self.bits as *mut u8, len) };
        tiny_skia::PixmapMut::from_bytes(slice, self.width as u32, self.height as u32)
            .expect("Failed to create PixmapMut from DIB bits")
    }

    pub fn present(&mut self) {
        if self.width <= 0 || self.height <= 0 || self.bits.is_null() {
            return;
        }

        unsafe {
            let pt_src = POINT { x: 0, y: 0 };
            let sz = SIZE { cx: self.width, cy: self.height };
            let pt_dst = POINT { x: self.origin_x, y: self.origin_y };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&pt_dst),
                Some(&sz),
                self.hmem_dc,
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    pub fn pump_messages(&mut self) -> bool {
        unsafe {
            if QUIT_REQUESTED {
                return false;
            }
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).into() {
                if msg.message == WM_QUIT {
                    QUIT_REQUESTED = true;
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            !QUIT_REQUESTED
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(self.hwnd, 1);
            if !self.old_bmp.0.is_null() {
                let _ = SelectObject(self.hmem_dc, self.old_bmp);
            }
            if !self.hbmp.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.hbmp.0));
            }
            if !self.hmem_dc.is_invalid() {
                let _ = DeleteDC(self.hmem_dc);
            }
            if !self.hdc.is_invalid() {
                let _ = ReleaseDC(self.hwnd, self.hdc);
            }
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(windows::core::w!("SnowfallOverlayClass"), HINSTANCE(std::ptr::null_mut()));
        }
    }
}
