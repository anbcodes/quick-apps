//! `qa demo`: a hard-coded display list that exercises the renderer (milestone M0).

use crate::gfx::glyph::GlyphCache;
use crate::gfx::raster::Canvas;
use crate::gfx::{render, Cmd, Color, NoPixmaps, RectF};
use crate::platform::Native;
use crate::platform::{PEvent, Platform, Rect};
use crate::text::{self, FontId, Fonts};

struct DemoFonts {
    sans: FontId,
    bold: FontId,
    mono: FontId,
}

fn text(out: &mut Vec<Cmd>, fonts: &Fonts, f: FontId, size: f32, x: f32, top: f32, color: Color, s: &str) -> f32 {
    let m = fonts.metrics(f, size);
    let mut glyphs = Vec::new();
    let w = fonts.shape(f, size, s, &mut glyphs);
    out.push(Cmd::Glyphs { size, color, x: x.round(), y: (top + m.ascent).round(), glyphs });
    w
}

fn para(out: &mut Vec<Cmd>, fonts: &Fonts, f: FontId, size: f32, x: f32, top: f32, max_w: f32, color: Color, s: &str) -> f32 {
    let lh = (fonts.metrics(f, size).height * 1.35 / 1.164).max(size * 1.2);
    let mut y = top;
    for (a, b) in text::wrap(fonts, f, size, s, max_w) {
        text(out, fonts, f, size, x, y, color, &s[a..b]);
        y += lh;
    }
    y - top
}

fn build(fonts: &Fonts, f: &DemoFonts, w: i32, h: i32, s: f32) -> Vec<Cmd> {
    let c = |v: u32| Color::hex(v);
    let (fg, muted, surface, border, accent) = (c(0x1f2328ff), c(0x656d76ff), c(0xf6f8faff), c(0xd0d7deff), c(0x0969daff));
    let (wf, hf) = (w as f32, h as f32);
    let mut l = Vec::new();
    l.push(Cmd::Fill { rect: RectF::new(0.0, 0.0, wf, hf), color: c(0xffffffff), radius: 0.0 });

    // Header.
    l.push(Cmd::Fill { rect: RectF::new(0.0, 0.0, wf, 56.0 * s), color: surface, radius: 0.0 });
    l.push(Cmd::Fill { rect: RectF::new(0.0, (56.0 * s).round() - s.round(), wf, s.round()), color: border, radius: 0.0 });
    text(&mut l, fonts, f.bold, 22.0 * s, 24.0 * s, 14.0 * s, fg, "Quick Apps");
    let info = format!("{w}×{h} px · scale {s}");
    let iw = fonts.measure(f.sans, 12.0 * s, &info);
    text(&mut l, fonts, f.sans, 12.0 * s, wf - iw - 24.0 * s, 21.0 * s, muted, &info);

    // Card.
    let pad = 16.0 * s;
    let cx = 24.0 * s;
    let cw = (wf - 48.0 * s).min(640.0 * s).max(100.0 * s);
    let cy = 80.0 * s;
    let mut inner = Vec::new();
    let mut y = cy + pad;
    text(&mut inner, fonts, f.bold, 17.0 * s, cx + pad, y, fg, "Milestone 0: pixels");
    y += 30.0 * s;
    y += para(
        &mut inner,
        fonts,
        f.sans,
        14.0 * s,
        cx + pad,
        y,
        cw - 2.0 * pad,
        fg,
        "This window is drawn by a hand-written Wayland client into shared-memory buffers. \
         Rounded rectangles use analytic coverage, glyphs are rasterized from their outlines \
         with a signed-area accumulator, and nothing runs while the window is idle. \
         Resize the window to see the text reflow.",
    );
    y += 12.0 * s;
    let mut x = cx + pad;
    for (sz, label) in [(12.0, "12 px"), (14.0, "14 px"), (17.0, "17 px"), (22.0, "22 px")] {
        let tw = text(&mut inner, fonts, f.sans, sz * s, x, y + (22.0 - sz) * s * 0.8, fg, label);
        x += tw + 20.0 * s;
    }
    y += 40.0 * s;
    text(&mut inner, fonts, f.mono, 13.0 * s, cx + pad, y, c(0x8250dfff), "fn main() { println!(\"hello, world\"); }");
    y += 32.0 * s;

    // Buttons.
    let mut bx = cx + pad;
    for (label, bg, tc, outline) in [
        ("Save", accent, c(0xffffffff), false),
        ("Cancel", surface, fg, true),
        ("Delete", c(0xcf222eff), c(0xffffffff), false),
    ] {
        let tw = fonts.measure(f.sans, 14.0 * s, label);
        let r = RectF::new(bx.round(), y.round(), (tw + 32.0 * s).round(), (32.0 * s).round());
        inner.push(Cmd::Fill { rect: r, color: bg, radius: 6.0 * s });
        if outline {
            inner.push(Cmd::Stroke { rect: r, color: border, radius: 6.0 * s, width: s.round().max(1.0) });
        }
        let m = fonts.metrics(f.sans, 14.0 * s);
        text(&mut inner, fonts, f.sans, 14.0 * s, r.x + 16.0 * s, r.y + (r.h - m.ascent - m.descent) / 2.0, tc, label);
        bx += r.w + 8.0 * s;
    }
    y += 32.0 * s + pad;

    let card = RectF::new(cx.round(), cy.round(), cw.round(), (y - cy).round());
    l.push(Cmd::Fill { rect: card, color: c(0xffffffff), radius: 6.0 * s });
    l.push(Cmd::Stroke { rect: card, color: border, radius: 6.0 * s, width: s.round().max(1.0) });
    l.extend(inner);

    // A rounded clip with stripes and overflowing text.
    let clip = RectF::new(cx, y + 24.0 * s, 260.0 * s, 120.0 * s);
    l.push(Cmd::Clip { rect: clip, radius: 20.0 * s });
    for i in 0..12 {
        let hue = [0x0969daff, 0x1a7f37ff, 0x9a6700ff, 0xcf222eff, 0x8250dfff, 0xbf3989ff][i % 6];
        l.push(Cmd::Fill {
            rect: RectF::new(clip.x - 40.0 * s + i as f32 * 26.0 * s, clip.y, 26.0 * s, clip.h),
            color: c(hue),
            radius: 0.0,
        });
    }
    text(&mut l, fonts, f.bold, 40.0 * s, clip.x + 12.0 * s, clip.y + 34.0 * s, c(0xffffffee), "Clipped text overflows");
    l.push(Cmd::Unclip);

    // Hairlines at fractional offsets.
    let hx = clip.x + clip.w + 24.0 * s;
    for i in 0..8 {
        let off = i as f32 * 0.25;
        l.push(Cmd::Stroke {
            rect: RectF::new(hx + i as f32 * 18.0 * s + off, clip.y + off, 14.0 * s, 120.0 * s),
            color: fg,
            radius: 7.0 * s,
            width: 1.0,
        });
    }
    // Translucent overlap.
    let ox = hx + 160.0 * s;
    l.push(Cmd::Fill { rect: RectF::new(ox, clip.y, 80.0 * s, 80.0 * s), color: c(0x0969da80), radius: 40.0 * s });
    l.push(Cmd::Fill { rect: RectF::new(ox + 40.0 * s, clip.y + 40.0 * s, 80.0 * s, 80.0 * s), color: c(0xcf222e80), radius: 12.0 * s });
    l
}

pub fn run() -> i32 {
    let mut fonts = Fonts::new();
    let Some(sans) = fonts.find_first(text::DEFAULT_SANS) else {
        eprintln!("qa: no font found. Set the font.family token in ~/.config/quick-apps/theme to a .ttf path.");
        return 1;
    };
    let bold = fonts.find_first(text::DEFAULT_SANS_BOLD).unwrap_or_else(|| {
        let id = fonts.fonts.len() as FontId;
        let path = fonts.fonts[sans as usize].path.clone();
        // A second copy of the regular face, drawn thicker.
        if let Ok(b) = fonts.load_path(&path) {
            fonts.fonts[b as usize].synth_bold = b == id;
        }
        sans
    });
    let mono = fonts.find_first(text::DEFAULT_MONO).unwrap_or(sans);
    let f = DemoFonts { sans, bold, mono };

    let mut p = match Native::connect("Quick Apps demo", "qa-demo") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("qa: {e}");
            return 1;
        }
    };
    let debug = std::env::var_os("QA_DEBUG").is_some();
    let mut cache = GlyphCache::new();
    let mut evs = Vec::new();
    let mut dirty = true;
    let mut list = Vec::new();
    let mut list_key = (0, 0, 0.0);
    p.dispatch(&mut evs);
    loop {
        for e in evs.drain(..) {
            match e {
                PEvent::Close => return 0,
                PEvent::Resize { .. } => dirty = true,
                PEvent::Key { sym: 0xff1b, pressed: true, .. } => return 0,
                PEvent::Key { text, pressed: true, .. } if text == "q" => return 0,
                _ => {}
            }
        }
        if dirty {
            let (w, h, s) = p.size();
            if (w, h, s) != list_key {
                list = build(&fonts, &f, w, h, s);
                list_key = (w, h, s);
            }
            let t = std::time::Instant::now();
            if let Some(fr) = p.frame() {
                let full = Rect::new(0, 0, fr.w, fr.h);
                let mut canvas = Canvas::new(fr.px, fr.w, fr.h, fr.stride);
                render(&list, &mut canvas, full, &fonts, &mut cache, &NoPixmaps);
                p.present(&[full]);
                dirty = false;
                if debug {
                    eprintln!("frame {w}x{h} @{s}: {:?}", t.elapsed());
                }
            }
        }
        p.flush();
        if !p.wait(&[], -1) {
            return 1;
        }
        p.dispatch(&mut evs);
    }
}
