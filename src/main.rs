//! Main entry point for snowfall application.

mod config;
mod overlay;
mod platform;
mod settings_ui;
mod snow;
mod tracker;
mod tray;

use overlay::Overlay;
use snow::{bgra, Snow};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, ShowWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE,
};

const LATENCY_COMP: f32 = 0.025;

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = timeBeginPeriod(1);
    }

    struct TimePeriodGuard;
    impl Drop for TimePeriodGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = timeEndPeriod(1);
            }
        }
    }
    let _time_guard = TimePeriodGuard;

    tracker::init();

    let debug_mode = std::env::args().any(|arg| arg == "--debug");
    let dump_mode = std::env::args().any(|arg| arg == "--dump");
    let brushlog_mode = std::env::args().any(|arg| arg == "--brushlog");

    let state = std::sync::Arc::new(tray::AppState::new());
    config::load(&state);
    let tray_handle = tray::spawn_tray_thread(state.clone());
    let mut overlay = Overlay::new(state.clone());

    if dump_mode {
        std::thread::sleep(Duration::from_secs(5));
        let own_hwnd = tracker::get_overlay_hwnd();
        let dump_str = tracker::dump_all(own_hwnd);
        print!("{}", dump_str);
        if let Err(e) = std::fs::write("dump.txt", &dump_str) {
            eprintln!("Failed to write dump.txt: {}", e);
        }
        state.quit.store(true, Ordering::Relaxed);
        let tray_hwnd = HWND(state.tray_hwnd.load(Ordering::Acquire) as *mut std::ffi::c_void);
        if !tray_hwnd.is_invalid() {
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    tray_hwnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
        let _ = tray_handle.join();
        return;
    }

    let (width, height) = unsafe {
        (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };

    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };

    let w = if width > 0 { width as f32 } else { 1920.0 };
    let h = if height > 0 { height as f32 } else { 1080.0 };

    let mut snow = Snow::new(w, h, 400);

    let safety_interval = Duration::from_secs(1);

    let mut last_time = Instant::now();
    let mut last_snapshot_time = Instant::now() - safety_interval;
    let mut last_window_change_time = Instant::now() - Duration::from_secs(10);

    let mut cached_windows = tracker::snapshot(windows::Win32::Foundation::HWND::default());
    let floor = platform::floor_y();
    let virt = (origin_x, origin_y, origin_x + width, origin_y + height);
    let acc_windows = state.acc_windows.load(Ordering::Relaxed);
    let acc_dock = state.acc_dock.load(Ordering::Relaxed);
    let acc_ground = state.acc_ground.load(Ordering::Relaxed);
    let mut prev_acc_windows = acc_windows;
    let mut prev_acc_dock = acc_dock;
    let mut prev_acc_ground = acc_ground;
    let raw_segments = platform::build_segments(
        &cached_windows,
        virt,
        (floor, acc_windows, acc_dock, acc_ground),
    );
    let mut cached_segments: Vec<platform::Segment> = raw_segments
        .into_iter()
        .map(|seg| platform::Segment {
            owner: seg.owner,
            x1: seg.x1 - origin_x as f32,
            x2: seg.x2 - origin_x as f32,
            y: seg.y - origin_y as f32,
            full_left: seg.full_left - origin_x as f32,
            full_right: seg.full_right - origin_x as f32,
        })
        .collect();

    snow.update(0.0, &cached_segments);
    snow.seed_initial_piles();

    let mut brushlog_last_print = Instant::now();
    if brushlog_mode {
        snow.enable_brushlog(true);
    }

    let overlay_hwnd = tracker::get_overlay_hwnd();

    let perf_mode = std::env::args().any(|arg| arg == "--perf");
    let mut perf_frame_count = 0;
    let mut perf_frame_time_sum = 0.0;
    let mut perf_frame_time_max = 0.0f32;
    let mut perf_tracker_sum = 0.0;
    let mut perf_update_sum = 0.0;
    let mut perf_draw_sum = 0.0;
    let mut perf_present_sum = 0.0;
    let mut perf_flakes_sum = 0;
    let mut perf_last_print = Instant::now();

    struct DragSample {
        time: Instant,
        x: f32,
        y: f32,
    }
    let mut drag_samples: VecDeque<DragSample> = VecDeque::with_capacity(8);
    let mut unchanged_frames = 0;

    let mut prev_cursor_pos: Option<(f32, f32)> = None;
    let mut smoothed_vel: (f32, f32) = (0.0, 0.0);
    let mut last_all_refresh = Instant::now();
    let mut was_dragging = false;
    let mut ease_start_time: Option<Instant> = None;
    let mut saved_comp: (f32, f32) = (0.0, 0.0);
    let mut last_moving_hwnd: isize = 0;

    let mut thread_priority_above_normal = false;
    struct ThreadPriorityGuard(bool);
    impl Drop for ThreadPriorityGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL);
                }
            }
        }
    }
    let mut priority_guard = ThreadPriorityGuard(false);
    let mut win_rects: Vec<(isize, tiny_skia::Rect)> = Vec::new();

    let mut last_saved_snapshot = config::snapshot(&state);
    let mut last_settings_check = Instant::now();

    loop {
        let frame_start = Instant::now();
        if last_settings_check.elapsed() >= Duration::from_secs(1) {
            last_settings_check = Instant::now();
            let snap = config::snapshot(&state);
            if snap != last_saved_snapshot {
                config::save(&state);
                last_saved_snapshot = snap;
            }
        }

        let opacity = state.opacity.load(Ordering::Relaxed);
        snow.flake_opacity = opacity as f32 / 100.0;

        if state.quit.load(Ordering::Relaxed) {
            break;
        }

        if !overlay.pump_messages() {
            state.quit.store(true, Ordering::Relaxed);
            break;
        }

        if state.clear_requested.swap(false, Ordering::Relaxed) {
            snow.start_clear();
        }

        let moving_hwnd = tracker::get_moving_hwnd();
        let is_dragging = moving_hwnd != 0;

        if is_dragging {
            if !thread_priority_above_normal {
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
                }
                thread_priority_above_normal = true;
                priority_guard.0 = true;
            }
        } else {
            if thread_priority_above_normal {
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL);
                }
                thread_priority_above_normal = false;
                priority_guard.0 = false;
            }
        }

        if !is_dragging {
            if tracker::fullscreen_active() {
                unsafe {
                    let _ = ShowWindow(overlay_hwnd, SW_HIDE);
                }
                std::thread::sleep(Duration::from_millis(150));
                last_time = Instant::now();
                prev_cursor_pos = None;
                continue;
            } else {
                unsafe {
                    let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
                }
            }
        } else {
            unsafe {
                let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
            }
        }

        if state.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(150));
            last_time = Instant::now();
            prev_cursor_pos = None;
            let mut pixmap = overlay.pixmap_mut();
            pixmap.data_mut().fill(0);
            snow.draw(&mut pixmap, &win_rects);
            overlay.present();
            continue;
        }

        if is_dragging {
            last_moving_hwnd = moving_hwnd;
        }

        for hwnd in tracker::take_minimized() {
            snow.release_owner(hwnd);
            cached_windows.retain(|w| w.hwnd != hwnd);
        }

        let now = Instant::now();
        let elapsed = now.duration_since(last_time);
        let dt = elapsed.as_secs_f32();
        last_time = now;

        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let cx = pt.x as f32 - origin_x as f32;
        let cy = pt.y as f32 - origin_y as f32;

        let mut cvx = 0.0;
        let mut cvy = 0.0;
        if let Some(prev_pos) = prev_cursor_pos {
            if dt > 0.0 {
                cvx = (cx - prev_pos.0) / dt;
                cvy = (cy - prev_pos.1) / dt;
            }
        }

        snow.target_flake_count = state.target_flakes.load(Ordering::Relaxed);
        snow.height_limit = state.height_limit.load(Ordering::Relaxed);
        snow.strength = state.strength.load(Ordering::Relaxed) as f32 / 100.0;

        let acc_windows = state.acc_windows.load(Ordering::Relaxed);
        let acc_dock = state.acc_dock.load(Ordering::Relaxed);
        let acc_ground = state.acc_ground.load(Ordering::Relaxed);

        let flags_changed = acc_windows != prev_acc_windows
            || acc_dock != prev_acc_dock
            || acc_ground != prev_acc_ground;
        if flags_changed {
            prev_acc_windows = acc_windows;
            prev_acc_dock = acc_dock;
            prev_acc_ground = acc_ground;
        }

        let mut segments_rebuilt = false;
        let tracker_timer = if perf_mode {
            Some(Instant::now())
        } else {
            None
        };

        if is_dragging {
            if let Some(new_rect) = tracker::read_rect(moving_hwnd) {
                let cur_x = new_rect.left as f32;
                let cur_y = new_rect.top as f32;
                let sample_now = Instant::now();

                let mut rect_changed = true;
                if let Some(latest) = drag_samples.back() {
                    if latest.x == cur_x && latest.y == cur_y {
                        rect_changed = false;
                    }
                }

                if !rect_changed {
                    unchanged_frames += 1;
                    if unchanged_frames >= 3 {
                        smoothed_vel.0 *= 0.8;
                        smoothed_vel.1 *= 0.8;
                    }
                } else {
                    unchanged_frames = 0;
                    if drag_samples.len() >= 8 {
                        drag_samples.pop_front();
                    }
                    drag_samples.push_back(DragSample {
                        time: sample_now,
                        x: cur_x,
                        y: cur_y,
                    });

                    if drag_samples.len() >= 2 {
                        let newest = drag_samples.back().unwrap();
                        let mut chosen_oldest = drag_samples.front().unwrap();
                        for sample in &drag_samples {
                            let span = newest.time.duration_since(sample.time);
                            if span >= Duration::from_millis(16) {
                                chosen_oldest = sample;
                                if span <= Duration::from_millis(60) {
                                    break;
                                }
                            }
                        }
                        let dt_span = newest.time.duration_since(chosen_oldest.time).as_secs_f32();
                        if dt_span >= 0.016 {
                            let raw_vx = (newest.x - chosen_oldest.x) / dt_span;
                            let raw_vy = (newest.y - chosen_oldest.y) / dt_span;
                            let alpha = 1.0 - (-dt.min(0.1) / 0.05).exp();
                            smoothed_vel.0 = smoothed_vel.0 * (1.0 - alpha) + raw_vx * alpha;
                            smoothed_vel.1 = smoothed_vel.1 * (1.0 - alpha) + raw_vy * alpha;
                        }
                    }
                }

                if let Some(win) = cached_windows.iter_mut().find(|w| w.hwnd == moving_hwnd) {
                    win.rect = new_rect;
                } else {
                    cached_windows.push(platform::WindowInfo {
                        hwnd: moving_hwnd,
                        rect: new_rect,
                    });
                }
                segments_rebuilt = true;
            }

            snow.fling_owner(moving_hwnd, smoothed_vel.0, smoothed_vel.1, dt);

            was_dragging = true;
            ease_start_time = None;

            if last_all_refresh.elapsed() >= Duration::from_millis(100) {
                last_all_refresh = Instant::now();
                tracker::refresh_rects(&mut cached_windows);
            }
        } else {
            drag_samples.clear();
            unchanged_frames = 0;

            if was_dragging {
                let speed_sq = smoothed_vel.0 * smoothed_vel.0 + smoothed_vel.1 * smoothed_vel.1;
                let speed = speed_sq.sqrt();
                if speed > 50.0 {
                    saved_comp = (smoothed_vel.0 * LATENCY_COMP, smoothed_vel.1 * LATENCY_COMP);
                } else {
                    saved_comp = (0.0, 0.0);
                }
                ease_start_time = Some(Instant::now());
                was_dragging = false;
                smoothed_vel = (0.0, 0.0);
            }

            let dirty = tracker::is_dirty();
            let time_since_last_snapshot = last_snapshot_time.elapsed();
            let safety_refresh = time_since_last_snapshot >= safety_interval;

            if dirty || safety_refresh {
                tracker::reset_dirty();
                last_snapshot_time = Instant::now();
                cached_windows = tracker::snapshot(windows::Win32::Foundation::HWND::default());
                segments_rebuilt = true;
                last_window_change_time = Instant::now();
            } else {
                let rects_changed = tracker::refresh_rects(&mut cached_windows);
                if rects_changed {
                    last_window_change_time = Instant::now();
                    segments_rebuilt = true;
                }
            }
            last_all_refresh = Instant::now();
        }

        let tracker_duration = tracker_timer.map_or(0.0, |t| t.elapsed().as_secs_f32());

        let mut lead_x = smoothed_vel.0 * LATENCY_COMP;
        let mut lead_y = smoothed_vel.1 * LATENCY_COMP;
        let lead_len = (lead_x * lead_x + lead_y * lead_y).sqrt();
        if lead_len > 60.0 {
            lead_x = (lead_x / lead_len) * 60.0;
            lead_y = (lead_y / lead_len) * 60.0;
        }
        let comp_offset = if is_dragging {
            (lead_x, lead_y)
        } else if let Some(start) = ease_start_time {
            let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
            if elapsed_ms < 100.0 {
                let factor = (1.0 - elapsed_ms / 100.0).max(0.0);
                (saved_comp.0 * factor, saved_comp.1 * factor)
            } else {
                ease_start_time = None;
                saved_comp = (0.0, 0.0);
                last_moving_hwnd = 0;
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        if segments_rebuilt || flags_changed || is_dragging || ease_start_time.is_some() {
            let floor = platform::floor_y();
            let virt = (origin_x, origin_y, origin_x + width, origin_y + height);
            let segments_windows = if is_dragging || ease_start_time.is_some() {
                let mut temp_wins = cached_windows.clone();
                let target_hwnd = if is_dragging {
                    moving_hwnd
                } else {
                    last_moving_hwnd
                };
                if target_hwnd != 0 {
                    if let Some(win) = temp_wins.iter_mut().find(|w| w.hwnd == target_hwnd) {
                        win.rect.left = (win.rect.left as f32 + comp_offset.0).round() as i32;
                        win.rect.top = (win.rect.top as f32 + comp_offset.1).round() as i32;
                        win.rect.right = (win.rect.right as f32 + comp_offset.0).round() as i32;
                        win.rect.bottom = (win.rect.bottom as f32 + comp_offset.1).round() as i32;
                    }
                }
                temp_wins
            } else {
                cached_windows.clone()
            };

            let raw_segments = platform::build_segments(
                &segments_windows,
                virt,
                (floor, acc_windows, acc_dock, acc_ground),
            );
            cached_segments = raw_segments
                .into_iter()
                .map(|seg| platform::Segment {
                    owner: seg.owner,
                    x1: seg.x1 - origin_x as f32,
                    x2: seg.x2 - origin_x as f32,
                    y: seg.y - origin_y as f32,
                    full_left: seg.full_left - origin_x as f32,
                    full_right: seg.full_right - origin_x as f32,
                })
                .collect();
        }

        win_rects.clear();
        win_rects.extend(cached_windows.iter().filter_map(|win| {
            let rx = win.rect.left - origin_x;
            let ry = win.rect.top - origin_y;
            let rw = win.rect.right - win.rect.left;
            let rh = win.rect.bottom - win.rect.top;
            tiny_skia::Rect::from_xywh(rx as f32, ry as f32, rw as f32, rh as f32)
                .map(|rect| (win.hwnd, rect))
        }));

        let (steps, start_x, start_y) = if let Some(prev_pos) = prev_cursor_pos {
            let dx = cx - prev_pos.0;
            let dy = cy - prev_pos.1;
            let dist = (dx * dx + dy * dy).sqrt();
            let s = ((dist / 12.0).ceil() as i32).clamp(1, 8);
            (s, prev_pos.0, prev_pos.1)
        } else {
            (1, cx, cy)
        };

        let speed = (cvx * cvx + cvy * cvy).sqrt();
        let mouse_flakes_enabled = state.mouse_flakes.load(Ordering::Relaxed);

        if let Some(ref mut d) = snow.brush_diag {
            d.last_steps = steps as usize;
            if let Some(prev_pos) = prev_cursor_pos {
                if (cx - prev_pos.0).abs() > 0.1 || (cy - prev_pos.1).abs() > 0.1 {
                    d.cursor_moved = true;
                }
            } else {
                d.cursor_moved = true;
            }
        }

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let bx = start_x + (cx - start_x) * t;
            let by = start_y + (cy - start_y) * t;
            snow.brush(bx, by, cvx, cvy, &win_rects);
            if mouse_flakes_enabled && speed >= 20.0 {
                snow.push_flakes(bx, by, cvx, cvy, 1.0 / steps as f32);
            }
        }

        prev_cursor_pos = Some((cx, cy));

        if brushlog_mode && brushlog_last_print.elapsed() >= Duration::from_millis(500) {
            if let Some(ref d) = snow.brush_diag {
                if d.cursor_moved {
                    let avg_speed = if d.speed_count > 0 {
                        d.speed_sum / d.speed_count as f32
                    } else {
                        0.0
                    };
                    let extra_count = snow.flakes.iter().filter(|f| f.extra).count();
                    let pile_h = snow.max_pile_height_under_cursor(cx, cy, &win_rects);
                    println!(
                        "BRUSH: speed={:.1}, steps={}, removed={:.1}, berm={:.1}, curtain={:.1}, thrown={}, extra={}, pile_h={:.1}",
                        avg_speed, d.last_steps, d.snow_removed, d.berm_mass, d.curtain_mass, d.thrown_spawned, extra_count, pile_h
                    );
                }
            }
            snow.reset_brush_diag();
            brushlog_last_print = Instant::now();
        }

        let mut pixmap = overlay.pixmap_mut();
        pixmap.data_mut().fill(0);

        let update_timer = if perf_mode {
            Some(Instant::now())
        } else {
            None
        };
        snow.window_rects.clear();
        snow.window_rects.extend_from_slice(&win_rects);
        snow.update(dt.min(0.1), &cached_segments);
        let update_duration = update_timer.map_or(0.0, |t| t.elapsed().as_secs_f32());

        let draw_timer = if perf_mode {
            Some(Instant::now())
        } else {
            None
        };
        snow.draw(&mut pixmap, &win_rects);
        let draw_duration = draw_timer.map_or(0.0, |t| t.elapsed().as_secs_f32());

        if debug_mode {
            for win in &cached_windows {
                let rx = win.rect.left - origin_x;
                let ry = win.rect.top - origin_y;
                let rw = win.rect.right - win.rect.left;
                let rh = win.rect.bottom - win.rect.top;

                if let Some(rect) =
                    tiny_skia::Rect::from_xywh(rx as f32, ry as f32, rw as f32, rh as f32)
                {
                    let mut pb = tiny_skia::PathBuilder::new();
                    pb.push_rect(rect);
                    if let Some(path) = pb.finish() {
                        let mut paint = tiny_skia::Paint::default();
                        let color = bgra(1.0, 0.0, 0.0, 1.0);
                        paint.set_color(color);
                        let stroke = tiny_skia::Stroke {
                            width: 2.0,
                            ..Default::default()
                        };
                        pixmap.stroke_path(
                            &path,
                            &paint,
                            &stroke,
                            tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                }
            }

            for seg in &cached_segments {
                let mut pb = tiny_skia::PathBuilder::new();
                pb.move_to(seg.x1, seg.y);
                pb.line_to(seg.x2, seg.y);
                if let Some(path) = pb.finish() {
                    let mut paint = tiny_skia::Paint::default();
                    paint.set_color(bgra(0.0, 1.0, 0.0, 1.0));
                    let stroke = tiny_skia::Stroke {
                        width: 3.0,
                        ..Default::default()
                    };
                    pixmap.stroke_path(
                        &path,
                        &paint,
                        &stroke,
                        tiny_skia::Transform::identity(),
                        None,
                    );
                }
            }
        }

        let present_timer = if perf_mode {
            Some(Instant::now())
        } else {
            None
        };
        overlay.present();
        let present_duration = present_timer.map_or(0.0, |t| t.elapsed().as_secs_f32());

        let frame_duration = frame_start.elapsed().as_secs_f32();

        if perf_mode {
            perf_frame_count += 1;
            perf_frame_time_sum += frame_duration;
            if frame_duration > perf_frame_time_max {
                perf_frame_time_max = frame_duration;
            }
            perf_tracker_sum += tracker_duration;
            perf_update_sum += update_duration;
            perf_draw_sum += draw_duration;
            perf_present_sum += present_duration;
            perf_flakes_sum += snow.flakes.len();

            if perf_last_print.elapsed() >= Duration::from_secs(1) {
                let count = perf_frame_count.max(1) as f32;
                let fps = perf_frame_count as f32 / perf_last_print.elapsed().as_secs_f32();
                let avg_frame = perf_frame_time_sum / count;
                let max_frame = perf_frame_time_max;
                let avg_tracker = perf_tracker_sum / count;
                let avg_update = perf_update_sum / count;
                let avg_draw = perf_draw_sum / count;
                let avg_present = perf_present_sum / count;
                let avg_flakes = perf_flakes_sum / perf_frame_count.max(1) as usize;

                println!(
                    "PERF: fps={:.1}, flakes={}, frame_avg={:.2}ms, frame_max={:.2}ms, tracker={:.2}ms, update={:.2}ms, draw={:.2}ms, present={:.2}ms",
                    fps, avg_flakes, avg_frame * 1000.0, max_frame * 1000.0, avg_tracker * 1000.0, avg_update * 1000.0, avg_draw * 1000.0, avg_present * 1000.0
                );

                perf_frame_count = 0;
                perf_frame_time_sum = 0.0;
                perf_frame_time_max = 0.0;
                perf_tracker_sum = 0.0;
                perf_update_sum = 0.0;
                perf_draw_sum = 0.0;
                perf_present_sum = 0.0;
                perf_flakes_sum = 0;
                perf_last_print = Instant::now();
            }
        }

        if is_dragging {
            let _ = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
            let frame_time = now.elapsed();
            let min_frame_duration = Duration::from_secs_f64(1.0 / 144.0);
            if frame_time < min_frame_duration {
                std::thread::sleep(min_frame_duration - frame_time);
            }
        } else {
            let target_fps = state.target_fps.load(Ordering::Relaxed);
            let boost = last_window_change_time.elapsed() < Duration::from_millis(500);
            let effective_fps = if boost { 60 } else { target_fps };
            let target_frame_duration = Duration::from_secs_f64(1.0 / effective_fps as f64);
            let min_frame_duration = Duration::from_secs_f64(1.0 / 60.0);
            let frame_time = now.elapsed();

            if boost {
                let flush_res = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() };
                if flush_res.is_err() {
                    if frame_time < target_frame_duration {
                        std::thread::sleep(target_frame_duration - frame_time);
                    }
                } else if frame_time < min_frame_duration {
                    std::thread::sleep(min_frame_duration - frame_time);
                }
            } else {
                if frame_time < target_frame_duration {
                    std::thread::sleep(target_frame_duration - frame_time);
                }
            }
        }
    }

    config::save(&state);
    state.quit.store(true, Ordering::Relaxed);
    let tray_hwnd = HWND(state.tray_hwnd.load(Ordering::Acquire) as *mut std::ffi::c_void);
    if !tray_hwnd.is_invalid() {
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                tray_hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    } else {
        let hwnd = unsafe {
            windows::Win32::UI::WindowsAndMessaging::FindWindowW(
                windows::core::w!("SnowfallTray"),
                None,
            )
            .unwrap_or_default()
        };
        if !hwnd.is_invalid() {
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
    }
    let _ = tray_handle.join();
}
