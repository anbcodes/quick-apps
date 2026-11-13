//! The X11 window, with the core protocol written by hand over the display's unix socket
//! (no libX11). It draws with PutImage, so it works on any X server, remote ones aside.
//! Keys come from the server's keyboard mapping: X keysyms are what the runtime uses.

use super::{Cursor, Edge, Frame, PEvent, Platform, Rect};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;

// Atoms every server has.
const XA_ATOM: u32 = 4;
const XA_CARDINAL: u32 = 6;
const XA_RESOURCE_MANAGER: u32 = 23;
const XA_STRING: u32 = 31;
const XA_WM_NAME: u32 = 39;
const XA_WM_NORMAL_HINTS: u32 = 40;
const XA_WM_SIZE_HINTS: u32 = 41;
const XA_WM_CLASS: u32 = 67;

const FRAME_MS: u64 = 16;

struct Atoms {
    wm_protocols: u32,
    wm_delete: u32,
    net_wm_name: u32,
    utf8: u32,
    clipboard: u32,
    targets: u32,
    qa_paste: u32,
    incr: u32,
}

pub struct X11 {
    s: UnixStream,
    fd: RawFd,
    out: Vec<u8>,
    inbuf: Vec<u8>,
    /// Events read while waiting for a reply.
    raw: VecDeque<Vec<u8>>,
    seq: u16,
    id_base: u32,
    id_shift: u32,
    next_id: u32,
    root: u32,
    depth: u8,
    /// The longest request, in bytes.
    max_req: usize,
    min_kc: u8,
    per: usize,
    keysyms: Vec<u32>,
    atoms: Atoms,
    win: u32,
    gc: u32,
    /// Physical pixels.
    w: i32,
    h: i32,
    scale: f32,
    buf: Vec<u32>,
    fresh: bool,
    pub default_size: (i32, i32),
    frame_due: Option<u64>,
    /// Text we offer on the CLIPBOARD selection while we own it.
    clip: Option<String>,
    cursors: HashMap<u8, u32>,
    cursor: u8,
    cursor_font: u32,
    title: String,
    app_id: String,
    mods: u8,
    min_size: (i32, i32),
    debug: bool,
    /// The connection broke.
    broken: bool,
    /// A paste of our own clipboard text is waiting to be delivered.
    paste_own: bool,
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn i16le(b: &[u8], i: usize) -> i16 {
    i16::from_le_bytes([b[i], b[i + 1]])
}
fn pad4(n: usize) -> usize {
    (4 - n % 4) % 4
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// The cookie for display `num` in an Xauthority file's bytes.
fn find_cookie(data: &[u8], num: &str, host: &str) -> Option<Vec<u8>> {
    let mut i = 0;
    let mut best: Option<(bool, Vec<u8>)> = None;
    let take = |i: &mut usize| -> Option<&[u8]> {
        let n = u16::from_be_bytes([*data.get(*i)?, *data.get(*i + 1)?]) as usize;
        let s = data.get(*i + 2..*i + 2 + n)?;
        *i += 2 + n;
        Some(s)
    };
    while i + 2 <= data.len() {
        let family = u16::from_be_bytes([data[i], data[i + 1]]);
        i += 2;
        let addr = take(&mut i)?;
        let number = take(&mut i)?;
        let name = take(&mut i)?;
        let cookie = take(&mut i)?;
        if name != b"MIT-MAGIC-COOKIE-1" || (!number.is_empty() && number != num.as_bytes()) {
            continue;
        }
        // Local entries name this machine; a wildcard entry fits any.
        let ours = family == 256 && addr == host.as_bytes();
        if ours || family == 0xffff || family == 256 {
            if best.as_ref().map_or(true, |(b, _)| ours && !*b) {
                best = Some((ours, cookie.to_vec()));
            }
        }
    }
    best.map(|(_, c)| c)
}

fn hostname() -> String {
    let mut b = [0u8; 256];
    unsafe { libc::gethostname(b.as_mut_ptr() as *mut _, b.len()) };
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn mods_of(state: u16) -> u8 {
    let mut m = 0;
    if state & 1 != 0 {
        m |= SHIFT;
    }
    if state & 4 != 0 {
        m |= CTRL;
    }
    if state & 8 != 0 {
        m |= ALT;
    }
    m
}

/// Folds keypad keysyms into the keys they act as.
fn plain_sym(s: u32) -> u32 {
    match s {
        0xff95 => 0xff50,
        0xff96 => 0xff51,
        0xff97 => 0xff52,
        0xff98 => 0xff53,
        0xff99 => 0xff54,
        0xff9a => 0xff55,
        0xff9b => 0xff56,
        0xff9c => 0xff57,
        0xff9f => 0xffff,
        0xffaa => '*' as u32,
        0xffab => '+' as u32,
        0xffad => '-' as u32,
        0xffae => '.' as u32,
        0xffaf => '/' as u32,
        0xffbd => '=' as u32,
        0xffb0..=0xffb9 => '0' as u32 + (s - 0xffb0),
        _ => s,
    }
}

/// The text a keysym types, if it types any.
fn sym_text(s: u32) -> String {
    let c = match s {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(s),
        0x1000100..=0x110ffff => char::from_u32(s - 0x1000000),
        _ => None,
    };
    c.map(|c| c.to_string()).unwrap_or_default()
}

impl X11 {
    pub fn connect(title: &str, app_id: &str) -> io::Result<X11> {
        let display = std::env::var("DISPLAY").map_err(|_| err("DISPLAY is not set"))?;
        let rest = display.strip_prefix("unix:").unwrap_or(&display);
        let (host, num) = rest.split_once(':').ok_or_else(|| err(format!("DISPLAY {display}: expected :N")))?;
        if !host.is_empty() {
            return Err(err(format!("DISPLAY {display}: only local displays are supported")));
        }
        let num = num.split('.').next().unwrap_or("0");
        let s = UnixStream::connect(format!("/tmp/.X11-unix/X{num}")).map_err(|e| err(format!("cannot connect to display {display}: {e}")))?;
        let _ = unsafe { libc::fcntl(s.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        let fd = s.as_raw_fd();
        let mut x = X11 {
            s,
            fd,
            out: Vec::new(),
            inbuf: Vec::new(),
            raw: VecDeque::new(),
            seq: 0,
            id_base: 0,
            id_shift: 0,
            next_id: 1,
            root: 0,
            depth: 24,
            max_req: 262140,
            min_kc: 8,
            per: 0,
            keysyms: Vec::new(),
            atoms: Atoms { wm_protocols: 0, wm_delete: 0, net_wm_name: 0, utf8: 0, clipboard: 0, targets: 0, qa_paste: 0, incr: 0 },
            win: 0,
            gc: 0,
            w: 0,
            h: 0,
            scale: 1.0,
            buf: Vec::new(),
            fresh: false,
            default_size: (900, 600),
            frame_due: None,
            clip: None,
            cursors: HashMap::new(),
            cursor: 0,
            cursor_font: 0,
            title: title.to_string(),
            app_id: app_id.to_string(),
            mods: 0,
            min_size: (0, 0),
            debug: std::env::var_os("QA_DEBUG").is_some(),
            broken: false,
            paste_own: false,
        };
        x.handshake(num)?;
        x.load_keymap()?;
        x.intern_atoms()?;
        x.scale = x.detect_scale();
        Ok(x)
    }

    fn handshake(&mut self, num: &str) -> io::Result<()> {
        let xauth = std::env::var_os("XAUTHORITY").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".Xauthority")));
        let cookie = xauth.and_then(|p| std::fs::read(p).ok()).and_then(|d| find_cookie(&d, num, &hostname()));
        let name: &[u8] = if cookie.is_some() { b"MIT-MAGIC-COOKIE-1" } else { b"" };
        let data = cookie.unwrap_or_default();
        let mut r = vec![b'l', 0, 11, 0, 0, 0];
        r.extend_from_slice(&(name.len() as u16).to_le_bytes());
        r.extend_from_slice(&(data.len() as u16).to_le_bytes());
        r.extend_from_slice(&[0, 0]);
        r.extend_from_slice(name);
        r.extend(std::iter::repeat(0).take(pad4(name.len())));
        r.extend_from_slice(&data);
        r.extend(std::iter::repeat(0).take(pad4(data.len())));
        self.s.write_all(&r)?;
        let mut head = [0u8; 8];
        self.s.read_exact(&mut head)?;
        let mut body = vec![0u8; u16le(&head, 6) as usize * 4];
        self.s.read_exact(&mut body)?;
        if head[0] != 1 {
            let n = head[1] as usize;
            return Err(err(format!("the X server refused the connection: {}", String::from_utf8_lossy(&body[..n.min(body.len())]))));
        }
        let b = &body;
        self.id_base = u32le(b, 4);
        let mask = u32le(b, 8);
        self.id_shift = mask.trailing_zeros();
        let vendor = u16le(b, 16) as usize;
        self.max_req = u16le(b, 18) as usize * 4;
        let (nroots, nfmt) = (b[20] as usize, b[21] as usize);
        if b[22] != 0 {
            return Err(err("the X server wants big-endian pixels"));
        }
        self.min_kc = b[26];
        let mut p = 32 + vendor + pad4(vendor);
        let mut bpp = HashMap::new();
        for _ in 0..nfmt {
            bpp.insert(b[p], b[p + 1]);
            p += 8;
        }
        if nroots == 0 {
            return Err(err("the X server has no screen"));
        }
        self.root = u32le(b, p);
        let root_visual = u32le(b, p + 32);
        let root_depth = b[p + 38];
        let ndepths = b[p + 39] as usize;
        p += 40;
        let mut masks = None;
        for _ in 0..ndepths {
            let nvis = u16le(b, p + 2) as usize;
            p += 8;
            for _ in 0..nvis {
                if u32le(b, p) == root_visual {
                    masks = Some((b[p + 4], u32le(b, p + 8), u32le(b, p + 12), u32le(b, p + 16)));
                }
                p += 24;
            }
        }
        let ok = matches!(masks, Some((4, 0xff0000, 0xff00, 0xff))) && matches!(root_depth, 24 | 32) && bpp.get(&root_depth) == Some(&32);
        if !ok {
            return Err(err("the X server's screen isn't 24-bit true color"));
        }
        self.depth = root_depth;
        Ok(())
    }

    fn new_id(&mut self) -> u32 {
        let id = self.id_base | (self.next_id << self.id_shift);
        self.next_id += 1;
        id
    }

    /// Queues a request and returns its sequence number.
    fn req(&mut self, opcode: u8, data: u8, body: &[u8]) -> u16 {
        let len = (4 + body.len() + pad4(body.len())) / 4;
        self.out.extend_from_slice(&[opcode, data]);
        self.out.extend_from_slice(&(len as u16).to_le_bytes());
        self.out.extend_from_slice(body);
        self.out.extend(std::iter::repeat(0).take(pad4(body.len())));
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn write_out(&mut self) {
        if self.out.is_empty() {
            return;
        }
        if self.s.write_all(&self.out).is_err() {
            self.broken = true;
        }
        self.out.clear();
    }

    /// Reads what the server has sent. Returns false when the connection is gone.
    fn fill(&mut self, block: bool) -> bool {
        let mut buf = [0u8; 16384];
        let flags = if block { 0 } else { libc::MSG_DONTWAIT };
        loop {
            let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut _, buf.len(), flags) };
            if n > 0 {
                self.inbuf.extend_from_slice(&buf[..n as usize]);
                if block || (n as usize) < buf.len() {
                    return true;
                }
            } else if n == 0 {
                return false;
            } else {
                let e = io::Error::last_os_error().raw_os_error();
                if e == Some(libc::EINTR) {
                    continue;
                }
                return e == Some(libc::EAGAIN);
            }
        }
    }

    /// The next whole message from the server, if it has arrived.
    fn next_msg(&mut self) -> Option<Vec<u8>> {
        if self.inbuf.len() < 32 {
            return None;
        }
        let t = self.inbuf[0] & 0x7f;
        let extra = if t == 1 || t == 35 { u32le(&self.inbuf, 4) as usize * 4 } else { 0 };
        if self.inbuf.len() < 32 + extra {
            return None;
        }
        Some(self.inbuf.drain(..32 + extra).collect())
    }

    /// Sends everything queued and waits for the reply to request `seq`.
    fn reply(&mut self, seq: u16) -> io::Result<Vec<u8>> {
        self.write_out();
        loop {
            while let Some(m) = self.next_msg() {
                match m[0] & 0x7f {
                    1 if u16le(&m, 2) == seq => return Ok(m),
                    0 if u16le(&m, 2) == seq => return Err(err(format!("X error {} on request {}", m[1], m[10]))),
                    0 => self.log_error(&m),
                    1 => {}
                    _ => self.raw.push_back(m),
                }
            }
            if !self.fill(true) {
                return Err(err("the X server closed the connection"));
            }
        }
    }

    fn log_error(&self, m: &[u8]) {
        if self.debug {
            eprintln!("qa: X error {} on request {}", m[1], m[10]);
        }
    }

    fn intern(&mut self, names: &[&str]) -> io::Result<Vec<u32>> {
        let seqs: Vec<u16> = names
            .iter()
            .map(|n| {
                let mut b = (n.len() as u16).to_le_bytes().to_vec();
                b.extend_from_slice(&[0, 0]);
                b.extend_from_slice(n.as_bytes());
                self.req(16, 0, &b)
            })
            .collect();
        seqs.into_iter().map(|s| self.reply(s).map(|r| u32le(&r, 8))).collect()
    }

    fn intern_atoms(&mut self) -> io::Result<()> {
        let a = self.intern(&["WM_PROTOCOLS", "WM_DELETE_WINDOW", "_NET_WM_NAME", "UTF8_STRING", "CLIPBOARD", "TARGETS", "QA_PASTE", "INCR"])?;
        self.atoms = Atoms { wm_protocols: a[0], wm_delete: a[1], net_wm_name: a[2], utf8: a[3], clipboard: a[4], targets: a[5], qa_paste: a[6], incr: a[7] };
        Ok(())
    }

    fn load_keymap(&mut self) -> io::Result<()> {
        let count = 255 - self.min_kc as usize;
        let s = self.req(101, 0, &[self.min_kc, count as u8, 0, 0]);
        let r = self.reply(s)?;
        self.per = r[1] as usize;
        let n = u32le(&r, 4) as usize;
        self.keysyms = (0..n).map(|i| u32le(&r, 32 + i * 4)).collect();
        Ok(())
    }

    /// The Xft.dpi resource, which desktops set for scaling, as a scale.
    fn detect_scale(&mut self) -> f32 {
        let mut body = Vec::new();
        for v in [self.root, XA_RESOURCE_MANAGER, XA_STRING, 0, 16384] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let s = self.req(20, 0, &body);
        let Ok(r) = self.reply(s) else { return 1.0 };
        let n = u32le(&r, 16) as usize;
        let text = String::from_utf8_lossy(&r[32..(32 + n).min(r.len())]).into_owned();
        let dpi = text.lines().find_map(|l| l.strip_prefix("Xft.dpi:").and_then(|v| v.trim().parse::<f32>().ok()));
        match dpi {
            Some(d) if d > 0.0 => ((d / 96.0 * 4.0).round() / 4.0).clamp(1.0, 4.0),
            _ => 1.0,
        }
    }

    fn set_prop(&mut self, prop: u32, ty: u32, format: u8, data: &[u8]) {
        let mut b = Vec::new();
        b.extend_from_slice(&self.win.to_le_bytes());
        b.extend_from_slice(&prop.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&[format, 0, 0, 0]);
        b.extend_from_slice(&((data.len() / (format as usize / 8)) as u32).to_le_bytes());
        b.extend_from_slice(data);
        self.req(18, 0, &b);
    }

    fn ensure_open(&mut self) {
        if self.win != 0 {
            return;
        }
        let (dw, dh) = self.default_size;
        self.w = (dw as f32 * self.scale).round() as i32;
        self.h = (dh as f32 * self.scale).round() as i32;
        self.win = self.new_id();
        self.gc = self.new_id();
        // Key, button, motion, enter/leave, expose, structure and focus events.
        let events: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x8000 | 0x20000 | 0x200000;
        let mut b = Vec::new();
        b.extend_from_slice(&self.win.to_le_bytes());
        b.extend_from_slice(&self.root.to_le_bytes());
        b.extend_from_slice(&0i16.to_le_bytes());
        b.extend_from_slice(&0i16.to_le_bytes());
        b.extend_from_slice(&(self.w as u16).to_le_bytes());
        b.extend_from_slice(&(self.h as u16).to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0x800u32.to_le_bytes());
        b.extend_from_slice(&events.to_le_bytes());
        self.req(1, 0, &b);
        let mut g = Vec::new();
        g.extend_from_slice(&self.gc.to_le_bytes());
        g.extend_from_slice(&self.win.to_le_bytes());
        // No GraphicsExposure events for our copies.
        g.extend_from_slice(&0x10000u32.to_le_bytes());
        g.extend_from_slice(&0u32.to_le_bytes());
        self.req(55, 0, &g);
        self.apply_title();
        let class = format!("{}\0{}\0", self.app_id, self.app_id);
        self.set_prop(XA_WM_CLASS, XA_STRING, 8, class.as_bytes());
        let protocols = self.atoms.wm_delete.to_le_bytes();
        self.set_prop(self.atoms.wm_protocols, XA_ATOM, 32, &protocols);
        self.apply_min_size();
        self.req(8, 0, &self.win.to_le_bytes());
    }

    fn apply_title(&mut self) {
        let t = self.title.clone();
        self.set_prop(XA_WM_NAME, self.atoms.utf8, 8, t.as_bytes());
        self.set_prop(self.atoms.net_wm_name, self.atoms.utf8, 8, t.as_bytes());
    }

    fn apply_min_size(&mut self) {
        let (mw, mh) = self.min_size;
        if mw == 0 && mh == 0 {
            return;
        }
        let mut hints = [0u32; 18];
        hints[0] = 16; // PMinSize
        hints[5] = (mw as f32 * self.scale) as u32;
        hints[6] = (mh as f32 * self.scale) as u32;
        let bytes: Vec<u8> = hints.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.set_prop(XA_WM_NORMAL_HINTS, XA_WM_SIZE_HINTS, 32, &bytes);
    }

    fn keysym(&self, kc: u8, state: u16) -> u32 {
        if self.per == 0 || kc < self.min_kc {
            return 0;
        }
        let base = (kc - self.min_kc) as usize * self.per;
        let col = |c: usize| self.keysyms.get(base + c).copied().unwrap_or(0);
        let (shift, caps, num) = (state & 1 != 0, state & 2 != 0, state & 0x10 != 0);
        let k0 = col(0);
        // Num Lock picks the digits on the keypad; Caps Lock only shifts letters.
        let keypad = (0xff80..=0xffbf).contains(&k0);
        let lower = (0x61..=0x7a).contains(&k0);
        let upper = if keypad { num != shift } else if lower { caps != shift } else { shift };
        let (a, b) = if state & 0x80 != 0 && self.per >= 6 { (4, 5) } else { (0, 1) };
        let s = if upper { col(b) } else { col(a) };
        if s == 0 {
            col(a)
        } else {
            s
        }
    }

    fn key_event(&mut self, m: &[u8], pressed: bool, out: &mut Vec<PEvent>) {
        let state = u16le(m, 28);
        let sym = self.keysym(m[1], state);
        if (0xffe1..=0xffee).contains(&sym) {
            let bit = match sym {
                0xffe1 | 0xffe2 => SHIFT,
                0xffe3 | 0xffe4 => CTRL,
                0xffe9 | 0xffea => ALT,
                _ => 0,
            };
            self.mods = if pressed { mods_of(state) | bit } else { mods_of(state) & !bit };
            out.push(PEvent::Mods(self.mods));
            return;
        }
        if sym == 0 {
            return;
        }
        let mods = mods_of(state);
        let sym = plain_sym(sym);
        let text = if mods & CTRL != 0 { String::new() } else { sym_text(sym) };
        self.mods = mods;
        out.push(PEvent::Key { sym, text, pressed, repeat: false, mods });
    }

    fn pointer_mods(&mut self, state: u16, out: &mut Vec<PEvent>) {
        let m = mods_of(state);
        if m != self.mods {
            self.mods = m;
            out.push(PEvent::Mods(m));
        }
    }

    fn handle(&mut self, m: &[u8], out: &mut Vec<PEvent>) {
        match m[0] & 0x7f {
            0 => self.log_error(m),
            2 => self.key_event(m, true, out),
            3 => self.key_event(m, false, out),
            4 | 5 => {
                let pressed = m[0] & 0x7f == 4;
                let state = u16le(m, 28);
                self.pointer_mods(state, out);
                let (x, y) = (i16le(m, 24) as f32, i16le(m, 26) as f32);
                let step = 30.0 * self.scale;
                match m[1] {
                    1 => out.push(PEvent::Button { button: super::BUTTON_LEFT, pressed, x, y }),
                    2 => out.push(PEvent::Button { button: super::BUTTON_MIDDLE, pressed, x, y }),
                    3 => out.push(PEvent::Button { button: super::BUTTON_RIGHT, pressed, x, y }),
                    4 if pressed => out.push(PEvent::Scroll { dx: 0.0, dy: -step }),
                    5 if pressed => out.push(PEvent::Scroll { dx: 0.0, dy: step }),
                    6 if pressed => out.push(PEvent::Scroll { dx: -step, dy: 0.0 }),
                    7 if pressed => out.push(PEvent::Scroll { dx: step, dy: 0.0 }),
                    _ => {}
                }
            }
            6 => {
                self.pointer_mods(u16le(m, 28), out);
                out.push(PEvent::PointerMove { x: i16le(m, 24) as f32, y: i16le(m, 26) as f32 });
            }
            7 => out.push(PEvent::PointerMove { x: i16le(m, 24) as f32, y: i16le(m, 26) as f32 }),
            8 => out.push(PEvent::PointerLeave),
            9 => out.push(PEvent::FocusIn),
            10 => out.push(PEvent::FocusOut),
            12 => {
                // Nothing keeps the window's pixels, so an exposed window is painted again.
                if u16le(m, 16) == 0 && self.fresh {
                    self.put_rect(Rect::new(0, 0, self.w, self.h));
                }
            }
            22 => {
                let (w, h) = (u16le(m, 20) as i32, u16le(m, 22) as i32);
                if w > 0 && h > 0 && (w, h) != (self.w, self.h) {
                    self.w = w;
                    self.h = h;
                    out.push(PEvent::Resize { w, h, scale: self.scale });
                }
            }
            29 => self.clip = None,
            30 => self.selection_request(m),
            31 => self.selection_notify(m, out),
            33 => {
                if m.len() >= 32 && u32le(m, 8) == self.atoms.wm_protocols && u32le(m, 12) == self.atoms.wm_delete {
                    out.push(PEvent::Close);
                }
            }
            34 if m[4] == 1 => {
                let _ = self.load_keymap();
            }
            _ => {}
        }
    }

    fn selection_request(&mut self, m: &[u8]) {
        let (requestor, selection, target) = (u32le(m, 12), u32le(m, 16), u32le(m, 20));
        let prop = if u32le(m, 24) == 0 { target } else { u32le(m, 24) };
        let mut answer = 0;
        if selection == self.atoms.clipboard {
            let (utf8, targets) = (self.atoms.utf8, self.atoms.targets);
            let change = |x: &mut X11, ty: u32, format: u8, data: &[u8]| {
                let mut b = Vec::new();
                b.extend_from_slice(&requestor.to_le_bytes());
                b.extend_from_slice(&prop.to_le_bytes());
                b.extend_from_slice(&ty.to_le_bytes());
                b.extend_from_slice(&[format, 0, 0, 0]);
                b.extend_from_slice(&((data.len() / (format as usize / 8)) as u32).to_le_bytes());
                b.extend_from_slice(data);
                x.req(18, 0, &b);
            };
            if target == targets {
                let list: Vec<u8> = [targets, utf8, XA_STRING].iter().flat_map(|v| v.to_le_bytes()).collect();
                change(self, XA_ATOM, 32, &list);
                answer = prop;
            } else if (target == utf8 || target == XA_STRING) && self.clip.is_some() {
                let text = self.clip.clone().unwrap_or_default();
                // A request can't be longer than the server allows; INCR isn't supported.
                let bytes = &text.as_bytes()[..text.len().min(self.max_req.saturating_sub(64))];
                change(self, target, 8, bytes);
                answer = prop;
            }
        }
        let mut ev = vec![0u8; 32];
        ev[0] = 31;
        ev[8..12].copy_from_slice(&requestor.to_le_bytes());
        ev[12..16].copy_from_slice(&selection.to_le_bytes());
        ev[16..20].copy_from_slice(&target.to_le_bytes());
        ev[20..24].copy_from_slice(&answer.to_le_bytes());
        let mut b = requestor.to_le_bytes().to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&ev);
        self.req(25, 0, &b);
    }

    fn selection_notify(&mut self, m: &[u8], out: &mut Vec<PEvent>) {
        let prop = u32le(m, 20);
        if prop == 0 {
            out.push(PEvent::Paste(String::new()));
            return;
        }
        let mut body = Vec::new();
        for v in [self.win, prop, 0, 0, 262144] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        // Delete the property once it's read.
        let s = self.req(20, 1, &body);
        let text = match self.reply(s) {
            Ok(r) if u32le(&r, 8) != self.atoms.incr => {
                let bytes = (u32le(&r, 16) as usize * (r[1] as usize / 8)).min(r.len().saturating_sub(32));
                String::from_utf8_lossy(&r[32..32 + bytes]).into_owned()
            }
            _ => String::new(),
        };
        out.push(PEvent::Paste(text));
    }

    /// Sends one rectangle of the buffer, in as many requests as the server allows.
    fn put_rect(&mut self, r: Rect) {
        let r = r.intersect(&Rect::new(0, 0, self.w, self.h));
        if r.is_empty() || self.buf.len() < (self.w * self.h) as usize {
            return;
        }
        let rows_per = ((self.max_req.saturating_sub(32)) / (r.w as usize * 4)).max(1) as i32;
        let mut y = r.y;
        while y < r.bottom() {
            let rows = rows_per.min(r.bottom() - y);
            let mut b = Vec::with_capacity(20 + (rows * r.w * 4) as usize);
            b.extend_from_slice(&self.win.to_le_bytes());
            b.extend_from_slice(&self.gc.to_le_bytes());
            b.extend_from_slice(&(r.w as u16).to_le_bytes());
            b.extend_from_slice(&(rows as u16).to_le_bytes());
            b.extend_from_slice(&(r.x as i16).to_le_bytes());
            b.extend_from_slice(&(y as i16).to_le_bytes());
            b.extend_from_slice(&[0, self.depth, 0, 0]);
            for row in y..y + rows {
                let at = (row * self.w + r.x) as usize;
                for px in &self.buf[at..at + r.w as usize] {
                    b.extend_from_slice(&px.to_le_bytes());
                }
            }
            self.req(72, 2, &b);
            y += rows;
        }
    }

    fn complete(&self) -> bool {
        if self.inbuf.len() < 32 {
            return false;
        }
        let t = self.inbuf[0] & 0x7f;
        let extra = if t == 1 || t == 35 { u32le(&self.inbuf, 4) as usize * 4 } else { 0 };
        self.inbuf.len() >= 32 + extra
    }

    fn cursor_for(&mut self, kind: u8) -> u32 {
        if let Some(&c) = self.cursors.get(&kind) {
            return c;
        }
        if self.cursor_font == 0 {
            self.cursor_font = self.new_id();
            let mut b = self.cursor_font.to_le_bytes().to_vec();
            b.extend_from_slice(&6u16.to_le_bytes());
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(b"cursor");
            self.req(45, 0, &b);
        }
        // Glyphs in the standard cursor font.
        let glyph: u16 = match kind {
            1 => 60,
            2 => 152,
            3 => 52,
            4 => 0,
            5 => 108,
            6 => 116,
            7 => 12,
            8 => 14,
            _ => 68,
        };
        let id = self.new_id();
        let mut b = Vec::new();
        for v in [id, self.cursor_font, self.cursor_font] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&glyph.to_le_bytes());
        b.extend_from_slice(&(glyph + 1).to_le_bytes());
        for v in [0u16, 0, 0, 0xffff, 0xffff, 0xffff] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        self.req(94, 0, &b);
        self.cursors.insert(kind, id);
        id
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        if self.win != 0 {
            self.req(4, 0, &self.win.to_le_bytes());
            self.write_out();
        }
    }
}

impl Platform for X11 {
    fn fd(&self) -> RawFd {
        self.fd
    }

    fn wait(&mut self, extra: &[RawFd], timeout_ms: i32) -> bool {
        self.ensure_open();
        let mut ms = timeout_ms;
        if let Some(due) = self.frame_due {
            let left = due.saturating_sub(crate::sys::now_ms()) as i32;
            ms = if ms < 0 { left } else { ms.min(left) };
        }
        if !self.raw.is_empty() || self.complete() || self.paste_own {
            ms = 0;
        }
        let mut fds = vec![libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 }];
        fds.extend(extra.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }));
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, ms) };
        r >= 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
    }

    fn dispatch(&mut self, out: &mut Vec<PEvent>) {
        self.ensure_open();
        let alive = self.fill(false);
        let mut msgs: Vec<Vec<u8>> = self.raw.drain(..).collect();
        while let Some(m) = self.next_msg() {
            msgs.push(m);
        }
        for m in msgs {
            self.handle(&m, out);
        }
        if !alive || self.broken {
            out.push(PEvent::Close);
        }
        if std::mem::take(&mut self.paste_own) {
            out.push(PEvent::Paste(self.clip.clone().unwrap_or_default()));
        }
        if let Some(due) = self.frame_due {
            if crate::sys::now_ms() >= due {
                self.frame_due = None;
                out.push(PEvent::FrameDone);
            }
        }
    }

    fn flush(&mut self) {
        self.write_out();
    }

    fn frame(&mut self) -> Option<Frame<'_>> {
        self.ensure_open();
        let (w, h) = (self.w, self.h);
        if w <= 0 || h <= 0 {
            return None;
        }
        let age = if self.buf.len() != (w * h) as usize {
            self.buf = vec![0; (w * h) as usize];
            self.fresh = false;
            0
        } else if self.fresh {
            1
        } else {
            0
        };
        Some(Frame { px: &mut self.buf, w, h, stride: w as usize, age })
    }

    fn present(&mut self, damage: &[Rect]) {
        for d in damage {
            self.put_rect(*d);
        }
        self.fresh = true;
    }

    fn request_frame(&mut self) {
        self.frame_due.get_or_insert(crate::sys::now_ms() + FRAME_MS);
    }

    fn size(&self) -> (i32, i32, f32) {
        (self.w, self.h, self.scale)
    }

    fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        if self.win != 0 {
            self.apply_title();
        }
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
        if self.win == 0 || k == self.cursor {
            return;
        }
        self.cursor = k;
        let c = self.cursor_for(k);
        let mut b = self.win.to_le_bytes().to_vec();
        b.extend_from_slice(&0x4000u32.to_le_bytes());
        b.extend_from_slice(&c.to_le_bytes());
        self.req(2, 0, &b);
    }

    fn set_min_size(&mut self, w: i32, h: i32) {
        self.min_size = (w, h);
        if self.win != 0 {
            self.apply_min_size();
        }
    }

    // The window manager draws the title bar and handles moving and resizing.
    fn begin_move(&mut self) {}
    fn begin_resize(&mut self, _edge: Edge) {}

    fn has_server_decorations(&self) -> bool {
        true
    }

    fn clipboard_set(&mut self, text: String) {
        self.clip = Some(text);
        let mut b = self.win.to_le_bytes().to_vec();
        b.extend_from_slice(&self.atoms.clipboard.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        self.req(22, 0, &b);
    }

    fn clipboard_get(&mut self) {
        if self.clip.is_some() {
            // We own it, so there is no one to ask. Answered by the next dispatch.
            self.paste_own = true;
            return;
        }
        let mut b = Vec::new();
        for v in [self.win, self.atoms.clipboard, self.atoms.utf8, self.atoms.qa_paste, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        self.req(24, 0, &b);
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn cookie() {
        fn put(v: &mut Vec<u8>, b: &[u8]) {
            v.extend_from_slice(&(b.len() as u16).to_be_bytes());
            v.extend_from_slice(b);
        }
        let mut d = Vec::new();
        for (fam, host, num, cookie) in [(256u16, "other", "0", b"aaaa"), (256, "me", "1", b"bbbb"), (256, "me", "0", b"cccc")] {
            d.extend_from_slice(&fam.to_be_bytes());
            put(&mut d, host.as_bytes());
            put(&mut d, num.as_bytes());
            put(&mut d, b"MIT-MAGIC-COOKIE-1");
            put(&mut d, cookie);
        }
        assert_eq!(find_cookie(&d, "0", "me"), Some(b"cccc".to_vec()));
        assert_eq!(find_cookie(&d, "1", "me"), Some(b"bbbb".to_vec()));
        assert_eq!(find_cookie(&d, "7", "me"), None);
    }

    #[test]
    fn syms() {
        assert_eq!(sym_text(0x61), "a");
        assert_eq!(sym_text(0xe9), "é");
        assert_eq!(sym_text(0x10020ac), "€");
        assert_eq!(sym_text(0xff0d), "");
        assert_eq!(plain_sym(0xffb5), '5' as u32);
        assert_eq!(plain_sym(0xff96), 0xff51);
    }
}
