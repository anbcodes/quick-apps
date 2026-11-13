//! Drawing into a premultiplied ARGB8888 buffer: rounded rects, strokes, masks.

use super::{Color, RectF};
use crate::platform::Rect;

/// Scales all four channels of a packed pixel by `a / 255`.
#[inline(always)]
pub fn mul(c: u32, a: u32) -> u32 {
    let rb = (c & 0x00ff_00ff) * a + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let ag = ((c >> 8) & 0x00ff_00ff) * a + 0x0080_0080;
    let ag = (ag + ((ag >> 8) & 0x00ff_00ff)) & 0xff00_ff00;
    rb | ag
}

/// Source-over with a premultiplied source.
#[inline(always)]
pub fn over(dst: u32, src: u32) -> u32 {
    src + mul(dst, 255 - (src >> 24))
}

#[derive(Clone, Copy, Debug)]
struct Clip {
    /// Integer pixel bounds everything is limited to.
    b: Rect,
    /// The rounded shape inside `b`, when it has corners.
    shape: RectF,
    r: f32,
}

pub struct Canvas<'a> {
    px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
    stride: usize,
    clip: Clip,
    stack: Vec<Clip>,
}

/// Coverage of pixel (px, py) by a rounded rect, 0..=1.
#[inline]
fn rrect_cov(r: &RectF, rad: f32, px: i32, py: i32) -> f32 {
    let (fx, fy) = (px as f32, py as f32);
    let ex = ((fx + 1.0).min(r.x + r.w) - fx.max(r.x)).clamp(0.0, 1.0);
    let ey = ((fy + 1.0).min(r.y + r.h) - fy.max(r.y)).clamp(0.0, 1.0);
    let mut c = ex * ey;
    if rad > 0.0 && c > 0.0 {
        let (cx, cy) = (fx + 0.5, fy + 0.5);
        let qx = if cx < r.x + rad {
            r.x + rad
        } else if cx > r.x + r.w - rad {
            r.x + r.w - rad
        } else {
            return c;
        };
        let qy = if cy < r.y + rad {
            r.y + rad
        } else if cy > r.y + r.h - rad {
            r.y + r.h - rad
        } else {
            return c;
        };
        let d = ((cx - qx) * (cx - qx) + (cy - qy) * (cy - qy)).sqrt() - rad;
        c *= (0.5 - d).clamp(0.0, 1.0);
    }
    c
}

/// The columns [x0, x1) of row `y` that a rounded rect covers fully.
fn full_span(r: &RectF, rad: f32, y: i32) -> (i32, i32) {
    let fy = y as f32;
    if fy < r.y || fy + 1.0 > r.y + r.h {
        return (0, 0);
    }
    let in_corner = fy < r.y + rad || fy + 1.0 > r.y + r.h - rad;
    let inset = if in_corner { rad } else { 0.0 };
    let x0 = (r.x + inset).ceil() as i32;
    let x1 = (r.x + r.w - inset).floor() as i32;
    if x1 > x0 {
        (x0, x1)
    } else {
        (0, 0)
    }
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u32], w: i32, h: i32, stride: usize) -> Canvas<'a> {
        let b = Rect::new(0, 0, w, h);
        Canvas { px, w, h, stride, clip: Clip { b, shape: RectF::default(), r: 0.0 }, stack: Vec::new() }
    }

    /// Resets the clip to `r` (a damage rect). Nothing outside it is touched.
    pub fn set_base_clip(&mut self, r: Rect) {
        self.stack.clear();
        self.clip = Clip { b: r.intersect(&Rect::new(0, 0, self.w, self.h)), shape: RectF::default(), r: 0.0 };
    }

    pub fn push_clip(&mut self, rect: RectF, r: f32) {
        self.stack.push(self.clip);
        let b = rect.outer().intersect(&self.clip.b);
        // Nested rounded clips keep only the innermost corners; good enough for UI.
        let (shape, r) = if r > 0.0 { (rect, r) } else { (self.clip.shape, self.clip.r) };
        self.clip = Clip { b, shape, r };
    }

    pub fn pop_clip(&mut self) {
        if let Some(c) = self.stack.pop() {
            self.clip = c;
        }
    }

    pub fn clip_bounds(&self) -> Rect {
        self.clip.b
    }

    #[inline]
    fn clip_cov(&self, x: i32, y: i32) -> f32 {
        if self.clip.r > 0.0 {
            rrect_cov(&self.clip.shape, self.clip.r, x, y)
        } else {
            1.0
        }
    }

    /// Whether row `y` may need per-pixel clip coverage.
    #[inline]
    fn clip_row_is_plain(&self, y: i32) -> bool {
        let c = &self.clip;
        c.r <= 0.0 || {
            let fy = y as f32;
            fy >= c.shape.y + c.r && fy + 1.0 <= c.shape.y + c.shape.h - c.r
        }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, c: u32, cov: f32) {
        let a = (cov * 255.0 + 0.5) as u32;
        if a == 0 {
            return;
        }
        let i = y as usize * self.stride + x as usize;
        let s = if a >= 255 { c } else { mul(c, a) };
        self.px[i] = over(self.px[i], s);
    }

    fn span(&mut self, y: i32, x0: i32, x1: i32, c: u32) {
        let i = y as usize * self.stride;
        let row = &mut self.px[i + x0 as usize..i + x1 as usize];
        if c >> 24 == 255 {
            row.fill(c);
        } else {
            for p in row {
                *p = over(*p, c);
            }
        }
    }

    pub fn fill(&mut self, rect: RectF, color: Color, rad: f32) {
        let c = color.premul();
        if c == 0 {
            return;
        }
        let rad = rad.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
        let b = rect.outer().intersect(&self.clip.b);
        for y in b.y..b.bottom() {
            let (mut f0, mut f1) = full_span(&rect, rad, y);
            if !self.clip_row_is_plain(y) {
                f0 = 0;
                f1 = 0;
            }
            f0 = f0.clamp(b.x, b.right());
            f1 = f1.clamp(f0, b.right());
            if f1 <= f0 {
                f0 = b.right();
                f1 = b.right();
            }
            for x in b.x..f0 {
                let cov = rrect_cov(&rect, rad, x, y) * self.clip_cov(x, y);
                self.put(x, y, c, cov);
            }
            if f1 > f0 {
                // Fully covered columns may still be under a rounded clip corner.
                if self.clip.r > 0.0 {
                    for x in f0..f1 {
                        let cov = self.clip_cov(x, y);
                        self.put(x, y, c, cov);
                    }
                } else {
                    self.span(y, f0, f1, c);
                }
            }
            for x in f1..b.right() {
                let cov = rrect_cov(&rect, rad, x, y) * self.clip_cov(x, y);
                self.put(x, y, c, cov);
            }
        }
    }

    pub fn stroke(&mut self, rect: RectF, color: Color, rad: f32, width: f32) {
        let c = color.premul();
        if c == 0 || width <= 0.0 {
            return;
        }
        let rad = rad.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
        let inner = RectF::new(rect.x + width, rect.y + width, rect.w - 2.0 * width, rect.h - 2.0 * width);
        if inner.w <= 0.0 || inner.h <= 0.0 {
            return self.fill(rect, color, rad);
        }
        let irad = (rad - width).max(0.0);
        let b = rect.outer().intersect(&self.clip.b);
        for y in b.y..b.bottom() {
            let (s0, s1) = full_span(&inner, irad, y);
            let mut x = b.x;
            while x < b.right() {
                if x >= s0 && x < s1 {
                    x = s1;
                    continue;
                }
                let cov = (rrect_cov(&rect, rad, x, y) - rrect_cov(&inner, irad, x, y)).max(0.0) * self.clip_cov(x, y);
                self.put(x, y, c, cov);
                x += 1;
            }
        }
    }

    /// Blends an A8 coverage mask at integer position (x, y).
    pub fn mask(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8], color: Color) {
        let c = color.premul();
        if c == 0 {
            return;
        }
        let b = Rect::new(x, y, w, h).intersect(&self.clip.b);
        for py in b.y..b.bottom() {
            let src = &data[((py - y) * w) as usize..];
            let plain = self.clip_row_is_plain(py);
            let row = py as usize * self.stride;
            for px in b.x..b.right() {
                let mut a = src[(px - x) as usize] as u32;
                if a == 0 {
                    continue;
                }
                if !plain {
                    a = (a as f32 * self.clip_cov(px, py)) as u32;
                }
                let i = row + px as usize;
                self.px[i] = over(self.px[i], mul(c, a));
            }
        }
    }

    /// Copies premultiplied pixels, scaled with bilinear filtering from `src_rect` into `dst`.
    pub fn image(&mut self, src: &[u32], sw: i32, sh: i32, src_rect: RectF, dst: RectF, smooth: bool) {
        if dst.w <= 0.0 || dst.h <= 0.0 || sw <= 0 || sh <= 0 {
            return;
        }
        let b = dst.outer().intersect(&self.clip.b);
        let sx = src_rect.w / dst.w;
        let sy = src_rect.h / dst.h;
        for py in b.y..b.bottom() {
            let v = src_rect.y + ((py as f32 + 0.5) - dst.y) * sy - 0.5;
            let plain = self.clip_row_is_plain(py);
            let row = py as usize * self.stride;
            for px in b.x..b.right() {
                let u = src_rect.x + ((px as f32 + 0.5) - dst.x) * sx - 0.5;
                let s = if smooth { bilinear(src, sw, sh, u, v) } else { nearest(src, sw, sh, u, v) };
                let mut cov = rrect_cov(&dst, 0.0, px, py);
                if !plain {
                    cov *= self.clip_cov(px, py);
                }
                let a = (cov * 255.0 + 0.5) as u32;
                if a == 0 || s == 0 {
                    continue;
                }
                let i = row + px as usize;
                self.px[i] = over(self.px[i], if a >= 255 { s } else { mul(s, a) });
            }
        }
    }
}

fn nearest(src: &[u32], w: i32, h: i32, u: f32, v: f32) -> u32 {
    let x = ((u + 0.5) as i32).clamp(0, w - 1);
    let y = ((v + 0.5) as i32).clamp(0, h - 1);
    src[(y * w + x) as usize]
}

fn bilinear(src: &[u32], w: i32, h: i32, u: f32, v: f32) -> u32 {
    let u = u.clamp(0.0, (w - 1) as f32);
    let v = v.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (u as i32, v as i32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let fx = ((u - x0 as f32) * 256.0) as u32;
    let fy = ((v - y0 as f32) * 256.0) as u32;
    let p = |x: i32, y: i32| src[(y * w + x) as usize];
    let lerp = |a: u32, b: u32, t: u32| -> u32 {
        let mut out = 0;
        for sh in [0, 8, 16, 24] {
            let ca = (a >> sh) & 255;
            let cb = (b >> sh) & 255;
            let c = (ca * (256 - t) + cb * t) >> 8;
            out |= c << sh;
        }
        out
    };
    lerp(lerp(p(x0, y0), p(x1, y0), fx), lerp(p(x0, y1), p(x1, y1), fx), fy)
}
