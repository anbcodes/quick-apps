//! Theme tokens: built-in defaults, the user theme file and per-app theme files.
//!
//! Lookup order for a node is: its own and its ancestors' CTX entries, the
//! per-app theme, the user theme, then the defaults. A token locked with `!`
//! in a theme file skips the CTX step.

use super::Val;
use crate::gfx::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DEFAULTS: &str = "
color.bg = #ffffff
color.fg = #1f2328
color.muted = #656d76
color.surface = #f6f8fa
color.border = #d0d7de
color.accent = #0969da
color.accent_fg = #ffffff
color.danger = #cf222e
color.danger_fg = #ffffff
color.success = #1a7f37
color.warning = #9a6700
color.hover = #0000000d
color.press = #0000001a
color.selection = #0969da26
color.focus = #0969da
font.body.size = 14
font.title.size = 22
font.heading.size = 17
font.caption.size = 12
font.mono.size = 13
font.body.weight = 400
font.title.weight = 600
font.heading.weight = 600
font.caption.weight = 400
font.mono.weight = 400
font.scale = 1
line.height = 1.35
space.gap = 8
space.pad = 8
radius = 6
surface.card.bg = #ffffff
surface.card.border = #d0d7de
";

pub type TokenMap = HashMap<String, (Val, bool)>;

pub struct Theme {
    pub defaults: TokenMap,
    pub user: TokenMap,
    pub app: TokenMap,
    pub user_path: Option<PathBuf>,
    pub app_path: Option<PathBuf>,
}

pub fn parse_value(v: &str) -> Val {
    let v = v.trim();
    if let Some(c) = Color::parse(v) {
        return Val::C(c);
    }
    if let Ok(f) = v.parse::<f32>() {
        return Val::F(f);
    }
    let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
    Val::S(v.to_string())
}

/// Parses `key = value` lines. `#` starts a comment (unless it starts a color), and a trailing `!` locks.
pub fn parse(text: &str) -> TokenMap {
    let mut m = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let mut v = v.trim();
        // A comment is " #" followed by a space or the end, so "#fff" values survive.
        if let Some(i) = v.find(" # ").or_else(|| v.strip_suffix(" #").map(|s| s.len())) {
            v = v[..i].trim();
        }
        let locked = v.ends_with('!');
        if locked {
            v = v[..v.len() - 1].trim();
        }
        m.insert(k.trim().to_string(), (parse_value(v), locked));
    }
    m
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("quick-apps")
}

impl Theme {
    pub fn new() -> Theme {
        Theme { defaults: parse(DEFAULTS), user: HashMap::new(), app: HashMap::new(), user_path: None, app_path: None }
    }

    /// Loads the user theme and, if `app_id` is given, the per-app theme.
    pub fn load(app_id: Option<&str>) -> Theme {
        let mut t = Theme::new();
        let dir = config_dir();
        t.user_path = Some(dir.join("theme"));
        t.app_path = app_id.map(|id| dir.join("apps").join(format!("{id}.theme")));
        t.reload();
        t
    }

    pub fn reload(&mut self) {
        let read = |p: &Option<PathBuf>| p.as_deref().and_then(|p: &Path| std::fs::read_to_string(p).ok()).map(|s| parse(&s)).unwrap_or_default();
        self.user = read(&self.user_path);
        self.app = read(&self.app_path);
    }

    pub fn locked(&self, key: &str) -> Option<&Val> {
        if let Some((v, true)) = self.app.get(key) {
            return Some(v);
        }
        if let Some((v, true)) = self.user.get(key) {
            return Some(v);
        }
        None
    }

    /// Steps 2–4 of the lookup.
    pub fn base(&self, key: &str) -> Option<&Val> {
        self.app.get(key).or_else(|| self.user.get(key)).or_else(|| self.defaults.get(key)).map(|(v, _)| v)
    }
}
