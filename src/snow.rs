//! Snow module for flakes, piles, physics, and drawing.

use crate::platform::Segment;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapMut, Stroke, Transform};

pub const GROUND_ID: isize = -1;

pub fn is_floor_like(owner: Option<isize>) -> bool {
    owner.is_none() || owner == Some(GROUND_ID)
}

const MAX_SLOPE: f32 = 2.2;
const ACCUMULATION: f32 = 1.8;
const EDGE_ZONE: f32 = 28.0;
const PLOW_FRAC: f32 = 0.4;
const CLEAR_FRONT_SPEED: f32 = 1000.0;
const CLEAR_GUST_WIND: f32 = 1250.0;
const CLEAR_SWIRL_SPEED: f32 = 500.0;
const CLEAR_SWIRL_RATE: f32 = 5.0;

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn relax_range(heights: &mut [f32], lo: usize, hi: usize, max_slope: f32) -> bool {
    let len = heights.len();
    if len < 2 {
        return false;
    }
    let start = lo.min(hi).min(len - 1);
    let end = lo.max(hi).min(len - 1);
    if start >= end {
        return false;
    }
    let orig = heights.to_vec();

    // Sweep 1: left-to-right
    for i in start..end {
        let d = heights[i] - heights[i + 1];
        if d.abs() > max_slope {
            let m = (d.abs() - max_slope) * 0.5;
            if heights[i] > heights[i + 1] {
                heights[i] -= m;
                heights[i + 1] += m;
            } else {
                heights[i] += m;
                heights[i + 1] -= m;
            }
        }
    }

    // Sweep 2: right-to-left
    for i in (start..end).rev() {
        let d = heights[i] - heights[i + 1];
        if d.abs() > max_slope {
            let m = (d.abs() - max_slope) * 0.5;
            if heights[i] > heights[i + 1] {
                heights[i] -= m;
                heights[i + 1] += m;
            } else {
                heights[i] += m;
                heights[i + 1] -= m;
            }
        }
    }

    let mut changed = false;
    for i in 0..len {
        if (heights[i] - orig[i]).abs() > 0.01 {
            changed = true;
            break;
        }
    }
    changed
}

pub fn bgra(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::from_rgba(b, g, r, a).unwrap()
}

pub struct Flake {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub bvx: f32,
    pub bvy: f32,
    pub r: f32,
    pub alpha: f32,
    pub sway_phase: f32,
    pub depth: f32,
    pub bucket_idx: usize,
    pub fade: f32,
    pub fade_target: f32,
    pub extra: bool,
    pub ballistic: f32,
    pub skip_owner: isize,
}

pub struct Pile {
    pub x_start: f32,
    pub y: f32,
    pub heights: Vec<f32>,
    pub dirty: bool,
    pub cached_intervals: Vec<(f32, f32)>,
    pub cached_shadow_path: Option<tiny_skia::Path>,
    pub cached_body_path: Option<tiny_skia::Path>,
    pub cached_top_path: Option<tiny_skia::Path>,
    pub cached_shader: Option<tiny_skia::Shader<'static>>,
}

impl Pile {
    pub fn update_cache(
        &mut self,
        owner: Option<isize>,
        owner_data: &HashMap<Option<isize>, OwnerData>,
    ) {
        let intervals = if is_floor_like(owner) {
            vec![(self.x_start, self.x_start + self.heights.len() as f32 * 2.0)]
        } else {
            owner_data
                .get(&owner)
                .map(|d| d.visible_intervals.clone())
                .unwrap_or_default()
        };

        if self.heights.is_empty() || intervals.is_empty() {
            self.cached_shadow_path = None;
            self.cached_body_path = None;
            self.cached_top_path = None;
            self.cached_shader = None;
            self.cached_intervals = intervals;
            self.dirty = false;
            return;
        }

        if !self.dirty && self.cached_intervals == intervals {
            return;
        }

        let mut shadow_pb = PathBuilder::new();
        let mut body_pb = PathBuilder::new();
        let mut top_pb = PathBuilder::new();

        let mut max_run_h = 0.0f32;
        let mut run_start_x_all = 0.0f32;

        let mut i = 0;
        while i < self.heights.len() {
            let col_x = self.x_start + i as f32 * 2.0 + 1.0;
            let h = self.heights[i];
            let is_visible =
                h > 0.05 && intervals.iter().any(|&(v1, v2)| col_x >= v1 && col_x <= v2);

            if !is_visible {
                i += 1;
                continue;
            }

            let start_idx = i;
            let mut end_idx = i;
            let mut max_h = h;

            while end_idx + 1 < self.heights.len() {
                let next_i = end_idx + 1;
                let next_col_x = self.x_start + next_i as f32 * 2.0 + 1.0;
                let next_h = self.heights[next_i];
                let next_visible = next_h > 0.05
                    && intervals
                        .iter()
                        .any(|&(v1, v2)| next_col_x >= v1 && next_col_x <= v2);
                if next_visible {
                    end_idx = next_i;
                    if next_h > max_h {
                        max_h = next_h;
                    }
                } else {
                    break;
                }
            }

            if max_h > max_run_h {
                max_run_h = max_h;
                run_start_x_all = self.x_start + start_idx as f32 * 2.0;
            }

            let run_start_x = self.x_start + start_idx as f32 * 2.0;
            let run_end_x = self.x_start + (end_idx + 1) as f32 * 2.0;
            let first_x = run_start_x;
            let first_y = self.y - self.heights[start_idx];

            shadow_pb.move_to(run_start_x, self.y + 2.0);
            shadow_pb.line_to(first_x, first_y + 2.0);
            for idx in start_idx..=end_idx {
                let x = self.x_start + idx as f32 * 2.0 + 1.0;
                let y = self.y - self.heights[idx];
                shadow_pb.line_to(x, y + 2.0);
            }
            shadow_pb.line_to(run_end_x, self.y + 2.0);
            shadow_pb.close();

            body_pb.move_to(run_start_x, self.y);
            body_pb.line_to(first_x, first_y);
            for idx in start_idx..=end_idx {
                let x = self.x_start + idx as f32 * 2.0 + 1.0;
                let y = self.y - self.heights[idx];
                body_pb.line_to(x, y);
            }
            body_pb.line_to(run_end_x, self.y);
            body_pb.close();

            top_pb.move_to(first_x, first_y);
            for idx in start_idx..=end_idx {
                let x = self.x_start + idx as f32 * 2.0 + 1.0;
                let y = self.y - self.heights[idx];
                top_pb.line_to(x, y);
            }

            i = end_idx + 1;
        }

        self.cached_shadow_path = shadow_pb.finish();
        self.cached_body_path = body_pb.finish();
        self.cached_top_path = top_pb.finish();

        if max_run_h > 0.0 {
            let top_y = self.y - max_run_h;
            if let Some(shader) = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(run_start_x_all, top_y),
                tiny_skia::Point::from_xy(run_start_x_all, self.y),
                vec![
                    tiny_skia::GradientStop::new(0.0, bgra(1.0, 1.0, 1.0, 1.0)),
                    tiny_skia::GradientStop::new(
                        1.0,
                        bgra(230.0 / 255.0, 238.0 / 255.0, 250.0 / 255.0, 1.0),
                    ),
                ],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                self.cached_shader = Some(shader);
            }
        } else {
            self.cached_shader = None;
        }

        self.cached_intervals = intervals;
        self.dirty = false;
    }
}

pub struct OwnerData {
    pub y: f32,
    pub full_left: f32,
    pub full_right: f32,
    pub visible_intervals: Vec<(f32, f32)>,
}

pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub struct BrushDiagnostics {
    pub snow_removed: f32,
    pub berm_mass: f32,
    pub curtain_mass: f32,
    pub thrown_spawned: usize,
    pub speed_sum: f32,
    pub speed_count: usize,
    pub last_steps: usize,
    pub cursor_moved: bool,
}

pub struct Snow {
    pub flakes: Vec<Flake>,
    pub width: f32,
    pub height: f32,
    pub piles: HashMap<Option<isize>, Pile>,
    pub target_flake_count: usize,
    pub owner_data: HashMap<Option<isize>, OwnerData>,
    pub wind_time: f32,
    pub sprites: Vec<Sprite>,
    pub scratch_current_owners: HashSet<Option<isize>>,
    pub scratch_spawn_actions: Vec<(f32, f32, f32, f32, isize)>,
    pub scratch_curtain_actions: Vec<(f32, f32, f32, f32)>,
    pub scratch_valid_cols: Vec<usize>,
    pub scratch_remove_indices: Vec<usize>,
    pub flake_opacity: f32,
    pub seeded: bool,
    pub height_limit: bool,
    pub strength: f32,
    pub brush_diag: Option<BrushDiagnostics>,
    pub window_rects: Vec<(isize, tiny_skia::Rect)>,
    pub clearing: bool,
    pub clear_front: f32,
    pub clear_gust: f32,
    pub clear_hold: bool,
    pub clear_hold_time: f32,
    pub refill_from_top: bool,
    pub clear_dir: f32,
}

impl Snow {
    pub fn get_bucket_idx(r: f32) -> usize {
        let t = (r - 0.8) / (4.0 - 0.8);
        (t * 7.0).round().clamp(0.0, 7.0) as usize
    }

    fn landing_cap(height_limit: bool, owner: Option<isize>, pile_y: f32) -> f32 {
        if height_limit {
            if is_floor_like(owner) {
                36.0
            } else {
                16.0
            }
        } else {
            (pile_y - 8.0).max(36.0)
        }
    }

    fn hard_cap(height_limit: bool, owner: Option<isize>, pile_y: f32) -> f32 {
        if height_limit {
            if is_floor_like(owner) {
                60.0
            } else {
                32.0
            }
        } else {
            (pile_y - 8.0).max(36.0)
        }
    }

    pub fn new(width: f32, height: f32, count: usize) -> Self {
        let mut rng = rand::thread_rng();
        let mut flakes = Vec::with_capacity(count);
        let sprites = Self::create_sprites();

        for _ in 0..count {
            let depth = rng.gen_range(0.0..1.0);
            let r = if depth < 0.5 {
                rng.gen_range(0.8..1.5)
            } else {
                rng.gen_range(2.5..4.0)
            };
            let bucket_idx = Self::get_bucket_idx(r);
            let vy = 40.0 + depth * 100.0;
            let alpha = 0.3 + depth * 0.65;

            flakes.push(Flake {
                x: rng.gen_range(0.0..width),
                y: rng.gen_range(0.0..height),
                vx: rng.gen_range(-10.0..10.0),
                vy,
                bvx: 0.0,
                bvy: 0.0,
                r,
                alpha,
                sway_phase: rng.gen_range(0.0..(std::f32::consts::TAU)),
                depth,
                bucket_idx,
                fade: 1.0,
                fade_target: 1.0,
                extra: false,
                ballistic: 0.0,
                skip_owner: 0,
            });
        }

        Self {
            flakes,
            width,
            height,
            piles: HashMap::new(),
            target_flake_count: count,
            owner_data: HashMap::new(),
            wind_time: 0.0,
            sprites,
            scratch_current_owners: HashSet::new(),
            scratch_spawn_actions: Vec::new(),
            scratch_curtain_actions: Vec::new(),
            scratch_valid_cols: Vec::new(),
            scratch_remove_indices: Vec::new(),
            flake_opacity: 1.0,
            seeded: false,
            height_limit: true,
            strength: 1.0,
            brush_diag: None,
            window_rects: Vec::new(),
            clearing: false,
            clear_front: 0.0,
            clear_gust: 0.0,
            clear_hold: false,
            clear_hold_time: 0.0,
            refill_from_top: false,
            clear_dir: 1.0,
        }
    }

    pub fn enable_brushlog(&mut self, enabled: bool) {
        if enabled {
            self.brush_diag = Some(BrushDiagnostics {
                snow_removed: 0.0,
                berm_mass: 0.0,
                curtain_mass: 0.0,
                thrown_spawned: 0,
                speed_sum: 0.0,
                speed_count: 0,
                last_steps: 1,
                cursor_moved: false,
            });
        } else {
            self.brush_diag = None;
        }
    }

    pub fn reset_brush_diag(&mut self) {
        if let Some(ref mut d) = self.brush_diag {
            d.snow_removed = 0.0;
            d.berm_mass = 0.0;
            d.curtain_mass = 0.0;
            d.thrown_spawned = 0;
            d.speed_sum = 0.0;
            d.speed_count = 0;
            d.cursor_moved = false;
        }
    }

    pub fn max_pile_height_under_cursor(
        &self,
        cx: f32,
        cy: f32,
        win_rects: &[(isize, tiny_skia::Rect)],
    ) -> f32 {
        let mut found_hwnd = None;
        for &(hwnd, ref rect) in win_rects.iter().rev() {
            if cx >= rect.x()
                && cx <= rect.x() + rect.width()
                && cy >= rect.y()
                && cy <= rect.y() + rect.height()
            {
                found_hwnd = Some(hwnd);
                break;
            }
        }

        let key = if let Some(hwnd) = found_hwnd {
            Some(hwnd)
        } else {
            if self.piles.contains_key(&None) {
                None
            } else {
                Some(GROUND_ID)
            }
        };

        if let Some(pile) = self.piles.get(&key) {
            pile.heights.iter().cloned().fold(0.0f32, f32::max)
        } else {
            0.0
        }
    }

    pub fn start_clear(&mut self) {
        let dir: f32 = if rand::thread_rng().gen_bool(0.5) {
            1.0
        } else {
            -1.0
        };
        self.clear_dir = dir;
        self.clearing = true;
        self.clear_front = if dir > 0.0 { 0.0 } else { self.width };
        self.clear_gust = 1.0;
        self.clear_hold = true;
        self.clear_hold_time = 0.0;
    }

    pub fn seed_initial_piles(&mut self) {
        if self.seeded {
            return;
        }
        self.seeded = true;
        let mut rng = rand::thread_rng();

        for (owner, pile) in self.piles.iter_mut() {
            let len = pile.heights.len();
            if len == 0 {
                continue;
            }
            let (min_h, max_h) = if is_floor_like(*owner) {
                (5.0, 9.0)
            } else {
                (2.0, 4.0)
            };

            let spacing = 12;
            let num_controls = len.div_ceil(spacing) + 1;
            let mut controls = Vec::with_capacity(num_controls);
            for _ in 0..num_controls {
                controls.push(rng.gen_range(min_h..max_h));
            }

            for (i, h) in pile.heights.iter_mut().enumerate().take(len) {
                let seg_idx = i / spacing;
                let t = (i % spacing) as f32 / spacing as f32;
                let st = t * t * (3.0 - 2.0 * t);
                let h1 = controls[seg_idx];
                let h2 = controls[(seg_idx + 1).min(num_controls - 1)];
                *h = h1 * (1.0 - st) + h2 * st;
            }

            relax_range(&mut pile.heights, 0, len, MAX_SLOPE);
            pile.dirty = true;
        }
    }

    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn create_sprites() -> Vec<Sprite> {
        let mut sprites = Vec::with_capacity(128);
        for i in 0..8 {
            let r = 0.8 + i as f32 * (3.2 / 7.0);
            let size = ((r * 2.0).ceil() as u32 + 4).max(4);
            let mut base_sprite = Pixmap::new(size, size).unwrap();
            let center = size as f32 / 2.0;

            let radial_shader = tiny_skia::RadialGradient::new(
                tiny_skia::Point::from_xy(center, center),
                tiny_skia::Point::from_xy(center, center),
                r,
                vec![
                    tiny_skia::GradientStop::new(0.0, bgra(1.0, 1.0, 1.0, 1.0)),
                    tiny_skia::GradientStop::new(1.0, bgra(1.0, 1.0, 1.0, 0.0)),
                ],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            );

            let mut pb = PathBuilder::new();
            pb.push_circle(center, center, r);
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                if let Some(shader) = radial_shader {
                    paint.shader = shader;
                }
                base_sprite.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }

            let base_data = base_sprite.data();
            for a_lvl in 0..16 {
                let factor = a_lvl as f32 / 15.0;
                let mut scaled_data = vec![0u8; base_data.len()];
                for (j, chunk) in base_data.chunks_exact(4).enumerate() {
                    let b = chunk[0] as f32 * factor + 0.5;
                    let g = chunk[1] as f32 * factor + 0.5;
                    let r_chan = chunk[2] as f32 * factor + 0.5;
                    let a = chunk[3] as f32 * factor + 0.5;
                    scaled_data[j * 4] = b as u8;
                    scaled_data[j * 4 + 1] = g as u8;
                    scaled_data[j * 4 + 2] = r_chan as u8;
                    scaled_data[j * 4 + 3] = a as u8;
                }
                sprites.push(Sprite {
                    width: size,
                    height: size,
                    data: scaled_data,
                });
            }
        }
        sprites
    }

    pub fn release_owner(&mut self, owner: isize) {
        let key = Some(owner);
        if let Some(pile) = self.piles.remove(&key) {
            let mut rng = rand::thread_rng();
            for (i, &h) in pile.heights.iter().enumerate() {
                if h > 0.5 {
                    let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                    let surface_y = pile.y - h;
                    let count = rng.gen_range(1..=2);
                    for _ in 0..count {
                        let extra_count = self.flakes.iter().filter(|f| f.extra).count();
                        if extra_count >= 1500 {
                            break;
                        }
                        let depth = rng.gen_range(0.5..0.8);
                        let r = rng.gen_range(2.0..3.5);
                        let bucket_idx = Self::get_bucket_idx(r);
                        let vy = rng.gen_range(80.0..110.0);
                        let vx = rng.gen_range(-12.0..12.0);
                        let alpha = 0.3 + depth * 0.65;
                        self.flakes.push(Flake {
                            x: col_x,
                            y: rng.gen_range(surface_y..pile.y),
                            vx,
                            vy,
                            bvx: 0.0,
                            bvy: 0.0,
                            r,
                            alpha,
                            sway_phase: rng.gen_range(0.0..std::f32::consts::TAU),
                            depth,
                            bucket_idx,
                            fade: 1.0,
                            fade_target: 1.0,
                            extra: true,
                            ballistic: 0.0,
                            skip_owner: 0,
                        });
                    }
                }
            }
        }
        self.owner_data.remove(&key);
    }

    pub fn spawn_thrown(&mut self, x: f32, y: f32, bvx: f32, bvy: f32, skip_owner: isize) {
        let extra_count = self.flakes.iter().filter(|f| f.extra).count();
        if extra_count >= 1500 {
            return;
        }
        let mut rng = rand::thread_rng();
        let r = rng.gen_range(1.4..3.0);
        let bucket_idx = Self::get_bucket_idx(r);
        let depth = 0.8;
        let vy = 40.0 + depth * 100.0;
        let vx = rng.gen_range(-10.0..10.0);
        let alpha = 0.9;
        let sway_phase = rng.gen_range(0.0..std::f32::consts::TAU);
        let ballistic = rng.gen_range(0.9..1.4);

        self.flakes.push(Flake {
            x,
            y,
            vx,
            vy,
            bvx,
            bvy,
            r,
            alpha,
            sway_phase,
            depth,
            bucket_idx,
            fade: 1.0,
            fade_target: 1.0,
            extra: true,
            ballistic,
            skip_owner,
        });
    }

    pub fn spawn_brush_thrown(&mut self, x: f32, y: f32, bvx: f32, bvy: f32, skip_owner: isize) {
        let extra_count = self.flakes.iter().filter(|f| f.extra).count();
        if extra_count >= 1500 {
            return;
        }
        let mut rng = rand::thread_rng();
        let r = rng.gen_range(1.2..2.2);
        let bucket_idx = Self::get_bucket_idx(r);
        let depth = 0.8;
        let vy = 40.0 + depth * 100.0;
        let vx = rng.gen_range(-10.0..10.0);
        let alpha = 0.9;
        let sway_phase = rng.gen_range(0.0..std::f32::consts::TAU);
        let ballistic = rng.gen_range(0.9..1.4);

        self.flakes.push(Flake {
            x,
            y,
            vx,
            vy,
            bvx,
            bvy,
            r,
            alpha,
            sway_phase,
            depth,
            bucket_idx,
            fade: 1.0,
            fade_target: 1.0,
            extra: true,
            ballistic,
            skip_owner,
        });
    }

    pub fn spawn_curtain(&mut self, edge_x: f32, y: f32, outward: f32, amount: f32) {
        let extra_count = self.flakes.iter().filter(|f| f.extra).count();
        if extra_count >= 1500 {
            return;
        }
        let mut rng = rand::thread_rng();
        let count = ((amount / 0.8).round() as usize).min(12);
        for _ in 0..count {
            let r = rng.gen_range(1.0..2.0);
            let bucket_idx = Self::get_bucket_idx(r);
            let depth = 0.8;
            let vy = 40.0 + depth * 100.0;
            let vx = 0.0;
            let alpha = 0.85;
            let sway_phase = rng.gen_range(0.0..std::f32::consts::TAU);
            let ballistic = rng.gen_range(0.5..0.8);

            let x = edge_x + outward * rng.gen_range(0.0..4.0);
            let curtain_y = y + rng.gen_range(0.0..6.0);
            let bvx = outward * rng.gen_range(6.0..28.0);
            let bvy = rng.gen_range(0.0..40.0);

            self.flakes.push(Flake {
                x,
                y: curtain_y,
                vx,
                vy,
                bvx,
                bvy,
                r,
                alpha,
                sway_phase,
                depth,
                bucket_idx,
                fade: 1.0,
                fade_target: 1.0,
                extra: true,
                ballistic,
                skip_owner: 0,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn distribute_berm(
        berm_amt: f32,
        start_c: isize,
        step: isize,
        pile_heights: &mut [f32],
        min_c_ref: &mut usize,
        max_c_ref: &mut usize,
        full_l: f32,
        full_r: f32,
        base_y: f32,
        scratch_curtain_actions: &mut Vec<(f32, f32, f32, f32)>,
        scratch_spawn_actions: &mut Vec<(f32, f32, f32, f32, isize)>,
        total_spawned: &mut usize,
        max_allowed: f32,
        diag: &mut Option<BrushDiagnostics>,
        w_cols: usize,
        skip_owner: isize,
        cx: f32,
        speed: f32,
        dir: i32,
        pile_x_start: f32,
    ) {
        let mut rng = rand::thread_rng();
        let mut sum_weights = 0.0f32;
        let mut weights = vec![0.0f32; w_cols];
        for (k, weight) in weights.iter_mut().enumerate().take(w_cols) {
            let w = 0.5 * (1.0 + (std::f32::consts::PI * k as f32 / w_cols as f32).cos());
            *weight = w;
            sum_weights += w;
        }
        if sum_weights > 0.0 {
            for w in &mut weights {
                *w /= sum_weights;
            }
        }

        let len = pile_heights.len() as isize;
        for (k, &w) in weights.iter().enumerate().take(w_cols) {
            let amt = berm_amt * w;
            let target_col = start_c + (k as isize) * step;
            if target_col >= 0 && target_col < len {
                let idx = target_col as usize;
                let old_h = pile_heights[idx];
                let new_h = old_h + amt;
                if new_h > max_allowed {
                    let actual_berm = (max_allowed - old_h).max(0.0);
                    pile_heights[idx] = max_allowed;
                    if *total_spawned < 40 {
                        let col_x = pile_x_start + idx as f32 * 2.0 + 1.0;
                        let surface_y = base_y - max_allowed;
                        let bvx = if dir != 0 {
                            ((dir as f32) * (30.0 + 0.1 * speed)).clamp(-350.0, 350.0)
                                + rng.gen_range(-40.0..40.0)
                        } else {
                            let s_sign = (col_x - cx).signum();
                            let s_sign = if s_sign == 0.0 { 1.0 } else { s_sign };
                            s_sign * rng.gen_range(30.0..90.0)
                        };
                        let bvy = rng.gen_range(-20.0..40.0);
                        scratch_spawn_actions.push((col_x, surface_y, bvx, bvy, skip_owner));
                        *total_spawned += 1;
                    }
                    if let Some(d) = diag {
                        d.berm_mass += actual_berm;
                    }
                } else {
                    pile_heights[idx] = new_h;
                    if let Some(d) = diag {
                        d.berm_mass += amt;
                    }
                }
                *min_c_ref = (*min_c_ref).min(idx);
                *max_c_ref = (*max_c_ref).max(idx);
            } else {
                let edge_x = if target_col < 0 { full_l } else { full_r };
                let outward = if target_col < 0 { -1.0 } else { 1.0 };
                scratch_curtain_actions.push((edge_x, base_y, outward, amt));
                if let Some(d) = diag {
                    d.curtain_mass += amt;
                }
            }
        }
    }

    pub fn fling_owner(&mut self, owner: isize, wvx: f32, wvy: f32, dt: f32) {
        let _ = dt;
        let speed = wvx.hypot(wvy);
        if speed < 700.0 {
            return;
        }

        let key = Some(owner);
        let data = match self.owner_data.get(&key) {
            Some(d) => d,
            None => return,
        };

        let intervals = &data.visible_intervals;
        if intervals.is_empty() {
            return;
        }

        self.scratch_valid_cols.clear();
        if let Some(pile) = self.piles.get(&key) {
            if pile.heights.is_empty() {
                return;
            }
            for (i, &h) in pile.heights.iter().enumerate() {
                if h > 0.3 {
                    let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                    let is_visible = intervals.iter().any(|&(v1, v2)| col_x >= v1 && col_x <= v2);
                    if is_visible {
                        self.scratch_valid_cols.push(i);
                    }
                }
            }
        } else {
            return;
        }

        if self.scratch_valid_cols.is_empty() {
            return;
        }

        let mut rng = rand::thread_rng();
        let num_centers = rng.gen_range(1..=3).min(self.scratch_valid_cols.len());
        let mut total_removed = 0.0f32;
        self.scratch_spawn_actions.clear();

        if let Some(pile) = self.piles.get_mut(&key) {
            let mut min_c = usize::MAX;
            let mut max_c = 0;
            for _ in 0..num_centers {
                if self.scratch_valid_cols.is_empty() {
                    break;
                }
                let rand_idx = rng.gen_range(0..self.scratch_valid_cols.len());
                let center_col = self.scratch_valid_cols.swap_remove(rand_idx);

                let patch_w = rng.gen_range(6..=12);
                let half_width = patch_w as f32 / 2.0;
                let start_c = (center_col as isize - half_width as isize).max(0) as usize;
                let end_c = (center_col as isize + half_width as isize)
                    .min(pile.heights.len() as isize - 1) as usize;

                for c in start_c..=end_c {
                    let h = pile.heights[c];
                    if h <= 0.0 {
                        continue;
                    }
                    let dist = (c as f32 - center_col as isize as f32).abs();
                    if dist <= half_width {
                        let f = 0.5 * (1.0 + (std::f32::consts::PI * dist / half_width).cos());
                        let reduction = h * 0.7 * f;
                        if reduction > 0.0 {
                            pile.heights[c] -= reduction;
                            total_removed += reduction;
                            min_c = min_c.min(c);
                            max_c = max_c.max(c);

                            let col_x = pile.x_start + c as f32 * 2.0 + 1.0;
                            let surface_y = pile.y - (h - reduction);
                            let bvx = -wvx * 0.35 + rng.gen_range(-80.0..80.0);
                            let bvy = -wvy * 0.35 - rng.gen_range(20.0..120.0);
                            self.scratch_spawn_actions
                                .push((col_x, surface_y, bvx, bvy, owner));
                        }
                    }
                }
            }

            if min_c <= max_c
                && relax_range(
                    &mut pile.heights,
                    min_c.saturating_sub(4),
                    max_c + 4,
                    MAX_SLOPE,
                )
            {
                pile.dirty = true;
            }
        }

        let flake_count = ((total_removed / 1.5).round() as usize).min(25);
        for _ in 0..flake_count {
            if self.scratch_spawn_actions.is_empty() {
                break;
            }
            let idx = rng.gen_range(0..self.scratch_spawn_actions.len());
            let (x, y, bvx, bvy, skip_owner) = self.scratch_spawn_actions.swap_remove(idx);
            self.spawn_thrown(x, y, bvx, bvy, skip_owner);
        }
    }

    pub fn push_flakes(&mut self, cx: f32, cy: f32, cvx: f32, cvy: f32, strength: f32) {
        let r = 80.0f32;
        for flake in &mut self.flakes {
            if flake.fade_target > 0.0 && flake.fade > 0.05 {
                let dx = flake.x - cx;
                let dy = flake.y - cy;
                let d = (dx * dx + dy * dy).sqrt();
                if d < r {
                    let w = (1.0 - d / r).powi(2) * strength;
                    flake.bvx += (cvx * 0.35 + dx / d.max(1.0) * 140.0) * w;
                    flake.bvy += (cvy * 0.35 + dy / d.max(1.0) * 140.0) * w;
                    flake.bvx = flake.bvx.clamp(-700.0, 700.0);
                    flake.bvy = flake.bvy.clamp(-700.0, 700.0);
                }
            }
        }
    }

    pub fn brush(
        &mut self,
        cx: f32,
        cy: f32,
        cvx: f32,
        cvy: f32,
        win_rects: &[(isize, tiny_skia::Rect)],
    ) {
        let speed = (cvx * cvx + cvy * cvy).sqrt();
        if let Some(ref mut d) = self.brush_diag {
            d.speed_sum += speed;
            d.speed_count += 1;
            if speed > 1.0 {
                d.cursor_moved = true;
            }
        }

        let mut rng = rand::thread_rng();
        let radius = 26.0f32;
        let mut total_spawned = 0;
        self.scratch_spawn_actions.clear();

        let owners: Vec<Option<isize>> = self.piles.keys().cloned().collect();

        for owner in owners {
            let mut hidden = false;
            if is_floor_like(owner) {
                for (_, rect) in win_rects {
                    if cx >= rect.x()
                        && cx <= rect.x() + rect.width()
                        && cy >= rect.y()
                        && cy <= rect.y() + rect.height()
                    {
                        hidden = true;
                        break;
                    }
                }
            } else {
                if let Some(hwnd) = owner {
                    if let Some(pos) = win_rects.iter().position(|(h, _)| *h == hwnd) {
                        for (_, rect) in win_rects.iter().take(pos) {
                            if cx >= rect.x()
                                && cx <= rect.x() + rect.width()
                                && cy >= rect.y()
                                && cy <= rect.y() + rect.height()
                            {
                                hidden = true;
                                break;
                            }
                        }
                    }
                }
            }

            if hidden {
                continue;
            }

            if let Some(pile) = self.piles.get_mut(&owner) {
                if pile.heights.is_empty() {
                    continue;
                }
                let base = pile.y;
                let len = pile.heights.len();

                if is_floor_like(owner) {
                    // Gradual, circle-shaped erase: only the part of each snow column that overlaps
                    // the brush circle is removed, and only a fraction per call.
                    const FLOOR_ERASE_FRAC: f32 = 0.1;
                    const FLOOR_ERASE_MAX: f32 = 1.5;
                    let mut min_c = usize::MAX;
                    let mut max_c = 0;
                    let mut floor_removed = 0.0f32;

                    for i in 0..len {
                        let h = pile.heights[i];
                        if h <= 0.0 {
                            continue;
                        }
                        let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                        let dx = col_x - cx;
                        if dx.abs() >= radius {
                            continue;
                        }
                        let s = (radius * radius - dx * dx).sqrt();
                        // vertical overlap of the circle chord [cy-s, cy+s] with the snow column [base-h, base]
                        let overlap = (cy + s).min(base) - (cy - s).max(base - h);
                        if overlap <= 0.0 {
                            continue;
                        }
                        let removed = (overlap * FLOOR_ERASE_FRAC).min(FLOOR_ERASE_MAX).min(h);
                        pile.heights[i] = h - removed;
                        floor_removed += removed;
                        min_c = min_c.min(i);
                        max_c = max_c.max(i);
                    }
                    if floor_removed > 0.0 {
                        if let Some(ref mut d) = self.brush_diag {
                            d.snow_removed += floor_removed;
                        }
                        let _ = relax_range(
                            &mut pile.heights,
                            min_c.saturating_sub(4),
                            max_c + 4,
                            MAX_SLOPE,
                        );
                        pile.dirty = true;
                    }
                } else {
                    let mut total_removed = 0.0f32;
                    let mut min_c = usize::MAX;
                    let mut max_c = 0;
                    let mut touched_points: Vec<(f32, f32, f32)> = Vec::new();

                    let cx_col = ((cx - pile.x_start - 1.0) / 2.0).round() as isize;
                    let start_search = (cx_col - 20).max(0) as usize;
                    let end_search = (cx_col + 20).min(pile.heights.len() as isize - 1) as usize;
                    let mut local_max = 0.0f32;
                    if start_search <= end_search && end_search < pile.heights.len() {
                        for &h in pile.heights.iter().take(end_search + 1).skip(start_search) {
                            if h > local_max {
                                local_max = h;
                            }
                        }
                    }
                    let hard_cap_val = Self::hard_cap(false, owner, base);
                    let plow_cap = hard_cap_val.min((local_max * 1.35 + 6.0).max(18.0));

                    for i in 0..len {
                        let h = pile.heights[i];
                        if h <= 0.0 {
                            continue;
                        }
                        let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                        let dx = col_x - cx;
                        if dx.abs() < radius {
                            let s = if radius * radius > dx * dx {
                                (radius * radius - dx * dx).sqrt()
                            } else {
                                0.0
                            };
                            let surface = base - h;
                            if cy - s <= base && cy + s >= surface {
                                let arc_h = (base - (cy + s)).max(0.0);
                                if h > arc_h {
                                    let removed = (h - arc_h) * PLOW_FRAC;
                                    let new_h = h - removed;
                                    if new_h != h {
                                        pile.heights[i] = new_h;
                                        total_removed += removed;
                                        min_c = min_c.min(i);
                                        max_c = max_c.max(i);
                                        let surface_y = base - new_h;
                                        touched_points.push((col_x, surface_y, dx));
                                    }
                                }
                            }
                        }
                    }

                    if total_removed > 0.0 {
                        let speed_val = (cvx * cvx + cvy * cvy).sqrt();
                        let dir = if cvx.abs() > 40.0 {
                            cvx.signum() as i32
                        } else {
                            0
                        };
                        let fling_frac = ((speed_val - 600.0) / 1400.0).clamp(0.0, 0.8);

                        let thrown_part = total_removed * fling_frac;
                        let flake_count =
                            ((thrown_part / 1.5).round() as usize).min(40 - total_spawned);
                        let skip_hwnd = owner.unwrap_or(0);
                        for _ in 0..flake_count {
                            if touched_points.is_empty() || total_spawned >= 40 {
                                break;
                            }
                            let p_idx = rng.gen_range(0..touched_points.len());
                            let (col_x, surface_y, dx) = touched_points[p_idx];
                            let bvx = if dir != 0 {
                                ((dir as f32) * (30.0 + 0.1 * speed_val)).clamp(-350.0, 350.0)
                                    + rng.gen_range(-40.0..40.0)
                            } else {
                                let s_sign = (col_x - cx).signum();
                                let s_sign = if s_sign == 0.0 {
                                    if dx >= 0.0 {
                                        1.0
                                    } else {
                                        -1.0
                                    }
                                } else {
                                    s_sign
                                };
                                s_sign * rng.gen_range(30.0..90.0)
                            };
                            let bvy = rng.gen_range(-20.0..40.0);
                            self.scratch_spawn_actions
                                .push((col_x, surface_y, bvx, bvy, skip_hwnd));
                            total_spawned += 1;
                        }

                        if let Some(ref mut d) = self.brush_diag {
                            d.snow_removed += total_removed;
                            d.thrown_spawned += flake_count;
                        }

                        let remaining_part = total_removed * (1.0 - fling_frac);
                        let o_data = self.owner_data.get(&owner);
                        let full_left = o_data.map_or(pile.x_start, |d| d.full_left);
                        let full_right =
                            o_data.map_or(pile.x_start + len as f32 * 2.0, |d| d.full_right);

                        let touched_columns = touched_points.len();
                        let w_cols = (1.5 * touched_columns as f32).max(24.0).round() as usize;

                        let mut diag_ref = self.brush_diag.take();
                        if dir != 0 {
                            let edge_x = cx + (dir as f32) * radius;
                            let start_c = ((edge_x - pile.x_start - 1.0) / 2.0).round() as isize;
                            Self::distribute_berm(
                                remaining_part,
                                start_c,
                                dir as isize,
                                &mut pile.heights,
                                &mut min_c,
                                &mut max_c,
                                full_left,
                                full_right,
                                base,
                                &mut self.scratch_curtain_actions,
                                &mut self.scratch_spawn_actions,
                                &mut total_spawned,
                                plow_cap,
                                &mut diag_ref,
                                w_cols,
                                skip_hwnd,
                                cx,
                                speed_val,
                                dir,
                                pile.x_start,
                            );
                        } else {
                            let half_rem = remaining_part * 0.5;
                            let cx_col_b = ((cx - pile.x_start - 1.0) / 2.0).round() as isize;
                            Self::distribute_berm(
                                half_rem,
                                cx_col_b,
                                1,
                                &mut pile.heights,
                                &mut min_c,
                                &mut max_c,
                                full_left,
                                full_right,
                                base,
                                &mut self.scratch_curtain_actions,
                                &mut self.scratch_spawn_actions,
                                &mut total_spawned,
                                plow_cap,
                                &mut diag_ref,
                                w_cols,
                                skip_hwnd,
                                cx,
                                speed_val,
                                dir,
                                pile.x_start,
                            );
                            Self::distribute_berm(
                                half_rem,
                                cx_col_b,
                                -1,
                                &mut pile.heights,
                                &mut min_c,
                                &mut max_c,
                                full_left,
                                full_right,
                                base,
                                &mut self.scratch_curtain_actions,
                                &mut self.scratch_spawn_actions,
                                &mut total_spawned,
                                plow_cap,
                                &mut diag_ref,
                                w_cols,
                                skip_hwnd,
                                cx,
                                speed_val,
                                dir,
                                pile.x_start,
                            );
                        }
                        self.brush_diag = diag_ref;

                        if min_c <= max_c
                            && relax_range(
                                &mut pile.heights,
                                min_c.saturating_sub(4),
                                max_c + 4,
                                2.6,
                            )
                        {
                            pile.dirty = true;
                        }
                    }
                }
            }
        }

        let curtain_actions = std::mem::take(&mut self.scratch_curtain_actions);
        for (edge_x, y, outward, amount) in curtain_actions {
            self.spawn_curtain(edge_x, y, outward, amount);
        }

        let actions = self.scratch_spawn_actions.clone();
        for (x, y, bvx, bvy, skip_owner) in actions {
            self.spawn_brush_thrown(x, y, bvx, bvy, skip_owner);
        }
    }

    pub fn update(&mut self, dt: f32, segments: &[Segment]) {
        let mut rng = rand::thread_rng();
        self.wind_time += dt;
        let height_limit = self.height_limit;
        let strength = self.strength;

        let global_wind =
            (self.wind_time * 0.6).sin() * 20.0 + (self.wind_time * 0.25).sin() * 35.0;
        if self.clearing || self.clear_hold {
            self.clear_gust = 1.0;
        } else if self.clear_gust > 0.0 {
            self.clear_gust = (self.clear_gust - dt / 0.8).max(0.0);
        }

        for data in self.owner_data.values_mut() {
            data.visible_intervals.clear();
        }

        self.scratch_current_owners.clear();
        for seg in segments {
            self.scratch_current_owners.insert(seg.owner);
            let data = self
                .owner_data
                .entry(seg.owner)
                .or_insert_with(|| OwnerData {
                    y: seg.y,
                    full_left: seg.full_left,
                    full_right: seg.full_right,
                    visible_intervals: Vec::new(),
                });
            data.y = seg.y;
            data.full_left = seg.full_left;
            data.full_right = seg.full_right;
            data.visible_intervals.push((seg.x1, seg.x2));
        }
        self.owner_data
            .retain(|owner, _| self.scratch_current_owners.contains(owner));

        // 1. Remove piles whose owner is no longer present
        let mut owners_to_remove = Vec::new();
        for owner in self.piles.keys() {
            if !self.scratch_current_owners.contains(owner) {
                owners_to_remove.push(*owner);
            }
        }
        for owner in owners_to_remove {
            if let Some(pile) = self.piles.remove(&owner) {
                for (i, &h) in pile.heights.iter().enumerate() {
                    if h > 0.5 {
                        let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                        let surface_y = pile.y - h;
                        let count = rng.gen_range(1..=2);
                        for _ in 0..count {
                            let extra_count = self.flakes.iter().filter(|f| f.extra).count();
                            if extra_count >= 1500 {
                                break;
                            }
                            let depth = rng.gen_range(0.5..0.8);
                            let r = rng.gen_range(2.0..3.5);
                            let bucket_idx = Self::get_bucket_idx(r);
                            let vy = rng.gen_range(80.0..110.0);
                            let vx = rng.gen_range(-12.0..12.0);
                            let alpha = 0.3 + depth * 0.65;
                            self.flakes.push(Flake {
                                x: col_x,
                                y: rng.gen_range(surface_y..pile.y),
                                vx,
                                vy,
                                bvx: 0.0,
                                bvy: 0.0,
                                r,
                                alpha,
                                sway_phase: rng.gen_range(0.0..std::f32::consts::TAU),
                                depth,
                                bucket_idx,
                                fade: 1.0,
                                fade_target: 1.0,
                                extra: true,
                                ballistic: 0.0,
                                skip_owner: 0,
                            });
                        }
                    }
                }
            }
            self.owner_data.remove(&owner);
        }

        // 2. Update existing piles or create/resize in place
        for (owner, data) in &self.owner_data {
            let num_cols = ((data.full_right - data.full_left) / 2.0).ceil().max(1.0) as usize;
            let x_start = data.full_left;
            let y = data.y;

            if let Some(pile) = self.piles.get_mut(owner) {
                let x_changed = pile.x_start != x_start;
                let y_changed = pile.y != y;
                let size_changed = pile.heights.len() != num_cols;

                pile.x_start = x_start;
                pile.y = y;

                if size_changed {
                    let old_heights = std::mem::take(&mut pile.heights);
                    let mut new_heights = vec![0.0; num_cols];
                    for (i, &h) in old_heights.iter().enumerate() {
                        if i < num_cols {
                            new_heights[i] = h;
                        } else if h > 0.5 {
                            let col_x = x_start + i as f32 * 2.0 + 1.0;
                            let surface_y = y - h;
                            let count = rng.gen_range(1..=2);
                            for _ in 0..count {
                                let extra_count = self.flakes.iter().filter(|f| f.extra).count();
                                if extra_count >= 1500 {
                                    break;
                                }
                                let depth = rng.gen_range(0.5..0.8);
                                let r = rng.gen_range(2.0..3.5);
                                let bucket_idx = Self::get_bucket_idx(r);
                                let vy = rng.gen_range(80.0..110.0);
                                let vx = rng.gen_range(-12.0..12.0);
                                let alpha = 0.3 + depth * 0.65;
                                self.flakes.push(Flake {
                                    x: col_x,
                                    y: rng.gen_range(surface_y..y),
                                    vx,
                                    vy,
                                    bvx: 0.0,
                                    bvy: 0.0,
                                    r,
                                    alpha,
                                    sway_phase: rng.gen_range(0.0..std::f32::consts::TAU),
                                    depth,
                                    bucket_idx,
                                    fade: 1.0,
                                    fade_target: 1.0,
                                    extra: true,
                                    ballistic: 0.0,
                                    skip_owner: 0,
                                });
                            }
                        }
                    }
                    pile.heights = new_heights;
                    pile.dirty = true;
                }

                if x_changed || y_changed || size_changed {
                    pile.dirty = true;
                }
            } else {
                self.piles.insert(
                    *owner,
                    Pile {
                        x_start,
                        y,
                        heights: vec![0.0; num_cols],
                        dirty: true,
                        cached_intervals: Vec::new(),
                        cached_shadow_path: None,
                        cached_body_path: None,
                        cached_top_path: None,
                        cached_shader: None,
                    },
                );
            }
        }

        if self.clearing {
            let dir = self.clear_dir;
            self.clear_front += dir * CLEAR_FRONT_SPEED * dt;
            let front = self.clear_front;
            let extra_count = self.flakes.iter().filter(|f| f.extra).count();
            let mut new_flakes = Vec::new();
            for pile in self.piles.values_mut() {
                let mut changed = false;
                for i in 0..pile.heights.len() {
                    let h = pile.heights[i];
                    if h <= 0.0 {
                        continue;
                    }
                    let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                    let passed = if dir > 0.0 {
                        col_x <= front
                    } else {
                        col_x >= front
                    };
                    if !passed {
                        continue;
                    }
                    pile.heights[i] = 0.0;
                    changed = true;
                    if h > 0.5 && i % 2 == 0 && (extra_count + new_flakes.len()) < 1500 {
                        let n = if h > 10.0 { 2 } else { 1 };
                        for _ in 0..n {
                            let depth = rng.gen_range(0.5..0.9);
                            let r = rng.gen_range(1.5..3.5);
                            new_flakes.push(Flake {
                                x: col_x,
                                y: rng.gen_range((pile.y - h)..pile.y),
                                vx: rng.gen_range(450.0..900.0) * dir,
                                vy: rng.gen_range(-30.0..-5.0),
                                bvx: 0.0,
                                bvy: 0.0,
                                r,
                                alpha: 0.3 + depth * 0.65,
                                sway_phase: rng.gen_range(0.0..std::f32::consts::TAU),
                                depth,
                                bucket_idx: Self::get_bucket_idx(r),
                                fade: 1.0,
                                fade_target: 1.0,
                                extra: true,
                                ballistic: 0.0,
                                skip_owner: 0,
                            });
                        }
                    }
                }
                if changed {
                    pile.dirty = true;
                }
            }
            self.flakes.extend(new_flakes);
            if (dir > 0.0 && self.clear_front > self.width + 200.0)
                || (dir < 0.0 && self.clear_front < -200.0)
            {
                self.clearing = false;
                self.clear_front = 0.0;
            }
        }

        if self.clear_hold && !self.clearing {
            self.clear_hold_time += dt;
            let extra_left = !self.flakes.is_empty();
            if !extra_left || self.clear_hold_time > 10.0 {
                self.clear_hold = false;
                self.refill_from_top = true;
            }
        }
        let hold = self.clear_hold;

        let clear_gust = self.clear_gust;
        let wind_time = self.wind_time;
        let clearing_now = self.clearing;
        let front_now = self.clear_front;
        let dir_now = self.clear_dir;

        self.scratch_remove_indices.clear();
        let alive_count = self
            .flakes
            .iter()
            .filter(|f| !f.extra && f.fade_target > 0.0)
            .count();
        for (i, flake) in self.flakes.iter_mut().enumerate() {
            if flake.fade < flake.fade_target {
                flake.fade = (flake.fade + 1.5 * dt).min(flake.fade_target);
            } else if flake.fade > flake.fade_target {
                flake.fade = (flake.fade - 1.5 * dt).max(flake.fade_target);
            }

            if flake.fade_target == 0.0 && flake.fade <= 0.01 {
                self.scratch_remove_indices.push(i);
                continue;
            }

            flake.sway_phase += dt * 2.0;
            let sway = flake.sway_phase.sin() * (5.0 + flake.depth * 10.0);
            let effective_wind = global_wind * (0.5 + 0.5 * flake.depth);
            // A flake is swept by the clearing gust once the wave front has passed it
            let passed_front = if dir_now > 0.0 {
                flake.x <= front_now
            } else {
                flake.x >= front_now
            };
            let swept = hold && (!clearing_now || passed_front);
            if swept {
                flake.bvx += CLEAR_GUST_WIND * 2.5 * dt * dir_now;
            }
            let mut swirl_x = 0.0f32;
            let mut swirl_y = 0.0f32;
            if swept && clear_gust > 0.0 {
                let dir = if flake.depth < 0.7 { 1.0 } else { -1.0 };
                let ang = flake.sway_phase * CLEAR_SWIRL_RATE * dir;
                swirl_x = ang.cos() * CLEAR_SWIRL_SPEED * clear_gust;
                swirl_y = (ang.sin() * CLEAR_SWIRL_SPEED
                    + (flake.x * 0.008 - wind_time * 6.0 * dir_now).sin() * 300.0)
                    * clear_gust;
            }
            flake.x +=
                (flake.vx * strength + (effective_wind + sway) * strength + flake.bvx + swirl_x)
                    * dt;
            flake.y += (flake.vy * strength + flake.bvy + swirl_y) * dt;

            if flake.extra && flake.ballistic > 0.0 {
                flake.bvy += 520.0 * dt;
                flake.bvx *= (-dt).exp();
                flake.bvy *= (-0.8 * dt).exp();
                flake.ballistic -= dt;
                if flake.ballistic <= 0.0 {
                    flake.bvx *= (-4.0 * dt).exp();
                    flake.bvy *= (-4.0 * dt).exp();
                }
            } else {
                flake.bvx *= (-2.5 * dt).exp();
                flake.bvy *= (-2.5 * dt).exp();
            }

            let mut landed = false;
            let mut land_seg_owner = None;

            if flake.fade_target > 0.0 && !hold {
                for seg in segments {
                    if flake.skip_owner != 0 && seg.owner == Some(flake.skip_owner) {
                        continue;
                    }
                    if flake.x >= seg.x1 && flake.x <= seg.x2 {
                        if let Some(pile) = self.piles.get(&seg.owner) {
                            let col_idx = ((flake.x - pile.x_start) / 2.0).floor() as i32;
                            if col_idx >= 0 && (col_idx as usize) < pile.heights.len() {
                                let h = pile.heights[col_idx as usize];
                                let mut surface_y = pile.y - h;
                                let mut land_limit = seg.y + 10.0;
                                if is_floor_like(seg.owner) && h > 0.0 {
                                    // If the base of this pile is covered by a window, the pile is not visible here:
                                    // let the flake fall down to the base line.
                                    let probe_y = pile.y - 1.0;
                                    let hidden = self.window_rects.iter().any(|(_, r)| {
                                        flake.x >= r.x()
                                            && flake.x <= r.x() + r.width()
                                            && probe_y >= r.y()
                                            && probe_y <= r.y() + r.height()
                                    });
                                    if hidden {
                                        surface_y = pile.y - 1.0;
                                        land_limit = seg.y + 24.0;
                                    }
                                }
                                if flake.y >= surface_y && flake.y <= land_limit {
                                    landed = true;
                                    land_seg_owner = seg.owner;
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if landed {
                if let Some(pile) = self.piles.get_mut(&land_seg_owner) {
                    let col_idx = ((flake.x - pile.x_start) / 2.0).floor() as i32;
                    if col_idx >= 0 && (col_idx as usize) < pile.heights.len() {
                        let target_col = col_idx as usize;
                        let amount = (0.5 + 0.35 * flake.r) * ACCUMULATION;
                        let max_h = Self::landing_cap(height_limit, land_seg_owner, pile.y);
                        let len = pile.heights.len();

                        let mut touched_min = target_col;
                        let mut touched_max = target_col;

                        if target_col > 0 {
                            let c_left = target_col - 1;
                            pile.heights[c_left] =
                                (pile.heights[c_left] + 0.25 * amount).min(max_h);
                            touched_min = touched_min.min(c_left);
                            touched_max = touched_max.max(c_left);
                        }

                        pile.heights[target_col] =
                            (pile.heights[target_col] + 0.5 * amount).min(max_h);
                        touched_min = touched_min.min(target_col);
                        touched_max = touched_max.max(target_col);

                        if target_col + 1 < len {
                            let c_right = target_col + 1;
                            pile.heights[c_right] =
                                (pile.heights[c_right] + 0.25 * amount).min(max_h);
                            touched_min = touched_min.min(c_right);
                            touched_max = touched_max.max(c_right);
                        }

                        let _ = relax_range(
                            &mut pile.heights,
                            touched_min.saturating_sub(4),
                            touched_max + 4,
                            MAX_SLOPE,
                        );
                        pile.dirty = true;
                    }
                }

                if flake.extra {
                    self.scratch_remove_indices.push(i);
                } else if !hold && alive_count <= self.target_flake_count {
                    flake.y = -flake.r;
                    flake.x = rng.gen_range(0.0..self.width);
                    let depth = rng.gen_range(0.0..1.0);
                    flake.depth = depth;
                    flake.r = if depth < 0.5 {
                        rng.gen_range(0.8..1.5)
                    } else {
                        rng.gen_range(2.5..4.0)
                    };
                    flake.bucket_idx = Self::get_bucket_idx(flake.r);
                    flake.vy = 40.0 + depth * 100.0;
                    flake.bvx = 0.0;
                    flake.bvy = 0.0;
                    flake.alpha = 0.3 + depth * 0.65;
                    flake.fade = 0.0;
                    flake.fade_target = 1.0;
                } else {
                    flake.fade_target = 0.0;
                }
            } else if flake.x < -flake.r
                || flake.x > self.width + flake.r
                || flake.y > self.height + flake.r
            {
                if flake.extra {
                    self.scratch_remove_indices.push(i);
                } else if !hold && alive_count <= self.target_flake_count {
                    flake.y = -flake.r;
                    flake.x = rng.gen_range(0.0..self.width);
                    let depth = rng.gen_range(0.0..1.0);
                    flake.depth = depth;
                    flake.r = if depth < 0.5 {
                        rng.gen_range(0.8..1.5)
                    } else {
                        rng.gen_range(2.5..4.0)
                    };
                    flake.bucket_idx = Self::get_bucket_idx(flake.r);
                    flake.vy = 40.0 + depth * 100.0;
                    flake.bvx = 0.0;
                    flake.bvy = 0.0;
                    flake.alpha = 0.3 + depth * 0.65;
                    flake.fade = 0.0;
                    flake.fade_target = 1.0;
                } else {
                    flake.fade_target = 0.0;
                }
            }
        }

        if !self.scratch_remove_indices.is_empty() {
            self.scratch_remove_indices
                .sort_unstable_by(|a, b| b.cmp(a));
            for &idx in &self.scratch_remove_indices {
                self.flakes.swap_remove(idx);
            }
        }

        // 4. Intensity decrease
        let alive_count = self
            .flakes
            .iter()
            .filter(|f| !f.extra && f.fade_target > 0.0)
            .count();
        if alive_count > self.target_flake_count {
            let excess = alive_count - self.target_flake_count;
            let n = ((excess as f32 * (1.0 - (-dt / 1.5).exp())).ceil() as usize).min(alive_count);
            let mut chosen = 0;
            let mut attempts = 0;
            while chosen < n && attempts < n * 10 {
                attempts += 1;
                let idx = rng.gen_range(0..self.flakes.len());
                if !self.flakes[idx].extra && self.flakes[idx].fade_target > 0.0 {
                    self.flakes[idx].fade_target = 0.0;
                    chosen += 1;
                }
            }
        }

        // 5. Intensity increase
        let mut current_alive = self
            .flakes
            .iter()
            .filter(|f| !f.extra && f.fade_target > 0.0)
            .count();
        while !hold && current_alive < self.target_flake_count {
            let depth = rng.gen_range(0.0..1.0);
            let r = if depth < 0.5 {
                rng.gen_range(0.8..1.5)
            } else {
                rng.gen_range(2.5..4.0)
            };
            let bucket_idx = Self::get_bucket_idx(r);
            let vy = 40.0 + depth * 100.0;
            let alpha = 0.3 + depth * 0.65;
            self.flakes.push(Flake {
                x: rng.gen_range(0.0..self.width),
                y: if self.refill_from_top {
                    rng.gen_range((-0.4 * self.height)..0.0)
                } else {
                    rng.gen_range((-0.1 * self.height)..self.height)
                },
                vx: rng.gen_range(-10.0..10.0),
                vy,
                bvx: 0.0,
                bvy: 0.0,
                r,
                alpha,
                sway_phase: rng.gen_range(0.0..(std::f32::consts::TAU)),
                depth,
                bucket_idx,
                fade: 0.0,
                fade_target: 1.0,
                extra: false,
                ballistic: 0.0,
                skip_owner: 0,
            });
            current_alive += 1;
        }
        if !hold {
            self.refill_from_top = false;
        }
        if self.flakes.len() > 6000 {
            self.flakes.drain(0..self.flakes.len() - 6000);
        }

        self.scratch_spawn_actions.clear();
        for (owner, pile) in &mut self.piles {
            if is_floor_like(*owner) {
                continue;
            }
            let data = match self.owner_data.get(owner) {
                Some(d) => d,
                None => continue,
            };
            let full_left = data.full_left;
            let full_right = data.full_right;
            let h_cap = Self::hard_cap(height_limit, *owner, pile.y);
            let edge_zone = EDGE_ZONE * (1.0f32).max(h_cap / 16.0);
            let mut spill_flakes_count = 0;
            let mut pile_modified = false;
            let mut min_c = usize::MAX;
            let mut max_c = 0;
            let num_cols = pile.heights.len();

            for i in 0..num_cols {
                let h = pile.heights[i];
                if h <= 0.0 {
                    continue;
                }
                let col_x = pile.x_start + i as f32 * 2.0 + 1.0;
                let left_dist = (col_x - full_left).abs();
                let right_dist = (full_right - col_x).abs();
                let d = left_dist.min(right_dist);

                if d < edge_zone {
                    let cap = h_cap * smoothstep(0.0, edge_zone, d);
                    if h > cap {
                        let excess = h - cap;
                        pile.heights[i] = cap;
                        pile_modified = true;
                        min_c = min_c.min(i);
                        max_c = max_c.max(i);

                        if spill_flakes_count < 24 {
                            spill_flakes_count += 1;
                            let nearest_is_left = left_dist < right_dist;
                            let edge_x = if nearest_is_left {
                                full_left
                            } else {
                                full_right
                            };
                            let outward = if nearest_is_left { -1.0 } else { 1.0 };
                            let y = pile.y - cap;
                            self.scratch_curtain_actions
                                .push((edge_x, y, outward, excess));
                        }
                    }
                } else {
                    if h > h_cap {
                        pile.heights[i] = h_cap;
                        pile_modified = true;
                        min_c = min_c.min(i);
                        max_c = max_c.max(i);
                    }
                }
            }

            if pile_modified
                && relax_range(
                    &mut pile.heights,
                    min_c.saturating_sub(4),
                    max_c + 4,
                    MAX_SLOPE,
                )
            {
                pile.dirty = true;
            }
        }

        let curtain_actions = std::mem::take(&mut self.scratch_curtain_actions);
        for (edge_x, y, outward, amount) in curtain_actions {
            self.spawn_curtain(edge_x, y, outward, amount);
        }

        for (owner, pile) in &mut self.piles {
            pile.update_cache(*owner, &self.owner_data);
        }
    }

    fn draw_pile(&self, pixmap: &mut PixmapMut, pile: &Pile) {
        if let Some(ref shadow_path) = pile.cached_shadow_path {
            let mut paint = Paint::default();
            paint.set_color(bgra(0.0, 0.0, 0.0, 0.18));
            pixmap.fill_path(
                shadow_path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        if let Some(ref body_path) = pile.cached_body_path {
            let mut paint = Paint::default();
            if let Some(ref shader) = pile.cached_shader {
                paint.shader = shader.clone();
            } else {
                paint.set_color(bgra(1.0, 1.0, 1.0, 1.0));
            }
            pixmap.fill_path(
                body_path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        if let Some(ref top_path) = pile.cached_top_path {
            let mut paint = Paint::default();
            paint.set_color(bgra(1.0, 1.0, 1.0, 0.85));
            let stroke = Stroke {
                width: 1.5,
                ..Default::default()
            };
            pixmap.stroke_path(top_path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn draw(&self, pixmap: &mut PixmapMut, win_rects: &[(isize, tiny_skia::Rect)]) {
        if let Some(pile) = self.piles.get(&None) {
            self.draw_pile(pixmap, pile);
        }
        if let Some(pile) = self.piles.get(&Some(GROUND_ID)) {
            self.draw_pile(pixmap, pile);
        }

        for &(hwnd, ref rect) in win_rects.iter().rev() {
            let clear_paint = Paint {
                blend_mode: tiny_skia::BlendMode::Clear,
                anti_alias: false,
                ..Default::default()
            };
            let mut pb = PathBuilder::new();
            pb.push_rect(*rect);
            if let Some(path) = pb.finish() {
                pixmap.fill_path(
                    &path,
                    &clear_paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }

            if let Some(pile) = self.piles.get(&Some(hwnd)) {
                self.draw_pile(pixmap, pile);
            }
        }

        let screen_width = pixmap.width() as i32;
        let screen_height = pixmap.height() as i32;
        let dst_data = pixmap.data_mut();

        for flake in &self.flakes {
            if flake.fade <= 0.01 {
                continue;
            }
            let effective_alpha = flake.alpha * flake.fade * self.flake_opacity;
            let alpha_idx = ((effective_alpha * 15.0).round() as usize).clamp(0, 15);
            let sprite = &self.sprites[flake.bucket_idx * 16 + alpha_idx];
            let sw = sprite.width as i32;
            let sh = sprite.height as i32;
            let px = (flake.x - sw as f32 / 2.0).round() as i32;
            let py = (flake.y - sh as f32 / 2.0).round() as i32;

            if px >= screen_width || px + sw <= 0 || py >= screen_height || py + sh <= 0 {
                continue;
            }

            let mut src_x = 0;
            let mut src_y = 0;
            let mut dst_x = px;
            let mut dst_y = py;
            let mut draw_w = sw;
            let mut draw_h = sh;

            if dst_x < 0 {
                src_x = -dst_x;
                draw_w -= src_x;
                dst_x = 0;
            }
            if dst_y < 0 {
                src_y = -dst_y;
                draw_h -= src_y;
                dst_y = 0;
            }
            if dst_x + draw_w > screen_width {
                draw_w = screen_width - dst_x;
            }
            if dst_y + draw_h > screen_height {
                draw_h = screen_height - dst_y;
            }
            if draw_w <= 0 || draw_h <= 0 {
                continue;
            }

            for y_off in 0..draw_h {
                let screen_row_idx = (dst_y + y_off) * screen_width + dst_x;
                let sprite_row_idx = (src_y + y_off) * sw + src_x;
                for x_off in 0..draw_w {
                    let si = ((sprite_row_idx + x_off) * 4) as usize;
                    let di = ((screen_row_idx + x_off) * 4) as usize;
                    let sa = sprite.data[si + 3];
                    if sa == 0 {
                        continue;
                    }
                    if sa == 255 {
                        dst_data[di] = sprite.data[si];
                        dst_data[di + 1] = sprite.data[si + 1];
                        dst_data[di + 2] = sprite.data[si + 2];
                        dst_data[di + 3] = 255;
                    } else {
                        let inv_a = 255 - sa as u32;
                        let b = sprite.data[si] as u32 + (dst_data[di] as u32 * inv_a + 127) / 255;
                        let g = sprite.data[si + 1] as u32
                            + (dst_data[di + 1] as u32 * inv_a + 127) / 255;
                        let r = sprite.data[si + 2] as u32
                            + (dst_data[di + 2] as u32 * inv_a + 127) / 255;
                        let a = sa as u32 + (dst_data[di + 3] as u32 * inv_a + 127) / 255;
                        dst_data[di] = b as u8;
                        dst_data[di + 1] = g as u8;
                        dst_data[di + 2] = r as u8;
                        dst_data[di + 3] = a as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bucket_idx() {
        assert_eq!(Snow::get_bucket_idx(0.8), 0);
        assert_eq!(Snow::get_bucket_idx(4.0), 7);
        assert_eq!(Snow::get_bucket_idx(100.0), 7);
        assert_eq!(Snow::get_bucket_idx(-5.0), 0);
        for i in 0..=32 {
            let v = 0.8 + i as f32 * 0.1;
            assert!(Snow::get_bucket_idx(v) <= 7);
        }
    }

    #[test]
    fn test_floor_like() {
        assert!(is_floor_like(None));
        assert!(is_floor_like(Some(GROUND_ID)));
        assert!(!is_floor_like(Some(42)));
    }

    #[test]
    fn test_new() {
        let snow_100 = Snow::new(800.0, 600.0, 100);
        assert_eq!(snow_100.flakes.len(), 100);
        for flake in &snow_100.flakes {
            assert!(flake.x >= 0.0 && flake.x <= 800.0);
            assert!(flake.y >= 0.0 && flake.y <= 600.0);
        }

        let snow_0 = Snow::new(800.0, 600.0, 0);
        assert!(snow_0.flakes.is_empty());
    }

    #[test]
    fn test_relax() {
        let mut heights = vec![0.0, 10.0, 0.0];
        let mut changed = false;
        for _ in 0..20 {
            if relax_range(&mut heights, 0, 2, 2.2) {
                changed = true;
            }
        }
        assert!(changed);
        for i in 0..heights.len() - 1 {
            let diff = (heights[i] - heights[i + 1]).abs();
            assert!(diff <= 2.2 + 0.001);
        }

        let mut smooth = vec![1.0, 1.5, 2.0];
        let orig = smooth.clone();
        let res = relax_range(&mut smooth, 0, 2, 2.2);
        assert!(!res);
        assert_eq!(smooth, orig);
    }

    #[test]
    fn test_smoothstep() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        let val = smoothstep(0.0, 1.0, 0.5);
        assert!((val - 0.5).abs() <= 0.001);
    }
}
