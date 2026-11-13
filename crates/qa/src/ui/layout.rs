//! Measuring and arranging: grid tracks with priorities (docs/layout.md).

use super::{State, Ui, Val};
use crate::gfx::RectF;
use qa_proto::{kind, prop, track, Track};

const INF: f32 = f32::INFINITY;

/// A child's demand on the tracks of one axis.
#[derive(Clone, Copy, Debug)]
pub struct TCell {
    pub start: usize,
    pub span: usize,
    pub min: f32,
    pub pref: f32,
    pub prio: i32,
}

#[derive(Debug, Clone)]
pub struct Sized {
    pub sizes: Vec<f32>,
    pub hidden: Vec<bool>,
}

impl Sized {
    pub fn total(&self, gap: f32) -> f32 {
        let vis = self.hidden.iter().filter(|h| !**h).count();
        self.sizes.iter().zip(&self.hidden).filter(|(_, h)| !**h).map(|(s, _)| *s).sum::<f32>() + gap * vis.saturating_sub(1) as f32
    }
    /// Start offsets; hidden tracks take no gap.
    pub fn offsets(&self, gap: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.sizes.len());
        let mut x = 0.0;
        let mut first = true;
        for (s, h) in self.sizes.iter().zip(&self.hidden) {
            if *h {
                out.push(x);
                continue;
            }
            if !first {
                x += gap;
            }
            first = false;
            out.push(x);
            x += s;
        }
        out
    }
    /// Extent of a span, or None when any of its tracks is hidden.
    pub fn span(&self, start: usize, span: usize, gap: f32) -> Option<f32> {
        let end = (start + span).min(self.sizes.len());
        if (start..end).any(|i| self.hidden[i]) {
            return None;
        }
        Some(self.sizes[start..end].iter().sum::<f32>() + gap * (end - start).saturating_sub(1) as f32)
    }
}

fn clamp_track(t: &Track, v: f32) -> f32 {
    let mut v = v;
    if t.max.is_finite() {
        v = v.min(t.max);
    }
    if t.min.is_finite() {
        v = v.max(t.min);
    }
    v
}

/// Sizes `n` tracks along one axis (docs/layout.md, "Sizing tracks along one axis").
pub fn size_tracks(specs: &[Track], n: usize, cells: &[TCell], avail: f32, gap: f32) -> Sized {
    let spec = |i: usize| specs.get(i).copied().unwrap_or(Track::auto());
    let mut mins = vec![0.0f32; n];
    let mut prefs = vec![0.0f32; n];
    let mut prio: Vec<Option<i32>> = vec![None; n];
    for c in cells {
        if c.start >= n {
            continue;
        }
        let p = &mut prio[c.start];
        *p = Some(p.map_or(c.prio, |q| q.max(c.prio)));
        if c.span == 1 {
            mins[c.start] = mins[c.start].max(c.min);
            prefs[c.start] = prefs[c.start].max(c.pref);
        }
    }
    let prio: Vec<i32> = prio.into_iter().map(|p| p.unwrap_or(0)).collect();
    let infinite = !avail.is_finite();
    let mut size = vec![0.0f32; n];
    let mut floor = vec![0.0f32; n];
    for i in 0..n {
        let t = spec(i);
        match t.kind {
            track::PX => {
                size[i] = t.value;
                floor[i] = if t.min.is_finite() { t.min.min(t.value) } else { t.value };
            }
            track::FR => {
                size[i] = clamp_track(&t, if infinite { prefs[i] } else { mins[i] });
                floor[i] = clamp_track(&t, mins[i]).min(size[i]);
            }
            _ => {
                size[i] = clamp_track(&t, prefs[i]);
                floor[i] = clamp_track(&t, mins[i]).min(size[i]);
            }
        }
    }
    // Spanning cells that still need room grow the auto and fr tracks they cover.
    for c in cells.iter().filter(|c| c.span > 1 && c.start < n) {
        let end = (c.start + c.span).min(n);
        let have: f32 = size[c.start..end].iter().sum::<f32>() + gap * (end - c.start - 1) as f32;
        let need = if infinite { c.pref } else { c.min };
        if need > have {
            let flex: Vec<usize> = (c.start..end).filter(|&i| spec(i).kind != track::PX).collect();
            let targets: Vec<usize> = if flex.is_empty() { (c.start..end).collect() } else { flex };
            let each = (need - have) / targets.len() as f32;
            for i in targets {
                size[i] += each;
                floor[i] = floor[i].max(size[i].min(if infinite { size[i] } else { floor[i] + each }));
            }
        }
    }
    let mut hidden = vec![false; n];
    if infinite || n == 0 {
        return Sized { sizes: size, hidden };
    }
    let total: f32 = size.iter().sum::<f32>() + gap * (n - 1) as f32;
    if total <= avail {
        // Surplus goes to fr tracks, freezing those that hit their base or max.
        let mut frozen: Vec<bool> = (0..n).map(|i| spec(i).kind != track::FR).collect();
        let mut fr_size = size.clone();
        loop {
            let flex: f32 = (0..n).filter(|&i| !frozen[i]).map(|i| spec(i).value.max(0.0)).sum();
            if flex <= 0.0 {
                break;
            }
            let used: f32 = (0..n).filter(|&i| frozen[i]).map(|i| fr_size[i]).sum::<f32>() + gap * (n - 1) as f32;
            let hyp = (avail - used).max(0.0) / flex.max(1.0).max(flex);
            let mut changed = false;
            for i in 0..n {
                if frozen[i] {
                    continue;
                }
                let t = spec(i);
                let target = t.value.max(0.0) * hyp;
                if target < size[i] {
                    fr_size[i] = size[i];
                    frozen[i] = true;
                    changed = true;
                } else if t.max.is_finite() && target > t.max {
                    fr_size[i] = t.max.max(size[i]);
                    frozen[i] = true;
                    changed = true;
                }
            }
            if !changed {
                for i in 0..n {
                    if !frozen[i] {
                        fr_size[i] = spec(i).value.max(0.0) * hyp;
                    }
                }
                break;
            }
        }
        return Sized { sizes: fr_size, hidden };
    }
    // Shortage: lowest priority gives up everything first.
    let mut short = total - avail;
    let mut levels: Vec<i32> = prio.clone();
    levels.sort_unstable();
    levels.dedup();
    for lv in levels {
        let idx: Vec<usize> = (0..n).filter(|&i| prio[i] == lv && !hidden[i]).collect();
        let room: f32 = idx.iter().map(|&i| (size[i] - floor[i]).max(0.0)).sum();
        if room > 0.0 {
            let take = short.min(room);
            for &i in &idx {
                size[i] -= (size[i] - floor[i]).max(0.0) / room * take;
            }
            short -= take;
        }
        if short > 0.01 && lv < 0 {
            for &i in idx.iter().rev() {
                let vis = hidden.iter().filter(|h| !**h).count();
                hidden[i] = true;
                short -= size[i] + if vis > 1 { gap } else { 0.0 };
                size[i] = 0.0;
                if short <= 0.01 {
                    break;
                }
            }
        }
        if short <= 0.01 {
            break;
        }
    }
    if short < -0.01 && hidden.iter().any(|h| *h) {
        // Hiding freed more than needed: size the remaining tracks again so fr tracks grow back.
        let keep: Vec<usize> = (0..n).filter(|&i| !hidden[i]).collect();
        let sub_specs: Vec<Track> = keep.iter().map(|&i| spec(i)).collect();
        let sub_cells: Vec<TCell> = cells
            .iter()
            .filter_map(|c| {
                let end = (c.start + c.span).min(n);
                if (c.start..end).any(|i| hidden[i]) {
                    return None;
                }
                let start = keep.iter().position(|&k| k == c.start)?;
                Some(TCell { start, ..*c })
            })
            .collect();
        let sub = size_tracks(&sub_specs, keep.len(), &sub_cells, avail, gap);
        for (j, &i) in keep.iter().enumerate() {
            size[i] = sub.sizes[j];
            hidden[i] = sub.hidden[j];
        }
    }
    Sized { sizes: size, hidden }
}

struct Cell {
    id: u32,
    r: usize,
    c: usize,
    rs: usize,
    cs: usize,
}

impl Ui {
    fn pad(&self, id: u32) -> [f32; 4] {
        self.nodes[&id].edges(prop::PAD)
    }

    fn clamp_w(&self, id: u32, v: f32) -> f32 {
        let n = &self.nodes[&id];
        v.min(n.f(prop::MAX_W, INF)).max(n.f(prop::MIN_W, 0.0))
    }

    fn clamp_h(&self, id: u32, v: f32) -> f32 {
        let n = &self.nodes[&id];
        v.min(n.f(prop::MAX_H, INF)).max(n.f(prop::MIN_H, 0.0))
    }

    pub fn ctl_pad(&self, id: u32) -> (f32, f32) {
        let p = self.num(id, &["space.pad"], 8.0);
        (p * 1.5, p * 0.75)
    }

    fn tracks(&self, id: u32, p: u16) -> Vec<Track> {
        match self.nodes[&id].get(p) {
            Some(Val::T(t)) => t.clone(),
            _ => Vec::new(),
        }
    }

    pub fn gap(&self, id: u32) -> f32 {
        let n = &self.nodes[&id];
        match n.get(prop::grid::GAP).and_then(|v| v.as_f32()) {
            Some(g) => g,
            None => self.num(id, &["space.gap"], 8.0),
        }
    }

    fn place(&self, id: u32) -> (Vec<Cell>, usize, usize) {
        let n = &self.nodes[&id];
        let ncols = match n.get(prop::grid::COLS) {
            Some(Val::T(t)) if !t.is_empty() => t.len(),
            _ => 1,
        };
        let mut occ: Vec<Vec<bool>> = Vec::new();
        let free = |occ: &Vec<Vec<bool>>, r: usize, c: usize, rs: usize, cs: usize| {
            (r..r + rs).all(|rr| (c..c + cs).all(|cc| occ.get(rr).map_or(true, |row| !row[cc])))
        };
        let mut cells = Vec::new();
        let (mut cr, mut cc) = (0usize, 0usize);
        for &ch in &n.children {
            let cn = &self.nodes[&ch];
            if cn.i(prop::HIDDEN, 0) != 0 {
                continue;
            }
            let row = cn.i(prop::ROW, -1);
            let col = cn.i(prop::COL, -1);
            let rs = cn.i(prop::ROW_SPAN, 1).max(1) as usize;
            let mut cs = (cn.i(prop::COL_SPAN, 1).max(1) as usize).min(ncols);
            let (r, c) = if row >= 0 && col >= 0 {
                let c = (col as usize).min(ncols - 1);
                cs = cs.min(ncols - c);
                (row as usize, c)
            } else if col >= 0 {
                let c = (col as usize).min(ncols - 1);
                cs = cs.min(ncols - c);
                let mut r = 0;
                while !free(&occ, r, c, rs, cs) {
                    r += 1;
                }
                (r, c)
            } else if row >= 0 {
                let r = row as usize;
                let mut c = 0;
                while c + cs <= ncols && !free(&occ, r, c, rs, cs) {
                    c += 1;
                }
                (r, c.min(ncols - cs))
            } else {
                loop {
                    if cc + cs > ncols {
                        cc = 0;
                        cr += 1;
                    }
                    if free(&occ, cr, cc, rs, cs) {
                        break;
                    }
                    cc += 1;
                }
                let p = (cr, cc);
                cc += cs;
                p
            };
            while occ.len() < r + rs {
                occ.push(vec![false; ncols]);
            }
            for rr in r..r + rs {
                for c2 in c..c + cs {
                    occ[rr][c2] = true;
                }
            }
            cells.push(Cell { id: ch, r, c, rs, cs });
        }
        let rows_spec = match n.get(prop::grid::ROWS) {
            Some(Val::T(t)) => t.len(),
            _ => 0,
        };
        (cells, ncols, occ.len().max(rows_spec))
    }

    fn grid_cols(&mut self, id: u32, cells: &[Cell], ncols: usize, avail: f32) -> Sized {
        let specs = self.tracks(id, prop::grid::COLS);
        let gap = self.gap(id);
        let tc: Vec<TCell> = cells
            .iter()
            .map(|c| TCell {
                start: c.c,
                span: c.cs,
                min: self.min_w(c.id),
                pref: self.pref_w(c.id),
                prio: self.nodes[&c.id].i(prop::PRIORITY, 0),
            })
            .collect();
        let specs = if specs.is_empty() { vec![Track::fr(1.0)] } else { specs };
        size_tracks(&specs, ncols, &tc, avail, gap)
    }

    fn cell_w(&mut self, child: u32, area_w: f32) -> f32 {
        let ax = self.nodes[&child].i(prop::ALIGN_X, 0);
        let w = if ax == 0 { area_w } else { self.pref_w(child).min(area_w) };
        self.clamp_w(child, w)
    }

    fn grid_rows(&mut self, id: u32, cells: &[Cell], cols: &Sized, nrows: usize, avail: f32) -> Sized {
        let specs = self.tracks(id, prop::grid::ROWS);
        let gap = self.gap(id);
        let mut tc = Vec::with_capacity(cells.len());
        for c in cells {
            let Some(aw) = cols.span(c.c, c.cs, gap) else { continue };
            let w = self.cell_w(c.id, aw);
            tc.push(TCell {
                start: c.r,
                span: c.rs,
                min: self.min_h(c.id, w),
                pref: self.height(c.id, w),
                prio: self.nodes[&c.id].i(prop::PRIORITY, 0),
            });
        }
        size_tracks(&specs, nrows, &tc, avail, gap)
    }

    fn text_lines(&mut self, id: u32, w: f32) -> Vec<(usize, usize)> {
        if let Some((cw, lines)) = &self.nodes[&id].m.lines {
            if (*cw - w).abs() < 0.01 {
                return lines.clone();
            }
        }
        let n = &self.nodes[&id];
        let style = n.s(prop::text::STYLE).to_string();
        let wrap = n.i(prop::text::WRAP, 0);
        let max_lines = n.i(prop::text::MAX_LINES, 0);
        let ts = self.text_style(id, &style);
        let txt = self.nodes[&id].s(prop::text::TEXT);
        let mut lines = if wrap == 0 {
            crate::text::wrap(&self.fonts, ts.font, ts.size, txt, w.max(1.0))
        } else {
            vec![(0, txt.len())]
        };
        if max_lines > 0 && lines.len() > max_lines as usize {
            lines.truncate(max_lines as usize);
            // The last line runs to the end so the painter can ellipsize it.
            let last = lines.len() - 1;
            lines[last].1 = txt.len();
        }
        self.nodes.get_mut(&id).unwrap().m.lines = Some((w, lines.clone()));
        lines
    }

    /// A checkbox label's lines when the whole node is `w` wide.
    pub(crate) fn checkbox_lines(&mut self, id: u32, w: f32) -> Vec<(usize, usize)> {
        let ts = self.text_style(id, "body");
        let n = &self.nodes[&id];
        let label = n.s(prop::checkbox::LABEL);
        if label.is_empty() {
            return Vec::new();
        }
        if n.i(prop::checkbox::WRAP, 0) != 0 {
            return vec![(0, label.len())];
        }
        // The box, its 1 px inset and the 8 px gap come off the width.
        let avail = w - ((ts.size + 2.0).round() + 9.0);
        crate::text::wrap(&self.fonts, ts.font, ts.size, label, avail.max(1.0))
    }

    /// Children that aren't hidden.
    fn shown(&self, id: u32) -> Vec<u32> {
        self.nodes[&id].children.iter().copied().filter(|c| self.nodes[c].i(prop::HIDDEN, 0) == 0).collect()
    }

    fn is_flow(&self, id: u32) -> bool {
        let n = &self.nodes[&id];
        n.kind == kind::GRID && n.i(prop::grid::FLOW, 0) != 0
    }

    /// A flow grid's lines at content width `w`: (child, width, height) per
    /// item, and each line's height.
    fn flow_lines(&mut self, id: u32, w: f32) -> Vec<(Vec<(u32, f32, f32)>, f32)> {
        let gap = self.gap(id);
        let kids = self.shown(id);
        let mut lines: Vec<(Vec<(u32, f32, f32)>, f32)> = Vec::new();
        let mut x = 0.0;
        for c in kids {
            let cw = self.pref_w(c).min(w).max(self.min_w(c).min(w));
            let ch = self.height(c, cw);
            match lines.last_mut() {
                Some((items, lh)) if x + gap + cw <= w + 0.01 => {
                    items.push((c, cw, ch));
                    *lh = lh.max(ch);
                    x += gap + cw;
                }
                _ => {
                    lines.push((vec![(c, cw, ch)], ch));
                    x = cw;
                }
            }
        }
        lines
    }

    fn flow_h(&mut self, id: u32, w: f32) -> f32 {
        let gap = self.gap(id);
        let lines = self.flow_lines(id, w);
        lines.iter().map(|l| l.1).sum::<f32>() + gap * (lines.len() as f32 - 1.0).max(0.0)
    }

    fn intrinsic_min_w(&mut self, id: u32) -> f32 {
        let k = self.nodes[&id].kind;
        let (px, _) = self.ctl_pad(id);
        if self.is_flow(id) {
            return self.shown(id).into_iter().map(|c| self.min_w(c)).fold(0.0, f32::max);
        }
        match k {
            kind::GRID => {
                let (cells, ncols, _) = self.place(id);
                let specs = self.tracks(id, prop::grid::COLS);
                let gap = self.gap(id);
                let mut mins = vec![0.0f32; ncols];
                let mut prio = vec![0i32; ncols];
                for c in &cells {
                    let p = self.nodes[&c.id].i(prop::PRIORITY, 0);
                    prio[c.c] = prio[c.c].max(p);
                    if c.cs == 1 {
                        mins[c.c] = mins[c.c].max(self.min_w(c.id));
                    }
                }
                let mut total = 0.0;
                let mut count = 0;
                for i in 0..ncols {
                    let t = specs.get(i).copied().unwrap_or(if specs.is_empty() { Track::fr(1.0) } else { Track::auto() });
                    if prio[i] < 0 {
                        continue;
                    }
                    let f = match t.kind {
                        track::PX => {
                            if t.min.is_finite() {
                                t.min.min(t.value)
                            } else {
                                t.value
                            }
                        }
                        _ => clamp_track(&t, mins[i]),
                    };
                    total += f;
                    count += 1;
                }
                total + gap * (count as f32 - 1.0).max(0.0)
            }
            kind::TEXT => {
                let n = &self.nodes[&id];
                let style = n.s(prop::text::STYLE).to_string();
                let wrap = n.i(prop::text::WRAP, 0);
                let ts = self.text_style(id, &style);
                let txt = self.nodes[&id].s(prop::text::TEXT);
                match wrap {
                    0 => txt.split_whitespace().map(|w| self.fonts.measure(ts.font, ts.size, w)).fold(0.0, f32::max),
                    1 => self.fonts.measure(ts.font, ts.size, "…").min(self.fonts.measure(ts.font, ts.size, txt)),
                    _ => 0.0,
                }
            }
            kind::BUTTON => {
                let ts = self.text_style(id, "body");
                let l = self.nodes[&id].s(prop::button::LABEL);
                self.fonts.measure(ts.font, ts.size, l).min(self.fonts.measure(ts.font, ts.size, "…") * 3.0) + 2.0 * px
            }
            kind::INPUT | kind::SELECT => {
                let ts = self.text_style(id, "body");
                ts.size * 0.6 * 4.0 + 2.0 * px
            }
            kind::CHECKBOX => {
                let ts = self.text_style(id, "body");
                let n = &self.nodes[&id];
                let word = if n.i(prop::checkbox::WRAP, 0) == 0 {
                    n.s(prop::checkbox::LABEL).split_whitespace().map(|w| self.fonts.measure(ts.font, ts.size, w)).fold(0.0, f32::max)
                } else {
                    0.0
                };
                ts.size + 4.0 + if word > 0.0 { 8.0 + word } else { 0.0 }
            }
            kind::TABS => {
                let ts = self.text_style(id, "body");
                ts.size * 4.0
            }
            kind::PROGRESS => 20.0,
            kind::DIVIDER => 1.0,
            kind::SCROLL => {
                let axis = self.nodes[&id].i(prop::scroll::AXIS, 0);
                match (axis, self.nodes[&id].children.first().copied()) {
                    (0, Some(c)) => self.min_w(c),
                    _ => 0.0,
                }
            }
            kind::IMAGE | kind::CANVAS => 0.0,
            _ => 0.0,
        }
    }

    fn intrinsic_pref_w(&mut self, id: u32) -> f32 {
        let k = self.nodes[&id].kind;
        let (px, _) = self.ctl_pad(id);
        if self.is_flow(id) {
            let gap = self.gap(id);
            let kids = self.shown(id);
            let n = kids.len();
            return kids.into_iter().map(|c| self.pref_w(c)).sum::<f32>() + gap * (n as f32 - 1.0).max(0.0);
        }
        match k {
            kind::GRID => {
                let (cells, ncols, _) = self.place(id);
                let gap = self.gap(id);
                self.grid_cols(id, &cells, ncols, INF).total(gap)
            }
            kind::TEXT => {
                let n = &self.nodes[&id];
                let style = n.s(prop::text::STYLE).to_string();
                let ts = self.text_style(id, &style);
                let txt = self.nodes[&id].s(prop::text::TEXT);
                txt.split('\n').map(|l| self.fonts.measure(ts.font, ts.size, l)).fold(0.0, f32::max).ceil()
            }
            kind::BUTTON => {
                let ts = self.text_style(id, "body");
                let l = self.nodes[&id].s(prop::button::LABEL);
                (self.fonts.measure(ts.font, ts.size, l) + 2.0 * px).ceil()
            }
            kind::INPUT => {
                let ts = self.text_style(id, "body");
                (ts.size * 14.0).round() + 2.0 * px
            }
            kind::SELECT => {
                let ts = self.text_style(id, "body");
                let n = &self.nodes[&id];
                let mut w = self.fonts.measure(ts.font, ts.size, n.s(prop::select::PLACEHOLDER));
                for o in n.s(prop::select::OPTIONS).split('\n') {
                    w = w.max(self.fonts.measure(ts.font, ts.size, o));
                }
                (w + 2.0 * px + ts.size * 1.5).ceil()
            }
            kind::CHECKBOX => {
                let ts = self.text_style(id, "body");
                let l = self.nodes[&id].s(prop::checkbox::LABEL);
                let lw = self.fonts.measure(ts.font, ts.size, l);
                (ts.size + 4.0 + if l.is_empty() { 0.0 } else { 8.0 + lw }).ceil()
            }
            kind::TABS => {
                let ts = self.text_style(id, "body");
                let labels = self.nodes[&id].s(prop::tabs::LABELS).to_string();
                labels.split('\n').map(|l| self.fonts.measure(ts.font, ts.size, l) + 2.0 * px).sum::<f32>().ceil()
            }
            kind::PROGRESS => 120.0,
            kind::DIVIDER => 1.0,
            kind::SCROLL => match self.nodes[&id].children.first().copied() {
                Some(c) => self.pref_w(c),
                None => 0.0,
            },
            kind::CANVAS => self.nodes[&id].i(prop::canvas::PX_W, 0) as f32,
            kind::IMAGE => self.image_size(id).0,
            _ => 0.0,
        }
    }

    /// Natural image size in logical px (filled in once the image is decoded).
    pub fn image_size(&self, id: u32) -> (f32, f32) {
        self.image_sizes.get(&id).copied().unwrap_or((0.0, 0.0))
    }

    fn intrinsic_h(&mut self, id: u32, w: f32) -> f32 {
        let k = self.nodes[&id].kind;
        let (_, py) = self.ctl_pad(id);
        if self.is_flow(id) {
            return self.flow_h(id, w);
        }
        match k {
            kind::GRID => {
                let (cells, ncols, nrows) = self.place(id);
                let gap = self.gap(id);
                let cols = self.grid_cols(id, &cells, ncols, w);
                self.grid_rows(id, &cells, &cols, nrows, INF).total(gap)
            }
            kind::TEXT => {
                let style = self.nodes[&id].s(prop::text::STYLE).to_string();
                let ts = self.text_style(id, &style);
                let n = self.text_lines(id, w).len().max(1);
                n as f32 * ts.line_h
            }
            kind::BUTTON | kind::SELECT => {
                let ts = self.text_style(id, "body");
                ts.line_h + 2.0 * py
            }
            kind::INPUT => {
                let ts = self.text_style(id, "body");
                let n = &self.nodes[&id];
                let lines = if n.i(prop::input::MULTILINE, 0) != 0 { n.i(prop::input::LINES, 4).max(1) } else { 1 };
                lines as f32 * ts.line_h + 2.0 * py
            }
            kind::CHECKBOX => {
                let ts = self.text_style(id, "body");
                let n = self.checkbox_lines(id, w).len().max(1);
                (n as f32 * ts.line_h).max(ts.size + 4.0)
            }
            kind::TABS => {
                let ts = self.text_style(id, "body");
                ts.line_h + 2.0 * py + 2.0
            }
            kind::PROGRESS => 6.0,
            kind::DIVIDER => 1.0,
            kind::SCROLL => {
                let axis = self.nodes[&id].i(prop::scroll::AXIS, 0);
                match self.nodes[&id].children.first().copied() {
                    Some(c) if axis == 1 => {
                        let cw = self.pref_w(c).max(w);
                        self.height(c, cw)
                    }
                    Some(c) => self.height(c, w),
                    None => 0.0,
                }
            }
            kind::CANVAS => {
                let n = &self.nodes[&id];
                let (pw, ph) = (n.i(prop::canvas::PX_W, 0) as f32, n.i(prop::canvas::PX_H, 0) as f32);
                if pw > 0.0 && n.i(prop::canvas::FIT, 0) != 3 {
                    w * ph / pw
                } else {
                    ph
                }
            }
            kind::IMAGE => {
                let (iw, ih) = self.image_size(id);
                let fit = self.nodes[&id].i(prop::image::FIT, 0);
                if iw > 0.0 && fit != 3 {
                    (w * ih / iw).min(ih.max(w * ih / iw))
                } else {
                    ih
                }
            }
            _ => 0.0,
        }
    }

    pub fn min_w(&mut self, id: u32) -> f32 {
        if let Some(v) = self.nodes[&id].m.min_w {
            return v;
        }
        let p = self.pad(id);
        let v = self.intrinsic_min_w(id) + p[1] + p[3];
        let v = self.clamp_w(id, v);
        self.nodes.get_mut(&id).unwrap().m.min_w = Some(v);
        v
    }

    pub fn pref_w(&mut self, id: u32) -> f32 {
        if let Some(v) = self.nodes[&id].m.pref_w {
            return v;
        }
        let p = self.pad(id);
        let v = self.intrinsic_pref_w(id) + p[1] + p[3];
        let v = self.clamp_w(id, v);
        self.nodes.get_mut(&id).unwrap().m.pref_w = Some(v);
        v
    }

    /// Height at outer width `w`.
    pub fn height(&mut self, id: u32, w: f32) -> f32 {
        if let Some((cw, h)) = self.nodes[&id].m.h {
            if (cw - w).abs() < 0.01 {
                return h;
            }
        }
        let p = self.pad(id);
        let inner = (w - p[1] - p[3]).max(0.0);
        let v = self.intrinsic_h(id, inner) + p[0] + p[2];
        let v = self.clamp_h(id, v);
        self.nodes.get_mut(&id).unwrap().m.h = Some((w, v));
        v
    }

    /// The least height a node can be squeezed to at width `w`.
    fn min_h(&mut self, id: u32, w: f32) -> f32 {
        let n = &self.nodes[&id];
        if n.kind == kind::SCROLL && n.i(prop::scroll::AXIS, 0) != 1 {
            return self.clamp_h(id, 0.0);
        }
        if n.kind != kind::GRID || self.is_flow(id) {
            return self.height(id, w);
        }
        // A grid can shrink each row to its floor (e.g. a row holding a scroll).
        let p = self.pad(id);
        let inner = (w - p[1] - p[3]).max(0.0);
        let (cells, ncols, nrows) = self.place(id);
        let gap = self.gap(id);
        let cols = self.grid_cols(id, &cells, ncols, inner);
        let specs = self.tracks(id, prop::grid::ROWS);
        let mut floors = vec![0.0f32; nrows];
        let mut prio = vec![0i32; nrows];
        for c in &cells {
            let Some(aw) = cols.span(c.c, c.cs, gap) else { continue };
            prio[c.r] = prio[c.r].max(self.nodes[&c.id].i(prop::PRIORITY, 0));
            if c.rs == 1 {
                let cw = self.cell_w(c.id, aw);
                floors[c.r] = floors[c.r].max(self.min_h(c.id, cw));
            }
        }
        let mut total = 0.0;
        let mut count = 0;
        for (i, f) in floors.iter().enumerate() {
            if prio[i] < 0 {
                continue;
            }
            let t = specs.get(i).copied().unwrap_or(Track::auto());
            total += match t.kind {
                track::PX => {
                    if t.min.is_finite() {
                        t.min.min(t.value)
                    } else {
                        t.value
                    }
                }
                _ => clamp_track(&t, *f),
            };
            count += 1;
        }
        let v = total + gap * (count as f32 - 1.0).max(0.0) + p[0] + p[2];
        let full = self.height(id, w);
        self.clamp_h(id, v.min(full))
    }

    /// Lays out the tree if anything changed. Returns whether it ran.
    pub fn layout(&mut self) -> bool {
        if !self.layout_dirty {
            return false;
        }
        self.layout_dirty = false;
        if self.root != 0 && self.nodes.contains_key(&self.root) {
            let t = self.top_inset;
            let r = RectF::new(0.0, t, self.win_w, (self.win_h - t).max(0.0));
            self.arrange(self.root, r, false);
        }
        for id in self.take_scroll_to() {
            self.scroll_into_view(id);
        }
        true
    }

    fn arrange(&mut self, id: u32, rect: RectF, parent_damaged: bool) {
        let n = self.nodes.get_mut(&id).unwrap();
        if n.rect == rect && !n.dirty {
            return;
        }
        let old = n.rect;
        let moved = old != rect;
        n.rect = rect;
        n.dirty = false;
        let k = n.kind;
        let mut damaged = parent_damaged;
        if moved && !parent_damaged {
            self.damage_rect(old);
            self.damage_rect(rect);
            damaged = true;
        }
        match k {
            kind::GRID => self.arrange_grid(id, rect, damaged),
            kind::SCROLL => self.arrange_scroll(id, rect, damaged),
            _ => {}
        }
    }

    fn arrange_grid(&mut self, id: u32, rect: RectF, damaged: bool) {
        let p = self.pad(id);
        let content = RectF::new(rect.x + p[3], rect.y + p[0], (rect.w - p[1] - p[3]).max(0.0), (rect.h - p[0] - p[2]).max(0.0));
        if self.is_flow(id) {
            return self.arrange_flow(id, content, damaged);
        }
        let (cells, ncols, nrows) = self.place(id);
        let gap = self.gap(id);
        let cols = self.grid_cols(id, &cells, ncols, content.w);
        let rows = self.grid_rows(id, &cells, &cols, nrows, content.h);
        let cx = cols.offsets(gap);
        let ry = rows.offsets(gap);
        let clips = cols.total(gap) > content.w + 0.5 || rows.total(gap) > content.h + 0.5;
        self.nodes.get_mut(&id).unwrap().clips = clips;
        // Children not placed (hidden) keep their state but are not arranged.
        for c in &cells {
            let area = match (cols.span(c.c, c.cs, gap), rows.span(c.r, c.rs, gap)) {
                (Some(w), Some(h)) => Some(RectF::new(content.x + cx[c.c], content.y + ry[c.r], w, h)),
                _ => None,
            };
            let was = self.nodes[&c.id].collapsed;
            let Some(area) = area else {
                if !was {
                    let r = self.nodes[&c.id].rect;
                    if !damaged {
                        self.damage_rect(r);
                    }
                    self.nodes.get_mut(&c.id).unwrap().collapsed = true;
                }
                continue;
            };
            if was {
                self.nodes.get_mut(&c.id).unwrap().collapsed = false;
                self.nodes.get_mut(&c.id).unwrap().dirty = true;
                if !damaged {
                    self.damage_rect(area);
                }
            }
            let (ax, ay) = (self.nodes[&c.id].i(prop::ALIGN_X, 0), self.nodes[&c.id].i(prop::ALIGN_Y, 0));
            let w = self.cell_w(c.id, area.w);
            let h = if ay == 0 { self.clamp_h(c.id, area.h) } else { self.height(c.id, w).min(area.h) };
            let x = match ax {
                2 => area.x + (area.w - w) / 2.0,
                3 => area.x + area.w - w,
                _ => area.x,
            };
            let y = match ay {
                2 => area.y + (area.h - h) / 2.0,
                3 => area.y + area.h - h,
                _ => area.y,
            };
            self.arrange(c.id, RectF::new(x, y, w, h), damaged);
        }
    }

    fn arrange_flow(&mut self, id: u32, content: RectF, damaged: bool) {
        let gap = self.gap(id);
        let lines = self.flow_lines(id, content.w);
        let total = lines.iter().map(|l| l.1).sum::<f32>() + gap * (lines.len() as f32 - 1.0).max(0.0);
        self.nodes.get_mut(&id).unwrap().clips = total > content.h + 0.5;
        let mut y = content.y;
        for (items, lh) in lines {
            let mut x = content.x;
            for (c, w, h) in items {
                let ay = self.nodes[&c].i(prop::ALIGN_Y, 0);
                let (cy, ch) = match ay {
                    0 => (y, lh),
                    2 => (y + (lh - h) / 2.0, h),
                    3 => (y + lh - h, h),
                    _ => (y, h),
                };
                if self.nodes[&c].collapsed {
                    let n = self.nodes.get_mut(&c).unwrap();
                    n.collapsed = false;
                    n.dirty = true;
                }
                self.arrange(c, RectF::new(x, cy, w, ch), damaged);
                x += w + gap;
            }
            y += lh + gap;
        }
    }

    fn arrange_scroll(&mut self, id: u32, rect: RectF, damaged: bool) {
        let Some(child) = self.nodes[&id].children.first().copied() else { return };
        if self.nodes[&child].i(prop::HIDDEN, 0) != 0 {
            return;
        }
        let axis = self.nodes[&id].i(prop::scroll::AXIS, 0);
        let stick = self.nodes[&id].i(prop::scroll::STICK_END, 0) != 0;
        let (cw, ch) = match axis {
            1 => {
                let w = self.pref_w(child).max(rect.w);
                (w, rect.h)
            }
            2 => {
                let w = self.pref_w(child).max(rect.w);
                (w, self.height(child, w).max(rect.h))
            }
            _ => (rect.w, self.height(child, rect.w).max(rect.h)),
        };
        let mut near_end = false;
        {
            let st = self.nodes.get_mut(&id).unwrap().scroll().unwrap();
            let grew = ch > st.content_h + 0.5 || cw > st.content_w + 0.5;
            if stick && st.at_end && grew {
                st.y = ch - rect.h;
            }
            if grew {
                st.near_end_sent = false;
            }
            st.content_w = cw;
            st.content_h = ch;
            st.x = st.x.clamp(0.0, (cw - rect.w).max(0.0));
            st.y = st.y.clamp(0.0, (ch - rect.h).max(0.0));
            st.at_end = st.y >= ch - rect.h - 1.0;
            let remaining = if axis == 1 { cw - rect.w - st.x } else { ch - rect.h - st.y };
            let viewport = if axis == 1 { rect.w } else { rect.h };
            if remaining <= viewport && !st.near_end_sent && remaining < f32::INFINITY {
                st.near_end_sent = true;
                near_end = (if axis == 1 { cw } else { ch }) > viewport + 0.5;
            }
        }
        if near_end {
            self.out.push(super::UiEvent::NearEnd(id));
        }
        let (sx, sy) = match &self.nodes[&id].st {
            State::Scroll(s) => (s.x, s.y),
            _ => (0.0, 0.0),
        };
        self.arrange(child, RectF::new(rect.x - sx, rect.y - sy, cw, ch), damaged);
    }

    /// Scrolls `id`'s scroll ancestors so it is visible.
    pub fn scroll_into_view(&mut self, id: u32) {
        let mut cur = self.nodes.get(&id).map(|n| n.parent).unwrap_or(0);
        while cur != 0 {
            let Some(n) = self.nodes.get(&cur) else { break };
            let parent = n.parent;
            if n.kind == kind::SCROLL {
                let target = self.nodes[&id].rect;
                let vp = n.rect;
                let mut dy = 0.0;
                let mut dx = 0.0;
                if target.y < vp.y {
                    dy = target.y - vp.y;
                } else if target.y + target.h > vp.y + vp.h {
                    dy = (target.y + target.h - (vp.y + vp.h)).min(target.y - vp.y);
                }
                if target.x < vp.x {
                    dx = target.x - vp.x;
                } else if target.x + target.w > vp.x + vp.w {
                    dx = (target.x + target.w - (vp.x + vp.w)).min(target.x - vp.x);
                }
                if dx != 0.0 || dy != 0.0 {
                    self.scroll_by(cur, dx, dy);
                }
            }
            cur = parent;
        }
    }

    /// Scrolls a scroll node and re-arranges its content. Returns whether it moved.
    pub fn scroll_by(&mut self, id: u32, dx: f32, dy: f32) -> bool {
        let rect = self.nodes[&id].rect;
        let axis = self.nodes[&id].i(prop::scroll::AXIS, 0);
        let Some(st) = self.nodes.get_mut(&id).and_then(|n| n.scroll()) else { return false };
        let (ox, oy) = (st.x, st.y);
        if axis != 0 {
            st.x = (st.x + dx).clamp(0.0, (st.content_w - rect.w).max(0.0));
        }
        if axis != 1 {
            st.y = (st.y + dy).clamp(0.0, (st.content_h - rect.h).max(0.0));
        }
        if (st.x, st.y) == (ox, oy) {
            return false;
        }
        self.nodes.get_mut(&id).unwrap().dirty = true;
        self.damage_rect(rect);
        // Re-arrange just this subtree in place.
        let r = self.nodes[&id].rect;
        self.nodes.get_mut(&id).unwrap().rect = RectF::default();
        self.arrange(id, r, true);
        true
    }
}
