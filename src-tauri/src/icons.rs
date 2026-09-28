//! The menu bar icon: the Viary mark, five rounded bars, drawn at run time
//! so each state can carry its badge.

/// The mark's bars in its 48-unit box: x, y, height. Width 6, radius 3.
const BARS: [(f32, f32, f32); 5] = [
    (2.0, 16.0, 16.0),
    (11.5, 11.0, 26.0),
    (21.0, 5.0, 38.0),
    (30.5, 11.0, 26.0),
    (40.0, 16.0, 16.0),
];

/// Distance from `(px, py)` to a rounded rectangle, negative inside.
fn rounded_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let qx = (px - cx).abs() - (w / 2.0 - r);
    let qy = (py - cy).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// An RGBA image `size` pixels square: the mark in `ink`, plus a dot in
/// `badge` at the top right.
pub fn tray(size: u32, ink: [u8; 3], badge: Option<[u8; 3]>) -> Vec<u8> {
    let scale = size as f32 / 48.0;
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    // The dot and the ring cut around it, in mark units.
    let (bx, by, br, gap) = (40.0, 8.0, 8.0, 3.5);
    for py in 0..size {
        for px in 0..size {
            let mut cover_mark: f32 = 0.0;
            let mut cover_badge: f32 = 0.0;
            // 4x4 supersampling for smooth edges at 18–22 px.
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (px as f32 + (sx as f32 + 0.5) / 4.0) / scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / 4.0) / scale;
                    let to_badge = ((x - bx).powi(2) + (y - by).powi(2)).sqrt();
                    if badge.is_some() && to_badge <= br {
                        cover_badge += 1.0 / 16.0;
                        continue;
                    }
                    if badge.is_some() && to_badge <= br + gap {
                        continue;
                    }
                    if BARS
                        .iter()
                        .any(|&(bx, by, bh)| rounded_rect(x, y, bx, by, 6.0, bh, 3.0) <= 0.0)
                    {
                        cover_mark += 1.0 / 16.0;
                    }
                }
            }
            let i = ((py * size + px) * 4) as usize;
            let (color, cover) = if cover_badge > 0.0 {
                (badge.unwrap_or(ink), cover_badge + cover_mark)
            } else {
                (ink, cover_mark)
            };
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (cover.min(1.0) * 255.0).round() as u8;
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_bar_is_solid_and_the_corners_clear() {
        let size = 48;
        let image = tray(size, [0, 0, 0], None);
        let alpha = |x: u32, y: u32| image[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(24, 24), 255);
        assert_eq!(alpha(0, 0), 0);
        let badged = tray(size, [0, 0, 0], Some([255, 0, 0]));
        assert_eq!(
            badged[((8 * size + 40) * 4) as usize],
            255,
            "the badge is red"
        );
    }
}
