//! Pieces shared by the tray menus of the Unix backends: setting presets and the icon.

/// `(label, value)` presets for a setting. The one closest to the current value is shown as selected.
pub type Presets = &'static [(&'static str, usize)];

/// Falling flakes (`AppState::target_flakes`).
pub const INTENSITY: Presets = &[
    ("Light", 150),
    ("Normal", 400),
    ("Medium", 800),
    ("Heavy", 1500),
    ("Blizzard", 3000),
];
/// Flake opacity in percent (`AppState::opacity`).
pub const OPACITY: Presets = &[("25%", 25), ("50%", 50), ("75%", 75), ("100%", 100)];
/// Fall speed and wind in percent (`AppState::strength`).
pub const STRENGTH: Presets = &[("50%", 50), ("100%", 100), ("150%", 150), ("200%", 200)];
/// Frames per second (`AppState::target_fps`).
pub const FPS: Presets = &[("20", 20), ("30", 30), ("60", 60)];

/// Index of the preset whose value is closest to `current`.
pub fn nearest(presets: Presets, current: usize) -> usize {
    presets
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, value))| value.abs_diff(current))
        .map_or(0, |(index, _)| index)
}

/// A white snowflake on a blue disc as straight (not premultiplied) RGBA, `size * size * 4` bytes.
pub fn snowflake_rgba(size: u32) -> Vec<u8> {
    use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

    let mut pixmap = Pixmap::new(size, size).expect("icon size is non-zero");
    let center = size as f32 / 2.0;

    let mut paint = Paint::default();
    paint.set_color_rgba8(0x2E, 0x7D, 0xD7, 0xFF);
    if let Some(disc) = PathBuilder::from_circle(center, center, center - 0.5) {
        pixmap.fill_path(
            &disc,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    paint.set_color_rgba8(0xFF, 0xFF, 0xFF, 0xFF);
    let stroke = Stroke {
        width: size as f32 / 11.0,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    let arm = center * 0.68;
    let mut builder = PathBuilder::new();
    for step in 0..3 {
        let angle = step as f32 * std::f32::consts::PI / 3.0;
        let (dx, dy) = (angle.cos() * arm, angle.sin() * arm);
        builder.move_to(center - dx, center - dy);
        builder.line_to(center + dx, center + dy);
    }
    if let Some(arms) = builder.finish() {
        pixmap.stroke_path(&arms, &paint, &stroke, Transform::identity(), None);
    }

    // tiny-skia stores premultiplied RGBA.
    let mut data = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.data().as_chunks::<4>().0 {
        let alpha = px[3];
        let straight = |c: u8| {
            if alpha == 0 {
                0
            } else {
                (c as u32 * 255 / alpha as u32).min(255) as u8
            }
        };
        data.extend_from_slice(&[straight(px[0]), straight(px[1]), straight(px[2]), alpha]);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_picks_closest_preset() {
        assert_eq!(nearest(INTENSITY, 400), 1);
        assert_eq!(nearest(INTENSITY, 500), 1);
        assert_eq!(nearest(INTENSITY, 700), 2);
        assert_eq!(nearest(INTENSITY, 100_000), INTENSITY.len() - 1);
        assert_eq!(nearest(FPS, 45), 1);
    }

    #[test]
    fn icon_has_the_requested_size_and_is_not_blank() {
        let rgba = snowflake_rgba(32);
        assert_eq!(rgba.len(), 32 * 32 * 4);
        // The center of the disc is opaque, the corner outside it transparent.
        assert_eq!(rgba[(16 * 32 + 16) * 4 + 3], 255);
        assert_eq!(rgba[3], 0);
    }
}
