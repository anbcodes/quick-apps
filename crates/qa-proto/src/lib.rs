//! The Quick Apps wire format, shared by the runtime and the SDK.
//!
//! Everything is little-endian. See docs/app-abi.md, docs/ui-tree.md and
//! docs/events.md for the meaning of each id.
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod val;

/// The ABI version this crate describes (the manifest's `runtime` field).
pub const ABI_VERSION: u32 = 1;

pub mod op {
    pub const CREATE: u8 = 1;
    pub const DESTROY: u8 = 2;
    pub const APPEND: u8 = 3;
    pub const INSERT: u8 = 4;
    pub const DETACH: u8 = 5;
    pub const SET: u8 = 6;
    pub const CTX: u8 = 7;
    pub const ROOT: u8 = 8;
    pub const FOCUS: u8 = 9;
    pub const SCROLL_TO: u8 = 10;
    pub const REPLACE: u8 = 11;
}

pub mod kind {
    pub const GRID: u8 = 1;
    pub const TEXT: u8 = 2;
    pub const IMAGE: u8 = 3;
    pub const CANVAS: u8 = 4;
    pub const SCROLL: u8 = 5;
    pub const BUTTON: u8 = 10;
    pub const INPUT: u8 = 11;
    pub const CHECKBOX: u8 = 12;
    pub const SELECT: u8 = 13;
    pub const TABS: u8 = 14;
    pub const PROGRESS: u8 = 15;
    pub const DIVIDER: u8 = 16;

    pub fn is_builtin(k: u8) -> bool {
        matches!(k, 1..=5 | 10..=16)
    }
}

/// Prop ids. 1–19 are common to every kind; 20 and up depend on the kind.
pub mod prop {
    pub const ROW: u16 = 1;
    pub const COL: u16 = 2;
    pub const ROW_SPAN: u16 = 3;
    pub const COL_SPAN: u16 = 4;
    pub const PRIORITY: u16 = 5;
    pub const MIN_W: u16 = 6;
    pub const MIN_H: u16 = 7;
    pub const MAX_W: u16 = 8;
    pub const MAX_H: u16 = 9;
    pub const PAD: u16 = 10;
    pub const ALIGN_X: u16 = 11;
    pub const ALIGN_Y: u16 = 12;
    pub const HIDDEN: u16 = 13;
    pub const VARIANT: u16 = 14;
    pub const TOOLTIP: u16 = 15;
    /// An i32 sent back with this node's events.
    pub const TAG: u16 = 16;

    pub mod grid {
        pub const COLS: u16 = 20;
        pub const ROWS: u16 = 21;
        pub const GAP: u16 = 22;
        pub const CLICKABLE: u16 = 23;
        pub const SELECTED: u16 = 24;
        pub const FLOW: u16 = 25;
    }
    pub mod text {
        pub const TEXT: u16 = 20;
        pub const WRAP: u16 = 21;
        pub const STYLE: u16 = 22;
        pub const TEXT_ALIGN: u16 = 23;
        pub const SELECTABLE: u16 = 24;
        pub const MAX_LINES: u16 = 25;
        /// Bytes, see `span`.
        pub const SPANS: u16 = 26;
    }
    pub mod image {
        pub const SRC: u16 = 20;
        pub const DATA: u16 = 21;
        pub const FIT: u16 = 22;
    }
    pub mod canvas {
        pub const PX_W: u16 = 20;
        pub const PX_H: u16 = 21;
        pub const FIT: u16 = 22;
        pub const FOCUSABLE: u16 = 23;
    }
    pub mod scroll {
        pub const AXIS: u16 = 20;
        pub const STICK_END: u16 = 21;
    }
    pub mod button {
        pub const LABEL: u16 = 20;
        pub const DISABLED: u16 = 21;
    }
    pub mod input {
        pub const VALUE: u16 = 20;
        pub const PLACEHOLDER: u16 = 21;
        pub const MULTILINE: u16 = 22;
        pub const SECRET: u16 = 23;
        pub const DISABLED: u16 = 24;
        pub const LINES: u16 = 25;
    }
    pub mod checkbox {
        pub const LABEL: u16 = 20;
        pub const CHECKED: u16 = 21;
        pub const DISABLED: u16 = 22;
    }
    pub mod select {
        pub const OPTIONS: u16 = 20;
        pub const SELECTED: u16 = 21;
        pub const PLACEHOLDER: u16 = 22;
        pub const DISABLED: u16 = 23;
    }
    pub mod tabs {
        pub const LABELS: u16 = 20;
        pub const SELECTED: u16 = 21;
    }
    pub mod progress {
        pub const VALUE: u16 = 20;
    }
}

pub mod ev {
    pub const RESIZE: u8 = 1;
    pub const CLICK: u8 = 2;
    pub const CHANGE: u8 = 3;
    pub const SUBMIT: u8 = 4;
    pub const TOGGLE: u8 = 5;
    pub const SELECT: u8 = 6;
    pub const KEY: u8 = 7;
    pub const POINTER: u8 = 8;
    pub const TIMER: u8 = 9;
    pub const HTTP: u8 = 10;
    pub const WS: u8 = 11;
    pub const MESSAGE: u8 = 12;
    pub const REQUEST_DONE: u8 = 13;
    pub const CLOSE: u8 = 14;
    pub const ACTIVATE: u8 = 15;
    pub const FOCUS_OUT: u8 = 16;
    pub const NEAR_END: u8 = 17;
    pub const LINK: u8 = 18;
}

/// Text span records (`prop::text::SPANS`): repeated
/// `u32 start, u32 end, u8 flags, str variant`, byte offsets into the text.
pub mod span {
    pub const BOLD: u8 = 1;
    pub const UNDERLINE: u8 = 2;
    pub const CODE: u8 = 4;
    pub const LINK: u8 = 16;
}

pub mod tag {
    pub const NONE: u8 = 0;
    pub const I32: u8 = 1;
    pub const F32: u8 = 2;
    pub const STR: u8 = 3;
    pub const COLOR: u8 = 4;
    pub const BYTES: u8 = 5;
    pub const TRACKS: u8 = 6;
    pub const EDGES: u8 = 7;
}

/// Error codes returned by imports.
pub mod err {
    pub const DENIED: i32 = -1;
    pub const NOT_FOUND: i32 = -2;
    pub const EXISTS: i32 = -3;
    pub const INVALID: i32 = -4;
    pub const IO: i32 = -5;
    pub const LIMIT: i32 = -6;
    pub const BAD_HANDLE: i32 = -7;
    pub const TOO_SMALL: i32 = -8;
}

pub mod track {
    pub const PX: u8 = 0;
    pub const FR: u8 = 1;
    pub const AUTO: u8 = 2;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    pub kind: u8,
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

impl Track {
    pub const fn px(v: f32) -> Track {
        Track { kind: track::PX, value: v, min: f32::INFINITY, max: f32::INFINITY }
    }
    pub const fn fr(v: f32) -> Track {
        Track { kind: track::FR, value: v, min: f32::INFINITY, max: f32::INFINITY }
    }
    pub const fn auto() -> Track {
        Track { kind: track::AUTO, value: 0.0, min: f32::INFINITY, max: f32::INFINITY }
    }
    pub const fn min(mut self, v: f32) -> Track {
        self.min = v;
        self
    }
    pub const fn max(mut self, v: f32) -> Track {
        self.max = v;
        self
    }
}

/// A decoded value that borrows from the buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    None,
    I32(i32),
    F32(f32),
    Str(&'a str),
    Color(u32),
    Bytes(&'a [u8]),
    /// Raw track records: 13 bytes each.
    Tracks(&'a [u8]),
    Edges([f32; 4]),
}

/// Iterates the tracks of a `Value::Tracks`.
pub fn tracks(raw: &[u8]) -> impl Iterator<Item = Track> + '_ {
    raw.chunks_exact(13).map(|c| Track {
        kind: c[0],
        value: f32::from_le_bytes([c[1], c[2], c[3], c[4]]),
        min: f32::from_le_bytes([c[5], c[6], c[7], c[8]]),
        max: f32::from_le_bytes([c[9], c[10], c[11], c[12]]),
    })
}

pub struct Reader<'a> {
    pub b: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    pub fn done(&self) -> bool {
        self.pos >= self.b.len()
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.b.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn i32(&mut self) -> Option<i32> {
        Some(self.u32()? as i32)
    }
    pub fn u64(&mut self) -> Option<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Some(u64::from_le_bytes(a))
    }
    pub fn i64(&mut self) -> Option<i64> {
        Some(self.u64()? as i64)
    }
    pub fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes()?).ok()
    }
    pub fn value(&mut self) -> Option<Value<'a>> {
        Some(match self.u8()? {
            tag::NONE => Value::None,
            tag::I32 => Value::I32(self.i32()?),
            tag::F32 => Value::F32(self.f32()?),
            tag::STR => Value::Str(self.str()?),
            tag::COLOR => Value::Color(self.u32()?),
            tag::BYTES => Value::Bytes(self.bytes()?),
            tag::TRACKS => {
                let n = self.u8()? as usize;
                Value::Tracks(self.take(n * 13)?)
            }
            tag::EDGES => Value::Edges([self.f32()?, self.f32()?, self.f32()?, self.f32()?]),
            _ => return None,
        })
    }
}

/// Appends encoded data to a byte buffer.
pub trait Sink {
    fn put(&mut self, b: &[u8]);

    fn u8(&mut self, v: u8) {
        self.put(&[v]);
    }
    fn u16(&mut self, v: u16) {
        self.put(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.put(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.put(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.put(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.put(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn value(&mut self, v: &Value) {
        match *v {
            Value::None => self.u8(tag::NONE),
            Value::I32(x) => {
                self.u8(tag::I32);
                self.i32(x)
            }
            Value::F32(x) => {
                self.u8(tag::F32);
                self.f32(x)
            }
            Value::Str(s) => {
                self.u8(tag::STR);
                self.str(s)
            }
            Value::Color(c) => {
                self.u8(tag::COLOR);
                self.u32(c)
            }
            Value::Bytes(b) => {
                self.u8(tag::BYTES);
                self.bytes(b)
            }
            Value::Tracks(raw) => {
                self.u8(tag::TRACKS);
                self.u8((raw.len() / 13) as u8);
                self.put(raw)
            }
            Value::Edges(e) => {
                self.u8(tag::EDGES);
                for x in e {
                    self.f32(x);
                }
            }
        }
    }
    fn tracks(&mut self, ts: &[Track]) {
        self.u8(tag::TRACKS);
        self.u8(ts.len() as u8);
        for t in ts {
            self.u8(t.kind);
            self.f32(t.value);
            self.f32(t.min);
            self.f32(t.max);
        }
    }

    // Patch ops.
    fn op_create(&mut self, id: u32, kind: u8) {
        self.u8(op::CREATE);
        self.u32(id);
        self.u8(kind);
    }
    fn op_destroy(&mut self, id: u32) {
        self.u8(op::DESTROY);
        self.u32(id);
    }
    fn op_append(&mut self, parent: u32, child: u32) {
        self.u8(op::APPEND);
        self.u32(parent);
        self.u32(child);
    }
    fn op_insert(&mut self, parent: u32, child: u32, before: u32) {
        self.u8(op::INSERT);
        self.u32(parent);
        self.u32(child);
        self.u32(before);
    }
    fn op_detach(&mut self, id: u32) {
        self.u8(op::DETACH);
        self.u32(id);
    }
    /// Writes the SET header; follow it with a value.
    fn op_set(&mut self, id: u32, prop: u16) {
        self.u8(op::SET);
        self.u32(id);
        self.u16(prop);
    }
    /// Writes the CTX header; follow it with a value.
    fn op_ctx(&mut self, id: u32, key: &str) {
        self.u8(op::CTX);
        self.u32(id);
        self.str(key);
    }
    fn op_root(&mut self, id: u32) {
        self.u8(op::ROOT);
        self.u32(id);
    }
    fn op_focus(&mut self, id: u32) {
        self.u8(op::FOCUS);
        self.u32(id);
    }
    fn op_scroll_to(&mut self, id: u32) {
        self.u8(op::SCROLL_TO);
        self.u32(id);
    }
    fn op_replace(&mut self, old: u32, new: u32) {
        self.u8(op::REPLACE);
        self.u32(old);
        self.u32(new);
    }
}

#[cfg(feature = "alloc")]
impl Sink for alloc::vec::Vec<u8> {
    fn put(&mut self, b: &[u8]) {
        self.extend_from_slice(b);
    }
}
