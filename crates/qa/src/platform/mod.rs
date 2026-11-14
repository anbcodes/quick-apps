//! The seam between the runtime and the windowing system.
//!
//! Coordinates in events and frames are physical pixels.

#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub mod x11;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::Mac as Native;

use std::os::fd::RawFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.w as i64 * self.h as i64
        }
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0), (b - y).max(0))
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(x, y, r - x, b - y)
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && y >= self.y as f32 && x < self.right() as f32 && y < self.bottom() as f32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PEvent {
    /// The window changed size or scale. `w`/`h` are physical pixels.
    Resize { w: i32, h: i32, scale: f32 },
    Close,
    /// A frame callback fired: a good time to draw the next animation frame.
    FrameDone,
    /// A buffer came back from the compositor; a blocked draw can retry.
    BufferFree,
    PointerMove { x: f32, y: f32 },
    PointerLeave,
    Button { button: u32, pressed: bool, x: f32, y: f32 },
    /// Scroll deltas in physical pixels. Positive `dy` scrolls down.
    Scroll { dx: f32, dy: f32 },
    Key { sym: u32, text: String, pressed: bool, repeat: bool, mods: u8 },
    Mods(u8),
    FocusIn,
    FocusOut,
    Paste(String),
}

pub const BUTTON_LEFT: u32 = 0x110;
pub const BUTTON_RIGHT: u32 = 0x111;
pub const BUTTON_MIDDLE: u32 = 0x112;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    ResizeEW,
    ResizeNS,
    ResizeNWSE,
    ResizeNESW,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub struct Frame<'a> {
    /// Premultiplied ARGB8888, row-major.
    pub px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
    /// In pixels, not bytes.
    pub stride: usize,
    /// How many frames old the buffer's contents are. 0 means unknown: redraw everything.
    pub age: u32,
}

pub trait Platform {
    /// A file descriptor the main loop polls for readability.
    fn fd(&self) -> RawFd;
    /// Blocks until the platform has events or one of `extra` is readable, or for `timeout_ms`
    /// (-1 for no limit). Platforms that can't poll their event source override this.
    fn wait(&mut self, extra: &[RawFd], timeout_ms: i32) -> bool {
        let mut fds = vec![libc::pollfd { fd: self.fd(), events: libc::POLLIN, revents: 0 }];
        fds.extend(extra.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }));
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout_ms) };
        r >= 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
    }
    /// Reads and handles everything pending, without blocking.
    fn dispatch(&mut self, out: &mut Vec<PEvent>);
    /// Sends queued requests. Call before sleeping in poll().
    fn flush(&mut self);
    /// The next buffer to draw into, or None when none is free yet.
    fn frame(&mut self) -> Option<Frame<'_>>;
    /// Shows the buffer returned by the last `frame()`. `damage` is in buffer pixels.
    fn present(&mut self, damage: &[Rect]);
    /// Asks for a `FrameDone` after the next present.
    fn request_frame(&mut self);
    fn size(&self) -> (i32, i32, f32);
    fn set_title(&mut self, title: &str);
    /// Shows a PNG as the window's icon, where the window system has such a thing.
    fn set_icon(&mut self, _png: &[u8]) {}
    fn set_cursor(&mut self, cursor: Cursor);
    fn set_min_size(&mut self, w: i32, h: i32);
    fn begin_move(&mut self);
    fn begin_resize(&mut self, edge: Edge);
    /// Whether the window manager draws the title bar.
    fn has_server_decorations(&self) -> bool;
    fn clipboard_set(&mut self, text: String);
    /// Answered later with `PEvent::Paste`.
    fn clipboard_get(&mut self);
}

/// The window system in use on Linux, chosen when the window opens.
#[cfg(target_os = "linux")]
pub enum Native {
    Wayland(Box<wayland::Wayland>),
    X11(Box<x11::X11>),
}

#[cfg(target_os = "linux")]
macro_rules! each {
    ($s:expr, $p:ident => $e:expr) => {
        match $s {
            Native::Wayland($p) => $e,
            Native::X11($p) => $e,
        }
    };
}

#[cfg(target_os = "linux")]
impl Native {
    /// Wayland if the session has it, else X11. `QA_BACKEND=wayland` or `x11` forces one.
    pub fn connect(title: &str, app_id: &str) -> std::io::Result<Native> {
        let way = || wayland::Wayland::connect(title, app_id).map(|w| Native::Wayland(Box::new(w)));
        let x = || x11::X11::connect(title, app_id).map(|x| Native::X11(Box::new(x)));
        match std::env::var("QA_BACKEND").as_deref() {
            Ok("wayland") => return way(),
            Ok("x11") => return x(),
            _ => {}
        }
        let has_x = std::env::var_os("DISPLAY").is_some();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() || !has_x {
            match way() {
                Ok(n) => return Ok(n),
                Err(e) if !has_x => return Err(e),
                Err(e) => {
                    return x().map_err(|e2| std::io::Error::other(format!("{e}; and X11: {e2}")));
                }
            }
        }
        x()
    }

    pub fn set_default_size(&mut self, size: (i32, i32)) {
        each!(self, p => p.default_size = size)
    }
}

#[cfg(target_os = "linux")]
impl Platform for Native {
    fn fd(&self) -> RawFd {
        each!(self, p => p.fd())
    }
    fn wait(&mut self, extra: &[RawFd], timeout_ms: i32) -> bool {
        each!(self, p => p.wait(extra, timeout_ms))
    }
    fn dispatch(&mut self, out: &mut Vec<PEvent>) {
        each!(self, p => p.dispatch(out))
    }
    fn flush(&mut self) {
        each!(self, p => p.flush())
    }
    fn frame(&mut self) -> Option<Frame<'_>> {
        each!(self, p => p.frame())
    }
    fn present(&mut self, damage: &[Rect]) {
        each!(self, p => p.present(damage))
    }
    fn request_frame(&mut self) {
        each!(self, p => p.request_frame())
    }
    fn size(&self) -> (i32, i32, f32) {
        each!(self, p => p.size())
    }
    fn set_title(&mut self, title: &str) {
        each!(self, p => p.set_title(title))
    }
    fn set_icon(&mut self, png: &[u8]) {
        each!(self, p => p.set_icon(png))
    }
    fn set_cursor(&mut self, cursor: Cursor) {
        each!(self, p => p.set_cursor(cursor))
    }
    fn set_min_size(&mut self, w: i32, h: i32) {
        each!(self, p => p.set_min_size(w, h))
    }
    fn begin_move(&mut self) {
        each!(self, p => p.begin_move())
    }
    fn begin_resize(&mut self, edge: Edge) {
        each!(self, p => p.begin_resize(edge))
    }
    fn has_server_decorations(&self) -> bool {
        each!(self, p => p.has_server_decorations())
    }
    fn clipboard_set(&mut self, text: String) {
        each!(self, p => p.clipboard_set(text))
    }
    fn clipboard_get(&mut self) {
        each!(self, p => p.clipboard_get())
    }
}
