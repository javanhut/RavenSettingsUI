//! Pixels: antialiased rounded rectangles and text into a wl_shm buffer.
//!
//! The same approach as RoostBar's canvas (coverage from a distance field,
//! premultiplied ARGB8888), with strokes and a colour type that carries
//! alpha as a float so a whole group can fade by multiplying one number.

use std::path::PathBuf;

use ab_glyph::{point, Font, FontVec, GlyphId, PxScale, ScaleFont};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8, a: f32) -> Rgba {
        Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    /// `#RRGGBB`.
    pub fn parse_hex(s: &str) -> Option<Rgba> {
        let hex = s.trim().strip_prefix('#')?;
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Rgba::rgb(channel(0)?, channel(2)?, channel(4)?, 1.0))
    }

    pub fn alpha(self, k: f32) -> Rgba {
        Rgba {
            a: self.a * k.clamp(0.0, 1.0),
            ..self
        }
    }

    /// `self` at `t = 0`, `other` at `t = 1`.
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let l = |a: f32, b: f32| a + (b - a) * t;
        Rgba {
            r: l(self.r, other.r),
            g: l(self.g, other.g),
            b: l(self.b, other.b),
            a: l(self.a, other.a),
        }
    }
}

pub struct Canvas<'a> {
    pub buf: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

impl Canvas<'_> {
    pub fn clear(&mut self) {
        self.buf.fill(0);
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Rgba, coverage: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let a = c.a * coverage;
        if a <= 0.0 {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        let dst = &mut self.buf[i..i + 4];
        // wl_shm ARGB8888 is little-endian: B, G, R, A, premultiplied.
        let src = [c.b * a, c.g * a, c.r * a, a];
        for k in 0..4 {
            let d = dst[k] as f32 / 255.0;
            dst[k] = ((src[k] + d * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    pub fn fill_rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba) {
        if w <= 0.0 || h <= 0.0 || c.a <= 0.0 {
            return;
        }
        for py in (y.floor() as i32 - 1)..=((y + h).ceil() as i32) {
            for px in (x.floor() as i32 - 1)..=((x + w).ceil() as i32) {
                let cov = rounded_coverage(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, radius);
                if cov > 0.0 {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }

    /// A line `width` wide just inside the edge of the rounded rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rounded(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        width: f32,
        c: Rgba,
    ) {
        if w <= 0.0 || h <= 0.0 || c.a <= 0.0 {
            return;
        }
        let inner = (radius - width).max(0.0);
        for py in (y.floor() as i32 - 1)..=((y + h).ceil() as i32) {
            for px in (x.floor() as i32 - 1)..=((x + w).ceil() as i32) {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                let outer = rounded_coverage(cx, cy, x, y, w, h, radius);
                let hole = rounded_coverage(
                    cx,
                    cy,
                    x + width,
                    y + width,
                    w - 2.0 * width,
                    h - 2.0 * width,
                    inner,
                );
                let cov = (outer - hole).max(0.0);
                if cov > 0.0 {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
}

/// How much of the pixel centred on `(px, py)` the rounded rectangle covers,
/// from its signed distance, antialiased over one pixel.
fn rounded_coverage(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let qx = (px - (x + w / 2.0)).abs() - (w / 2.0 - r);
    let qy = (py - (y + h / 2.0)).abs() - (h / 2.0 - r);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    (0.5 - (outside + inside - r)).clamp(0.0, 1.0)
}

/// Bold sans for the caps, with fallbacks tried per character so the arrows
/// and the minus sign still draw when the first face lacks them.
pub struct Text {
    faces: Vec<FontVec>,
}

impl Text {
    pub fn load() -> Result<Text, String> {
        let faces: Vec<FontVec> = font_candidates()
            .into_iter()
            .filter_map(|p| std::fs::read(&p).ok())
            .filter_map(|bytes| FontVec::try_from_vec(bytes).ok())
            .collect();
        if faces.is_empty() {
            return Err("no usable font: install a sans-serif font (fc-match sans-serif)".into());
        }
        Ok(Text { faces })
    }

    fn glyph(&self, c: char) -> (usize, GlyphId) {
        for (i, face) in self.faces.iter().enumerate() {
            let id = face.glyph_id(c);
            if id.0 != 0 {
                return (i, id);
            }
        }
        (0, self.faces[0].glyph_id(c))
    }

    pub fn width(&self, s: &str, px: f32) -> f32 {
        let scale = PxScale::from(px);
        s.chars()
            .map(|c| {
                let (face, id) = self.glyph(c);
                self.faces[face].as_scaled(scale).h_advance(id)
            })
            .sum()
    }

    /// Draw `s` from `x`, with its capitals centred on `centre_y`.
    pub fn draw(&self, canvas: &mut Canvas, s: &str, x: f32, centre_y: f32, px: f32, color: Rgba) {
        let scale = PxScale::from(px);
        // Cap height is about 0.71 em in the faces Raven ships, so this puts
        // the middle of a capital, not of the line box, on the centre.
        let baseline = (centre_y + px * 0.355).round();
        let mut pen = x;
        for c in s.chars() {
            let (face, id) = self.glyph(c);
            let font = &self.faces[face];
            let glyph = id.with_scale_and_position(scale, point(pen, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    canvas.blend(
                        b.min.x as i32 + gx as i32,
                        b.min.y as i32 + gy as i32,
                        color,
                        cov,
                    );
                });
            }
            pen += font.as_scaled(scale).h_advance(id);
        }
    }
}

fn font_candidates() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for pattern in [
        "sans-serif:bold",
        "DejaVu Sans:bold",
        "Noto Sans Symbols 2",
        "monospace",
    ] {
        let Ok(out) = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", pattern])
            .output()
        else {
            break;
        };
        let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    for fallback in [
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
    ] {
        let path = PathBuf::from(fallback);
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours() {
        assert_eq!(
            Rgba::parse_hex("#FF8000"),
            Some(Rgba::rgb(255, 128, 0, 1.0))
        );
        assert_eq!(Rgba::parse_hex("FF8000"), None);
        assert_eq!(Rgba::parse_hex("#FFF"), None);
    }

    #[test]
    fn coverage_is_full_inside_and_empty_outside() {
        assert_eq!(rounded_coverage(10.5, 10.5, 0.0, 0.0, 20.0, 20.0, 4.0), 1.0);
        assert_eq!(rounded_coverage(30.5, 10.5, 0.0, 0.0, 20.0, 20.0, 4.0), 0.0);
        // The very corner is cut away by the radius.
        assert_eq!(rounded_coverage(0.5, 0.5, 0.0, 0.0, 20.0, 20.0, 8.0), 0.0);
    }

    #[test]
    fn a_fill_writes_premultiplied_bgra() {
        let mut buf = vec![0u8; 4 * 4 * 4];
        let mut canvas = Canvas {
            buf: &mut buf,
            width: 4,
            height: 4,
        };
        canvas.fill_rounded(0.0, 0.0, 4.0, 4.0, 0.0, Rgba::rgb(255, 0, 0, 0.5));
        assert_eq!(&buf[4 * 5..4 * 5 + 4], &[0, 0, 128, 128]);
    }
}
