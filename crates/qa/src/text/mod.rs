//! Font loading, simple shaping (cmap, advances, pair kerning) and line breaking.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use ttf_parser::{Face, GlyphId};

pub type FontId = u16;

pub struct Font {
    pub face: Face<'static>,
    pub upem: f32,
    pub path: PathBuf,
    /// Draw thicker outlines because no real bold face was found.
    pub synth_bold: bool,
    /// Glyphs are color bitmaps (CBDT or sbix), such as Noto Color Emoji.
    pub color: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    /// Distance between baselines, before `line.height` is applied.
    pub height: f32,
}

/// One positioned glyph: (font, glyph id, x offset from the run origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PGlyph {
    pub font: FontId,
    pub id: u16,
    pub x: f32,
}

pub struct Fonts {
    pub fonts: Vec<Font>,
    by_path: HashMap<PathBuf, FontId>,
    index: Option<HashMap<String, PathBuf>>,
    index_fresh: bool,
    /// Fonts tried, in order, for characters the primary font lacks.
    pub fallback: Vec<FontId>,
}

pub const DEFAULT_SANS: &[&str] = &["DejaVuSans.ttf", "NotoSans-Regular.ttf", "LiberationSans-Regular.ttf", "Arial.ttf"];
pub const DEFAULT_SANS_BOLD: &[&str] = &["DejaVuSans-Bold.ttf", "NotoSans-Bold.ttf", "LiberationSans-Bold.ttf", "Arial Bold.ttf"];
/// Tried in order for characters a style's font lacks (`font.fallback`
/// overrides it with a comma-separated list).
pub const DEFAULT_FALLBACK: &[&str] = &[
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansMath-Regular.ttf",
    "DejaVuSansMono.ttf",
    "NotoEmoji-Regular.ttf",
    "Symbola.ttf",
    "NotoColorEmoji.ttf",
    "Apple Color Emoji.ttc",
    "NotoSansCJK-Regular.otf",
    "DroidSansFallbackFull.ttf",
    "unifont.otf",
    "Arial Unicode.ttf",
    "Apple Symbols.ttf",
];
pub const DEFAULT_MONO: &[&str] = &["DejaVuSansMono.ttf", "NotoSansMono-Regular.ttf", "LiberationMono-Regular.ttf", "Monaco.ttf", "Courier New.ttf"];

fn mmap_file(path: &Path) -> io::Result<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(path)?;
    let len = f.metadata()?.len() as usize;
    if len == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty font file"));
    }
    unsafe {
        let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0);
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // Fonts stay mapped for the life of the process; the page cache shares them between apps.
        Ok(std::slice::from_raw_parts(p as *const u8, len))
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn font_dirs() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".config/quick-apps/fonts"),
        h.join(".local/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/usr/share/fonts"),
        h.join("Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/System/Library/Fonts"),
    ]
}

fn index_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join("quick-apps/fonts.idx")
}

fn walk(dir: &Path, depth: u32, out: &mut HashMap<String, PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            if depth < 8 {
                walk(&p, depth + 1, out);
            }
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc") {
                // Earlier directories win, so user fonts shadow system ones.
                out.entry(name.to_string()).or_insert(p);
            }
        }
    }
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts { fonts: Vec::new(), by_path: HashMap::new(), index: None, index_fresh: false, fallback: Vec::new() }
    }

    pub fn load_path(&mut self, path: &Path) -> io::Result<FontId> {
        if let Some(&id) = self.by_path.get(path) {
            return Ok(id);
        }
        let data = mmap_file(path)?;
        let face = Face::parse(data, 0).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
        let upem = face.units_per_em() as f32;
        let t = face.tables();
        let color = t.cbdt.is_some() || t.sbix.is_some();
        let id = self.fonts.len() as FontId;
        self.fonts.push(Font { face, upem, path: path.to_path_buf(), synth_bold: false, color });
        self.by_path.insert(path.to_path_buf(), id);
        Ok(id)
    }

    /// A copy of `regular` drawn with thickened outlines, for when no bold face exists.
    pub fn synth_bold(&mut self, regular: FontId) -> FontId {
        if let Some(i) = self.fonts.iter().position(|f| f.synth_bold && f.path == self.fonts[regular as usize].path) {
            return i as FontId;
        }
        let r = &self.fonts[regular as usize];
        let f = Font { face: r.face.clone(), upem: r.upem, path: r.path.clone(), synth_bold: true, color: r.color };
        self.fonts.push(f);
        (self.fonts.len() - 1) as FontId
    }

    fn load_index(&mut self) {
        if self.index.is_some() {
            return;
        }
        let mut map = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(index_path()) {
            for line in s.lines() {
                if let Some((n, p)) = line.split_once('\t') {
                    map.insert(n.to_string(), PathBuf::from(p));
                }
            }
        }
        if map.is_empty() {
            self.rebuild_index();
        } else {
            self.index = Some(map);
        }
    }

    fn rebuild_index(&mut self) {
        let mut map = HashMap::new();
        for d in font_dirs() {
            walk(&d, 0, &mut map);
        }
        let mut s = String::new();
        let mut names: Vec<_> = map.iter().collect();
        names.sort();
        for (n, p) in names {
            s.push_str(n);
            s.push('\t');
            s.push_str(&p.to_string_lossy());
            s.push('\n');
        }
        let ip = index_path();
        if let Some(dir) = ip.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&ip, s);
        self.index = Some(map);
        self.index_fresh = true;
    }

    /// Loads a font named by an absolute path or a file name.
    pub fn find(&mut self, name: &str) -> Option<FontId> {
        let name = name.trim().trim_matches('"');
        if name.starts_with('/') {
            return self.load_path(Path::new(name)).ok();
        }
        self.load_index();
        let mut hit = self.index.as_ref().and_then(|m| m.get(name).cloned());
        if hit.as_ref().is_some_and(|p| !p.exists()) || (hit.is_none() && !self.index_fresh) {
            self.rebuild_index();
            hit = self.index.as_ref().and_then(|m| m.get(name).cloned());
        }
        self.load_path(&hit?).ok()
    }

    /// Sets the fallback list from font names, skipping missing fonts and
    /// those with neither outlines nor color bitmaps.
    pub fn set_fallback(&mut self, names: &[&str]) {
        let mut v = Vec::new();
        for n in names {
            let Some(id) = self.find(n) else { continue };
            let f = &self.fonts[id as usize];
            let t = f.face.tables();
            if (t.glyf.is_some() || t.cff.is_some() || f.color) && !v.contains(&id) {
                v.push(id);
            }
        }
        self.fallback = v;
    }

    pub fn find_first(&mut self, names: &[&str]) -> Option<FontId> {
        names.iter().find_map(|n| self.find(n))
    }

    pub fn metrics(&self, f: FontId, size: f32) -> LineMetrics {
        let font = &self.fonts[f as usize];
        let s = size / font.upem;
        let face = &font.face;
        let ascent = face.ascender() as f32 * s;
        let descent = -face.descender() as f32 * s;
        LineMetrics { ascent, descent, height: ascent + descent + face.line_gap() as f32 * s }
    }

    fn kern(&self, f: FontId, a: u16, b: u16) -> f32 {
        let face = &self.fonts[f as usize].face;
        let Some(k) = face.tables().kern else { return 0.0 };
        for st in k.subtables {
            if st.horizontal && !st.variable && !st.has_cross_stream {
                if let Some(v) = st.glyphs_kerning(GlyphId(a), GlyphId(b)) {
                    return v as f32;
                }
            }
        }
        0.0
    }

    /// The glyph for `ch`, from a color font first when `color` is set
    /// (emoji presentation), else from `f` and then the fallbacks in order.
    fn glyph_in(&self, f: FontId, ch: char, color: bool) -> (FontId, u16) {
        if color {
            for &fb in &self.fallback {
                if self.fonts[fb as usize].color {
                    if let Some(g) = self.fonts[fb as usize].face.glyph_index(ch) {
                        return (fb, g.0);
                    }
                }
            }
        }
        if let Some(g) = self.fonts[f as usize].face.glyph_index(ch) {
            return (f, g.0);
        }
        for &fb in &self.fallback {
            if let Some(g) = self.fonts[fb as usize].face.glyph_index(ch) {
                return (fb, g.0);
            }
        }
        (f, 0)
    }

    fn glyph_for(&self, f: FontId, ch: char) -> (FontId, u16) {
        self.glyph_in(f, ch, emoji_presentation(ch))
    }

    fn adv(&self, font: FontId, id: u16, size: f32) -> f32 {
        let fo = &self.fonts[font as usize];
        fo.face.glyph_hor_advance(GlyphId(id)).unwrap_or(0) as f32 * size / fo.upem
    }

    /// What `ch` does to the glyph before it: `Some(Some(color))` switches
    /// that glyph to emoji (FE0F) or text (FE0E) presentation, `Some(None)`
    /// means `ch` takes no space at all, `None` means it is drawn normally.
    fn modifier(prev: Option<char>, ch: char) -> Option<Option<bool>> {
        let p = prev?;
        match ch {
            '\u{FE0F}' => Some(Some(true)),
            '\u{FE0E}' => Some(Some(false)),
            // Joiners, other variation selectors, skin tones and tag
            // characters only make sense to a ligature-aware emoji font. We
            // draw the base emoji and drop the rest.
            '\u{200D}' | '\u{FE00}'..='\u{FE0D}' | '\u{E0020}'..='\u{E007F}' => Some(None),
            '\u{1F3FB}'..='\u{1F3FF}' if emoji_presentation(p) || p == '\u{FE0F}' => Some(None),
            _ => None,
        }
    }

    /// Positions `text` on one line, one glyph per char (chars that a
    /// selector, joiner or ligature swallowed get `id == NO_GLYPH`).
    /// Returns the advance width.
    pub fn shape(&self, f: FontId, size: f32, text: &str, out: &mut Vec<PGlyph>) -> f32 {
        let chars: Vec<char> = text.chars().filter(|&c| c != '\n' && c != '\r').map(|c| if c == '\t' { ' ' } else { c }).collect();
        let mut x = 0.0;
        let mut prev: Option<(FontId, u16)> = None;
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if Self::modifier(i.checked_sub(1).map(|j| chars[j]), ch).is_some() {
                out.push(PGlyph { font: f, id: NO_GLYPH, x });
                i += 1;
                continue;
            }
            let next = chars.get(i + 1).copied();
            let color = match next {
                Some('\u{FE0F}') => true,
                Some('\u{FE0E}') => false,
                Some(n) => emoji_presentation(ch) || (is_flag_letter(ch) && is_flag_letter(n)),
                None => emoji_presentation(ch),
            };
            let (font, mut id) = self.glyph_in(f, ch, color);
            let mut used = 1;
            if self.fonts[font as usize].color && next.is_some() {
                (id, used) = self.ligature(font, id, &chars[i..]);
            }
            let s = size / self.fonts[font as usize].upem;
            if let Some((pf, pid)) = prev {
                if pf == font {
                    x += self.kern(font, pid, id) * s;
                }
            }
            out.push(PGlyph { font, id, x });
            x += self.adv(font, id, size);
            for _ in 1..used {
                out.push(PGlyph { font: f, id: NO_GLYPH, x });
            }
            prev = Some((font, id));
            i += used;
        }
        x
    }

    /// Applies a color font's ligatures (emoji ZWJ sequences, flags, skin
    /// tones, keycaps) to the glyph `first` for `chars[0]`. Returns the glyph
    /// and how many chars it covers.
    fn ligature(&self, font: FontId, first: u16, chars: &[char]) -> (u16, usize) {
        use ttf_parser::gsub::SubstitutionSubtable;
        let face = &self.fonts[font as usize].face;
        let Some(gsub) = face.tables().gsub else { return (first, 1) };
        // (glyph, chars it stands for), for the run the font covers.
        let mut seq: Vec<(u16, usize)> = vec![(first, 1)];
        for &c in chars.iter().skip(1).take(15) {
            match face.glyph_index(c) {
                Some(g) => seq.push((g.0, 1)),
                None => break,
            }
        }
        if seq.len() < 2 {
            return (first, 1);
        }
        for li in 0..gsub.lookups.len() {
            let Some(lookup) = gsub.lookups.get(li) else { continue };
            for si in 0..lookup.subtables.len() {
                let Some(SubstitutionSubtable::Ligature(lig)) = lookup.subtables.get::<SubstitutionSubtable>(si) else { continue };
                let mut k = 0;
                while k < seq.len() {
                    let hit = lig.coverage.get(GlyphId(seq[k].0)).and_then(|ci| lig.ligature_sets.get(ci)).and_then(|set| {
                        set.into_iter().find(|l| {
                            let n = l.components.len() as usize;
                            k + n < seq.len() && l.components.into_iter().enumerate().all(|(j, g)| seq[k + 1 + j].0 == g.0)
                        })
                    });
                    if let Some(l) = hit {
                        let n = l.components.len() as usize;
                        let covered = seq[k..=k + n].iter().map(|e| e.1).sum();
                        seq.splice(k..=k + n, [(l.glyph.0, covered)]);
                    }
                    k += 1;
                }
            }
        }
        seq[0]
    }

    pub fn measure(&self, f: FontId, size: f32, text: &str) -> f32 {
        let mut g = Vec::new();
        self.shape(f, size, text, &mut g)
    }

    /// The width `ch` adds after `prev` on the same line, kerning included.
    /// Summed over a line it gives the same width as `measure`, except
    /// around presentation selectors after a kerned pair, which is rare
    /// enough to ignore.
    pub fn advance(&self, f: FontId, size: f32, prev: Option<char>, ch: char) -> f32 {
        let tab = |c: char| if c == '\t' { ' ' } else { c };
        let (ch, prev) = (tab(ch), prev.map(tab));
        if let Some(m) = Self::modifier(prev, ch) {
            // A selector swaps the previous glyph; the difference lands here.
            let (Some(color), Some(p)) = (m, prev) else { return 0.0 };
            let (of, oid) = self.glyph_for(f, p);
            let (nf, nid) = self.glyph_in(f, p, color);
            if (nf, nid) == (of, oid) || nid == 0 {
                return 0.0;
            }
            return self.adv(nf, nid, size) - self.adv(of, oid, size);
        }
        let (font, id) = self.glyph_for(f, ch);
        let s = size / self.fonts[font as usize].upem;
        let mut x = self.adv(font, id, size);
        if let Some(p) = prev {
            if !matches!(p, '\u{FE00}'..='\u{FE0F}' | '\u{200D}') {
                let (pf, pid) = self.glyph_for(f, p);
                if pf == font {
                    x += self.kern(font, pid, id) * s;
                }
            }
        }
        x
    }

    /// The x position of each char boundary in `text` (len = chars + 1).
    pub fn caret_stops(&self, f: FontId, size: f32, text: &str) -> Vec<f32> {
        let mut g = Vec::new();
        let w = self.shape(f, size, text, &mut g);
        let mut stops: Vec<f32> = g.iter().map(|p| p.x).collect();
        stops.push(w);
        stops
    }
}

/// Marks a char that takes no space (see `Fonts::shape`).
pub const NO_GLYPH: u16 = u16::MAX;

/// Whether `c` defaults to emoji (color) presentation. Approximates the
/// Unicode Emoji_Presentation property: everything from the Mahjong tiles up
/// that a color font has, plus the older symbols listed here.
pub fn emoji_presentation(c: char) -> bool {
    let c = c as u32;
    if (0x1F000..0x1FB00).contains(&c) {
        // Regional indicators and other letter-like symbols stay text.
        return !(0x1F100..0x1F18E).contains(&c) && !(0x1F1E6..=0x1F1FF).contains(&c);
    }
    const BMP: &[(u32, u32)] = &[
        (0x231A, 0x231B), (0x23E9, 0x23EC), (0x23F0, 0x23F0), (0x23F3, 0x23F3), (0x25FD, 0x25FE),
        (0x2614, 0x2615), (0x2648, 0x2653), (0x267F, 0x267F), (0x2693, 0x2693), (0x26A1, 0x26A1),
        (0x26AA, 0x26AB), (0x26BD, 0x26BE), (0x26C4, 0x26C5), (0x26CE, 0x26CE), (0x26D4, 0x26D4),
        (0x26EA, 0x26EA), (0x26F2, 0x26F3), (0x26F5, 0x26F5), (0x26FA, 0x26FA), (0x26FD, 0x26FD),
        (0x2705, 0x2705), (0x270A, 0x270B), (0x2728, 0x2728), (0x274C, 0x274C), (0x274E, 0x274E),
        (0x2753, 0x2755), (0x2757, 0x2757), (0x2795, 0x2797), (0x27B0, 0x27B0), (0x27BF, 0x27BF),
        (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55),
    ];
    BMP.iter().any(|&(a, b)| (a..=b).contains(&c))
}

/// A regional indicator letter; two make a flag.
fn is_flag_letter(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

/// Breaks `text` into lines no wider than `max_w`. Returns byte ranges.
/// Lines break at '\n', at spaces, after hyphens, and anywhere in CJK text.
/// A word longer than a line breaks at the last character that fits.
pub fn wrap(fonts: &Fonts, f: FontId, size: f32, text: &str, max_w: f32) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    for (pstart, para) in split_paras(text) {
        if para.is_empty() {
            lines.push((pstart, pstart));
            continue;
        }
        let mut start = 0;
        while start < para.len() {
            let rest = &para[start..];
            let mut w = 0.0;
            let mut last_break: Option<usize> = None;
            let mut end = rest.len();
            let mut next_start = rest.len();
            let mut prev_ch: Option<char> = None;
            for (i, ch) in rest.char_indices() {
                if let Some(p) = prev_ch {
                    if p == '-' || is_cjk(p) || is_cjk(ch) {
                        last_break = Some(i);
                    }
                }
                if ch == ' ' {
                    last_break = Some(i);
                }
                let cw = fonts.advance(f, size, prev_ch, ch);
                // The tolerance absorbs float drift against `measure`'s sum.
                if w + cw > max_w + 0.01 && ch != ' ' && i > 0 && Fonts::modifier(prev_ch, ch).is_none()
                    && prev_ch != Some('\u{200D}')
                    && !(prev_ch.is_some_and(is_flag_letter) && is_flag_letter(ch))
                {
                    match last_break {
                        Some(b) if b > 0 => {
                            end = b;
                            next_start = b;
                        }
                        _ => {
                            end = i;
                            next_start = i;
                        }
                    }
                    break;
                }
                w += cw;
                prev_ch = Some(ch);
            }
            // Spaces at a break belong to neither line.
            let line = rest[..end].trim_end_matches(' ');
            lines.push((pstart + start, pstart + start + line.len()));
            let skip = rest[next_start..].len() - rest[next_start..].trim_start_matches(' ').len();
            let adv = next_start + skip;
            if adv == 0 {
                // Never loop forever on a too-narrow width.
                let c = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                start += c;
            } else {
                start += adv;
            }
        }
    }
    lines
}

fn split_paras(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut s = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            out.push((s, text[s..i].trim_end_matches('\r')));
            s = i + 1;
        }
    }
    out.push((s, &text[s..]));
    out
}
