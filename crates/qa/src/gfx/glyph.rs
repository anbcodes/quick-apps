//! Glyph rasterizing and an LRU cache of A8 bitmaps.

use super::path::PathRaster;
use crate::text::{FontId, Fonts};
use std::collections::HashMap;
use ttf_parser::{GlyphId, OutlineBuilder};

pub struct GlyphBitmap {
    /// Offset from the pen position to the bitmap's top-left corner.
    pub left: i32,
    pub top: i32,
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct Key {
    font: FontId,
    glyph: u16,
    size: u32,
    sub: u8,
}

pub struct GlyphCache {
    map: HashMap<Key, (GlyphBitmap, u64)>,
    bytes: usize,
    tick: u64,
    raster: PathRaster,
    scratch: Vec<u8>,
    pub budget: usize,
}

struct Builder<'a> {
    r: &'a mut PathRaster,
    s: f32,
    ox: f32,
    oy: f32,
}

impl OutlineBuilder for Builder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.r.move_to(x * self.s + self.ox, self.oy - y * self.s);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.r.line_to(x * self.s + self.ox, self.oy - y * self.s);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (s, ox, oy) = (self.s, self.ox, self.oy);
        self.r.quad_to(x1 * s + ox, oy - y1 * s, x * s + ox, oy - y * s);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (s, ox, oy) = (self.s, self.ox, self.oy);
        self.r.cubic_to(x1 * s + ox, oy - y1 * s, x2 * s + ox, oy - y2 * s, x * s + ox, oy - y * s);
    }
    fn close(&mut self) {
        self.r.close();
    }
}

/// Subpixel x positions per pixel.
pub const SUBPIXELS: f32 = 4.0;

impl GlyphCache {
    pub fn new() -> GlyphCache {
        GlyphCache { map: HashMap::new(), bytes: 0, tick: 0, raster: PathRaster::new(), scratch: Vec::new(), budget: 4 << 20 }
    }

    pub fn get(&mut self, fonts: &Fonts, font: FontId, glyph: u16, size: f32, sub: u8) -> &GlyphBitmap {
        self.tick += 1;
        let key = Key { font, glyph, size: size.to_bits(), sub };
        if !self.map.contains_key(&key) {
            if self.bytes > self.budget {
                self.evict();
            }
            let bm = self.render(fonts, font, glyph, size, sub);
            self.bytes += bm.data.len() + 64;
            self.map.insert(key, (bm, self.tick));
        }
        let e = self.map.get_mut(&key).unwrap();
        e.1 = self.tick;
        &e.0
    }

    fn evict(&mut self) {
        let mut ticks: Vec<u64> = self.map.values().map(|v| v.1).collect();
        ticks.sort_unstable();
        let cut = ticks[ticks.len() / 2];
        self.map.retain(|_, v| v.1 > cut);
        self.bytes = self.map.values().map(|v| v.0.data.len() + 64).sum();
    }

    fn render(&mut self, fonts: &Fonts, font: FontId, glyph: u16, size: f32, sub: u8) -> GlyphBitmap {
        let f = &fonts.fonts[font as usize];
        let empty = GlyphBitmap { left: 0, top: 0, w: 0, h: 0, data: Vec::new() };
        let Some(bb) = f.face.glyph_bounding_box(GlyphId(glyph)) else { return empty };
        let s = size / f.upem;
        let dx = sub as f32 / SUBPIXELS;
        let bold = if f.synth_bold { (size / 30.0).max(0.5) } else { 0.0 };
        let x0 = (bb.x_min as f32 * s + dx).floor() as i32 - 1;
        let x1 = (bb.x_max as f32 * s + dx + bold).ceil() as i32 + 1;
        let y0 = (-(bb.y_max as f32) * s).floor() as i32 - 1;
        let y1 = (-(bb.y_min as f32) * s).ceil() as i32 + 1;
        let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
        if w > 4096 || h > 4096 {
            return empty;
        }
        self.raster.reset(w as usize, h as usize);
        let mut b = Builder { r: &mut self.raster, s, ox: dx - x0 as f32, oy: -y0 as f32 };
        if f.face.outline_glyph(GlyphId(glyph), &mut b).is_none() {
            return empty;
        }
        self.raster.close();
        self.raster.finish(&mut self.scratch);
        let mut data = std::mem::take(&mut self.scratch);
        if bold > 0.0 {
            embolden(&mut data, w as usize, bold);
        }
        GlyphBitmap { left: x0, top: y0, w, h, data }
    }
}

/// Thickens a mask horizontally by `amount` pixels (synthetic bold).
fn embolden(data: &mut [u8], w: usize, amount: f32) {
    let whole = amount.floor() as usize;
    let frac = amount - whole as f32;
    for row in data.chunks_mut(w) {
        let src = row.to_vec();
        for x in 0..w {
            let mut v = src[x] as f32;
            for k in 1..=whole {
                if x >= k {
                    v = v.max(src[x - k] as f32);
                }
            }
            if x > whole {
                v = v.max(src[x - whole - 1] as f32 * frac);
            }
            row[x] = v as u8;
        }
    }
}
