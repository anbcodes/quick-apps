//! The macOS window, through a small Cocoa shim (shim.m). It draws by handing the layer an
//! image of the frame, and it waits for events with the Cocoa run loop, so `wait` is
//! overridden: window events can't be polled with a file descriptor.

use super::{Cursor, Edge, Frame, PEvent, Platform, Rect};
use std::ffi::{c_char, c_double, c_int, c_uint, CString};
use std::io;
use std::os::fd::RawFd;

#[repr(C)]
#[derive(Clone, Copy)]
struct QaEv {
    kind: c_int,
    a: c_double,
    b: c_double,
    c: c_double,
    button: c_uint,
    sym: c_uint,
    mods: c_uint,
    pressed: c_int,
    repeat: c_int,
    text: [c_char; 16],
}

extern "C" {
    fn qa_open(title: *const c_char, autosave: *const c_char, w: c_int, h: c_int) -> c_int;
    fn qa_poll(out: *mut QaEv, max: c_int) -> c_int;
    fn qa_size(w: *mut c_int, h: *mut c_int, scale: *mut c_double);
    fn qa_wait(fds: *const c_int, n: c_int, ms: c_int);
    fn qa_present(px: *const u32, w: c_int, h: c_int, stride: usize);
    fn qa_close();
    fn qa_set_title(t: *const c_char);
    fn qa_set_cursor(kind: c_int);
    fn qa_set_min_size(w: c_int, h: c_int);
    fn qa_clip_set(t: *const c_char);
    fn qa_clip_get() -> *mut c_char;
    fn qa_wait_url(ms: c_int) -> *mut c_char;
}

pub struct Mac {
    buf: Vec<u32>,
    w: i32,
    h: i32,
    /// Whether the buffer still holds the last frame that was presented.
    fresh: bool,
    /// Logical size the window opened at.
    pub default_size: (i32, i32),
    /// When the next `FrameDone` is due, in `sys::now_ms` time.
    frame_due: Option<u64>,
    paste: bool,
    /// Never readable. `fd()` has to return something.
    idle: RawFd,
    pending: Vec<PEvent>,
    title: String,
    app_id: String,
}

const FRAME_MS: u64 = 16;

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Mac {
    pub fn connect(title: &str, app_id: &str) -> io::Result<Mac> {
        // The window opens on first use, after the caller has set `default_size`.
        let idle = crate::sys::new_wake();
        Ok(Mac { buf: Vec::new(), w: 0, h: 0, fresh: false, default_size: (900, 600), frame_due: None, paste: false, idle, pending: Vec::new(), title: title.to_string(), app_id: app_id.to_string() })
    }

    fn ensure_open(&mut self) {
        if self.w != 0 {
            return;
        }
        let (w, h) = self.default_size;
        unsafe { qa_open(cstr(&self.title).as_ptr(), cstr(&self.app_id).as_ptr(), w, h) };
        self.pull();
    }

    fn pull(&mut self) {
        let mut evs = [QaEv { kind: 0, a: 0.0, b: 0.0, c: 0.0, button: 0, sym: 0, mods: 0, pressed: 0, repeat: 0, text: [0; 16] }; 64];
        loop {
            let n = unsafe { qa_poll(evs.as_mut_ptr(), evs.len() as c_int) } as usize;
            for e in &evs[..n] {
                let text = unsafe { std::ffi::CStr::from_ptr(e.text.as_ptr()) }.to_string_lossy().into_owned();
                let ev = match e.kind {
                    1 => {
                        self.w = e.a as i32;
                        self.h = e.b as i32;
                        PEvent::Resize { w: e.a as i32, h: e.b as i32, scale: e.c as f32 }
                    }
                    2 => PEvent::Close,
                    3 => PEvent::PointerMove { x: e.a as f32, y: e.b as f32 },
                    4 => PEvent::PointerLeave,
                    5 => PEvent::Button { button: e.button, pressed: e.pressed != 0, x: e.a as f32, y: e.b as f32 },
                    6 => PEvent::Scroll { dx: e.a as f32, dy: e.b as f32 },
                    7 => PEvent::Key { sym: e.sym, text, pressed: e.pressed != 0, repeat: e.repeat != 0, mods: e.mods as u8 },
                    8 => PEvent::Mods(e.mods as u8),
                    9 => PEvent::FocusIn,
                    10 => PEvent::FocusOut,
                    _ => continue,
                };
                self.pending.push(ev);
            }
            if n < evs.len() {
                break;
            }
        }
    }
}

/// Waits for the `qa:` link the system hands the URL-handler app, if one comes.
pub fn wait_url(ms: i32) -> Option<String> {
    let p = unsafe { qa_wait_url(ms) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { libc::free(p as *mut _) };
    Some(s)
}

impl Drop for Mac {
    fn drop(&mut self) {
        if self.w != 0 {
            unsafe { qa_close() };
        }
        unsafe { libc::close(self.idle) };
    }
}

impl Platform for Mac {
    fn fd(&self) -> RawFd {
        self.idle
    }

    fn wait(&mut self, extra: &[RawFd], timeout_ms: i32) -> bool {
        self.ensure_open();
        let mut ms = timeout_ms;
        if let Some(due) = self.frame_due {
            let left = due.saturating_sub(crate::sys::now_ms()) as i32;
            ms = if ms < 0 { left } else { ms.min(left) };
        }
        if self.paste || !self.pending.is_empty() {
            ms = 0;
        }
        unsafe { qa_wait(extra.as_ptr(), extra.len() as c_int, ms) };
        true
    }

    fn dispatch(&mut self, out: &mut Vec<PEvent>) {
        self.ensure_open();
        self.pull();
        out.append(&mut self.pending);
        if let Some(due) = self.frame_due {
            if crate::sys::now_ms() >= due {
                self.frame_due = None;
                out.push(PEvent::FrameDone);
            }
        }
        if std::mem::take(&mut self.paste) {
            let p = unsafe { qa_clip_get() };
            let text = if p.is_null() {
                String::new()
            } else {
                let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
                unsafe { libc::free(p as *mut _) };
                s
            };
            out.push(PEvent::Paste(text));
        }
    }

    fn flush(&mut self) {}

    fn frame(&mut self) -> Option<Frame<'_>> {
        self.ensure_open();
        let (mut w, mut h, mut s) = (0, 0, 0.0);
        unsafe { qa_size(&mut w, &mut h, &mut s) };
        if w <= 0 || h <= 0 {
            return None;
        }
        let age = if self.buf.len() != (w * h) as usize || (w, h) != (self.w, self.h) {
            self.buf = vec![0; (w * h) as usize];
            self.fresh = false;
            0
        } else if self.fresh {
            1
        } else {
            0
        };
        self.w = w;
        self.h = h;
        Some(Frame { px: &mut self.buf, w, h, stride: w as usize, age })
    }

    fn present(&mut self, _damage: &[Rect]) {
        if self.buf.is_empty() {
            return;
        }
        unsafe { qa_present(self.buf.as_ptr(), self.w, self.h, self.w as usize) };
        self.fresh = true;
    }

    fn request_frame(&mut self) {
        self.frame_due.get_or_insert(crate::sys::now_ms() + FRAME_MS);
    }

    fn size(&self) -> (i32, i32, f32) {
        let (mut w, mut h, mut s) = (0, 0, 1.0);
        if self.w != 0 {
            unsafe { qa_size(&mut w, &mut h, &mut s) };
        }
        (w, h, s as f32)
    }

    fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        self.ensure_open();
        unsafe { qa_set_title(cstr(title).as_ptr()) };
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let k = match cursor {
            Cursor::Default => 0,
            Cursor::Pointer => 1,
            Cursor::Text => 2,
            Cursor::Move => 3,
            Cursor::NotAllowed => 4,
            Cursor::ResizeEW => 5,
            Cursor::ResizeNS => 6,
            Cursor::ResizeNWSE => 7,
            Cursor::ResizeNESW => 8,
        };
        if self.w != 0 {
            unsafe { qa_set_cursor(k) };
        }
    }

    fn set_min_size(&mut self, w: i32, h: i32) {
        if self.w != 0 {
            unsafe { qa_set_min_size(w, h) };
        }
    }

    // The system draws the title bar and handles moving and resizing.
    fn begin_move(&mut self) {}
    fn begin_resize(&mut self, _edge: Edge) {}

    fn has_server_decorations(&self) -> bool {
        true
    }

    fn clipboard_set(&mut self, text: String) {
        unsafe { qa_clip_set(cstr(&text).as_ptr()) };
    }

    fn clipboard_get(&mut self) {
        self.paste = true;
    }
}
