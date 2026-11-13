//! A hand-written Wayland client over libc.

mod wire;
mod xkb;

use super::{Cursor, Edge, Frame, PEvent, Platform, Rect};
use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex};
use wire::{Arg, Conn, Rd};

const DISPLAY: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Obj {
    Registry,
    SyncCb,
    FrameCb,
    Surface,
    Buffer,
    WmBase,
    XdgSurface,
    Toplevel,
    Deco,
    Frac,
    Seat,
    Pointer,
    Keyboard,
    DataDevice,
    DataSource,
    DataOffer,
    Other,
}

struct Buf {
    id: u32,
    pool: u32,
    fd: RawFd,
    ptr: *mut u8,
    cap: usize,
    w: i32,
    h: i32,
    busy: bool,
    drawn: u64,
}

#[derive(Default)]
struct Globals {
    compositor: (u32, u32),
    shm: u32,
    wm_base: u32,
    deco_mgr: u32,
    viewporter: u32,
    frac_mgr: u32,
    cursor_mgr: u32,
    seat: (u32, u32),
    data_mgr: u32,
}

pub struct Wayland {
    c: Conn,
    objs: HashMap<u32, Obj>,
    g: Globals,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    viewport: u32,
    frac: u32,
    deco: u32,
    server_deco: bool,
    pointer: u32,
    keyboard: u32,
    cursor_dev: u32,
    data_device: u32,
    data_source: u32,
    clip_text: Arc<String>,
    /// The current selection offer and whether it has text.
    sel_offer: u32,
    offer_text: HashMap<u32, bool>,
    paste: Arc<Mutex<Option<String>>>,
    paste_efd: RawFd,

    pend_w: i32,
    pend_h: i32,
    w: i32,
    h: i32,
    scale120: u32,
    int_scale: i32,
    configured: bool,
    /// The size used when the compositor leaves it to us.
    pub default_size: (i32, i32),
    sent_size: (i32, i32, u32),
    applied_dest: (i32, i32),
    applied_scale: i32,

    bufs: Vec<Buf>,
    cur: Option<usize>,
    frame_no: u64,
    want_frame: bool,
    frame_cb: u32,

    ptr_x: f32,
    ptr_y: f32,
    ptr_serial: u32,
    input_serial: u32,
    cursor: Cursor,
    scroll: (f32, f32),

    xkb: Option<xkb::Xkb>,
    repeat: (i32, i32),
    repeat_key: u32,
    timerfd: RawFd,
    epfd: RawFd,
    dead: bool,
    trace: bool,
    /// Events produced during startup roundtrips, handed out by the first dispatch.
    early: Vec<PEvent>,
}

fn cvt(r: i32) -> io::Result<i32> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

impl Drop for Wayland {
    fn drop(&mut self) {
        for b in std::mem::take(&mut self.bufs) {
            unsafe {
                libc::munmap(b.ptr as *mut _, b.cap);
                libc::close(b.fd);
            }
        }
        unsafe {
            libc::close(self.epfd);
            libc::close(self.timerfd);
            libc::close(self.paste_efd);
        }
    }
}

impl Wayland {
    pub fn connect(title: &str, app_id: &str) -> io::Result<Wayland> {
        let c = Conn::connect()?;
        let (epfd, timerfd, paste_efd) = unsafe {
            let epfd = cvt(libc::epoll_create1(libc::EPOLL_CLOEXEC))?;
            let tfd = cvt(libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_CLOEXEC | libc::TFD_NONBLOCK))?;
            let efd = cvt(libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK))?;
            for fd in [c.fd, tfd, efd] {
                let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: fd as u64 };
                cvt(libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd, &mut ev))?;
            }
            (epfd, tfd, efd)
        };
        let mut w = Wayland {
            c,
            objs: HashMap::new(),
            g: Globals::default(),
            surface: 0,
            xdg_surface: 0,
            toplevel: 0,
            viewport: 0,
            frac: 0,
            deco: 0,
            server_deco: false,
            pointer: 0,
            keyboard: 0,
            cursor_dev: 0,
            data_device: 0,
            data_source: 0,
            clip_text: Arc::new(String::new()),
            sel_offer: 0,
            offer_text: HashMap::new(),
            paste: Arc::new(Mutex::new(None)),
            paste_efd,
            pend_w: 0,
            pend_h: 0,
            w: 0,
            h: 0,
            scale120: 120,
            int_scale: 1,
            configured: false,
            default_size: (900, 600),
            sent_size: (0, 0, 0),
            applied_dest: (0, 0),
            applied_scale: 1,
            bufs: Vec::new(),
            cur: None,
            frame_no: 0,
            want_frame: false,
            frame_cb: 0,
            ptr_x: 0.0,
            ptr_y: 0.0,
            ptr_serial: 0,
            input_serial: 0,
            cursor: Cursor::Default,
            scroll: (0.0, 0.0),
            xkb: xkb::Xkb::load(),
            repeat: (25, 600),
            repeat_key: 0,
            timerfd,
            epfd,
            dead: false,
            trace: std::env::var_os("QA_WAYLAND_DEBUG").is_some(),
            early: Vec::new(),
        };
        w.init(title, app_id)?;
        Ok(w)
    }

    fn new_obj(&mut self, kind: Obj) -> u32 {
        let id = self.c.new_id();
        self.objs.insert(id, kind);
        id
    }

    fn roundtrip(&mut self, globals: &mut Vec<(u32, String, u32)>) -> io::Result<()> {
        let cb = self.new_obj(Obj::SyncCb);
        self.c.send(DISPLAY, 0, &[Arg::N(cb)]);
        self.c.flush()?;
        loop {
            let mut p = libc::pollfd { fd: self.c.fd, events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut p, 1, -1) };
            self.c.read()?;
            let mut out = std::mem::take(&mut self.early);
            while let Some((obj, op, body)) = self.c.next() {
                if obj == cb && op == 0 {
                    self.objs.remove(&cb);
                    // Keep anything that arrived after the callback.
                    self.handle_all(&mut out, Some(globals));
                    self.early = out;
                    return Ok(());
                }
                self.handle(obj, op, &body, &mut out, Some(globals));
            }
            self.early = out;
        }
    }

    fn handle_all(&mut self, out: &mut Vec<PEvent>, mut globals: Option<&mut Vec<(u32, String, u32)>>) {
        while let Some((obj, op, body)) = self.c.next() {
            self.handle(obj, op, &body, out, globals.as_deref_mut());
        }
    }

    fn init(&mut self, title: &str, app_id: &str) -> io::Result<()> {
        let reg = self.new_obj(Obj::Registry);
        self.c.send(DISPLAY, 1, &[Arg::N(reg)]);
        let mut globals = Vec::new();
        self.roundtrip(&mut globals)?;
        for (name, iface, ver) in &globals {
            let (name, ver) = (*name, *ver);
            let bind = |me: &mut Self, kind: Obj, v: u32| {
                let id = me.new_obj(kind);
                me.c.send(reg, 0, &[Arg::U(name), Arg::S(iface), Arg::U(v), Arg::N(id)]);
                id
            };
            match iface.as_str() {
                "wl_compositor" => {
                    let v = ver.min(6);
                    self.g.compositor = (bind(self, Obj::Other, v), v);
                }
                "wl_shm" => self.g.shm = bind(self, Obj::Other, 1),
                "xdg_wm_base" => self.g.wm_base = bind(self, Obj::WmBase, ver.min(5)),
                "zxdg_decoration_manager_v1" => self.g.deco_mgr = bind(self, Obj::Other, 1),
                "wp_viewporter" => self.g.viewporter = bind(self, Obj::Other, 1),
                "wp_fractional_scale_manager_v1" => self.g.frac_mgr = bind(self, Obj::Other, 1),
                "wp_cursor_shape_manager_v1" => self.g.cursor_mgr = bind(self, Obj::Other, 1),
                "wl_seat" if self.g.seat.0 == 0 => {
                    let v = ver.min(7);
                    self.g.seat = (bind(self, Obj::Seat, v), v);
                }
                "wl_data_device_manager" => self.g.data_mgr = bind(self, Obj::Other, ver.min(3)),
                _ => {}
            }
        }
        let missing = [
            (self.g.compositor.0, "wl_compositor"),
            (self.g.shm, "wl_shm"),
            (self.g.wm_base, "xdg_wm_base"),
        ];
        for (id, name) in missing {
            if id == 0 {
                return Err(io::Error::new(io::ErrorKind::Unsupported, format!("compositor lacks {name}")));
            }
        }

        self.surface = self.new_obj(Obj::Surface);
        self.c.send(self.g.compositor.0, 0, &[Arg::N(self.surface)]);
        self.xdg_surface = self.new_obj(Obj::XdgSurface);
        self.c.send(self.g.wm_base, 2, &[Arg::N(self.xdg_surface), Arg::O(self.surface)]);
        self.toplevel = self.new_obj(Obj::Toplevel);
        self.c.send(self.xdg_surface, 1, &[Arg::N(self.toplevel)]);
        self.c.send(self.toplevel, 2, &[Arg::S(title)]);
        self.c.send(self.toplevel, 3, &[Arg::S(app_id)]);
        self.c.send(self.toplevel, 8, &[Arg::I(200), Arg::I(120)]);
        if self.g.deco_mgr != 0 {
            self.deco = self.new_obj(Obj::Deco);
            self.c.send(self.g.deco_mgr, 1, &[Arg::N(self.deco), Arg::O(self.toplevel)]);
            self.c.send(self.deco, 1, &[Arg::U(2)]);
        }
        if self.g.frac_mgr != 0 && self.g.viewporter != 0 {
            self.frac = self.new_obj(Obj::Frac);
            self.c.send(self.g.frac_mgr, 1, &[Arg::N(self.frac), Arg::O(self.surface)]);
            self.viewport = self.new_obj(Obj::Other);
            self.c.send(self.g.viewporter, 1, &[Arg::N(self.viewport), Arg::O(self.surface)]);
        }
        self.c.send(self.surface, 6, &[]);
        self.roundtrip(&mut Vec::new())?;
        Ok(())
    }

    fn scale(&self) -> f32 {
        if self.frac != 0 {
            self.scale120 as f32 / 120.0
        } else {
            self.int_scale as f32
        }
    }

    fn phys(&self) -> (i32, i32) {
        if self.frac != 0 {
            let s = self.scale120 as i64;
            (((self.w as i64 * s + 60) / 120) as i32, ((self.h as i64 * s + 60) / 120) as i32)
        } else {
            (self.w * self.int_scale, self.h * self.int_scale)
        }
    }

    fn maybe_resize(&mut self, out: &mut Vec<PEvent>) {
        if !self.configured {
            return;
        }
        let key = (self.w, self.h, if self.frac != 0 { self.scale120 } else { self.int_scale as u32 * 120 });
        if key != self.sent_size {
            self.sent_size = key;
            let (w, h) = self.phys();
            out.push(PEvent::Resize { w, h, scale: self.scale() });
        }
    }

    fn handle(&mut self, obj: u32, op: u16, body: &[u8], out: &mut Vec<PEvent>, globals: Option<&mut Vec<(u32, String, u32)>>) {
        let mut r = Rd::new(body);
        if self.trace {
            eprintln!("<- {obj} {:?} op {op} ({} bytes)", self.objs.get(&obj), body.len());
        }
        if obj == DISPLAY {
            match op {
                0 => {
                    let (o, code, msg) = (r.u32(), r.u32(), r.str());
                    eprintln!("qa: wayland error on object {o} ({:?}), code {code}: {msg}", self.objs.get(&o));
                    std::process::exit(1);
                }
                1 => {
                    let id = r.u32();
                    if self.objs.remove(&id).is_some() || id < 0xff00_0000 {
                        self.c.free_id(id);
                    }
                }
                _ => {}
            }
            return;
        }
        let Some(&kind) = self.objs.get(&obj) else { return };
        match (kind, op) {
            (Obj::Registry, 0) => {
                if let Some(g) = globals {
                    let (name, iface, ver) = (r.u32(), r.str().to_string(), r.u32());
                    g.push((name, iface, ver));
                }
            }
            (Obj::FrameCb, 0) => {
                self.objs.remove(&obj);
                if self.frame_cb == obj {
                    self.frame_cb = 0;
                }
                out.push(PEvent::FrameDone);
            }
            (Obj::WmBase, 0) => {
                let serial = r.u32();
                self.c.send(self.g.wm_base, 3, &[Arg::U(serial)]);
            }
            (Obj::XdgSurface, 0) => {
                let serial = r.u32();
                self.c.send(self.xdg_surface, 4, &[Arg::U(serial)]);
                if self.pend_w > 0 && self.pend_h > 0 {
                    self.w = self.pend_w;
                    self.h = self.pend_h;
                } else if self.w == 0 {
                    (self.w, self.h) = self.default_size;
                }
                self.configured = true;
                self.maybe_resize(out);
            }
            (Obj::Toplevel, 0) => {
                self.pend_w = r.i32();
                self.pend_h = r.i32();
            }
            (Obj::Toplevel, 1) => out.push(PEvent::Close),
            (Obj::Deco, 0) => self.server_deco = r.u32() == 2,
            (Obj::Frac, 0) => {
                self.scale120 = r.u32().max(1);
                self.maybe_resize(out);
            }
            (Obj::Surface, 2) => {
                self.int_scale = r.i32().max(1);
                if self.frac == 0 {
                    self.maybe_resize(out);
                }
            }
            (Obj::Buffer, 0) => {
                if let Some(b) = self.bufs.iter_mut().find(|b| b.id == obj) {
                    b.busy = false;
                    out.push(PEvent::BufferFree);
                } else {
                    // A buffer replaced while the compositor held it.
                    self.c.send(obj, 0, &[]);
                    self.objs.remove(&obj);
                }
            }
            (Obj::Seat, 0) => self.seat_caps(r.u32()),
            (Obj::Pointer, _) => self.pointer_event(op, &mut r, out),
            (Obj::Keyboard, _) => self.keyboard_event(op, &mut r, out),
            (Obj::DataDevice, 0) => {
                let id = r.u32();
                self.objs.insert(id, Obj::DataOffer);
                self.offer_text.insert(id, false);
            }
            (Obj::DataDevice, 5) => {
                let id = r.u32();
                if self.sel_offer != 0 && self.sel_offer != id {
                    self.destroy_offer(self.sel_offer);
                }
                self.sel_offer = id;
            }
            (Obj::DataOffer, 0) => {
                let mime = r.str();
                if mime == "text/plain;charset=utf-8" || mime == "UTF8_STRING" || mime == "text/plain" {
                    self.offer_text.insert(obj, true);
                }
            }
            (Obj::DataSource, 1) => {
                let _mime = r.str();
                if let Some(fd) = self.c.in_fds.pop_front() {
                    let text = self.clip_text.clone();
                    std::thread::spawn(move || unsafe {
                        let b = text.as_bytes();
                        let mut off = 0;
                        while off < b.len() {
                            let n = libc::write(fd, b[off..].as_ptr() as *const _, b.len() - off);
                            if n <= 0 {
                                break;
                            }
                            off += n as usize;
                        }
                        libc::close(fd);
                    });
                }
            }
            (Obj::DataSource, 2) => {
                self.c.send(obj, 1, &[]);
                self.objs.remove(&obj);
                if self.data_source == obj {
                    self.data_source = 0;
                }
            }
            _ => {}
        }
    }

    fn destroy_offer(&mut self, id: u32) {
        self.c.send(id, 2, &[]);
        self.objs.remove(&id);
        self.offer_text.remove(&id);
    }

    fn seat_caps(&mut self, caps: u32) {
        let seat = self.g.seat.0;
        if caps & 1 != 0 && self.pointer == 0 {
            self.pointer = self.new_obj(Obj::Pointer);
            self.c.send(seat, 0, &[Arg::N(self.pointer)]);
            if self.g.cursor_mgr != 0 {
                self.cursor_dev = self.new_obj(Obj::Other);
                self.c.send(self.g.cursor_mgr, 1, &[Arg::N(self.cursor_dev), Arg::O(self.pointer)]);
            }
        }
        if caps & 2 != 0 && self.keyboard == 0 {
            self.keyboard = self.new_obj(Obj::Keyboard);
            self.c.send(seat, 1, &[Arg::N(self.keyboard)]);
        }
        if self.g.data_mgr != 0 && self.data_device == 0 {
            self.data_device = self.new_obj(Obj::DataDevice);
            self.c.send(self.g.data_mgr, 1, &[Arg::N(self.data_device), Arg::O(seat)]);
        }
    }

    fn to_phys(&self, v: f64) -> f32 {
        (v * self.scale() as f64) as f32
    }

    fn pointer_event(&mut self, op: u16, r: &mut Rd, out: &mut Vec<PEvent>) {
        match op {
            0 => {
                self.ptr_serial = r.u32();
                let _surface = r.u32();
                self.ptr_x = self.to_phys(r.fixed());
                self.ptr_y = self.to_phys(r.fixed());
                self.apply_cursor();
                out.push(PEvent::PointerMove { x: self.ptr_x, y: self.ptr_y });
            }
            1 => out.push(PEvent::PointerLeave),
            2 => {
                let _time = r.u32();
                self.ptr_x = self.to_phys(r.fixed());
                self.ptr_y = self.to_phys(r.fixed());
                out.push(PEvent::PointerMove { x: self.ptr_x, y: self.ptr_y });
            }
            3 => {
                self.input_serial = r.u32();
                let _time = r.u32();
                let button = r.u32();
                let pressed = r.u32() == 1;
                out.push(PEvent::Button { button, pressed, x: self.ptr_x, y: self.ptr_y });
            }
            4 => {
                let _time = r.u32();
                let axis = r.u32();
                let v = self.to_phys(r.fixed());
                if axis == 0 {
                    self.scroll.1 += v;
                } else {
                    self.scroll.0 += v;
                }
            }
            5 => {
                let (dx, dy) = std::mem::take(&mut self.scroll);
                if dx != 0.0 || dy != 0.0 {
                    out.push(PEvent::Scroll { dx, dy });
                }
            }
            _ => {}
        }
        // Pointers older than v5 have no frame event.
        if op == 4 && self.g.seat.1 < 5 {
            let (dx, dy) = std::mem::take(&mut self.scroll);
            out.push(PEvent::Scroll { dx, dy });
        }
    }

    fn set_repeat(&mut self, key: u32) {
        self.repeat_key = key;
        let (rate, delay) = self.repeat;
        let mut spec: libc::itimerspec = unsafe { std::mem::zeroed() };
        if key != 0 && rate > 0 {
            spec.it_value.tv_sec = (delay / 1000) as _;
            spec.it_value.tv_nsec = ((delay % 1000) * 1_000_000) as _;
            let iv = 1_000_000_000i64 / rate as i64;
            spec.it_interval.tv_sec = (iv / 1_000_000_000) as _;
            spec.it_interval.tv_nsec = (iv % 1_000_000_000) as _;
        }
        unsafe { libc::timerfd_settime(self.timerfd, 0, &spec, std::ptr::null_mut()) };
    }

    fn keyboard_event(&mut self, op: u16, r: &mut Rd, out: &mut Vec<PEvent>) {
        match op {
            0 => {
                let format = r.u32();
                let size = r.u32() as usize;
                let Some(fd) = self.c.in_fds.pop_front() else { return };
                if format == 1 {
                    if let Some(x) = self.xkb.as_mut() {
                        unsafe {
                            let p = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_PRIVATE, fd, 0);
                            if p != libc::MAP_FAILED {
                                x.set_keymap(p as *const _);
                                libc::munmap(p, size);
                            }
                        }
                    }
                }
                unsafe { libc::close(fd) };
            }
            1 => {
                self.input_serial = r.u32();
                out.push(PEvent::FocusIn);
            }
            2 => {
                self.set_repeat(0);
                out.push(PEvent::FocusOut);
            }
            3 => {
                self.input_serial = r.u32();
                let _time = r.u32();
                let key = r.u32();
                let pressed = r.u32() == 1;
                let Some(x) = self.xkb.as_ref() else { return };
                let (sym, text, mods, repeats) = (x.sym(key), x.text(key), x.mods(), x.repeats(key));
                out.push(PEvent::Key { sym, text, pressed, repeat: false, mods });
                if pressed && repeats {
                    self.set_repeat(key);
                } else if !pressed && key == self.repeat_key {
                    self.set_repeat(0);
                }
            }
            4 => {
                let _serial = r.u32();
                let (d, l, k, g) = (r.u32(), r.u32(), r.u32(), r.u32());
                if let Some(x) = self.xkb.as_mut() {
                    x.update_mask(d, l, k, g);
                    out.push(PEvent::Mods(x.mods()));
                }
            }
            5 => {
                self.repeat = (r.i32(), r.i32());
            }
            _ => {}
        }
    }

    fn apply_cursor(&mut self) {
        if self.cursor_dev == 0 || self.ptr_serial == 0 {
            return;
        }
        let shape = match self.cursor {
            Cursor::Default => 1,
            Cursor::Pointer => 4,
            Cursor::Text => 9,
            Cursor::Move => 13,
            Cursor::NotAllowed => 15,
            Cursor::ResizeEW => 26,
            Cursor::ResizeNS => 27,
            Cursor::ResizeNESW => 28,
            Cursor::ResizeNWSE => 29,
        };
        self.c.send(self.cursor_dev, 1, &[Arg::U(self.ptr_serial), Arg::U(shape)]);
    }

    fn alloc_buf(&mut self, need: usize) -> io::Result<Buf> {
        unsafe {
            let fd = cvt(libc::memfd_create(c"qa-shm".as_ptr(), libc::MFD_CLOEXEC))?;
            if libc::ftruncate(fd, need as i64) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            let ptr = libc::mmap(std::ptr::null_mut(), need, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
            if ptr == libc::MAP_FAILED {
                let e = io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            let pool = self.new_obj(Obj::Other);
            self.c.send(self.g.shm, 0, &[Arg::N(pool), Arg::Fd(fd), Arg::I(need as i32)]);
            // The fd must stay open until the request is flushed; we keep it for resizes anyway.
            Ok(Buf { id: 0, pool, fd, ptr: ptr as *mut u8, cap: need, w: 0, h: 0, busy: false, drawn: 0 })
        }
    }

    fn free_buf(&mut self, b: Buf) {
        if b.id != 0 {
            self.c.send(b.id, 0, &[]);
            self.objs.remove(&b.id);
        }
        self.c.send(b.pool, 1, &[]);
        // Flush before closing the fd so a pending create_pool still carries a valid fd.
        let _ = self.c.flush();
        unsafe {
            libc::munmap(b.ptr as *mut _, b.cap);
            libc::close(b.fd);
        }
    }

    fn prepare_buf(&mut self, i: usize, pw: i32, ph: i32) -> io::Result<()> {
        let need = pw as usize * ph as usize * 4;
        let b = &self.bufs[i];
        if b.w == pw && b.h == ph && b.id != 0 {
            return Ok(());
        }
        // Reallocate when too small, or when much too big (after shrinking the window).
        if need > b.cap || b.cap > need * 3 + (1 << 20) {
            let old = self.bufs.remove(i);
            if old.busy {
                // Cannot happen: only free buffers are prepared.
                self.bufs.insert(i, old);
                return Err(io::Error::other("buffer busy"));
            }
            self.free_buf(old);
            let nb = self.alloc_buf(need)?;
            self.bufs.insert(i, nb);
        } else if b.id != 0 {
            let id = b.id;
            self.c.send(id, 0, &[]);
            self.objs.remove(&id);
        }
        let id = self.new_obj(Obj::Buffer);
        let b = &mut self.bufs[i];
        self.c.send(b.pool, 0, &[Arg::N(id), Arg::I(0), Arg::I(pw), Arg::I(ph), Arg::I(pw * 4), Arg::U(0)]);
        let b = &mut self.bufs[i];
        b.id = id;
        b.w = pw;
        b.h = ph;
        b.drawn = 0;
        Ok(())
    }
}

impl Platform for Wayland {
    fn fd(&self) -> RawFd {
        self.epfd
    }

    fn dispatch(&mut self, out: &mut Vec<PEvent>) {
        if self.dead {
            return;
        }
        out.append(&mut self.early);
        let mut n = 0u64;
        let got = unsafe { libc::read(self.timerfd, &mut n as *mut u64 as *mut _, 8) };
        if got == 8 && self.repeat_key != 0 {
            if let Some(x) = self.xkb.as_ref() {
                let k = self.repeat_key;
                let (sym, text, mods) = (x.sym(k), x.text(k), x.mods());
                for _ in 0..n.min(4) {
                    out.push(PEvent::Key { sym, text: text.clone(), pressed: true, repeat: true, mods });
                }
            }
        }
        if unsafe { libc::read(self.paste_efd, &mut n as *mut u64 as *mut _, 8) } == 8 {
            if let Some(s) = self.paste.lock().unwrap().take() {
                out.push(PEvent::Paste(s));
            }
        }
        loop {
            match self.c.read() {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("qa: {e}");
                    self.dead = true;
                    out.push(PEvent::Close);
                    return;
                }
            }
        }
        self.handle_all(out, None);
    }

    fn flush(&mut self) {
        if !self.dead && self.c.flush().is_err() {
            self.dead = true;
        }
    }

    fn frame(&mut self) -> Option<Frame<'_>> {
        if !self.configured || self.dead {
            return None;
        }
        let (pw, ph) = self.phys();
        let idx = self
            .bufs
            .iter()
            .position(|b| !b.busy && b.w == pw && b.h == ph)
            .or_else(|| self.bufs.iter().position(|b| !b.busy));
        let idx = match idx {
            Some(i) => i,
            None if self.bufs.len() < 3 => {
                let b = self.alloc_buf(pw as usize * ph as usize * 4).ok()?;
                self.bufs.push(b);
                self.bufs.len() - 1
            }
            None => return None,
        };
        if let Err(e) = self.prepare_buf(idx, pw, ph) {
            eprintln!("qa: cannot allocate a window buffer: {e}");
            return None;
        }
        self.cur = Some(idx);
        let b = &self.bufs[idx];
        let age = if b.drawn == 0 { 0 } else { (self.frame_no + 1 - b.drawn) as u32 };
        let px = unsafe { std::slice::from_raw_parts_mut(b.ptr as *mut u32, pw as usize * ph as usize) };
        Some(Frame { px, w: pw, h: ph, stride: pw as usize, age })
    }

    fn present(&mut self, damage: &[Rect]) {
        let Some(i) = self.cur.take() else { return };
        let id = self.bufs[i].id;
        self.c.send(self.surface, 1, &[Arg::O(id), Arg::I(0), Arg::I(0)]);
        if self.g.compositor.1 >= 4 {
            for d in damage {
                self.c.send(self.surface, 9, &[Arg::I(d.x), Arg::I(d.y), Arg::I(d.w), Arg::I(d.h)]);
            }
        } else {
            self.c.send(self.surface, 2, &[Arg::I(0), Arg::I(0), Arg::I(i32::MAX), Arg::I(i32::MAX)]);
        }
        if self.viewport != 0 {
            if self.applied_dest != (self.w, self.h) {
                self.applied_dest = (self.w, self.h);
                self.c.send(self.viewport, 2, &[Arg::I(self.w), Arg::I(self.h)]);
            }
        } else if self.applied_scale != self.int_scale && self.g.compositor.1 >= 3 {
            self.applied_scale = self.int_scale;
            self.c.send(self.surface, 8, &[Arg::I(self.int_scale)]);
        }
        if self.want_frame && self.frame_cb == 0 {
            self.want_frame = false;
            self.frame_cb = self.new_obj(Obj::FrameCb);
            self.c.send(self.surface, 3, &[Arg::N(self.frame_cb)]);
        }
        self.c.send(self.surface, 6, &[]);
        self.frame_no += 1;
        let b = &mut self.bufs[i];
        b.busy = true;
        b.drawn = self.frame_no;
    }

    fn request_frame(&mut self) {
        self.want_frame = true;
    }

    fn size(&self) -> (i32, i32, f32) {
        let (w, h) = self.phys();
        (w, h, self.scale())
    }

    fn set_title(&mut self, title: &str) {
        self.c.send(self.toplevel, 2, &[Arg::S(title)]);
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        if cursor != self.cursor {
            self.cursor = cursor;
            self.apply_cursor();
        }
    }

    fn set_min_size(&mut self, w: i32, h: i32) {
        self.c.send(self.toplevel, 8, &[Arg::I(w), Arg::I(h)]);
    }

    fn begin_move(&mut self) {
        self.c.send(self.toplevel, 5, &[Arg::O(self.g.seat.0), Arg::U(self.input_serial)]);
    }

    fn begin_resize(&mut self, edge: Edge) {
        let e = match edge {
            Edge::Top => 1,
            Edge::Bottom => 2,
            Edge::Left => 4,
            Edge::TopLeft => 5,
            Edge::BottomLeft => 6,
            Edge::Right => 8,
            Edge::TopRight => 9,
            Edge::BottomRight => 10,
        };
        self.c.send(self.toplevel, 6, &[Arg::O(self.g.seat.0), Arg::U(self.input_serial), Arg::U(e)]);
    }

    fn has_server_decorations(&self) -> bool {
        self.server_deco
    }

    fn clipboard_set(&mut self, text: String) {
        if self.g.data_mgr == 0 || self.data_device == 0 {
            return;
        }
        self.clip_text = Arc::new(text);
        if self.data_source != 0 {
            self.c.send(self.data_source, 1, &[]);
            self.objs.remove(&self.data_source);
        }
        let src = self.new_obj(Obj::DataSource);
        self.data_source = src;
        self.c.send(self.g.data_mgr, 0, &[Arg::N(src)]);
        for m in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
            self.c.send(src, 0, &[Arg::S(m)]);
        }
        self.c.send(self.data_device, 1, &[Arg::O(src), Arg::U(self.input_serial)]);
    }

    fn clipboard_get(&mut self) {
        let offer = self.sel_offer;
        let deliver = |paste: &Arc<Mutex<Option<String>>>, efd: RawFd, s: String| {
            *paste.lock().unwrap() = Some(s);
            let one = 1u64;
            unsafe { libc::write(efd, &one as *const u64 as *const _, 8) };
        };
        if offer == 0 || !self.offer_text.get(&offer).copied().unwrap_or(false) {
            deliver(&self.paste, self.paste_efd, String::new());
            return;
        }
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
            deliver(&self.paste, self.paste_efd, String::new());
            return;
        }
        self.c.send(offer, 1, &[Arg::S("text/plain;charset=utf-8"), Arg::Fd(fds[1])]);
        let _ = self.c.flush();
        unsafe { libc::close(fds[1]) };
        let (paste, efd, rfd) = (self.paste.clone(), self.paste_efd, fds[0]);
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = unsafe { libc::read(rfd, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 || data.len() > (16 << 20) {
                    break;
                }
                data.extend_from_slice(&buf[..n as usize]);
            }
            unsafe { libc::close(rfd) };
            deliver(&paste, efd, String::from_utf8_lossy(&data).into_owned());
        });
    }
}
