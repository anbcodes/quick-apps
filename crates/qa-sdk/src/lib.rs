//! Write Quick Apps in Rust. See docs/sdk.md.
//!
//! ```text
//! #![no_std]
//! use qa_sdk::*;
//! struct Hello;
//! impl App for Hello {
//!     fn start(ui: &mut Ui) -> Self {
//!         ui.text("Hello").root();
//!         Hello
//!     }
//!     fn event(&mut self, _ui: &mut Ui, _ev: Event) {}
//! }
//! app!(Hello);
//! ```
#![no_std]

extern crate alloc as alloc_crate;

#[cfg(all(target_arch = "wasm32", not(feature = "no-alloc")))]
mod alloc;
#[cfg(all(target_arch = "wasm32", not(feature = "no-alloc")))]
#[global_allocator]
static ALLOC: alloc::Alloc = alloc::Alloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

pub use alloc_crate::{string::String, vec, vec::Vec};
pub use qa_proto::{err, kind, prop, span, Track};
use qa_proto::{ev, Reader, Sink, Value};

pub mod json;
mod list;
pub mod time;
pub use json::{Doc, Val};
pub use list::List;

pub mod sys {
    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "qa")]
    extern "C" {
        pub fn ui_patch(ptr: *const u8, len: usize) -> i32;
        pub fn canvas_put(node: u32, x: u32, y: u32, w: u32, h: u32, ptr: *const u8) -> i32;
        pub fn set_title(ptr: *const u8, len: usize);
        pub fn quit();
        pub fn env(out: *mut u8, cap: usize) -> i32;
        pub fn now_ms() -> i64;
        pub fn mono_ms() -> i64;
        pub fn random(ptr: *mut u8, len: usize);
        pub fn log(level: u32, ptr: *const u8, len: usize);
        pub fn timer(ms: u32, repeat: u32) -> u32;
        pub fn timer_cancel(id: u32);
        pub fn clipboard_write(ptr: *const u8, len: usize) -> i32;
        pub fn open_url(ptr: *const u8, len: usize) -> i32;
        pub fn request(kind: u32, arg: *const u8, len: usize) -> u32;
        pub fn fs_open(dir: i32, path: *const u8, len: usize, flags: u32) -> i32;
        pub fn fs_read(fd: i32, ptr: *mut u8, len: usize) -> i32;
        pub fn fs_write(fd: i32, ptr: *const u8, len: usize) -> i32;
        pub fn fs_seek(fd: i32, off: i64, whence: u32) -> i64;
        pub fn fs_close(fd: i32) -> i32;
        pub fn fs_stat(dir: i32, path: *const u8, len: usize, out: *mut u8) -> i32;
        pub fn fs_list(dir: i32, path: *const u8, len: usize, out: *mut u8, cap: usize, needed: *mut u32) -> i32;
        pub fn fs_mkdir(dir: i32, path: *const u8, len: usize) -> i32;
        pub fn fs_remove(dir: i32, path: *const u8, len: usize) -> i32;
        pub fn fs_rename(dir: i32, from: *const u8, from_len: usize, dir2: i32, to: *const u8, to_len: usize) -> i32;
        pub fn http(ptr: *const u8, len: usize) -> u32;
        pub fn http_cancel(req: u32);
        pub fn ws_open(ptr: *const u8, len: usize) -> u32;
        pub fn ws_send(ws: u32, kind: u32, ptr: *const u8, len: usize) -> i32;
        pub fn ws_close(ws: u32, code: u32);
        pub fn msg_send(to: *const u8, to_len: usize, data: *const u8, data_len: usize) -> i32;
        pub fn json_parse(ptr: *const u8, len: usize, out: *mut u8, cap: usize, needed: *mut u32) -> i32;
        pub fn secret_get(key: *const u8, key_len: usize, out: *mut u8, cap: usize, needed: *mut u32) -> i32;
        pub fn secret_set(key: *const u8, key_len: usize, val: *const u8, val_len: usize) -> i32;
    }
}

/// A node id.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, PartialOrd, Ord, Hash)]
pub struct Node(pub u32);

pub const STRETCH: i32 = 0;
pub const START: i32 = 1;
pub const CENTER: i32 = 2;
pub const END: i32 = 3;

pub const fn px(v: f32) -> Track {
    Track::px(v)
}
pub const fn fr(v: f32) -> Track {
    Track::fr(v)
}
pub const fn auto() -> Track {
    Track::auto()
}

/// A theme token value for `ctx`.
#[derive(Clone, Copy)]
pub enum Tok<'a> {
    /// 0xRRGGBBAA
    Color(u32),
    Num(f32),
    Str(&'a str),
}

impl<'a> Tok<'a> {
    fn value(self) -> Value<'a> {
        match self {
            Tok::Color(c) => Value::Color(c),
            Tok::Num(n) => Value::F32(n),
            Tok::Str(s) => Value::Str(s),
        }
    }
}

/// Collects patch ops; the `app!` glue sends them after each call.
pub struct Ui {
    buf: Vec<u8>,
    next: u32,
    tag: i32,
    /// What each input holds, kept up to date from `Change` and `Submit`.
    values: Vec<(u32, String)>,
    /// Set by `input()`, so apps without inputs don't carry the tracking code.
    tracker: Option<fn(&mut Ui, &Event)>,
}

/// A node being built. Finish with `add_to`, `root` or `detached`.
pub struct B<'a> {
    ui: &'a mut Ui,
    n: Node,
    k: u8,
}

impl Ui {
    pub const fn new() -> Ui {
        Ui { buf: Vec::new(), next: 0, tag: 0, values: Vec::new(), tracker: None }
    }

    /// The text an input holds now: what the user typed, or the last value
    /// set. "" for an input nobody has typed in or set.
    pub fn value(&self, n: Node) -> &str {
        self.values.iter().find(|v| v.0 == n.0).map(|v| v.1.as_str()).unwrap_or("")
    }
    fn remember(&mut self, n: u32, s: &str) {
        match self.values.iter_mut().find(|v| v.0 == n) {
            Some(v) => {
                v.1.clear();
                v.1.push_str(s);
            }
            None => self.values.push((n, String::from(s))),
        }
    }
    #[doc(hidden)]
    pub fn __track(&mut self, ev: &Event) {
        if let Some(f) = self.tracker {
            f(self, ev);
        }
    }
    fn track(&mut self, ev: &Event) {
        if let Event::Change(n, s) | Event::Submit(n, s) = *ev {
            self.remember(n.0, s);
        }
    }

    /// The `tag` of the node the current event came from (0 if none).
    pub fn tag(&self) -> i32 {
        self.tag
    }
    #[doc(hidden)]
    pub fn __set_tag(&mut self, t: i32) {
        self.tag = t;
    }

    pub fn flush(&mut self) {
        if !self.buf.is_empty() {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                sys::ui_patch(self.buf.as_ptr(), self.buf.len());
            }
            self.buf.clear();
        }
    }

    /// The raw patch buffer, for ops the API does not cover.
    pub fn raw(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    pub fn create(&mut self, k: u8) -> B<'_> {
        self.next += 1;
        let n = Node(self.next);
        self.buf.op_create(n.0, k);
        B { ui: self, n, k }
    }

    pub fn grid(&mut self) -> B<'_> {
        self.create(kind::GRID)
    }
    pub fn text(&mut self, s: &str) -> B<'_> {
        self.create(kind::TEXT).str(prop::text::TEXT, s)
    }
    pub fn button(&mut self, label: &str) -> B<'_> {
        self.create(kind::BUTTON).str(prop::button::LABEL, label)
    }
    pub fn input(&mut self) -> B<'_> {
        self.tracker = Some(Ui::track);
        self.create(kind::INPUT)
    }
    pub fn checkbox(&mut self, label: &str) -> B<'_> {
        self.create(kind::CHECKBOX).str(prop::checkbox::LABEL, label)
    }
    /// Options separated by '\n'.
    pub fn select(&mut self, options: &str) -> B<'_> {
        self.create(kind::SELECT).str(prop::select::OPTIONS, options)
    }
    /// Labels separated by '\n'.
    pub fn tabs(&mut self, labels: &str) -> B<'_> {
        self.create(kind::TABS).str(prop::tabs::LABELS, labels)
    }
    pub fn scroll(&mut self) -> B<'_> {
        self.create(kind::SCROLL)
    }
    pub fn image(&mut self, src: &str) -> B<'_> {
        self.create(kind::IMAGE).str(prop::image::SRC, src)
    }
    pub fn canvas(&mut self, w: i32, h: i32) -> B<'_> {
        self.create(kind::CANVAS).int(prop::canvas::PX_W, w).int(prop::canvas::PX_H, h)
    }
    pub fn progress(&mut self, v: f32) -> B<'_> {
        self.create(kind::PROGRESS).num(prop::progress::VALUE, v)
    }
    pub fn divider(&mut self) -> B<'_> {
        self.create(kind::DIVIDER)
    }
    /// A popup: shown over everything else when opened (`open`,
    /// `context_menu`). Give it one child, and leave it detached.
    pub fn popup(&mut self) -> B<'_> {
        self.create(kind::POPUP)
    }
    /// A menu. `items` has one item per line: `-` is a separator, a leading
    /// `!` disables an item, `*` checks it, and text after a tab is a
    /// shortcut hint. Choosing one sends `Select(menu, line)`.
    pub fn menu(&mut self, items: &str) -> B<'_> {
        self.create(kind::MENU).str(prop::menu::ITEMS, items)
    }
    /// Shows a popup next to `anchor`. `Node(0)` places it at the pointer.
    pub fn open(&mut self, popup: Node, anchor: Node) {
        self.set(popup, prop::popup::ANCHOR, anchor.0 as i32);
        self.set(popup, prop::popup::OPEN, 1);
    }
    /// Hides a popup. The runtime also closes popups itself and then sends `Dismiss`.
    pub fn close(&mut self, popup: Node) {
        self.set(popup, prop::popup::OPEN, 0);
    }
    pub fn set_items(&mut self, menu: Node, items: &str) {
        self.set_str(menu, prop::menu::ITEMS, items);
    }

    pub fn set(&mut self, n: Node, p: u16, v: i32) {
        self.buf.op_set(n.0, p);
        self.buf.value(&Value::I32(v));
    }
    pub fn set_f(&mut self, n: Node, p: u16, v: f32) {
        self.buf.op_set(n.0, p);
        self.buf.value(&Value::F32(v));
    }
    pub fn set_str(&mut self, n: Node, p: u16, v: &str) {
        self.buf.op_set(n.0, p);
        self.buf.value(&Value::Str(v));
    }
    pub fn reset(&mut self, n: Node, p: u16) {
        self.buf.op_set(n.0, p);
        self.buf.value(&Value::None);
    }
    /// Sets a text node's text.
    pub fn set_text(&mut self, n: Node, s: &str) {
        self.set_str(n, prop::text::TEXT, s);
    }
    pub fn set_label(&mut self, n: Node, s: &str) {
        self.set_str(n, prop::button::LABEL, s);
    }
    /// Sets an input's value.
    pub fn set_value(&mut self, n: Node, s: &str) {
        self.set_str(n, prop::input::VALUE, s);
        self.remember(n.0, s);
    }
    /// Dims a node and everything inside it and stops their input.
    pub fn set_disabled(&mut self, n: Node, on: bool) {
        self.set(n, prop::DISABLED, on as i32);
    }
    pub fn set_checked(&mut self, n: Node, on: bool) {
        self.set(n, prop::checkbox::CHECKED, on as i32);
    }
    pub fn hide(&mut self, n: Node, hidden: bool) {
        self.set(n, prop::HIDDEN, hidden as i32);
    }
    pub fn variant(&mut self, n: Node, v: &str) {
        self.set_str(n, prop::VARIANT, v);
    }
    pub fn ctx(&mut self, n: Node, key: &str, v: Tok) {
        self.buf.op_ctx(n.0, key);
        self.buf.value(&v.value());
    }
    pub fn append(&mut self, parent: Node, child: Node) {
        self.buf.op_append(parent.0, child.0);
    }
    pub fn insert(&mut self, parent: Node, child: Node, before: Node) {
        self.buf.op_insert(parent.0, child.0, before.0);
    }
    pub fn detach(&mut self, n: Node) {
        self.buf.op_detach(n.0);
    }
    pub fn destroy(&mut self, n: Node) {
        self.buf.op_destroy(n.0);
        self.values.retain(|v| v.0 != n.0);
    }
    pub fn focus(&mut self, n: Node) {
        self.buf.op_focus(n.0);
    }
    pub fn scroll_to(&mut self, n: Node) {
        self.buf.op_scroll_to(n.0);
    }
    pub fn set_root(&mut self, n: Node) {
        self.buf.op_root(n.0);
    }
    /// Puts `new` where `old` is (or makes it the root) and destroys `old`.
    pub fn replace(&mut self, old: Node, new: Node) {
        self.buf.op_replace(old.0, new.0);
        self.values.retain(|v| v.0 != old.0);
    }
    pub fn set_tag(&mut self, n: Node, tag: i32) {
        self.set(n, prop::TAG, tag);
    }
    pub fn set_cols(&mut self, n: Node, t: &[Track]) {
        self.buf.op_set(n.0, prop::grid::COLS);
        self.buf.tracks(t);
    }
    pub fn set_rows(&mut self, n: Node, t: &[Track]) {
        self.buf.op_set(n.0, prop::grid::ROWS);
        self.buf.tracks(t);
    }
    /// Sets a text node's text and spans together.
    pub fn set_rich(&mut self, n: Node, r: &Rich) {
        self.set_text(n, &r.text);
        self.buf.op_set(n.0, prop::text::SPANS);
        self.buf.value(&Value::Bytes(&r.spans));
    }
    /// A text node with spans.
    pub fn rich(&mut self, r: &Rich) -> B<'_> {
        self.text(&r.text).bytes(prop::text::SPANS, &r.spans)
    }
    /// Copies RGBA8 pixels into a canvas.
    pub fn canvas_put(&mut self, n: Node, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) -> i32 {
        if rgba.len() < (w * h * 4) as usize {
            return err::INVALID;
        }
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::canvas_put(n.0, x, y, w, h, rgba.as_ptr());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (n, x, y);
            0
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Ui::new()
    }
}

impl<'a> B<'a> {
    pub fn node(&self) -> Node {
        self.n
    }
    pub fn int(self, p: u16, v: i32) -> Self {
        self.ui.buf.op_set(self.n.0, p);
        self.ui.buf.value(&Value::I32(v));
        self
    }
    pub fn num(self, p: u16, v: f32) -> Self {
        self.ui.buf.op_set(self.n.0, p);
        self.ui.buf.value(&Value::F32(v));
        self
    }
    pub fn str(self, p: u16, v: &str) -> Self {
        self.ui.buf.op_set(self.n.0, p);
        self.ui.buf.value(&Value::Str(v));
        self
    }
    pub fn bytes(self, p: u16, v: &[u8]) -> Self {
        self.ui.buf.op_set(self.n.0, p);
        self.ui.buf.value(&Value::Bytes(v));
        self
    }
    pub fn cols(self, t: &[Track]) -> Self {
        self.ui.buf.op_set(self.n.0, prop::grid::COLS);
        self.ui.buf.tracks(t);
        self
    }
    pub fn rows(self, t: &[Track]) -> Self {
        self.ui.buf.op_set(self.n.0, prop::grid::ROWS);
        self.ui.buf.tracks(t);
        self
    }
    pub fn gap(self, v: f32) -> Self {
        self.num(prop::grid::GAP, v)
    }
    pub fn pad(self, v: f32) -> Self {
        self.pad4(v, v, v, v)
    }
    pub fn pad4(self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        self.ui.buf.op_set(self.n.0, prop::PAD);
        self.ui.buf.value(&Value::Edges([top, right, bottom, left]));
        self
    }
    pub fn at(self, row: i32, col: i32) -> Self {
        self.int(prop::ROW, row).int(prop::COL, col)
    }
    pub fn span(self, rows: i32, cols: i32) -> Self {
        self.int(prop::ROW_SPAN, rows).int(prop::COL_SPAN, cols)
    }
    pub fn priority(self, p: i32) -> Self {
        self.int(prop::PRIORITY, p)
    }
    pub fn min_w(self, v: f32) -> Self {
        self.num(prop::MIN_W, v)
    }
    pub fn min_h(self, v: f32) -> Self {
        self.num(prop::MIN_H, v)
    }
    pub fn max_w(self, v: f32) -> Self {
        self.num(prop::MAX_W, v)
    }
    pub fn max_h(self, v: f32) -> Self {
        self.num(prop::MAX_H, v)
    }
    pub fn align_x(self, a: i32) -> Self {
        self.int(prop::ALIGN_X, a)
    }
    pub fn align_y(self, a: i32) -> Self {
        self.int(prop::ALIGN_Y, a)
    }
    pub fn hidden(self, h: bool) -> Self {
        self.int(prop::HIDDEN, h as i32)
    }
    pub fn variant(self, v: &str) -> Self {
        self.str(prop::VARIANT, v)
    }
    pub fn tooltip(self, v: &str) -> Self {
        self.str(prop::TOOLTIP, v)
    }
    /// Right-clicking this node (or something inside it) opens `popup` at
    /// the pointer, after a `ContextMenu` event naming this node.
    pub fn context_menu(self, popup: Node) -> Self {
        self.int(prop::CONTEXT_MENU, popup.0 as i32)
    }
    /// Popups: 0 below the anchor, 1 above, 2 right, 3 left.
    pub fn placement(self, p: i32) -> Self {
        self.int(prop::popup::PLACEMENT, p)
    }
    /// A number sent back with this node's events, read with `Ui::tag`.
    pub fn tag(self, t: i32) -> Self {
        self.int(prop::TAG, t)
    }
    /// Grids: place children left to right, wrapping onto new lines.
    pub fn flow(self, on: bool) -> Self {
        self.int(prop::grid::FLOW, on as i32)
    }
    pub fn ctx(self, key: &str, v: Tok) -> Self {
        self.ui.buf.op_ctx(self.n.0, key);
        self.ui.buf.value(&v.value());
        self
    }
    /// Text style: `body`, `title`, `heading`, `caption`, `mono`.
    pub fn style(self, s: &str) -> Self {
        self.str(prop::text::STYLE, s)
    }
    /// 0 wrap, 1 one line + ellipsis, 2 one line clipped.
    /// 0 wraps onto more lines, 1 keeps one line and cuts it with "…".
    /// For text and checkbox labels.
    pub fn wrap(self, w: i32) -> Self {
        let p = if self.k == kind::CHECKBOX { prop::checkbox::WRAP } else { prop::text::WRAP };
        self.int(p, w)
    }
    pub fn text_align(self, a: i32) -> Self {
        self.int(prop::text::TEXT_ALIGN, a)
    }
    pub fn max_lines(self, n: i32) -> Self {
        self.int(prop::text::MAX_LINES, n)
    }
    pub fn selectable(self, on: bool) -> Self {
        self.int(prop::text::SELECTABLE, on as i32)
    }
    pub fn placeholder(self, s: &str) -> Self {
        let p = if self.k == kind::SELECT { prop::select::PLACEHOLDER } else { prop::input::PLACEHOLDER };
        self.str(p, s)
    }
    pub fn value(self, s: &str) -> Self {
        self.ui.remember(self.n.0, s);
        self.str(prop::input::VALUE, s)
    }
    pub fn multiline(self, on: bool) -> Self {
        self.int(prop::input::MULTILINE, on as i32)
    }
    pub fn secret(self, on: bool) -> Self {
        self.int(prop::input::SECRET, on as i32)
    }
    pub fn lines(self, n: i32) -> Self {
        self.int(prop::input::LINES, n)
    }
    /// Dims the node and everything inside it and stops their input.
    pub fn disabled(self, on: bool) -> Self {
        self.int(prop::DISABLED, on as i32)
    }
    pub fn checked(self, on: bool) -> Self {
        self.int(prop::checkbox::CHECKED, on as i32)
    }
    pub fn clickable(self, on: bool) -> Self {
        self.int(prop::grid::CLICKABLE, on as i32)
    }
    /// For grids: 1 draws the selection background. For select and tabs: the chosen index.
    pub fn selected(self, i: i32) -> Self {
        let p = match self.k {
            kind::SELECT => prop::select::SELECTED,
            kind::TABS => prop::tabs::SELECTED,
            _ => prop::grid::SELECTED,
        };
        self.int(p, i)
    }
    /// Scroll axis: 0 vertical, 1 horizontal, 2 both.
    pub fn axis(self, a: i32) -> Self {
        self.int(prop::scroll::AXIS, a)
    }
    pub fn stick_end(self, on: bool) -> Self {
        self.int(prop::scroll::STICK_END, on as i32)
    }
    /// 0 contain, 1 cover, 2 fill, 3 natural.
    pub fn fit(self, f: i32) -> Self {
        let p = if self.k == kind::CANVAS { prop::canvas::FIT } else { prop::image::FIT };
        self.int(p, f)
    }
    pub fn focusable(self, on: bool) -> Self {
        self.int(prop::canvas::FOCUSABLE, on as i32)
    }
    pub fn add_to(self, parent: Node) -> Node {
        self.ui.buf.op_append(parent.0, self.n.0);
        self.n
    }
    pub fn root(self) -> Node {
        self.ui.buf.op_root(self.n.0);
        self.n
    }
    pub fn detached(self) -> Node {
        self.n
    }
}

/// An event from the runtime. Strings borrow from the event buffer.
#[derive(Debug)]
pub enum Event<'a> {
    Resize { w: f32, h: f32, scale: f32 },
    Click(Node),
    Change(Node, &'a str),
    Submit(Node, &'a str),
    Toggle(Node, bool),
    Select(Node, i32),
    Key { node: Node, sym: u32, mods: u8, text: &'a str },
    Pointer { node: Node, kind: u8, x: f32, y: f32, buttons: u8, dx: f32, dy: f32 },
    Timer(u32),
    Http { req: u32, status: u16, headers: &'a str, body: &'a [u8] },
    Ws { ws: u32, kind: u8, data: &'a [u8] },
    Message { from: &'a str, data: &'a [u8] },
    RequestDone { req: u32, result: i32, info: &'a str },
    Close,
    Activate,
    FocusOut(Node),
    NearEnd(Node),
    /// A scroll came within one viewport of its start (load older rows).
    NearStart(Node),
    /// A right-click (or the Menu key) on a node with a `context_menu`.
    /// The popup opens right after; change its content here.
    ContextMenu { node: Node, x: f32, y: f32 },
    /// The runtime closed a popup: Esc, a click outside, or a menu choice.
    Dismiss(Node),
    /// A link span was clicked: the node and the span's index.
    Link(Node, u32),
    /// The second of two quick clicks on a button, checkbox or clickable grid.
    DoubleClick(Node),
    Unknown,
}

pub fn decode(b: &[u8]) -> Event<'_> {
    __decode(b).0
}

/// The event and the tag of the node it came from (0 if none).
#[doc(hidden)]
pub fn __decode(b: &[u8]) -> (Event<'_>, i32) {
    let f = || -> Option<(Event, i32)> {
        let mut r = Reader::new(b);
        let t = r.u8()?;
        let n = |r: &mut Reader| r.u32().map(Node);
        let e = match t {
            ev::RESIZE => Event::Resize { w: r.f32()?, h: r.f32()?, scale: r.f32()? },
            ev::CLICK => Event::Click(n(&mut r)?),
            ev::CHANGE => Event::Change(n(&mut r)?, r.str()?),
            ev::SUBMIT => Event::Submit(n(&mut r)?, r.str()?),
            ev::TOGGLE => Event::Toggle(n(&mut r)?, r.u8()? != 0),
            ev::SELECT => Event::Select(n(&mut r)?, r.i32()?),
            ev::KEY => Event::Key { node: n(&mut r)?, sym: r.u32()?, mods: r.u8()?, text: r.str()? },
            ev::POINTER => Event::Pointer {
                node: n(&mut r)?,
                kind: r.u8()?,
                x: r.f32()?,
                y: r.f32()?,
                buttons: r.u8()?,
                dx: r.f32()?,
                dy: r.f32()?,
            },
            ev::TIMER => Event::Timer(r.u32()?),
            ev::HTTP => Event::Http { req: r.u32()?, status: r.u16()?, headers: r.str()?, body: r.bytes()? },
            ev::WS => Event::Ws { ws: r.u32()?, kind: r.u8()?, data: r.bytes()? },
            ev::MESSAGE => Event::Message { from: r.str()?, data: r.bytes()? },
            ev::REQUEST_DONE => Event::RequestDone { req: r.u32()?, result: r.i32()?, info: r.str()? },
            ev::CLOSE => Event::Close,
            ev::ACTIVATE => Event::Activate,
            ev::FOCUS_OUT => Event::FocusOut(n(&mut r)?),
            ev::NEAR_END => Event::NearEnd(n(&mut r)?),
            ev::LINK => Event::Link(n(&mut r)?, r.u32()?),
            ev::DOUBLE_CLICK => Event::DoubleClick(n(&mut r)?),
            ev::NEAR_START => Event::NearStart(n(&mut r)?),
            ev::CONTEXT_MENU => Event::ContextMenu { node: n(&mut r)?, x: r.f32()?, y: r.f32()? },
            ev::DISMISS => Event::Dismiss(n(&mut r)?),
            _ => Event::Unknown,
        };
        // Events that name a node end with that node's tag.
        let tag = match t {
            ev::RESIZE | ev::TIMER | ev::HTTP | ev::WS | ev::MESSAGE | ev::REQUEST_DONE | ev::CLOSE | ev::ACTIVATE => 0,
            _ => r.i32().unwrap_or(0),
        };
        Some((e, tag))
    };
    f().unwrap_or((Event::Unknown, 0))
}

/// Text with styled ranges, for `Ui::rich` and `Ui::set_rich`.
///
/// ```text
/// let mut r = Rich::new();
/// r.push("Hello ");
/// r.styled("world", span::BOLD, "");
/// let link = r.styled("docs", span::LINK, "accent"); // Event::Link(node, link)
/// ui.rich(&r).add_to(col);
/// ```
#[derive(Clone, Default)]
pub struct Rich {
    pub text: String,
    /// Encoded span records (see docs/components.md).
    pub spans: Vec<u8>,
    count: u32,
}

impl Rich {
    pub fn new() -> Rich {
        Rich::default()
    }
    pub fn push(&mut self, s: &str) -> &mut Self {
        self.text.push_str(s);
        self
    }
    /// Appends styled text. `flags` combines `span::*`; `variant` names a
    /// color (`accent`, `muted`, `danger`, …) or is empty. Returns the span's
    /// index, which `Event::Link` reports.
    pub fn styled(&mut self, s: &str, flags: u8, variant: &str) -> u32 {
        let start = self.text.len() as u32;
        self.text.push_str(s);
        self.mark(start, self.text.len() as u32, flags, variant)
    }
    /// Styles a byte range of the text already pushed.
    pub fn mark(&mut self, start: u32, end: u32, flags: u8, variant: &str) -> u32 {
        self.spans.u32(start);
        self.spans.u32(end);
        self.spans.u8(flags);
        self.spans.str(variant);
        self.count += 1;
        self.count - 1
    }
    pub fn clear(&mut self) {
        *self = Rich::default();
    }
}

pub trait App: Sized {
    fn start(ui: &mut Ui) -> Self;
    fn event(&mut self, ui: &mut Ui, ev: Event);
}

#[doc(hidden)]
pub fn __alloc(len: u32) -> u32 {
    let mut v: Vec<u8> = Vec::with_capacity(len as usize);
    let p = v.as_mut_ptr() as u32;
    core::mem::forget(v);
    p
}

#[doc(hidden)]
pub unsafe fn __take(ptr: u32, len: u32) -> Vec<u8> {
    Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize)
}

/// Generates `qa_alloc`, `qa_start` and `qa_event` for an `App`.
#[macro_export]
macro_rules! app {
    ($t:ty) => {
        static mut __QA_APP: Option<$t> = None;
        static mut __QA_UI: $crate::Ui = $crate::Ui::new();

        #[no_mangle]
        pub extern "C" fn qa_alloc(len: u32) -> u32 {
            $crate::__alloc(len)
        }

        #[no_mangle]
        pub extern "C" fn qa_start() {
            unsafe {
                let ui = &mut *core::ptr::addr_of_mut!(__QA_UI);
                let app = <$t as $crate::App>::start(ui);
                *core::ptr::addr_of_mut!(__QA_APP) = Some(app);
                ui.flush();
            }
        }

        #[no_mangle]
        pub extern "C" fn qa_event(ptr: u32, len: u32) {
            unsafe {
                let buf = $crate::__take(ptr, len);
                let ui = &mut *core::ptr::addr_of_mut!(__QA_UI);
                if let Some(app) = (*core::ptr::addr_of_mut!(__QA_APP)).as_mut() {
                    let (ev, tag) = $crate::__decode(&buf);
                    ui.__set_tag(tag);
                    ui.__track(&ev);
                    $crate::App::event(app, ui, ev);
                }
                ui.flush();
            }
        }
    };
}

/// A number formatted without `core::fmt`.
pub struct Num {
    b: [u8; 24],
    n: u8,
}

impl Num {
    pub fn as_str(&self) -> &str {
        // Only ASCII digits, '-' and '.' are written.
        unsafe { core::str::from_utf8_unchecked(&self.b[..self.n as usize]) }
    }
}

impl core::ops::Deref for Num {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

pub fn itoa(v: i64) -> Num {
    let mut tmp = [0u8; 24];
    let mut i = tmp.len();
    let neg = v < 0;
    let mut u = v.unsigned_abs();
    loop {
        i -= 1;
        tmp[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        tmp[i] = b'-';
    }
    let mut b = [0u8; 24];
    let n = tmp.len() - i;
    b[..n].copy_from_slice(&tmp[i..]);
    Num { b, n: n as u8 }
}

/// Formats `v` with a fixed number of decimals (at most 6).
pub fn ftoa(v: f32, decimals: u32) -> Num {
    let d = decimals.min(6);
    let scale = 10i64.pow(d);
    let f = v as f64 * scale as f64;
    let x = if f < 0.0 { (f - 0.5) as i64 } else { (f + 0.5) as i64 };
    let mut out = itoa(x.abs() / scale);
    let mut b = [0u8; 24];
    let mut n = 0;
    if x < 0 {
        b[0] = b'-';
        n = 1;
    }
    let ip = out.as_str().as_bytes();
    b[n..n + ip.len()].copy_from_slice(ip);
    n += ip.len();
    if d > 0 {
        b[n] = b'.';
        n += 1;
        let mut frac = x.abs() % scale;
        let mut digits = [b'0'; 6];
        for k in (0..d as usize).rev() {
            digits[k] = b'0' + (frac % 10) as u8;
            frac /= 10;
        }
        b[n..n + d as usize].copy_from_slice(&digits[..d as usize]);
        n += d as usize;
    }
    out.b = b;
    out.n = n as u8;
    out
}

/// Joins string pieces (a `format!` without `core::fmt`).
pub fn cat(parts: &[&str]) -> String {
    let mut s = String::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        s.push_str(p);
    }
    s
}

pub fn set_title(s: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::set_title(s.as_ptr(), s.len())
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = s;
}

pub fn quit() {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::quit()
    }
}

pub fn log(level: u32, s: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::log(level, s.as_ptr(), s.len())
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (level, s);
}

/// Fires `Event::Timer(id)` once after `ms`.
pub fn timer(ms: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return sys::timer(ms, 0);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        0
    }
}

/// Fires `Event::Timer(id)` every `ms`.
pub fn interval(ms: u32) -> u32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return sys::timer(ms, 1);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        0
    }
}

pub fn timer_cancel(id: u32) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::timer_cancel(id)
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

pub fn now_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return sys::now_ms();
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

pub fn mono_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return sys::mono_ms();
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

pub fn random(buf: &mut [u8]) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::random(buf.as_mut_ptr(), buf.len())
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = buf;
}

pub struct EnvInfo {
    pub win_w: f32,
    pub win_h: f32,
    pub screen_w: f32,
    pub screen_h: f32,
    pub scale: f32,
    pub is_mobile: bool,
    /// The local zone's offset from UTC in seconds.
    pub utc_offset: i32,
}

pub fn env() -> EnvInfo {
    let mut b = [0u8; 25];
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::env(b.as_mut_ptr(), b.len());
    }
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    EnvInfo { win_w: f(0), win_h: f(4), screen_w: f(8), screen_h: f(12), scale: f(16), is_mobile: b[20] != 0, utc_offset: f(21).to_bits() as i32 }
}

pub mod clipboard {
    pub fn write(s: &str) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return super::sys::clipboard_write(s.as_ptr(), s.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = s;
            0
        }
    }
}

/// Common X11 keysyms.
pub mod key {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const DELETE: u32 = 0xffff;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const F1: u32 = 0xffbe;
    pub const SHIFT: u8 = 1;
    pub const CTRL: u8 = 2;
    pub const ALT: u8 = 4;
    pub const SUPER: u8 = 8;
}

/// Files, through handles (docs/io.md). Errors are the negative codes in `err`.
pub mod fs {
    use super::*;

    /// The app's own folder.
    pub const APP_DIR: i32 = 0;
    /// The app's bundle (read-only).
    pub const BUNDLE_DIR: i32 = 1;

    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const CREATE: u32 = 4;
    pub const TRUNC: u32 = 8;
    pub const APPEND: u32 = 16;
    pub const EXCL: u32 = 32;

    pub fn open(dir: i32, path: &str, flags: u32) -> Result<File, i32> {
        #[cfg(target_arch = "wasm32")]
        let r = unsafe { sys::fs_open(dir, path.as_ptr(), path.len(), flags) };
        #[cfg(not(target_arch = "wasm32"))]
        let r = {
            let _ = (dir, path, flags);
            err::IO
        };
        if r < 0 {
            Err(r)
        } else {
            Ok(File(r))
        }
    }

    pub struct File(pub i32);

    impl File {
        pub fn read(&self, buf: &mut [u8]) -> i32 {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                return sys::fs_read(self.0, buf.as_mut_ptr(), buf.len());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = buf;
                err::IO
            }
        }
        pub fn write(&self, buf: &[u8]) -> i32 {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                return sys::fs_write(self.0, buf.as_ptr(), buf.len());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = buf;
                err::IO
            }
        }
        pub fn read_all(&self) -> Result<Vec<u8>, i32> {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = self.read(&mut buf);
                if n < 0 {
                    return Err(n);
                }
                if n == 0 {
                    return Ok(out);
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
        }
        pub fn write_all(&self, mut b: &[u8]) -> Result<(), i32> {
            while !b.is_empty() {
                let n = self.write(b);
                if n <= 0 {
                    return Err(if n < 0 { n } else { err::IO });
                }
                b = &b[n as usize..];
            }
            Ok(())
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                sys::fs_close(self.0);
            }
        }
    }

    pub fn read(dir: i32, path: &str) -> Result<Vec<u8>, i32> {
        open(dir, path, READ)?.read_all()
    }

    pub fn write(dir: i32, path: &str, data: &[u8]) -> Result<(), i32> {
        open(dir, path, WRITE | CREATE | TRUNC)?.write_all(data)
    }

    /// Writes `path` by writing `path.tmp` and renaming it over `path`, so a
    /// crash part-way leaves the old file whole. Use it for saved state.
    pub fn write_atomic(dir: i32, path: &str, data: &[u8]) -> Result<(), i32> {
        let tmp = cat(&[path, ".tmp"]);
        write(dir, &tmp, data)?;
        match rename(dir, &tmp, dir, path) {
            r if r < 0 => Err(r),
            _ => Ok(()),
        }
    }

    /// Moves a file or folder. `to` is replaced if it exists.
    pub fn rename(dir: i32, from: &str, to_dir: i32, to: &str) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::fs_rename(dir, from.as_ptr(), from.len(), to_dir, to.as_ptr(), to.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (dir, from, to_dir, to);
            err::IO
        }
    }

    /// (is_dir, name) pairs.
    pub fn list(dir: i32, path: &str) -> Result<Vec<(bool, String)>, i32> {
        let mut cap = 4096usize;
        loop {
            let mut buf = vec![0u8; cap];
            let mut needed = 0u32;
            #[cfg(target_arch = "wasm32")]
            let n = unsafe { sys::fs_list(dir, path.as_ptr(), path.len(), buf.as_mut_ptr(), cap, &mut needed) };
            #[cfg(not(target_arch = "wasm32"))]
            let n = {
                let _ = (dir, path, &mut needed);
                err::IO
            };
            if n == err::TOO_SMALL {
                cap = needed as usize;
                continue;
            }
            if n < 0 {
                return Err(n);
            }
            let mut r = qa_proto::Reader::new(&buf[..n as usize]);
            let mut out = Vec::new();
            while let (Some(k), Some(name)) = (r.u8(), r.str()) {
                out.push((k == 1, String::from(name)));
            }
            return Ok(out);
        }
    }

    pub fn mkdir(dir: i32, path: &str) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::fs_mkdir(dir, path.as_ptr(), path.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (dir, path);
            err::IO
        }
    }

    pub fn remove(dir: i32, path: &str) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::fs_remove(dir, path.as_ptr(), path.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (dir, path);
            err::IO
        }
    }
}

/// Requests that may prompt. Each returns a request id; the answer arrives as
/// `Event::RequestDone { req, result, info }`.
pub mod request {
    use super::*;

    fn raw(kind: u32, arg: &[u8]) -> u32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::request(kind, arg.as_ptr(), arg.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (kind, arg);
            0
        }
    }
    fn s(v: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.str(v);
        b
    }
    /// `filter` like `"*.md;*.txt"`. The result is a read-only file handle.
    pub fn open_file(filter: &str) -> u32 {
        raw(1, &s(filter))
    }
    /// The result is a write-only file handle.
    pub fn save_file(suggested: &str) -> u32 {
        raw(2, &s(suggested))
    }
    pub fn pick_folder(write: bool) -> u32 {
        raw(3, &[write as u8])
    }
    /// A folder by absolute path (prompts unless a grant exists).
    pub fn folder(path: &str, write: bool) -> u32 {
        let mut b = vec![write as u8];
        b.str(path);
        raw(4, &b)
    }
    pub fn internet() -> u32 {
        raw(5, &[])
    }
    pub fn memory(bytes: u64) -> u32 {
        raw(6, &bytes.to_le_bytes())
    }
    pub fn cpu() -> u32 {
        raw(7, &[])
    }
    pub fn clipboard_read() -> u32 {
        raw(8, &[])
    }
    pub fn message(target: &str) -> u32 {
        raw(9, &s(target))
    }
}

/// HTTP(S) and WebSocket. Results arrive as `Event::Http` / `Event::Ws`.
pub mod net {
    use super::*;
    mod client;
    pub use client::*;

    pub struct Request<'a> {
        pub method: &'a str,
        pub url: &'a str,
        /// "Name: value\n" lines.
        pub headers: &'a str,
        pub body: &'a [u8],
        /// 0 means 30 s.
        pub timeout_ms: u32,
        /// 0 means 8 MB.
        pub max_body: u32,
    }

    pub fn request(r: &Request) -> u32 {
        let mut b = Vec::new();
        b.str(r.method);
        b.str(r.url);
        b.str(r.headers);
        b.bytes(r.body);
        b.u32(r.timeout_ms);
        b.u32(r.max_body);
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::http(b.as_ptr(), b.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        0
    }

    pub fn get(url: &str) -> u32 {
        request(&Request { method: "GET", url, headers: "", body: &[], timeout_ms: 0, max_body: 0 })
    }

    pub fn cancel(req: u32) {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            sys::http_cancel(req)
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = req;
    }

    /// WebSocket event kinds.
    pub const WS_OPEN: u8 = 0;
    pub const WS_TEXT: u8 = 1;
    pub const WS_BINARY: u8 = 2;
    pub const WS_CLOSED: u8 = 3;
    pub const WS_ERROR: u8 = 4;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Ws(pub u32);

    pub fn ws(url: &str, headers: &str) -> Ws {
        let mut b = Vec::new();
        b.str(url);
        b.str(headers);
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return Ws(sys::ws_open(b.as_ptr(), b.len()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        Ws(0)
    }

    impl Ws {
        pub fn send_text(&self, s: &str) -> i32 {
            self.send(1, s.as_bytes())
        }
        pub fn send_binary(&self, b: &[u8]) -> i32 {
            self.send(2, b)
        }
        fn send(&self, kind: u32, b: &[u8]) -> i32 {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                return sys::ws_send(self.0, kind, b.as_ptr(), b.len());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (kind, b);
                0
            }
        }
        pub fn close(&self) {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                sys::ws_close(self.0, 1000)
            }
        }
    }
}

/// Messages to other apps. Returns 0 delivered, 1 queued in its inbox, or an error.
pub mod msg {
    pub fn send(to: &str, data: &[u8]) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return super::sys::msg_send(to.as_ptr(), to.len(), data.as_ptr(), data.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (to, data);
            super::err::IO
        }
    }
}

/// Small per-app secrets (tokens, passwords), kept by the runtime outside
/// the app's data folder in a file only the user can read. They are not
/// encrypted. See docs/io.md.
pub mod secret {
    use super::*;

    /// The secret stored under `key`.
    pub fn get(key: &str) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        {
            let mut buf: Vec<u8> = Vec::with_capacity(256);
            loop {
                let mut needed = 0u32;
                let n = unsafe { sys::secret_get(key.as_ptr(), key.len(), buf.as_mut_ptr(), buf.capacity(), &mut needed) };
                if n == err::TOO_SMALL {
                    buf.reserve(needed as usize);
                    continue;
                }
                if n < 0 {
                    return None;
                }
                unsafe { buf.set_len(n as usize) };
                return String::from_utf8(buf).ok();
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = key;
            None
        }
    }
    /// Stores `value` under `key`; an empty value removes it.
    pub fn set(key: &str, value: &str) -> i32 {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            return sys::secret_set(key.as_ptr(), key.len(), value.as_ptr(), value.len());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (key, value);
            err::IO
        }
    }
}

/// Opens an http(s) link in the browser. Only while handling a click or key.
pub fn open_url(url: &str) -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return sys::open_url(url.as_ptr(), url.len());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
        0
    }
}
