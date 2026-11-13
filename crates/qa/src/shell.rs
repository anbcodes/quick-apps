//! The main loop: platform events in, UI updates, damage-limited redraws out.

use crate::gfx::glyph::GlyphCache;
use crate::gfx::raster::Canvas;
use crate::gfx::{render, PixmapSource, RectF};
use crate::platform::wayland::Wayland;
use crate::platform::{PEvent, Platform, Rect};
use crate::ui::{Ui, UiEvent};
use std::collections::VecDeque;
use std::os::fd::RawFd;

/// What runs the app: a native harness, or the wasm host.
pub trait Driver {
    /// Extra file descriptors for the main loop to poll.
    fn fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
    /// Milliseconds until the driver wants `poll` called even if no fd is ready.
    fn timeout_ms(&mut self) -> Option<i32> {
        None
    }
    fn start(&mut self, ui: &mut Ui);
    fn events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>);
    fn resize(&mut self, _ui: &mut Ui, _w: f32, _h: f32, _scale: f32) {}
    /// Called after every wakeup. Apply pending patches here.
    fn poll(&mut self, _ui: &mut Ui) {}
    /// The user closed the window. Return true to exit now.
    fn close(&mut self, _ui: &mut Ui) -> bool {
        true
    }
    /// Set when the app asked to change the window title.
    fn take_title(&mut self) -> Option<String> {
        None
    }
    fn quit(&self) -> bool {
        false
    }
    fn pixmaps(&self) -> &dyn PixmapSource;
    /// Runtime-owned text bands drawn over the app. Default: none.
    fn overlay(&mut self, _ui: &mut Ui, _list: &mut Vec<crate::gfx::Cmd>) {}
    /// A second tree drawn over the app that takes all input while active (prompts, pickers).
    fn overlay_ui(&mut self) -> Option<&mut Ui> {
        None
    }
    fn overlay_events(&mut self, _ui: &mut Ui, _evs: Vec<UiEvent>) {}
    /// Clipboard text the app asked for (`ui.app_paste`).
    fn pasted(&mut self, _ui: &mut Ui, _text: String) {}
    /// Theme files changed: restyle any trees the driver owns.
    fn theme_changed(&mut self) {}
    /// Why the app stopped, if it crashed (shown by `qa shot`).
    fn error(&self) -> Option<&str> {
        None
    }
}

/// Watches the theme files so a change restyles the window (docs/theming.md).
struct ThemeWatch {
    fd: RawFd,
}

impl ThemeWatch {
    fn new() -> ThemeWatch {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        let w = ThemeWatch { fd };
        let _ = std::fs::create_dir_all(crate::ui::theme::config_dir());
        w.add_watches();
        w
    }

    fn add_watches(&self) {
        let dir = crate::ui::theme::config_dir();
        let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB;
        for d in [dir.clone(), dir.join("themes"), dir.join("apps")] {
            if let Ok(c) = std::ffi::CString::new(d.as_os_str().as_encoded_bytes()) {
                unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), mask) };
            }
        }
    }

    /// True when something changed since the last call.
    fn changed(&self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        while unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {
            any = true;
        }
        if any {
            // themes/ or apps/ may have just been created.
            self.add_watches();
        }
        any
    }
}

/// Merges damage per docs/rendering.md: more than 8 rects, or a bounding box
/// smaller than twice their total area, become the bounding box.
pub fn merge(rects: &mut Vec<Rect>) {
    rects.retain(|r| !r.is_empty());
    if rects.len() <= 1 {
        return;
    }
    let bbox = rects.iter().fold(Rect::default(), |a, r| a.union(r));
    let total: i64 = rects.iter().map(|r| r.area()).sum();
    if rects.len() > 8 || bbox.area() < 2 * total {
        rects.clear();
        rects.push(bbox);
    }
}

fn now_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

pub struct Shell {
    pub p: Wayland,
    pub ui: Ui,
    cache: GlyphCache,
    ocache: GlyphCache,
    overlay_was: bool,
    themes: ThemeWatch,
    history: VecDeque<Vec<Rect>>,
    debug: bool,
    paste_pending: bool,
    pub last_title: String,
    /// Draw our own title bar (no server-side decorations).
    csd: bool,
    close_hover: bool,
}

const TITLE_BAR: f32 = 32.0;
const EDGE: f32 = 6.0;

impl Shell {
    pub fn new(p: Wayland, ui: Ui) -> Shell {
        Shell {
            p,
            ui,
            cache: GlyphCache::new(),
            ocache: GlyphCache::new(),
            overlay_was: false,
            themes: ThemeWatch::new(),
            history: VecDeque::new(),
            debug: std::env::var_os("QA_DEBUG").is_some(),
            paste_pending: false,
            last_title: String::new(),
            csd: false,
            close_hover: false,
        }
    }

    fn edge_at(&self, x: f32, y: f32) -> Option<crate::platform::Edge> {
        use crate::platform::Edge::*;
        let (w, h) = (self.ui.win_w, self.ui.win_h);
        let (l, r, t, b) = (x < EDGE, x > w - EDGE, y < EDGE, y > h - EDGE);
        Some(match (l, r, t, b) {
            (true, _, true, _) => TopLeft,
            (_, true, true, _) => TopRight,
            (true, _, _, true) => BottomLeft,
            (_, true, _, true) => BottomRight,
            (true, _, _, _) => Left,
            (_, true, _, _) => Right,
            (_, _, true, _) => Top,
            (_, _, _, true) => Bottom,
            _ => return None,
        })
    }

    /// Pointer input on the title bar or window edges. Returns true if used.
    fn decoration_input(&mut self, e: &PEvent) -> bool {
        use crate::platform::{Cursor, Edge};
        if !self.csd {
            return false;
        }
        let s = self.ui.scale.max(0.01);
        let (x, y) = match e {
            PEvent::PointerMove { x, y } | PEvent::Button { x, y, .. } => (x / s, y / s),
            _ => return false,
        };
        let on_close = y < TITLE_BAR && x >= self.ui.win_w - TITLE_BAR;
        if on_close != self.close_hover {
            self.close_hover = on_close;
            self.ui.damage_rect(RectF::new(0.0, 0.0, self.ui.win_w, TITLE_BAR));
        }
        let edge = self.edge_at(x, y);
        let on_bar = y < TITLE_BAR || edge.is_some();
        match e {
            PEvent::PointerMove { .. } if on_bar => {
                self.ui.pointer_leave();
                self.ui.cursor = match edge {
                    Some(Edge::Left | Edge::Right) => Cursor::ResizeEW,
                    Some(Edge::Top | Edge::Bottom) => Cursor::ResizeNS,
                    Some(Edge::TopLeft | Edge::BottomRight) => Cursor::ResizeNWSE,
                    Some(_) => Cursor::ResizeNESW,
                    None => Cursor::Default,
                };
                true
            }
            PEvent::Button { pressed: true, .. } if on_bar => {
                if let Some(ed) = edge {
                    self.p.begin_resize(ed);
                } else if !on_close {
                    self.p.begin_move();
                }
                true
            }
            PEvent::Button { pressed: false, .. } if on_close => {
                self.ui.quit_requested = true;
                true
            }
            PEvent::Button { .. } => on_bar,
            _ => false,
        }
    }

    fn handle(&mut self, d: &mut dyn Driver, e: PEvent) -> bool {
        if self.decoration_input(&e) {
            return false;
        }
        if std::mem::take(&mut self.ui.quit_requested) {
            return d.close(&mut self.ui);
        }
        let s = self.ui.scale.max(0.01);
        // While an overlay is up it takes all input.
        let over = d.overlay_ui().is_some_and(|o| o.root != 0);
        let ui: &mut Ui = if over && is_input(&e) { d.overlay_ui().unwrap() } else { &mut self.ui };
        match e {
            PEvent::Resize { w, h, scale } => {
                self.ui.set_window(w, h, scale);
                self.history.clear();
                d.resize(&mut self.ui, w as f32 / scale, h as f32 / scale, scale);
            }
            PEvent::Close => return d.close(&mut self.ui),
            PEvent::FrameDone => {
                self.ui.anim_ms = now_ms();
                for id in std::mem::take(&mut self.ui.animated) {
                    self.ui.damage_node(id);
                }
            }
            PEvent::BufferFree => {}
            PEvent::PointerMove { x, y } => ui.pointer_move(x / s, y / s),
            PEvent::PointerLeave => ui.pointer_leave(),
            PEvent::Button { button, pressed, x, y } => ui.pointer_button(button, pressed, x / s, y / s),
            PEvent::Scroll { dx, dy } => ui.wheel(dx / s, dy / s),
            PEvent::Key { sym, text, pressed: true, mods, .. } => {
                ui.mods = mods;
                ui.key(sym, &text, mods);
            }
            PEvent::Key { .. } => {}
            PEvent::Mods(m) => {
                self.ui.mods = m;
                if let Some(o) = d.overlay_ui() {
                    o.mods = m;
                }
            }
            PEvent::FocusIn | PEvent::FocusOut => {}
            PEvent::Paste(t) => {
                self.paste_pending = false;
                let over_paste = d.overlay_ui().is_some_and(|o| o.paste_into != 0);
                if over_paste {
                    d.overlay_ui().unwrap().paste(t);
                } else if self.ui.paste_into != 0 {
                    self.ui.paste(t);
                } else if !self.ui.app_paste.is_empty() {
                    d.pasted(&mut self.ui, t);
                }
            }
        }
        false
    }

    /// Lays out and redraws what changed. Returns false when no buffer was free.
    fn draw(&mut self, d: &mut dyn Driver) -> bool {
        self.ui.layout();
        let mut over_damage = Vec::new();
        let over_now = match d.overlay_ui() {
            Some(o) if o.root != 0 => {
                o.match_window(&self.ui);
                o.layout();
                over_damage = std::mem::take(&mut o.damage);
                true
            }
            _ => false,
        };
        if over_now != self.overlay_was {
            self.overlay_was = over_now;
            self.ui.damage_all();
        }
        self.ui.damage.extend(over_damage);
        if self.ui.damage.is_empty() {
            return true;
        }
        let s = self.ui.scale;
        let (ww, wh, _) = self.p.size();
        let win = Rect::new(0, 0, ww, wh);
        let mut rects: Vec<Rect> = self
            .ui
            .damage
            .iter()
            .map(|r| RectF::new(r.x * s, r.y * s, r.w * s, r.h * s).outer().intersect(&win))
            .collect();
        merge(&mut rects);
        if rects.is_empty() {
            self.ui.damage.clear();
            return true;
        }
        let Some(fr) = self.p.frame() else { return false };
        self.ui.damage.clear();
        let age = fr.age as usize;
        // The buffer is `age` frames old: repaint what changed since then too.
        let mut draw = rects.clone();
        if age == 0 || age > self.history.len() + 1 || (fr.w, fr.h) != (ww, wh) {
            draw = vec![win];
        } else {
            for h in self.history.iter().take(age - 1) {
                draw.extend_from_slice(h);
            }
            merge(&mut draw);
        }
        let bbox = draw.iter().fold(Rect::default(), |a, r| a.union(r));
        let lb = RectF::new(bbox.x as f32 / s, bbox.y as f32 / s, bbox.w as f32 / s, bbox.h as f32 / s);
        let t = std::time::Instant::now();
        let mut list = self.ui.paint(lb);
        d.overlay(&mut self.ui, &mut list);
        if self.csd {
            let t = self.last_title.clone();
            let hover = self.close_hover;
            self.ui.title_bar(&mut list, &t, hover);
        }
        let olist = match d.overlay_ui() {
            Some(o) if o.root != 0 => Some(o.paint(lb)),
            _ => None,
        };
        let (fw, fh, stride) = (fr.w, fr.h, fr.stride);
        let mut canvas = Canvas::new(fr.px, fw, fh, stride);
        for r in &draw {
            render(&list, &mut canvas, *r, &self.ui.fonts, &mut self.cache, d.pixmaps());
            if let (Some(ol), Some(o)) = (&olist, d.overlay_ui()) {
                render(ol, &mut canvas, *r, &o.fonts, &mut self.ocache, &crate::gfx::NoPixmaps);
            }
        }
        if !self.ui.animated.is_empty() {
            self.p.request_frame();
        }
        self.p.present(&rects);
        if self.debug {
            eprintln!("frame: damage {:?} drawn {:?} age {age} cmds {} in {:?}", rects, draw, list.len(), t.elapsed());
        }
        self.history.push_front(rects);
        self.history.truncate(4);
        true
    }

    pub fn run(&mut self, d: &mut dyn Driver) -> i32 {
        let mut evs = Vec::new();
        self.p.dispatch(&mut evs);
        for e in evs.drain(..) {
            if let PEvent::Resize { w, h, scale } = e {
                self.ui.set_window(w, h, scale);
            }
        }
        self.csd = !self.p.has_server_decorations() || std::env::var_os("QA_CSD").is_some();
        if self.csd {
            self.ui.top_inset = TITLE_BAR;
            self.p.set_min_size(200, 120 + TITLE_BAR as i32);
        }
        d.start(&mut self.ui);
        loop {
            for e in std::mem::take(&mut evs) {
                if self.handle(d, e) {
                    return 0;
                }
            }
            if self.themes.changed() {
                self.ui.theme.reload();
                self.ui.invalidate_all();
                self.ui.damage_all();
                d.theme_changed();
            }
            d.poll(&mut self.ui);
            let out = self.ui.take_events();
            if !out.is_empty() {
                d.events(&mut self.ui, out);
            }
            let oevs = d.overlay_ui().map(|o| o.take_events()).unwrap_or_default();
            if !oevs.is_empty() {
                d.overlay_events(&mut self.ui, oevs);
            }
            if let Some(t) = d.take_title() {
                if t != self.last_title {
                    self.p.set_title(&t);
                    self.last_title = t;
                }
            }
            if std::mem::take(&mut self.ui.quit_requested) && d.close(&mut self.ui) {
                return 0;
            }
            if self.ui.quit || d.quit() {
                return 0;
            }
            if let Some(text) = self.ui.clipboard_out.take() {
                self.p.clipboard_set(text);
            }
            let over_paste = d.overlay_ui().is_some_and(|o| o.paste_into != 0);
            if (self.ui.paste_into != 0 || over_paste || !self.ui.app_paste.is_empty()) && !self.paste_pending {
                self.paste_pending = true;
                self.p.clipboard_get();
            }
            let cursor = match d.overlay_ui() {
                Some(o) if o.root != 0 => o.cursor,
                _ => self.ui.cursor,
            };
            self.p.set_cursor(cursor);
            self.draw(d);
            // Layout may emit events (NearEnd).
            let out = self.ui.take_events();
            if !out.is_empty() {
                d.events(&mut self.ui, out);
                self.draw(d);
            }
            self.p.flush();
            let mut fds = vec![
                libc::pollfd { fd: self.p.fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.themes.fd, events: libc::POLLIN, revents: 0 },
            ];
            for fd in d.fds() {
                fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
            }
            let timeout = d.timeout_ms().unwrap_or(-1);
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
            if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return 1;
            }
            self.p.dispatch(&mut evs);
        }
    }
}

fn is_input(e: &PEvent) -> bool {
    matches!(e, PEvent::PointerMove { .. } | PEvent::PointerLeave | PEvent::Button { .. } | PEvent::Scroll { .. } | PEvent::Key { .. })
}
