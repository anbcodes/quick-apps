//! Signed-area accumulation rasterizer for glyph outlines.
//!
//! Each line segment adds signed coverage deltas into a buffer; a running sum
//! over the buffer then gives the coverage of every pixel (non-zero winding,
//! clamped). This is the approach popularized by font-rs.

pub struct PathRaster {
    pub w: usize,
    pub h: usize,
    a: Vec<f32>,
    start: (f32, f32),
    last: (f32, f32),
}

impl PathRaster {
    pub fn new() -> PathRaster {
        PathRaster { w: 0, h: 0, a: Vec::new(), start: (0.0, 0.0), last: (0.0, 0.0) }
    }

    pub fn reset(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.a.clear();
        self.a.resize(w * h + 4, 0.0);
        self.start = (0.0, 0.0);
        self.last = (0.0, 0.0);
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        if self.last != self.start {
            self.line(self.last, self.start);
        }
        self.start = (x, y);
        self.last = (x, y);
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.line(self.last, (x, y));
        self.last = (x, y);
    }

    pub fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let ddx = p0.0 - 2.0 * x1 + x;
        let ddy = p0.1 - 2.0 * y1 + y;
        let n = 1 + (3.0 * (ddx * ddx + ddy * ddy)).sqrt().sqrt() as usize;
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let p = (
                mt * mt * p0.0 + 2.0 * mt * t * x1 + t * t * x,
                mt * mt * p0.1 + 2.0 * mt * t * y1 + t * t * y,
            );
            self.line(prev, p);
            prev = p;
        }
        self.last = (x, y);
    }

    pub fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let d1 = (p0.0 - 2.0 * x1 + x2).powi(2) + (p0.1 - 2.0 * y1 + y2).powi(2);
        let d2 = (x1 - 2.0 * x2 + x).powi(2) + (y1 - 2.0 * y2 + y).powi(2);
        let n = 1 + ((3.0 * d1.max(d2)).sqrt().sqrt() * 1.5) as usize;
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
            let p = (a * p0.0 + b * x1 + c * x2 + d * x, a * p0.1 + b * y1 + c * y2 + d * y);
            self.line(prev, p);
            prev = p;
        }
        self.last = (x, y);
    }

    pub fn close(&mut self) {
        if self.last != self.start {
            self.line(self.last, self.start);
        }
        self.last = self.start;
    }

    fn line(&mut self, p0: (f32, f32), p1: (f32, f32)) {
        if (p0.1 - p1.1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        let mut x = p0.0;
        let y0 = p0.1.max(0.0) as usize;
        if p0.1 < 0.0 {
            x -= p0.1 * dxdy;
        }
        let w = self.w;
        let a = &mut self.a;
        let ymax = self.h.min(p1.1.ceil().max(0.0) as usize);
        for y in y0..ymax {
            let ls = y * w;
            let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0f = x0.floor();
            let x0i = x0f as isize;
            let x1c = x1.ceil();
            let x1i = x1c as isize;
            let base = ls as isize + x0i;
            if base < 0 || (ls as isize + x1i) as usize + 1 >= a.len() {
                x = xnext;
                continue;
            }
            let base = base as usize;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x + xnext) - x0f;
                a[base] += d - d * xmf;
                a[base + 1] += d * xmf;
            } else {
                let s = (x1 - x0).recip();
                let x0r = x0 - x0f;
                let a0 = 0.5 * s * (1.0 - x0r) * (1.0 - x0r);
                let x1r = x1 - x1c + 1.0;
                let am = 0.5 * s * x1r * x1r;
                a[base] += d * a0;
                if x1i == x0i + 2 {
                    a[base + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0r);
                    a[base + 1] += d * (a1 - a0);
                    for xi in (x0i + 2)..(x1i - 1) {
                        a[ls + xi as usize] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    a[ls + (x1i - 1) as usize] += d * (1.0 - a2 - am);
                }
                a[ls + x1i as usize] += d * am;
            }
            x = xnext;
        }
    }

    /// Turns the accumulated deltas into an A8 mask of w*h bytes.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.w * self.h);
        let mut acc = 0.0f32;
        for i in 0..self.w * self.h {
            acc += self.a[i];
            let v = acc.abs().min(1.0);
            out.push((v * 255.0 + 0.5) as u8);
        }
    }
}
