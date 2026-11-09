//! Menu bar icon while recording: the logo's bars move like a sound meter, with a red dot in
//! the corner, or a muted mic when the user's mic is muted. Frames are drawn here (44×44 RGBA,
//! @2x of the 22 pt menu bar) instead of shipping a sprite sheet. They aren't template images
//! (the dot has to stay red), so the bars are drawn black or white to match the menu bar.

// Pixel math on a 44 px grid: the casts are lossless here.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]

const N: usize = 44;
const RED: [u8; 3] = [0xd9, 0x3b, 0x30];

#[derive(Clone, Copy)]
pub enum Badge {
    Dot,
    Muted,
}

/// One frame at time `t` (seconds). `dark` picks white bars for a dark menu bar.
pub fn frame(t: f32, badge: Badge, dark: bool) -> Vec<u8> {
    let ink = if dark { [255; 3] } else { [0; 3] };
    // The logo (design-kit/brand/tray-template.svg, viewBox 16 22 68 62) mapped onto 44 px.
    let s = N as f32 / 68.0;
    let at = |u: f32, v: f32| ((u - 16.0) * s, (v - 22.0) * s + (N as f32 - 62.0 * s) / 2.0);
    let bars = [
        (28.0, 46.0),
        (39.0, 32.0),
        (50.0, 26.0),
        (61.0, 36.0),
        (72.0, 48.0),
    ];
    let mut shapes: Vec<(Shape, [u8; 3])> = Vec::new();
    for (i, &(x, top)) in bars.iter().enumerate() {
        // Each bar bobs at its own pace between a short stub and its logo height.
        let i = i as f32;
        let level = 0.5 + 0.5 * (t * (5.0 + 1.7 * i) + i * 1.3).sin() * (t * 2.3 + i).cos();
        let h = 6.0 + (72.0 - top - 6.0) * level.clamp(0.0, 1.0);
        shapes.push((Shape::Line(at(x, 72.0), at(x, 72.0 - h), 4.0 * s), ink));
    }
    shapes.push((Shape::Line(at(20.5, 76.5), at(79.5, 76.5), 4.5 * s), ink));

    // The badge sits top right, with a clear gap cut out of the bars around it.
    let c = (36.5, 7.5);
    let cut = Shape::Dot(c, 8.5);
    let mut badge_shapes = Vec::new();
    match badge {
        Badge::Dot => badge_shapes.push((Shape::Dot(c, 6.0), RED)),
        Badge::Muted => {
            let (x, y) = c;
            badge_shapes.push((Shape::Line((x, y - 3.5), (x, y), 2.4), ink)); // mic body
            badge_shapes.push((Shape::Cup((x, y), 4.6, 0.9), ink)); // holder
            badge_shapes.push((Shape::Line((x, y + 5.0), (x, y + 6.5), 0.9), ink)); // stem
            badge_shapes.push((
                Shape::Line((x - 5.5, y - 5.5), (x + 5.5, y + 5.5), 1.1),
                RED,
            ));
        }
    }

    let mut px = vec![0u8; N * N * 4];
    for row in 0..N {
        for col in 0..N {
            let p = (col as f32 + 0.5, row as f32 + 0.5);
            let mut out = [0.0f32; 4]; // premultiplied
            let gap = 1.0 - cut.cover(p);
            for (shape, rgb) in shapes.iter().map(|(sh, c)| (sh, c)) {
                over(&mut out, *rgb, shape.cover(p) * gap);
            }
            for (shape, rgb) in &badge_shapes {
                over(&mut out, *rgb, shape.cover(p));
            }
            let i = (row * N + col) * 4;
            let a = out[3];
            for k in 0..3 {
                px[i + k] = if a > 0.0 {
                    (out[k] / a).round() as u8
                } else {
                    0
                };
            }
            px[i + 3] = (a * 255.0).round() as u8;
        }
    }
    px
}

pub const SIZE: u32 = N as u32;

fn over(out: &mut [f32; 4], rgb: [u8; 3], a: f32) {
    for k in 0..3 {
        out[k] = f32::from(rgb[k]) * a + out[k] * (1.0 - a);
    }
    out[3] = a + out[3] * (1.0 - a);
}

enum Shape {
    /// Segment with round ends, half-width `r`.
    Line((f32, f32), (f32, f32), f32),
    Dot((f32, f32), f32),
    /// The lower half of a ring (radius, half-width): a mic's holder.
    Cup((f32, f32), f32, f32),
}

impl Shape {
    /// How much of the pixel at `p` the shape covers, 0..1 (one-pixel antialiasing).
    fn cover(&self, p: (f32, f32)) -> f32 {
        let d = match *self {
            Self::Line(a, b, r) => seg(p, a, b) - r,
            Self::Dot(c, r) => dist(p, c) - r,
            Self::Cup(c, r, w) if p.1 >= c.1 => (dist(p, c) - r).abs() - w,
            Self::Cup((cx, cy), r, w) => dist(p, (cx - r, cy)).min(dist(p, (cx + r, cy))) - w,
        };
        (0.5 - d).clamp(0.0, 1.0)
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn seg(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0.0, 1.0)
    };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_have_a_red_dot_or_a_muted_mic() {
        let red = |f: &[u8]| {
            f.chunks(4)
                .filter(|p| p[3] > 200 && p[0] > 200 && p[1] < 80)
                .count()
        };
        let dot = frame(0.0, Badge::Dot, false);
        let muted = frame(0.0, Badge::Muted, true);
        assert_eq!(dot.len(), N * N * 4);
        assert!(red(&dot) > 80, "a filled red dot");
        assert!((5..80).contains(&red(&muted)), "only the slash is red");
        // The bars move.
        assert_ne!(frame(0.0, Badge::Dot, false), frame(0.3, Badge::Dot, false));
    }
}
