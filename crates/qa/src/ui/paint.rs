//! Painting: the arranged tree becomes display-list commands in physical px.

use super::{State, TextStyle, Ui};
use crate::gfx::{Cmd, Color, RectF};
use crate::text::PGlyph;
use qa_proto::{kind, prop, span, Reader};

/// One styled range of a text node (`prop::text::SPANS`).
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub flags: u8,
    pub variant: String,
}

fn align_x(align: Align, x: f32, avail: f32, width: f32) -> f32 {
    match align {
        Align::Start => x,
        Align::Center => x + (avail - width) / 2.0,
        Align::End => x + avail - width,
    }
}

fn overlaps(a: &RectF, b: &RectF) -> bool {
    a.x < b.x + b.w && a.y < b.y + b.h && a.x + a.w > b.x && a.y + a.h > b.y
}

fn intersect(a: &RectF, b: &RectF) -> RectF {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = (a.x + a.w).min(b.x + b.w);
    let bt = (a.y + a.h).min(b.y + b.h);
    RectF::new(x, y, (r - x).max(0.0), (bt - y).max(0.0))
}

pub struct Paint {
    pub list: Vec<Cmd>,
    s: f32,
    /// Logical bounds of the region being repainted.
    dmg: RectF,
    clip: RectF,
    /// 1.0, or 0.5 inside disabled nodes.
    alpha: f32,
}

impl Paint {
    fn snap(&self, r: RectF) -> RectF {
        let s = self.s;
        let x0 = (r.x * s).round();
        let y0 = (r.y * s).round();
        let x1 = ((r.x + r.w) * s).round();
        let y1 = ((r.y + r.h) * s).round();
        RectF::new(x0, y0, x1 - x0, y1 - y0)
    }
    fn col(&self, c: Color) -> Color {
        if self.alpha < 1.0 {
            c.with_alpha(self.alpha)
        } else {
            c
        }
    }
    pub fn fill(&mut self, r: RectF, c: Color, radius: f32) {
        if c.a == 0 {
            return;
        }
        let rect = self.snap(r);
        self.list.push(Cmd::Fill { rect, color: self.col(c), radius: radius * self.s });
    }
    pub fn stroke(&mut self, r: RectF, c: Color, radius: f32, width: f32) {
        if c.a == 0 {
            return;
        }
        let rect = self.snap(r);
        let width = (width * self.s).round().max(1.0);
        self.list.push(Cmd::Stroke { rect, color: self.col(c), radius: radius * self.s, width });
    }
    fn clip(&mut self, r: RectF, radius: f32) -> RectF {
        let old = self.clip;
        self.clip = intersect(&self.clip, &r);
        let rect = self.snap(r);
        self.list.push(Cmd::Clip { rect, radius: radius * self.s });
        old
    }
    fn unclip(&mut self, old: RectF) {
        self.clip = old;
        self.list.push(Cmd::Unclip);
    }
}

pub enum Align {
    Start,
    Center,
    End,
}

impl Ui {
    /// Builds the display list for everything that touches `dmg` (logical).
    pub fn paint(&mut self, dmg: RectF) -> Vec<Cmd> {
        let win = RectF::new(0.0, 0.0, self.win_w, self.win_h);
        let mut p = Paint { list: Vec::new(), s: self.scale, dmg, clip: win, alpha: 1.0 };
        if !self.transparent {
            let bg = self.color(self.root, &["color.bg"]);
            p.fill(win, bg, 0.0);
        }
        if self.root != 0 && self.nodes.contains_key(&self.root) {
            self.paint_node(&mut p, self.root);
        }
        self.paint_popup(&mut p);
        p.list
    }

    /// The client-side title bar: title and a close button. Returns the close button rect.
    pub fn title_bar(&mut self, list: &mut Vec<Cmd>, title: &str, close_hover: bool) -> RectF {
        let win = RectF::new(0.0, 0.0, self.win_w, self.win_h);
        let mut p = Paint { list: Vec::new(), s: self.scale, dmg: win, clip: win, alpha: 1.0 };
        let h = self.top_inset;
        let bar = RectF::new(0.0, 0.0, self.win_w, h);
        let bg = self.color(0, &["titlebar.bg", "color.surface"]);
        p.fill(bar, bg, 0.0);
        let b = self.color(0, &["color.border"]);
        p.fill(RectF::new(0.0, h - 1.0, self.win_w, 1.0), b, 0.0);
        let mut ts = self.text_style(0, "body");
        ts.line_h = h;
        let fg = self.color(0, &["titlebar.fg", "color.fg"]);
        self.draw_text(&mut p, ts, fg, 40.0, 0.0, (self.win_w - 80.0).max(0.0), Align::Center, title, true);
        let close = RectF::new(self.win_w - h, 0.0, h, h - 1.0);
        if close_hover {
            let c = self.color(0, &["color.danger"]);
            p.fill(close, c, 0.0);
        }
        let cc = if close_hover { self.color(0, &["color.danger_fg"]) } else { fg };
        self.draw_text(&mut p, ts, cc, close.x, 0.0, close.w, Align::Center, "×", false);
        list.extend(p.list);
        close
    }

    /// A runtime-owned band across the top of the window (errors, notices).
    pub fn band(&mut self, list: &mut Vec<Cmd>, msg: &str, error: bool) -> RectF {
        let win = RectF::new(0.0, 0.0, self.win_w, self.win_h);
        let mut p = Paint { list: Vec::new(), s: self.scale, dmg: win, clip: win, alpha: 1.0 };
        let ts = self.text_style(0, "body");
        let h = ts.line_h + 16.0;
        let (bg, fg) = if error {
            (self.color(0, &["color.danger"]), self.color(0, &["color.danger_fg"]))
        } else {
            (self.color(0, &["color.surface"]), self.color(0, &["color.fg"]))
        };
        let r = RectF::new(0.0, 0.0, self.win_w, h);
        p.fill(r, bg, 0.0);
        let b = self.color(0, &["color.border"]);
        p.fill(RectF::new(0.0, h - 1.0, self.win_w, 1.0), b, 0.0);
        self.draw_text(&mut p, ts, fg, 12.0, 8.0, self.win_w - 24.0, Align::Start, msg, true);
        list.extend(p.list);
        r
    }

    fn paint_popup(&mut self, p: &mut Paint) {
        let Some((r, row, opts)) = self.popup_geometry() else { return };
        let id = self.open_select;
        let sel = self.nodes[&id].i(prop::select::SELECTED, -1);
        let hl = if self.popup_hl >= 0 { self.popup_hl } else { sel };
        let radius = self.num(id, &["radius"], 6.0);
        let shadow = Color::rgba(0, 0, 0, 30);
        p.fill(RectF::new(r.x, r.y + 2.0, r.w, r.h + 2.0), shadow, radius + 2.0);
        let bg = self.color(id, &["popup.bg", "color.bg"]);
        let border = self.color(id, &["color.border"]);
        p.fill(r, bg, radius);
        p.stroke(r, border, radius, 1.0);
        let ts = self.text_style(id, "body");
        let fg = self.color(id, &["color.fg"]);
        let old = p.clip(r, radius);
        for (i, o) in opts.iter().enumerate() {
            let rr = RectF::new(r.x + 4.0, r.y + 4.0 + i as f32 * row, r.w - 8.0, row);
            if rr.y > r.y + r.h {
                break;
            }
            if i as i32 == hl {
                let c = self.color(id, &["color.selection"]);
                p.fill(rr, c, radius - 2.0);
            }
            self.draw_text(p, ts, fg, rr.x + 8.0, rr.y + 4.0, rr.w - 16.0, Align::Start, o, true);
            if i as i32 == sel {
                let ac = self.color(id, &["color.accent"]);
                self.draw_text(p, ts, ac, rr.x, rr.y + 4.0, rr.w - 8.0, Align::End, "✓", false);
            }
        }
        p.unclip(old);
    }

    /// Draws one line of text. `x`/`top`/`w` are logical; returns the drawn width (logical).
    pub fn draw_text(&mut self, p: &mut Paint, ts: TextStyle, color: Color, x: f32, top: f32, w: f32, align: Align, text: &str, ellipsis: bool) -> f32 {
        if text.is_empty() || color.a == 0 {
            return 0.0;
        }
        let s = self.scale;
        let size = ts.size * s;
        let (glyphs, _, width) = self.shape_line(ts, text, w, ellipsis);
        let x0 = align_x(align, x * s, w * s, width);
        let baseline = self.baseline(ts, top);
        let color = if p.alpha < 1.0 { color.with_alpha(p.alpha) } else { color };
        p.list.push(Cmd::Glyphs { size, color, x: x0.round(), y: baseline.round(), glyphs });
        width / s
    }

    /// Shapes one line at physical size, cutting it with "…" if `ellipsis`
    /// and it's wider than `w`. Returns the glyphs, how many of them come
    /// from `text` (the rest are the ellipsis) and the width, in physical px.
    fn shape_line(&self, ts: TextStyle, text: &str, w: f32, ellipsis: bool) -> (Vec<PGlyph>, usize, f32) {
        let size = ts.size * self.scale;
        let mut glyphs: Vec<PGlyph> = Vec::new();
        let mut width = self.fonts.shape(ts.font, size, text, &mut glyphs);
        let mut keep = glyphs.len();
        let avail = w * self.scale;
        if ellipsis && width > avail + 0.5 {
            let mut eg = Vec::new();
            let ew = self.fonts.shape(ts.font, size, "…", &mut eg);
            // Keep the glyphs that fit before the ellipsis.
            keep = 0;
            for i in 0..glyphs.len() {
                let next = glyphs.get(i + 1).map(|n| n.x).unwrap_or(width);
                if next + ew > avail {
                    break;
                }
                keep = i + 1;
            }
            let end = glyphs.get(keep).map(|g| g.x).unwrap_or(width);
            glyphs.truncate(keep);
            for mut g in eg {
                g.x += end;
                glyphs.push(g);
            }
            width = end + ew;
        }
        (glyphs, keep, width)
    }

    /// The physical baseline y of a line whose top is at logical `top`.
    fn baseline(&self, ts: TextStyle, top: f32) -> f32 {
        let s = self.scale;
        let m = self.fonts.metrics(ts.font, ts.size * s);
        top * s + (ts.line_h * s - (m.ascent + m.descent)) / 2.0 + m.ascent
    }

    /// Like `draw_text`, with the node's spans applied. `start` is the
    /// line's byte offset in the node's text; `line` has the same length as
    /// that part of the text.
    #[allow(clippy::too_many_arguments)]
    fn draw_rich(&mut self, p: &mut Paint, id: u32, ts: TextStyle, color: Color, x: f32, top: f32, w: f32, align: Align, line: &str, start: usize, spans: &[Span], ellipsis: bool) {
        if line.is_empty() {
            return;
        }
        let s = self.scale;
        let size = ts.size * s;
        let (glyphs, keep, width) = self.shape_line(ts, line, w, ellipsis);
        let offs: Vec<usize> = line.char_indices().filter(|(_, c)| *c != '\n' && *c != '\r').map(|(i, _)| start + i).collect();
        let x0 = align_x(align, x * s, w * s, width).round();
        let baseline = self.baseline(ts, top).round();
        let m = self.fonts.metrics(ts.font, size);
        let style = |i: usize| -> (u8, &str) {
            if i >= keep {
                return (0, "");
            }
            let o = offs.get(i).copied().unwrap_or(usize::MAX);
            let mut st = (0u8, "");
            for sp in spans.iter().filter(|sp| sp.start <= o && o < sp.end) {
                st.0 |= sp.flags;
                if !sp.variant.is_empty() {
                    st.1 = &sp.variant;
                }
            }
            st
        };
        let mut i = 0;
        while i < glyphs.len() {
            let st = style(i);
            let mut j = i + 1;
            while j < glyphs.len() && style(j) == st {
                j += 1;
            }
            let gx0 = glyphs[i].x;
            let gx1 = glyphs.get(j).map(|g| g.x).unwrap_or(width);
            let (flags, v) = st;
            let mut c = if !v.is_empty() {
                self.color(id, &[&format!("color.{v}"), "color.fg"])
            } else if flags & span::LINK != 0 {
                self.color(id, &["color.link", "color.accent"])
            } else {
                color
            };
            if p.alpha < 1.0 {
                c = c.with_alpha(p.alpha);
            }
            if flags & span::CODE != 0 {
                let bg = self.color(id, &["color.code_bg", "color.surface_alt", "color.surface"]);
                let pad = 2.0 * s;
                let r = RectF::new((x0 + gx0 - pad) / s, top + 1.0, (gx1 - gx0 + 2.0 * pad) / s, ts.line_h - 2.0);
                p.fill(r, bg, 3.0);
            }
            let mut run: Vec<PGlyph> = glyphs[i..j].iter().map(|g| PGlyph { x: g.x - gx0, ..*g }).collect();
            if flags & span::BOLD != 0 {
                for g in &mut run {
                    if !self.fonts.fonts[g.font as usize].synth_bold {
                        g.font = self.fonts.synth_bold(g.font);
                    }
                }
            }
            p.list.push(Cmd::Glyphs { size, color: c, x: x0 + gx0, y: baseline, glyphs: run });
            if flags & (span::UNDERLINE | span::LINK) != 0 {
                let th = (size / 14.0).round().max(1.0);
                let uy = baseline + (m.descent * 0.4).max(th).round();
                p.list.push(Cmd::Fill { rect: RectF::new(x0 + gx0, uy, gx1 - gx0, th), color: c, radius: 0.0 });
            }
            i = j;
        }
    }

    /// The link span under logical point (x, y) in text node `id`.
    pub fn link_at(&mut self, id: u32, x: f32, y: f32) -> Option<u32> {
        let spans = self.spans(id);
        if !spans.iter().any(|s| s.flags & span::LINK != 0) {
            return None;
        }
        let n = &self.nodes[&id];
        let (r, pad) = (n.rect, n.edges(prop::PAD));
        let style = n.s(prop::text::STYLE).to_string();
        let align = n.i(prop::text::TEXT_ALIGN, 0);
        let ts = self.text_style(id, &style);
        let inner = RectF::new(r.x + pad[3], r.y + pad[0], (r.w - pad[1] - pad[3]).max(0.0), (r.h - pad[0] - pad[2]).max(0.0));
        let lines = self.text_lines_pub(id, inner.w);
        let li = ((y - inner.y) / ts.line_h).floor();
        if li < 0.0 {
            return None;
        }
        let &(a, b) = lines.get(li as usize)?;
        let text = self.nodes[&id].s(prop::text::TEXT);
        let line = &text[a..b];
        let lw = self.fonts.measure(ts.font, ts.size, line);
        let mut cx = match align {
            1 => inner.x + (inner.w - lw) / 2.0,
            2 => inner.x + inner.w - lw,
            _ => inner.x,
        };
        let mut prev = None;
        let mut hit = None;
        for (i, ch) in line.char_indices() {
            if ch == '\n' || ch == '\r' {
                continue;
            }
            let adv = self.fonts.advance(ts.font, ts.size, prev, ch);
            if x >= cx && x < cx + adv {
                hit = Some(a + i);
                break;
            }
            cx += adv;
            prev = Some(ch);
        }
        let o = hit?;
        spans.iter().rposition(|s| s.flags & span::LINK != 0 && s.start <= o && o < s.end).map(|i| i as u32)
    }

    /// A text node's span records.
    pub fn spans(&self, id: u32) -> Vec<Span> {
        let Some(super::Val::B(b)) = self.nodes.get(&id).and_then(|n| n.get(prop::text::SPANS)) else { return Vec::new() };
        let mut r = Reader::new(b);
        let mut out = Vec::new();
        while !r.done() {
            let (Some(start), Some(end), Some(flags), Some(v)) = (r.u32(), r.u32(), r.u8(), r.str()) else { break };
            out.push(Span { start: start as usize, end: end as usize, flags, variant: v.to_string() });
        }
        out
    }

    fn focus_ring(&self, p: &mut Paint, id: u32, r: RectF, radius: f32) {
        if self.focus == id && self.focus_visible {
            let c = self.color(id, &["color.focus"]);
            p.stroke(RectF::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), c, radius + 2.0, 2.0);
        }
    }

    fn paint_node(&mut self, p: &mut Paint, id: u32) {
        let n = &self.nodes[&id];
        if n.hidden() {
            return;
        }
        let r = n.rect;
        let grow = RectF::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0);
        if !overlaps(&grow, &p.dmg) || !overlaps(&grow, &p.clip) {
            return;
        }
        let old_alpha = p.alpha;
        if n.disabled() {
            p.alpha *= 0.5;
        }
        match n.kind {
            kind::GRID => self.paint_grid(p, id),
            kind::TEXT => self.paint_text(p, id),
            kind::BUTTON => self.paint_button(p, id),
            kind::INPUT => self.paint_input(p, id),
            kind::CHECKBOX => self.paint_checkbox(p, id),
            kind::SCROLL => self.paint_scroll(p, id),
            kind::SELECT => self.paint_select(p, id),
            kind::TABS => self.paint_tabs(p, id),
            kind::PROGRESS => self.paint_progress(p, id),
            kind::IMAGE | kind::CANVAS => self.paint_pixels(p, id),
            kind::DIVIDER => {
                let c = self.color(id, &["color.border"]);
                let t = 1.0 / self.scale.max(1.0);
                let line = if r.w >= r.h {
                    RectF::new(r.x, r.y + (r.h - t) / 2.0, r.w, t)
                } else {
                    RectF::new(r.x + (r.w - t) / 2.0, r.y, t, r.h)
                };
                let snapped = p.snap(line);
                let fixed = RectF::new(snapped.x, snapped.y, snapped.w.max(1.0), snapped.h.max(1.0));
                p.list.push(Cmd::Fill { rect: fixed, color: c, radius: 0.0 });
            }
            _ => {}
        }
        p.alpha = old_alpha;
    }

    fn paint_grid(&mut self, p: &mut Paint, id: u32) {
        let n = &self.nodes[&id];
        let r = n.rect;
        let v = n.s(prop::VARIANT).to_string();
        let clickable = n.i(prop::grid::CLICKABLE, 0) != 0;
        let selected = n.i(prop::grid::SELECTED, 0) != 0;
        let clips = n.clips;
        let children = n.children.clone();
        let radius = if v.is_empty() {
            self.num(id, &["radius"], 6.0)
        } else {
            self.num(id, &[&format!("surface.{v}.radius"), "radius"], 6.0)
        };
        if !v.is_empty() {
            if let Some(bg) = self.color_opt(id, &[&format!("surface.{v}.bg")]) {
                p.fill(r, bg, radius);
            }
        }
        if selected {
            let c = self.color(id, &["color.selection"]);
            p.fill(r, c, radius);
        }
        if clickable {
            if self.press == id && self.hover == id {
                let c = self.color(id, &["color.press"]);
                p.fill(r, c, radius);
            } else if self.hover == id {
                let c = self.color(id, &["color.hover"]);
                p.fill(r, c, radius);
            }
        }
        if !v.is_empty() {
            if let Some(b) = self.color_opt(id, &[&format!("surface.{v}.border")]) {
                p.stroke(r, b, radius, 1.0);
            }
        }
        let old = if clips { Some(p.clip(r, 0.0)) } else { None };
        for c in children {
            self.paint_node(p, c);
        }
        if let Some(o) = old {
            p.unclip(o);
        }
        self.focus_ring(p, id, r, radius);
    }

    fn paint_text(&mut self, p: &mut Paint, id: u32) {
        let n = &self.nodes[&id];
        let r = n.rect;
        let pad = n.edges(prop::PAD);
        let style = n.s(prop::text::STYLE).to_string();
        let v = n.s(prop::VARIANT).to_string();
        let wrap = n.i(prop::text::WRAP, 0);
        let align = match n.i(prop::text::TEXT_ALIGN, 0) {
            1 => 1,
            2 => 2,
            _ => 0,
        };
        let max_lines = n.i(prop::text::MAX_LINES, 0);
        let ts = self.text_style(id, &style);
        let color = if v.is_empty() {
            self.color(id, &["color.fg"])
        } else {
            self.color(id, &[&format!("color.{v}"), "color.fg"])
        };
        let inner = RectF::new(r.x + pad[3], r.y + pad[0], (r.w - pad[1] - pad[3]).max(0.0), (r.h - pad[0] - pad[2]).max(0.0));
        let lines = self.text_lines_pub(id, inner.w);
        let text = self.nodes[&id].s(prop::text::TEXT).to_string();
        let spans = self.spans(id);
        let clip = wrap == 2 || inner.h + 0.5 < lines.len() as f32 * ts.line_h;
        let old = if clip { Some(p.clip(r, 0.0)) } else { None };
        let nlines = lines.len();
        for (i, (a, b)) in lines.into_iter().enumerate() {
            let top = inner.y + i as f32 * ts.line_h;
            if top > p.dmg.y + p.dmg.h || top + ts.line_h < p.dmg.y {
                continue;
            }
            let mut s = &text[a..b];
            let owned;
            if wrap != 0 {
                owned = s.replace('\n', " ");
                s = &owned;
            }
            let last_cut = max_lines > 0 && i + 1 == nlines && b == text.len() && wrap == 0;
            let owned2;
            if last_cut {
                owned2 = s.replace('\n', " ");
                s = &owned2;
            }
            let ell = wrap == 1 || last_cut;
            let al = match align {
                1 => Align::Center,
                2 => Align::End,
                _ => Align::Start,
            };
            if spans.is_empty() {
                self.draw_text(p, ts, color, inner.x, top, inner.w, al, s, ell);
            } else {
                self.draw_rich(p, id, ts, color, inner.x, top, inner.w, al, s, a, &spans, ell);
            }
        }
        if let Some(o) = old {
            p.unclip(o);
        }
    }

    pub fn text_lines_pub(&mut self, id: u32, w: f32) -> Vec<(usize, usize)> {
        // Reuse the layout cache.
        let _ = self.height(id, self.nodes[&id].rect.w);
        if let Some((cw, l)) = &self.nodes[&id].m.lines {
            if (*cw - w).abs() < 0.01 {
                return l.clone();
            }
        }
        let n = &self.nodes[&id];
        let style = n.s(prop::text::STYLE).to_string();
        let wrap = n.i(prop::text::WRAP, 0);
        let ts = self.text_style(id, &style);
        let txt = self.nodes[&id].s(prop::text::TEXT);
        if wrap == 0 {
            crate::text::wrap(&self.fonts, ts.font, ts.size, txt, w.max(1.0))
        } else {
            vec![(0, txt.len())]
        }
    }

    fn button_colors(&self, id: u32) -> (Color, Color, Option<Color>) {
        let v = self.variant(id).to_string();
        let k = |s: &str| format!("button.{v}.{s}");
        let (bgk, fgk) = match v.as_str() {
            "primary" => ("color.accent", "color.accent_fg"),
            "danger" => ("color.danger", "color.danger_fg"),
            _ => ("color.surface", "color.fg"),
        };
        let bg = if v == "flat" {
            self.color_opt(id, &[&k("bg")]).unwrap_or(Color::TRANSPARENT)
        } else {
            self.color(id, &[&k("bg"), bgk])
        };
        let fg = self.color(id, &[&k("fg"), fgk]);
        let border = match v.as_str() {
            "" => Some(self.color(id, &[&k("border"), "color.border"])),
            _ => self.color_opt(id, &[&k("border")]),
        };
        (bg, fg, border)
    }

    fn paint_button(&mut self, p: &mut Paint, id: u32) {
        let r = self.nodes[&id].rect;
        let label = self.nodes[&id].s(prop::button::LABEL).to_string();
        let disabled = self.nodes[&id].disabled();
        let (bg, fg, border) = self.button_colors(id);
        let radius = self.num(id, &["radius"], 6.0);
        p.fill(r, bg, radius);
        if !disabled {
            if self.press == id && self.hover == id {
                let c = self.color(id, &["color.press"]);
                p.fill(r, c, radius);
            } else if self.hover == id {
                let c = self.color(id, &["color.hover"]);
                p.fill(r, c, radius);
            }
        }
        if let Some(b) = border {
            p.stroke(r, b, radius, 1.0);
        }
        let ts = self.text_style(id, "body");
        let (px, _) = self.ctl_pad(id);
        let top = r.y + (r.h - ts.line_h) / 2.0;
        self.draw_text(p, ts, fg, r.x + px, top, (r.w - 2.0 * px).max(0.0), Align::Center, &label, true);
        self.focus_ring(p, id, r, radius);
    }

    fn paint_field_box(&mut self, p: &mut Paint, id: u32) -> RectF {
        let r = self.nodes[&id].rect;
        let radius = self.num(id, &["radius"], 6.0);
        let bg = self.color(id, &["input.bg", "color.surface"]);
        let focused = self.focus == id;
        let border = if focused { self.color(id, &["color.focus"]) } else { self.color(id, &["input.border", "color.border"]) };
        p.fill(r, bg, radius);
        p.stroke(r, border, radius, 1.0);
        if focused {
            // A soft outer ring makes focus obvious without moving anything.
            let ring = self.color(id, &["color.focus"]).with_alpha(0.25);
            p.stroke(RectF::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), ring, radius + 2.0, 2.0);
        }
        let (px, py) = self.ctl_pad(id);
        RectF::new(r.x + px, r.y + py, (r.w - 2.0 * px).max(0.0), (r.h - 2.0 * py).max(0.0))
    }

    fn paint_input(&mut self, p: &mut Paint, id: u32) {
        let inner = self.paint_field_box(p, id);
        let ts = self.text_style(id, "body");
        let n = &self.nodes[&id];
        let multiline = n.i(prop::input::MULTILINE, 0) != 0;
        let secret = n.i(prop::input::SECRET, 0) != 0;
        let placeholder = n.s(prop::input::PLACEHOLDER).to_string();
        let focused = self.focus == id;
        let (text, caret, anchor) = match &n.st {
            State::Input(e) => (e.text.clone(), e.caret, e.anchor),
            _ => return,
        };
        let shown: String = if secret { text.chars().map(|c| if c == '\n' { c } else { '•' }).collect() } else { text.clone() };
        // Map byte offsets in `text` to offsets in `shown`.
        let map = |i: usize| -> usize {
            if !secret {
                return i;
            }
            text[..i].chars().map(|c| if c == '\n' { 1 } else { '•'.len_utf8() }).sum()
        };
        let (caret_s, anchor_s) = (map(caret), map(anchor));
        let fg = self.color(id, &["color.fg"]);
        let old = p.clip(RectF::new(inner.x - 1.0, inner.y, inner.w + 2.0, inner.h), 0.0);
        if text.is_empty() && !placeholder.is_empty() {
            let c = self.color(id, &["color.placeholder", "color.muted"]);
            self.draw_text(p, ts, c, inner.x, inner.y, inner.w, Align::Start, &placeholder, true);
        }
        let lines: Vec<(usize, usize)> = if multiline { crate::text::wrap(&self.fonts, ts.font, ts.size, &shown, inner.w.max(1.0)) } else { vec![(0, shown.len())] };
        // Which line holds the caret.
        let cl = lines.iter().rposition(|&(a, _)| a <= caret_s).unwrap_or(0);
        let stops_for = |ui: &Ui, a: usize, b: usize| ui.fonts.caret_stops(ts.font, ts.size, &shown[a..b]);
        let caret_x = {
            let (a, b) = lines[cl];
            let stops = stops_for(self, a, b);
            let idx = shown[a..caret_s.min(b).max(a)].chars().count();
            stops.get(idx).copied().unwrap_or(0.0)
        };
        // Keep the caret visible.
        let (sx, sy) = {
            let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
            if focused {
                if multiline {
                    let cy = cl as f32 * ts.line_h;
                    if cy < e.scroll_y {
                        e.scroll_y = cy;
                    } else if cy + ts.line_h > e.scroll_y + inner.h {
                        e.scroll_y = cy + ts.line_h - inner.h;
                    }
                } else {
                    if caret_x - e.scroll_x > inner.w {
                        e.scroll_x = caret_x - inner.w;
                    } else if caret_x < e.scroll_x {
                        e.scroll_x = caret_x;
                    }
                }
            }
            if !multiline {
                let full = self.fonts.measure(ts.font, ts.size, &shown);
                let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
                e.scroll_x = e.scroll_x.min((full - inner.w + 1.0).max(0.0));
            }
            let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
            (e.scroll_x, e.scroll_y)
        };
        let (s0, s1) = (caret_s.min(anchor_s), caret_s.max(anchor_s));
        let sel_c = self.color(id, &["color.selection"]);
        for (i, &(a, b)) in lines.iter().enumerate() {
            let top = inner.y + i as f32 * ts.line_h - sy;
            if top + ts.line_h < inner.y || top > inner.y + inner.h {
                continue;
            }
            if focused && s1 > s0 && s0 <= b && s1 >= a {
                let stops = stops_for(self, a, b);
                let ia = shown[a..s0.max(a)].chars().count();
                let ib = shown[a..s1.min(b)].chars().count();
                let xa = stops.get(ia).copied().unwrap_or(0.0);
                let mut xb = stops.get(ib).copied().unwrap_or(0.0);
                if s1 > b {
                    xb += ts.size * 0.3;
                }
                p.fill(RectF::new(inner.x - sx + xa, top, xb - xa, ts.line_h), sel_c, 0.0);
            }
            let line = shown[a..b].trim_end_matches('\n').to_string();
            self.draw_text(p, ts, fg, inner.x - sx, top, f32::INFINITY, Align::Start, &line, false);
        }
        if focused {
            let top = inner.y + cl as f32 * ts.line_h - sy;
            let cx = inner.x - sx + caret_x;
            let w = 1.0f32.max(1.0 / self.scale).max((self.scale).round() / self.scale);
            let cc = self.color(id, &["color.caret", "color.fg"]);
            p.list.push(Cmd::Fill {
                rect: RectF::new((cx * self.scale).round(), (top * self.scale).round(), (w * self.scale).round().max(1.0), (ts.line_h * self.scale).round()),
                color: cc,
                radius: 0.0,
            });
        }
        p.unclip(old);
    }

    fn paint_checkbox(&mut self, p: &mut Paint, id: u32) {
        let r = self.nodes[&id].rect;
        let checked = self.nodes[&id].i(prop::checkbox::CHECKED, 0) != 0;
        let label = self.nodes[&id].s(prop::checkbox::LABEL).to_string();
        let ts = self.text_style(id, "body");
        let bs = (ts.size + 2.0).round();
        let b = RectF::new(r.x + 1.0, r.y + ((r.h - bs) / 2.0).round(), bs, bs);
        let radius = (self.num(id, &["radius"], 6.0) * 0.6).min(bs / 3.0);
        if checked {
            let c = self.color(id, &["color.accent"]);
            p.fill(b, c, radius);
            let mark = self.color(id, &["color.accent_fg"]);
            let mts = TextStyle { size: ts.size * 0.95, line_h: bs, ..ts };
            self.draw_text(p, mts, mark, b.x, b.y, b.w, Align::Center, "✓", false);
        } else {
            let bg = self.color(id, &["input.bg", "color.surface"]);
            p.fill(b, bg, radius);
            let c = self.color(id, &["color.border"]);
            p.stroke(b, c, radius, 1.0);
        }
        if self.hover == id && !self.nodes[&id].disabled() {
            let c = self.color(id, &["color.hover"]);
            p.fill(b, c, radius);
        }
        if !label.is_empty() {
            let fg = self.color(id, &["color.fg"]);
            let x = b.x + bs + 8.0;
            self.draw_text(p, ts, fg, x, r.y + (r.h - ts.line_h) / 2.0, (r.x + r.w - x).max(0.0), Align::Start, &label, true);
        }
        self.focus_ring(p, id, b, radius);
    }

    fn paint_scroll(&mut self, p: &mut Paint, id: u32) {
        let r = self.nodes[&id].rect;
        let child = self.nodes[&id].children.first().copied();
        let old = p.clip(r, 0.0);
        if let Some(c) = child {
            self.paint_node(p, c);
        }
        p.unclip(old);
        let (sx, sy, cw, ch) = match &self.nodes[&id].st {
            State::Scroll(s) => (s.x, s.y, s.content_w, s.content_h),
            _ => return,
        };
        let inside = r.contains(self.ptr.0, self.ptr.1);
        if inside {
            let c = self.color(id, &["color.scrollbar", "color.border"]);
            if ch > r.h + 0.5 {
                let th = (r.h * r.h / ch).max(24.0).min(r.h);
                let ty = r.y + (r.h - th) * (sy / (ch - r.h));
                p.fill(RectF::new(r.x + r.w - 7.0, ty + 2.0, 5.0, th - 4.0), c, 2.5);
            }
            if cw > r.w + 0.5 {
                let tw = (r.w * r.w / cw).max(24.0).min(r.w);
                let tx = r.x + (r.w - tw) * (sx / (cw - r.w));
                p.fill(RectF::new(tx + 2.0, r.y + r.h - 7.0, tw - 4.0, 5.0), c, 2.5);
            }
        }
    }

    fn paint_select(&mut self, p: &mut Paint, id: u32) {
        let inner = self.paint_field_box(p, id);
        let ts = self.text_style(id, "body");
        let n = &self.nodes[&id];
        let sel = n.i(prop::select::SELECTED, -1);
        let opts: Vec<String> = n.s(prop::select::OPTIONS).split('\n').map(|s| s.to_string()).collect();
        let ph = n.s(prop::select::PLACEHOLDER).to_string();
        let (txt, c) = match opts.get(sel.max(-1) as usize).filter(|_| sel >= 0) {
            Some(o) => (o.clone(), self.color(id, &["color.fg"])),
            None => (ph, self.color(id, &["color.placeholder", "color.muted"])),
        };
        let arrow_w = ts.size;
        self.draw_text(p, ts, c, inner.x, inner.y, inner.w - arrow_w, Align::Start, &txt, true);
        let mc = self.color(id, &["color.muted"]);
        self.draw_text(p, ts, mc, inner.x + inner.w - arrow_w, inner.y, arrow_w, Align::End, "▾", false);
        if self.hover == id && !self.nodes[&id].disabled() {
            let r = self.nodes[&id].rect;
            let c = self.color(id, &["color.hover"]);
            let radius = self.num(id, &["radius"], 6.0);
            p.fill(r, c, radius);
        }
    }

    /// Tab label rects (logical) for hit testing and painting.
    pub fn tab_rects(&mut self, id: u32) -> Vec<RectF> {
        let r = self.nodes[&id].rect;
        let ts = self.text_style(id, "body");
        let (px, _) = self.ctl_pad(id);
        let labels = self.nodes[&id].s(prop::tabs::LABELS).to_string();
        let mut x = r.x;
        let mut out = Vec::new();
        for l in labels.split('\n') {
            let w = self.fonts.measure(ts.font, ts.size, l) + 2.0 * px;
            out.push(RectF::new(x, r.y, w, r.h));
            x += w;
        }
        out
    }

    fn paint_tabs(&mut self, p: &mut Paint, id: u32) {
        let r = self.nodes[&id].rect;
        let sel = self.nodes[&id].i(prop::tabs::SELECTED, 0);
        let labels: Vec<String> = self.nodes[&id].s(prop::tabs::LABELS).split('\n').map(|s| s.to_string()).collect();
        let rects = self.tab_rects(id);
        let ts = self.text_style(id, "body");
        let border = self.color(id, &["color.border"]);
        p.fill(RectF::new(r.x, r.y + r.h - 1.0, r.w, 1.0), border, 0.0);
        let accent = self.color(id, &["color.accent"]);
        let muted = self.color(id, &["color.muted"]);
        let fg = self.color(id, &["color.fg"]);
        let old = p.clip(r, 0.0);
        for (i, (tr, l)) in rects.iter().zip(&labels).enumerate() {
            let active = i as i32 == sel;
            let c = if active { fg } else { muted };
            self.draw_text(p, ts, c, tr.x, tr.y + (tr.h - 2.0 - ts.line_h) / 2.0, tr.w, Align::Center, l, false);
            if active {
                p.fill(RectF::new(tr.x, tr.y + tr.h - 2.0, tr.w, 2.0), accent, 1.0);
            }
        }
        p.unclip(old);
        if self.focus == id && self.focus_visible {
            if let Some(tr) = rects.get(sel.max(0) as usize).copied() {
                self.focus_ring(p, id, tr, 4.0);
            }
        }
    }

    fn paint_pixels(&mut self, p: &mut Paint, id: u32) {
        let n = &self.nodes[&id];
        let r = n.rect;
        let (iw, ih, fit) = if n.kind == kind::IMAGE {
            let (w, h) = self.image_size(id);
            (w, h, n.i(prop::image::FIT, 0))
        } else {
            (n.i(prop::canvas::PX_W, 0) as f32, n.i(prop::canvas::PX_H, 0) as f32, n.i(prop::canvas::FIT, 0))
        };
        if iw <= 0.0 || ih <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let dst = match fit {
            1 => {
                let s = (r.w / iw).max(r.h / ih);
                RectF::new(r.x + (r.w - iw * s) / 2.0, r.y + (r.h - ih * s) / 2.0, iw * s, ih * s)
            }
            2 => r,
            3 => RectF::new(r.x, r.y, iw, ih),
            _ => {
                let s = (r.w / iw).min(r.h / ih);
                RectF::new(r.x + (r.w - iw * s) / 2.0, r.y + (r.h - ih * s) / 2.0, iw * s, ih * s)
            }
        };
        let clip = fit == 1 || fit == 3;
        let old = if clip { Some(p.clip(r, 0.0)) } else { None };
        let s = self.scale;
        let d = RectF::new(dst.x * s, dst.y * s, dst.w * s, dst.h * s);
        if n.kind == kind::IMAGE {
            p.list.push(Cmd::Image { id, src: RectF::new(0.0, 0.0, iw, ih), dst: d });
        } else {
            p.list.push(Cmd::Pixels { id, dst: d });
        }
        if let Some(o) = old {
            p.unclip(o);
        }
        self.focus_ring(p, id, r, 0.0);
    }

    fn paint_progress(&mut self, p: &mut Paint, id: u32) {
        let r = self.nodes[&id].rect;
        let v = self.nodes[&id].f(prop::progress::VALUE, 0.0);
        let h = r.h.min(6.0);
        let track = RectF::new(r.x, r.y + (r.h - h) / 2.0, r.w, h);
        let tc = self.color(id, &["color.surface"]);
        let bc = self.color(id, &["color.accent"]);
        let bd = self.color(id, &["color.border"]);
        p.fill(track, tc, h / 2.0);
        p.stroke(track, bd.with_alpha(0.6), h / 2.0, 1.0);
        if v < 0.0 {
            self.animated.push(id);
            let t = (self.anim_ms % 1500) as f32 / 1500.0;
            let w = track.w * 0.3;
            let x = track.x - w + (track.w + w) * t;
            let old = p.clip(track, h / 2.0);
            p.fill(RectF::new(x, track.y, w, h), bc, h / 2.0);
            p.unclip(old);
        } else if v > 0.0 {
            p.fill(RectF::new(track.x, track.y, track.w * v.min(1.0), h), bc, h / 2.0);
        }
    }
}
