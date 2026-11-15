//! The display list and the CPU renderer that draws it.

pub mod glyph;
pub mod path;
pub mod png;
pub mod raster;

use crate::platform::Rect;
use crate::text::{Fonts, PGlyph};
use glyph::{GlyphCache, SUBPIXELS};
use raster::Canvas;

/// A straight-alpha RGBA color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// From 0xRRGGBBAA.
    pub const fn hex(v: u32) -> Color {
        Color { r: (v >> 24) as u8, g: (v >> 16) as u8, b: (v >> 8) as u8, a: v as u8 }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Color> {
        let h = s.strip_prefix('#')?;
        let v = u32::from_str_radix(h, 16).ok()?;
        match h.len() {
            3 => {
                let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
                Some(Color::rgba((r * 17) as u8, (g * 17) as u8, (b * 17) as u8, 255))
            }
            6 => Some(Color::hex((v << 8) | 0xff)),
            8 => Some(Color::hex(v)),
            _ => None,
        }
    }

    pub fn with_alpha(self, a: f32) -> Color {
        Color { a: (self.a as f32 * a).round().clamp(0.0, 255.0) as u8, ..self }
    }

    /// Premultiplied ARGB8888.
    pub fn premul(self) -> u32 {
        let a = self.a as u32;
        let m = |c: u8| (c as u32 * a + 127) / 255;
        (a << 24) | (m(self.r) << 16) | (m(self.g) << 8) | m(self.b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }
    /// The smallest integer rect that contains this one.
    pub fn outer(&self) -> Rect {
        let x0 = self.x.floor() as i32;
        let y0 = self.y.floor() as i32;
        let x1 = (self.x + self.w).ceil() as i32;
        let y1 = (self.y + self.h).ceil() as i32;
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
    pub fn inset(&self, d: f32) -> RectF {
        RectF::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Premultiplied ARGB pixels owned by the runtime (decoded images, canvases).
pub struct Pixmap {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

impl Pixmap {
    /// A square copy at most `max` pixels wide, with the image centred and
    /// scaled down (never up) to fit. Window icons have to be square.
    pub fn fit_square(&self, max: i32) -> Pixmap {
        let side = self.w.max(self.h).clamp(1, max.max(1));
        let mut out = Pixmap { w: side, h: side, px: vec![0; (side * side) as usize] };
        if self.w <= 0 || self.h <= 0 || self.px.len() < (self.w * self.h) as usize {
            return out;
        }
        // Source pixels per output pixel.
        let k = self.w.max(self.h) as f32 / side as f32;
        let (ox, oy) = ((side as f32 - self.w as f32 / k) / 2.0, (side as f32 - self.h as f32 / k) / 2.0);
        for y in 0..side {
            for x in 0..side {
                // Average the block of source pixels this one covers.
                let (x0, x1) = (((x as f32 - ox) * k).floor() as i32, ((x as f32 + 1.0 - ox) * k).ceil() as i32);
                let (y0, y1) = (((y as f32 - oy) * k).floor() as i32, ((y as f32 + 1.0 - oy) * k).ceil() as i32);
                let (mut sum, mut n) = ([0u32; 4], 0u32);
                for sy in y0.max(0)..y1.min(self.h) {
                    for sx in x0.max(0)..x1.min(self.w) {
                        let p = self.px[(sy * self.w + sx) as usize];
                        for (i, s) in sum.iter_mut().enumerate() {
                            *s += (p >> (i * 8)) & 0xff;
                        }
                        n += 1;
                    }
                }
                let n = (((x1 - x0) * (y1 - y0)).max(1) as u32).max(n);
                out.px[(y * side + x) as usize] = (0..4).fold(0, |a, i| a | ((sum[i] / n) << (i * 8)));
            }
        }
        out
    }
}

/// One display-list command. Coordinates are physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Fill { rect: RectF, color: Color, radius: f32 },
    Stroke { rect: RectF, color: Color, radius: f32, width: f32 },
    /// `x`, `y` is the baseline origin; glyph x offsets are relative to it.
    Glyphs { size: f32, color: Color, x: f32, y: f32, glyphs: Vec<PGlyph> },
    Image { id: u32, src: RectF, dst: RectF },
    Pixels { id: u32, dst: RectF },
    Clip { rect: RectF, radius: f32 },
    Unclip,
}

impl Cmd {
    /// Conservative bounds, used to skip commands outside the damage.
    pub fn bounds(&self) -> Option<RectF> {
        match self {
            Cmd::Fill { rect, .. } | Cmd::Stroke { rect, .. } => Some(*rect),
            Cmd::Image { dst, .. } | Cmd::Pixels { dst, .. } => Some(*dst),
            Cmd::Glyphs { size, x, y, glyphs, .. } => {
                let w = glyphs.last().map(|g| g.x).unwrap_or(0.0) + size * 2.0;
                Some(RectF::new(x - size, y - size * 1.5, w + size, size * 3.0))
            }
            Cmd::Clip { .. } | Cmd::Unclip => None,
        }
    }
}

/// Where `Image` and `Pixels` commands find their pixels.
pub trait PixmapSource {
    fn image(&self, id: u32) -> Option<&Pixmap>;
    fn canvas(&self, id: u32) -> Option<&Pixmap>;
}

pub struct NoPixmaps;
impl PixmapSource for NoPixmaps {
    fn image(&self, _: u32) -> Option<&Pixmap> {
        None
    }
    fn canvas(&self, _: u32) -> Option<&Pixmap> {
        None
    }
}

fn overlaps(a: &RectF, b: &Rect) -> bool {
    a.x < b.right() as f32 && a.y < b.bottom() as f32 && a.x + a.w > b.x as f32 && a.y + a.h > b.y as f32
}

/// Draws `list` into `canvas`, touching only pixels inside `damage`.
pub fn render(list: &[Cmd], canvas: &mut Canvas, damage: Rect, fonts: &Fonts, cache: &mut GlyphCache, pix: &dyn PixmapSource) {
    canvas.set_base_clip(damage);
    for cmd in list {
        let clip = canvas.clip_bounds();
        if let Some(b) = cmd.bounds() {
            if clip.is_empty() || !overlaps(&b, &clip) {
                continue;
            }
        }
        match cmd {
            Cmd::Fill { rect, color, radius } => canvas.fill(*rect, *color, *radius),
            Cmd::Stroke { rect, color, radius, width } => canvas.stroke(*rect, *color, *radius, *width),
            Cmd::Glyphs { size, color, x, y, glyphs } => {
                let by = y.round() as i32;
                for g in glyphs {
                    if g.id == crate::text::NO_GLYPH {
                        continue;
                    }
                    let gx = x + g.x;
                    let ix = gx.floor();
                    let color_font = fonts.fonts[g.font as usize].color;
                    // Bitmaps don't get subpixel positions; the cache would
                    // just hold four copies.
                    let sub = if color_font { 0 } else { (((gx - ix) * SUBPIXELS) as u8).min(SUBPIXELS as u8 - 1) };
                    let bm = cache.get(fonts, g.font, g.id, *size, sub);
                    if bm.w > 0 && !bm.rgba.is_empty() {
                        let ix = gx.round() as i32;
                        canvas.pixels(ix + bm.left, by + bm.top, bm.w, bm.h, &bm.rgba, color.premul() >> 24);
                    } else if bm.w > 0 {
                        canvas.mask(ix as i32 + bm.left, by + bm.top, bm.w, bm.h, &bm.data, *color);
                    }
                }
            }
            Cmd::Image { id, src, dst } => {
                if let Some(p) = pix.image(*id) {
                    canvas.image(&p.px, p.w, p.h, *src, *dst, true);
                }
            }
            Cmd::Pixels { id, dst } => {
                if let Some(p) = pix.canvas(*id) {
                    let src = RectF::new(0.0, 0.0, p.w as f32, p.h as f32);
                    canvas.image(&p.px, p.w, p.h, src, *dst, false);
                }
            }
            Cmd::Clip { rect, radius } => canvas.push_clip(*rect, *radius),
            Cmd::Unclip => canvas.pop_clip(),
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn fit_square_pads_and_shrinks() {
        // 4×2 opaque white becomes 2×2 with a transparent band above and below.
        let pm = Pixmap { w: 4, h: 2, px: vec![0xffffffff; 8] };
        let s = pm.fit_square(2);
        assert_eq!((s.w, s.h), (2, 2));
        assert_eq!(s.px, vec![0x7f7f7f7f; 4]);
        // Small images are centred, not scaled up.
        let s = Pixmap { w: 2, h: 2, px: vec![0xff112233; 4] }.fit_square(64);
        assert_eq!((s.w, s.px), (2, vec![0xff112233; 4]));
        let s = Pixmap { w: 4, h: 2, px: vec![0xff112233; 8] }.fit_square(64);
        assert_eq!(s.px[..4], [0; 4]);
        assert_eq!(s.px[4..12], [0xff112233; 8]);
    }
}
