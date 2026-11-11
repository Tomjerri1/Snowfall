//! Settings UI module for snowfall configuration window.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::Win32::Foundation::{HWND, WPARAM, LPARAM, LRESULT, HINSTANCE, RECT, COLORREF};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateCompatibleBitmap, CreateSolidBrush, CreateFontW, CreatePen,
    DeleteDC, DeleteObject, SelectObject, BitBlt, FillRect, SetBkMode, SetTextColor,
    DrawTextW, InvalidateRect, BeginPaint, EndPaint, DRAW_TEXT_FORMAT,
    HGDIOBJ, TRANSPARENT, DT_CENTER, DT_VCENTER, DT_SINGLELINE, DT_LEFT,
    FW_NORMAL, FW_BOLD, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, DEFAULT_PITCH,
    RoundRect, Ellipse, Pie, MoveToEx, LineTo, PS_SOLID
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WNDCLASSW, ShowWindow,
    SW_SHOW, SW_HIDE, IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW,
    GetWindowLongPtrW, GWLP_USERDATA, SetTimer, KillTimer,
    GetClientRect, AdjustWindowRectEx, GetWindowRect, SetWindowPos,
    SystemParametersInfoW, SPI_GETWORKAREA,
    WS_POPUP, WS_CAPTION, WS_SYSMENU, WS_EX_TOPMOST, WS_EX_TOOLWINDOW,
    HWND_TOPMOST, SWP_NOSIZE, SWP_NOACTIVATE,
    WM_CLOSE, WM_DESTROY, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_LBUTTONUP, WM_PAINT, WM_TIMER, WM_CREATE, WM_ERASEBKGND,
    LoadCursorW, LoadIconW, IDC_ARROW
};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, ReleaseCapture};
use crate::tray::AppState;

// Layout constants
const MARGIN_X: i32 = 20;
const TRACK_WIDTH: i32 = 300;
const SLIDER1_TOP: i32 = 15;
const SLIDER2_TOP: i32 = 80;
const ROW_HEIGHT: i32 = 55;
const TRACK_HEIGHT: i32 = 6;
const THUMB_SIZE: i32 = 16;

const ACC_HEADER_TOP: i32 = 145;
const TILES_TOP: i32 = 170;
const TILE_WIDTH: i32 = 90;
const TILE_HEIGHT: i32 = 66;
const TILE_GAP: i32 = 15;

const SWITCH1_TOP: i32 = 250;
const SWITCH2_TOP: i32 = 295;
const SWITCH_ROW_HEIGHT: i32 = 35;
const SWITCH_WIDTH: i32 = 44;
const SWITCH_HEIGHT: i32 = 22;

const FPS_TOP: i32 = 345;
const BUTTON_TOP: i32 = 400;
const CLIENT_WIDTH: i32 = 340;
const CLIENT_HEIGHT: i32 = 455;

#[derive(Clone, Copy, PartialEq)]
enum SliderKind {
    Intensity,
    Opacity,
}

struct SettingsData {
    state: Arc<AppState>,
    dragging: Option<SliderKind>,
}

fn place_window(hwnd: HWND) {
    let mut work = RECT::default();
    let mut rect = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut std::ffi::c_void),
            Default::default(),
        );
        let _ = GetWindowRect(hwnd, &mut rect);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let x = work.right - w - 12;
        let y = work.bottom - h - 12;
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Toggle the settings window visibility or create it if not yet created.
pub fn toggle(state: &Arc<AppState>) {
    let hwnd_val = state.settings_hwnd.load(Ordering::Acquire);
    if hwnd_val == 0 {
        create_hidden(state);
    }
    let hwnd_val = state.settings_hwnd.load(Ordering::Acquire);
    if hwnd_val != 0 {
        let hwnd = HWND(hwnd_val as *mut std::ffi::c_void);
        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            } else {
                place_window(hwnd);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

pub fn create_hidden(state: &Arc<AppState>) {
    let hwnd_val = state.settings_hwnd.load(Ordering::Acquire);
    if hwnd_val != 0 {
        return;
    }
    unsafe {
        let hinstance = HINSTANCE(std::ptr::null_mut());
        let class_name = windows::core::w!("SnowfallSettings");

        let hcursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hicon = GetModuleHandleW(None)
            .and_then(|h| LoadIconW(h, windows::core::PCWSTR(1 as *const u16)))
            .unwrap_or_default();

        let wnd_class = WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(settings_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: hicon,
            hCursor: hcursor,
            hbrBackground: Default::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: class_name,
        };

        let _ = RegisterClassW(&wnd_class);

        let mut rect = RECT { left: 0, top: 0, right: CLIENT_WIDTH, bottom: CLIENT_HEIGHT };
        let ex_style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let _ = AdjustWindowRectEx(&mut rect, style, false, ex_style);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            windows::core::w!("Snowfall Settings"),
            style,
            -32000, -32000,
            width,
            height,
            None,
            None,
            hinstance,
            None,
        ).expect("Failed to create settings window");

        state.settings_hwnd.store(hwnd.0 as usize, Ordering::Release);

        let settings_data = Box::new(SettingsData {
            state: state.clone(),
            dragging: None,
        });
        let data_ptr = Box::into_raw(settings_data) as isize;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr);

        // Enable immersive dark mode (attribute 20)
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(20),
            &dark as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );

        place_window(hwnd);
    }
}

unsafe extern "system" fn settings_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let _ = SetTimer(hwnd, 1, 250, None);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == 1 {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let cx = client_rect.right - client_rect.left;
            let cy = client_rect.bottom - client_rect.top;

            let mem_dc = CreateCompatibleDC(hdc);
            let hbmp = CreateCompatibleBitmap(hdc, cx, cy);
            let old_bmp = SelectObject(mem_dc, HGDIOBJ(hbmp.0));

            // Background RGB(32,33,36) -> BGR: 0x00242120
            let bg_brush = CreateSolidBrush(COLORREF(0x00242120));
            FillRect(mem_dc, &client_rect, bg_brush);
            let _ = DeleteObject(HGDIOBJ(bg_brush.0));

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let data = &*(ptr as *const SettingsData);
                draw_content(mem_dc, &data.state);
            }

            let _ = BitBlt(hdc, 0, 0, cx, cy, mem_dc, 0, 0, windows::Win32::Graphics::Gdi::SRCCOPY);

            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            let _ = DeleteDC(mem_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let data = &mut *(ptr as *mut SettingsData);
                if let Some(slider) = hit_test_slider(x, y) {
                    data.dragging = Some(slider);
                    let _ = SetCapture(hwnd);
                    update_slider_from_mouse(data, slider, x);
                    let _ = InvalidateRect(hwnd, None, false);
                } else if handle_click(&data.state, x, y) {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let data = &mut *(ptr as *mut SettingsData);
                if let Some(slider) = data.dragging {
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    update_slider_from_mouse(data, slider, x);
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let data = &mut *(ptr as *mut SettingsData);
                if data.dragging.is_some() {
                    data.dragging = None;
                    let _ = ReleaseCapture();
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, 1);
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if ptr != 0 {
                let _ = Box::from_raw(ptr as *mut SettingsData);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Button rect helpers
fn fps_btn_rect(fps: usize) -> RECT {
    let idx = match fps {
        20 => 0,
        60 => 2,
        _ => 1,
    };
    let x = 160 + (idx * 50);
    RECT { left: x, top: FPS_TOP, right: x + 45, bottom: FPS_TOP + 40 }
}

fn pause_btn_rect() -> RECT {
    RECT { left: 20, top: BUTTON_TOP, right: 160, bottom: BUTTON_TOP + 40 }
}

fn clear_btn_rect() -> RECT {
    RECT { left: 180, top: BUTTON_TOP, right: 320, bottom: BUTTON_TOP + 40 }
}

fn hit_test_slider(x: i32, y: i32) -> Option<SliderKind> {
    if x >= MARGIN_X && x <= MARGIN_X + TRACK_WIDTH {
        if y >= SLIDER1_TOP && y < SLIDER1_TOP + ROW_HEIGHT {
            return Some(SliderKind::Intensity);
        }
        if y >= SLIDER2_TOP && y < SLIDER2_TOP + ROW_HEIGHT {
            return Some(SliderKind::Opacity);
        }
    }
    None
}

fn update_slider_from_mouse(data: &SettingsData, slider: SliderKind, x: i32) {
    let track_left = MARGIN_X;
    let ratio = ((x - track_left) as f32 / TRACK_WIDTH as f32).clamp(0.0, 1.0);

    match slider {
        SliderKind::Intensity => {
            let val = 50 + ((ratio * 2950.0 / 50.0).round() * 50.0) as usize;
            let val = val.clamp(50, 3000);
            data.state.target_flakes.store(val, Ordering::Relaxed);
        }
        SliderKind::Opacity => {
            let val = 10 + (ratio * 90.0).round() as usize;
            let val = val.clamp(10, 100);
            data.state.opacity.store(val, Ordering::Relaxed);
        }
    }
}

unsafe fn draw_text_str(hdc: windows::Win32::Graphics::Gdi::HDC, s: &str, rc: &mut RECT, format: DRAW_TEXT_FORMAT) {
    let mut utf16: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = DrawTextW(hdc, &mut utf16, rc, format);
}

unsafe fn draw_slider(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    title: &str,
    value_str: &str,
    row_top: i32,
    min_val: f32,
    max_val: f32,
    current_val: f32,
) {
    let track_left = MARGIN_X;
    let track_right = MARGIN_X + TRACK_WIDTH;
    let track_y = row_top + 32;

    // 1. Label above (title left, value right)
    let mut title_rc = RECT { left: MARGIN_X, top: row_top + 5, right: MARGIN_X + 150, bottom: row_top + 25 };
    let _ = SetTextColor(hdc, COLORREF(0x00DEDEDE));
    draw_text_str(hdc, title, &mut title_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let mut val_rc = RECT { left: MARGIN_X + 150, top: row_top + 5, right: MARGIN_X + TRACK_WIDTH, bottom: row_top + 25 };
    let _ = SetTextColor(hdc, COLORREF(0x007C8084));
    draw_text_str(hdc, value_str, &mut val_rc, DRAW_TEXT_FORMAT(2) | DT_VCENTER | DT_SINGLELINE);

    // 2. Track (6 px high rounded track, RGB 70,72,78 -> BGR 0x004E4846)
    let track_brush = CreateSolidBrush(COLORREF(0x004E4846));
    let old_brush = SelectObject(hdc, HGDIOBJ(track_brush.0));
    let _ = RoundRect(hdc, track_left, track_y, track_right, track_y + TRACK_HEIGHT, TRACK_HEIGHT, TRACK_HEIGHT);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(HGDIOBJ(track_brush.0));

    // 3. Filled part (RGB 120,170,255 -> BGR 0x00FFAA78)
    let ratio = ((current_val - min_val) / (max_val - min_val)).clamp(0.0, 1.0);
    let thumb_x = track_left as f32 + ratio * TRACK_WIDTH as f32;
    let fill_right = (thumb_x as i32).clamp(track_left, track_right);

    if fill_right > track_left {
        let fill_brush = CreateSolidBrush(COLORREF(0x00FFAA78));
        let old_brush2 = SelectObject(hdc, HGDIOBJ(fill_brush.0));
        let _ = RoundRect(hdc, track_left, track_y, fill_right, track_y + TRACK_HEIGHT, TRACK_HEIGHT, TRACK_HEIGHT);
        SelectObject(hdc, old_brush2);
        let _ = DeleteObject(HGDIOBJ(fill_brush.0));
    }

    // 4. Round white thumb (16 px)
    let thumb_radius = THUMB_SIZE / 2;
    let thumb_cx = thumb_x as i32;
    let thumb_cy = track_y + TRACK_HEIGHT / 2;
    let thumb_left = thumb_cx - thumb_radius;
    let thumb_right = thumb_cx + thumb_radius;
    let thumb_top = thumb_cy - thumb_radius;
    let thumb_bottom = thumb_cy + thumb_radius;

    let white_brush = CreateSolidBrush(COLORREF(0x00FFFFFF));
    let old_brush3 = SelectObject(hdc, HGDIOBJ(white_brush.0));
    let _ = Ellipse(hdc, thumb_left, thumb_top, thumb_right, thumb_bottom);
    SelectObject(hdc, old_brush3);
    let _ = DeleteObject(HGDIOBJ(white_brush.0));
}

unsafe fn draw_tile(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    title: &str,
    is_on: bool,
    left: i32,
    top: i32,
    hfont_normal: windows::Win32::Graphics::Gdi::HGDIOBJ,
    tile_kind: &str,
) {
    let right = left + TILE_WIDTH;
    let bottom = top + TILE_HEIGHT;

    // Background brush: RGB(45,46,50) -> BGR 0x00322E2D
    let bg_brush = CreateSolidBrush(COLORREF(0x00322E2D));
    let border_color = if is_on { COLORREF(0x00FFAA78) } else { COLORREF(0x008A8482) };
    let pen = CreatePen(PS_SOLID, 1, border_color);

    let old_brush = SelectObject(hdc, HGDIOBJ(bg_brush.0));
    let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));

    let _ = RoundRect(hdc, left, top, right, bottom, 8, 8);

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(HGDIOBJ(pen.0));
    let _ = DeleteObject(HGDIOBJ(bg_brush.0));

    // Icon and label color
    let fg_color = if is_on { COLORREF(0x00FFFFFF) } else { COLORREF(0x008A8482) };
    let pen_fg = CreatePen(PS_SOLID, 1, fg_color);
    let brush_fg = CreateSolidBrush(fg_color);
    let old_pen2 = SelectObject(hdc, HGDIOBJ(pen_fg.0));
    let old_brush2 = SelectObject(hdc, HGDIOBJ(brush_fg.0));

    let icon_l = left + 25;
    let icon_t = top + 8;
    let icon_r = left + 65;
    let icon_b = top + 30;

    match tile_kind {
        "windows" => {
            let _ = RoundRect(hdc, icon_l, icon_t, icon_r, icon_b, 4, 4);
            let bar_brush = CreateSolidBrush(fg_color);
            let bar_rect = RECT { left: icon_l, top: icon_t, right: icon_r, bottom: icon_t + 5 };
            let _ = FillRect(hdc, &bar_rect, bar_brush);
            let _ = DeleteObject(HGDIOBJ(bar_brush.0));
        }
        "dock" => {
            let _ = RoundRect(hdc, icon_l, icon_t, icon_r, icon_b, 4, 4);
            let bar_brush = CreateSolidBrush(fg_color);
            let bar_rect = RECT { left: icon_l, top: icon_b - 7, right: icon_r, bottom: icon_b };
            let _ = FillRect(hdc, &bar_rect, bar_brush);
            let _ = DeleteObject(HGDIOBJ(bar_brush.0));
        }
        "ground" => {
            let _ = Pie(hdc, icon_l, icon_t, icon_r, icon_b, icon_r, icon_b, icon_l, icon_b);
            let _ = MoveToEx(hdc, icon_l, icon_b, None);
            let _ = LineTo(hdc, icon_r, icon_b);
        }
        _ => {}
    }

    SelectObject(hdc, old_brush2);
    SelectObject(hdc, old_pen2);
    let _ = DeleteObject(HGDIOBJ(brush_fg.0));
    let _ = DeleteObject(HGDIOBJ(pen_fg.0));

    // Label at bottom
    let mut label_rc = RECT { left: left + 5, top: top + 36, right: right - 5, bottom: bottom - 4 };
    let _ = SetTextColor(hdc, fg_color);
    SelectObject(hdc, hfont_normal);
    draw_text_str(hdc, title, &mut label_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

unsafe fn draw_switch(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    title: &str,
    is_on: bool,
    row_top: i32,
    hfont_normal: windows::Win32::Graphics::Gdi::HGDIOBJ,
) {
    let row_left = MARGIN_X;
    let row_right = MARGIN_X + TRACK_WIDTH;

    // Label on the left
    let mut title_rc = RECT { left: row_left, top: row_top, right: row_left + 220, bottom: row_top + SWITCH_ROW_HEIGHT };
    let _ = SetTextColor(hdc, COLORREF(0x00DEDEDE));
    SelectObject(hdc, hfont_normal);
    draw_text_str(hdc, title, &mut title_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    // Pill switch on the right
    let switch_l = row_right - SWITCH_WIDTH;
    let switch_t = row_top + (SWITCH_ROW_HEIGHT - SWITCH_HEIGHT) / 2;
    let switch_r = row_right;
    let switch_b = switch_t + SWITCH_HEIGHT;

    let track_color = if is_on { COLORREF(0x00FFAA78) } else { COLORREF(0x004E4846) };
    let track_brush = CreateSolidBrush(track_color);
    let pen_null = CreatePen(PS_SOLID, 1, track_color);
    let old_brush = SelectObject(hdc, HGDIOBJ(track_brush.0));
    let old_pen = SelectObject(hdc, HGDIOBJ(pen_null.0));

    let _ = RoundRect(hdc, switch_l, switch_t, switch_r, switch_b, SWITCH_HEIGHT, SWITCH_HEIGHT);

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(HGDIOBJ(pen_null.0));
    let _ = DeleteObject(HGDIOBJ(track_brush.0));

    // White round knob
    let knob_size = SWITCH_HEIGHT - 4;
    let knob_t = switch_t + 2;
    let knob_l = if is_on {
        switch_r - 2 - knob_size
    } else {
        switch_l + 2
    };
    let knob_r = knob_l + knob_size;
    let knob_b = knob_t + knob_size;

    let white_brush = CreateSolidBrush(COLORREF(0x00FFFFFF));
    let old_brush2 = SelectObject(hdc, HGDIOBJ(white_brush.0));
    let _ = Ellipse(hdc, knob_l, knob_t, knob_r, knob_b);
    SelectObject(hdc, old_brush2);
    let _ = DeleteObject(HGDIOBJ(white_brush.0));
}

unsafe fn draw_content(hdc: windows::Win32::Graphics::Gdi::HDC, state: &AppState) {
    let hfont_normal = CreateFontW(
        15, 0, 0, 0,
        FW_NORMAL.0 as i32,
        0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        DEFAULT_QUALITY.0 as u32,
        DEFAULT_PITCH.0 as u32,
        windows::core::w!("Segoe UI"),
    );
    let hfont_bold = CreateFontW(
        15, 0, 0, 0,
        FW_BOLD.0 as i32,
        0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        DEFAULT_QUALITY.0 as u32,
        DEFAULT_PITCH.0 as u32,
        windows::core::w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, HGDIOBJ(hfont_normal.0));
    let _ = SetBkMode(hdc, TRANSPARENT);

    // Draw Intensity slider
    let target_flakes = state.target_flakes.load(Ordering::Relaxed) as f32;
    let intensity_str = format!("{}", target_flakes as usize);
    draw_slider(
        hdc,
        "Intensity",
        &intensity_str,
        SLIDER1_TOP,
        50.0,
        3000.0,
        target_flakes,
    );

    // Draw Opacity slider
    let opacity = state.opacity.load(Ordering::Relaxed) as f32;
    let opacity_str = format!("{}%", opacity as usize);
    draw_slider(
        hdc,
        "Opacity",
        &opacity_str,
        SLIDER2_TOP,
        10.0,
        100.0,
        opacity,
    );

    // Accumulate header
    let mut acc_header_rc = RECT { left: MARGIN_X, top: ACC_HEADER_TOP, right: MARGIN_X + TRACK_WIDTH, bottom: ACC_HEADER_TOP + 20 };
    let _ = SetTextColor(hdc, COLORREF(0x00DEDEDE));
    SelectObject(hdc, HGDIOBJ(hfont_bold.0));
    draw_text_str(hdc, "Accumulate", &mut acc_header_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    // Accumulate tiles
    let acc_w = state.acc_windows.load(Ordering::Relaxed);
    let acc_d = state.acc_dock.load(Ordering::Relaxed);
    let acc_g = state.acc_ground.load(Ordering::Relaxed);

    let t0_l = MARGIN_X;
    let t1_l = t0_l + TILE_WIDTH + TILE_GAP;
    let t2_l = t1_l + TILE_WIDTH + TILE_GAP;

    draw_tile(hdc, "Windows", acc_w, t0_l, TILES_TOP, HGDIOBJ(hfont_normal.0), "windows");
    draw_tile(hdc, "Dock", acc_d, t1_l, TILES_TOP, HGDIOBJ(hfont_normal.0), "dock");
    draw_tile(hdc, "Ground", acc_g, t2_l, TILES_TOP, HGDIOBJ(hfont_normal.0), "ground");

    // Height limit switch
    let height_lmt = state.height_limit.load(Ordering::Relaxed);
    draw_switch(hdc, "Height limit", height_lmt, SWITCH1_TOP, HGDIOBJ(hfont_normal.0));

    // Mouse pushes snow switch
    let mouse_flk = state.mouse_flakes.load(Ordering::Relaxed);
    draw_switch(hdc, "Mouse pushes snow", mouse_flk, SWITCH2_TOP, HGDIOBJ(hfont_normal.0));

    // Frame rate row label
    let _ = SetTextColor(hdc, COLORREF(0x00DEDEDE));
    let mut rc_fps = RECT { left: 20, top: FPS_TOP, right: 150, bottom: FPS_TOP + 40 };
    SelectObject(hdc, HGDIOBJ(hfont_normal.0));
    draw_text_str(hdc, "Frame rate", &mut rc_fps, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let current_fps = state.target_fps.load(Ordering::Relaxed);
    for &fps in &[20, 30, 60] {
        let r = fps_btn_rect(fps);
        let active = current_fps == fps;
        let bg_color = if active { COLORREF(0x00F48442) } else { COLORREF(0x003A3D41) };
        let brush = CreateSolidBrush(bg_color);
        let mut r_sys = r;
        FillRect(hdc, &r_sys, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
        let text_str = match fps {
            20 => "20",
            30 => "30",
            60 => "60",
            _ => "?",
        };
        draw_text_str(hdc, text_str, &mut r_sys, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }

    // Pause / Resume button
    let is_paused = state.paused.load(Ordering::Relaxed);
    let pause_r = pause_btn_rect();
    let p_brush = CreateSolidBrush(COLORREF(0x003A3D41));
    let mut p_sys = pause_r;
    FillRect(hdc, &p_sys, p_brush);
    let _ = DeleteObject(HGDIOBJ(p_brush.0));
    let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
    let pause_str = if is_paused { "Resume" } else { "Pause" };
    draw_text_str(hdc, pause_str, &mut p_sys, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // Clear snow button
    let clear_r = clear_btn_rect();
    let c_brush = CreateSolidBrush(COLORREF(0x003A3D41));
    let mut c_sys = clear_r;
    FillRect(hdc, &c_sys, c_brush);
    let _ = DeleteObject(HGDIOBJ(c_brush.0));
    let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
    draw_text_str(hdc, "Clear snow", &mut c_sys, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(hfont_normal.0));
    let _ = DeleteObject(HGDIOBJ(hfont_bold.0));
}

fn handle_click(state: &AppState, x: i32, y: i32) -> bool {
    // Check accumulate tiles
    let t0_l = MARGIN_X;
    let t0_r = t0_l + TILE_WIDTH;
    let t1_l = t0_r + TILE_GAP;
    let t1_r = t1_l + TILE_WIDTH;
    let t2_l = t1_r + TILE_GAP;
    let t2_r = t2_l + TILE_WIDTH;

    if y >= TILES_TOP && y <= TILES_TOP + TILE_HEIGHT {
        if x >= t0_l && x <= t0_r {
            let cur = state.acc_windows.load(Ordering::Relaxed);
            state.acc_windows.store(!cur, Ordering::Relaxed);
            return true;
        }
        if x >= t1_l && x <= t1_r {
            let cur = state.acc_dock.load(Ordering::Relaxed);
            state.acc_dock.store(!cur, Ordering::Relaxed);
            return true;
        }
        if x >= t2_l && x <= t2_r {
            let cur = state.acc_ground.load(Ordering::Relaxed);
            state.acc_ground.store(!cur, Ordering::Relaxed);
            return true;
        }
    }

    // Check switches
    if x >= MARGIN_X && x <= MARGIN_X + TRACK_WIDTH {
        if y >= SWITCH1_TOP && y <= SWITCH1_TOP + SWITCH_ROW_HEIGHT {
            let cur = state.height_limit.load(Ordering::Relaxed);
            state.height_limit.store(!cur, Ordering::Relaxed);
            return true;
        }
        if y >= SWITCH2_TOP && y <= SWITCH2_TOP + SWITCH_ROW_HEIGHT {
            let cur = state.mouse_flakes.load(Ordering::Relaxed);
            state.mouse_flakes.store(!cur, Ordering::Relaxed);
            return true;
        }
    }

    // Check frame rate buttons
    for &fps in &[20, 30, 60] {
        let r = fps_btn_rect(fps);
        if x >= r.left && x <= r.right && y >= r.top && y <= r.bottom {
            state.target_fps.store(fps, Ordering::Relaxed);
            return true;
        }
    }

    // Check pause button
    let pr = pause_btn_rect();
    if x >= pr.left && x <= pr.right && y >= pr.top && y <= pr.bottom {
        let current = state.paused.load(Ordering::Relaxed);
        state.paused.store(!current, Ordering::Relaxed);
        return true;
    }

    // Check clear snow button
    let cr = clear_btn_rect();
    if x >= cr.left && x <= cr.right && y >= cr.top && y <= cr.bottom {
        state.clear_requested.store(true, Ordering::Relaxed);
        return true;
    }

    false
}
