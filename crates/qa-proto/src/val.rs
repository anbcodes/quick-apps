//! Structured values: a small, language-neutral encoding for JSON-shaped
//! data (null, bools, numbers, strings, lists and maps). The runtime's
//! `json_parse` import produces it; see docs/app-abi.md.
//!
//! ```text
//! 0 null | 1 false | 2 true | 3 f64 number | 4 u32 len, UTF-8
//! 5 list: u32 count, u32 size, items
//! 6 map:  u32 count, u32 size, count × (u32 len + UTF-8 key, value)
//! ```
//!
//! `size` is the byte length of what follows it, so readers can skip a list
//! or map without walking it. Reading never panics: anything missing or
//! malformed reads as null.

pub const NULL: u8 = 0;
pub const FALSE: u8 = 1;
pub const TRUE: u8 = 2;
pub const NUM: u8 = 3;
pub const STR: u8 = 4;
pub const LIST: u8 = 5;
pub const MAP: u8 = 6;

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    let s = b.get(i..i.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The encoded length of the value at the start of `b`.
fn size(b: &[u8]) -> Option<usize> {
    let n = match *b.first()? {
        NULL | FALSE | TRUE => 1,
        NUM => 9,
        STR => 5 + u32_at(b, 1)? as usize,
        LIST | MAP => 9 + u32_at(b, 5)? as usize,
        _ => return None,
    };
    (n <= b.len()).then_some(n)
}

/// A value that borrows its encoded bytes.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Val<'a> {
    b: &'a [u8],
}

impl<'a> Val<'a> {
    pub const NULL: Val<'static> = Val { b: &[] };

    /// The value at the start of `b` (null if it's malformed).
    pub fn new(b: &'a [u8]) -> Val<'a> {
        match size(b) {
            Some(n) => Val { b: &b[..n] },
            None => Val::NULL,
        }
    }
    /// The encoded bytes of this value.
    pub fn raw(&self) -> &'a [u8] {
        self.b
    }
    /// One of the type ids above.
    pub fn kind(&self) -> u8 {
        self.b.first().copied().unwrap_or(NULL)
    }
    pub fn is_null(&self) -> bool {
        self.kind() == NULL
    }
    pub fn bool(&self) -> bool {
        self.kind() == TRUE
    }
    /// The number, else 0. (Strings aren't parsed: that pulls float
    /// parsing into every app that reads a number.)
    pub fn num(&self) -> f64 {
        match self.kind() {
            NUM => {
                let mut a = [0u8; 8];
                a.copy_from_slice(&self.b[1..9]);
                f64::from_le_bytes(a)
            }
            _ => 0.0,
        }
    }
    pub fn int(&self) -> i64 {
        self.num() as i64
    }
    /// The string, else "".
    pub fn str(&self) -> &'a str {
        match self.kind() {
            STR => core::str::from_utf8(&self.b[5..]).unwrap_or(""),
            _ => "",
        }
    }
    /// The string if it's a non-empty one.
    pub fn opt_str(&self) -> Option<&'a str> {
        Some(self.str()).filter(|s| !s.is_empty())
    }
    /// Items in a list, entries in a map, else 0.
    pub fn len(&self) -> usize {
        match self.kind() {
            LIST | MAP => u32_at(self.b, 1).unwrap_or(0) as usize,
            _ => 0,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// List items (or map values), in order.
    pub fn iter(&self) -> Iter<'a> {
        match self.kind() {
            LIST => Iter { b: &self.b[9..], map: false },
            MAP => Iter { b: &self.b[9..], map: true },
            _ => Iter { b: &[], map: false },
        }
    }
    /// Map entries, in order.
    pub fn entries(&self) -> Entries<'a> {
        Entries { b: if self.kind() == MAP { &self.b[9..] } else { &[] } }
    }
    /// The i-th list item.
    pub fn at(&self, i: usize) -> Val<'a> {
        if self.kind() != LIST {
            return Val::NULL;
        }
        self.iter().nth(i).unwrap_or(Val::NULL)
    }
    /// A map entry (the first, if the key repeats).
    pub fn get(&self, key: &str) -> Val<'a> {
        self.entries().find(|(k, _)| *k == key).map(|(_, v)| v).unwrap_or(Val::NULL)
    }
    /// Follows a dotted path; numeric parts index lists: "messages.0.ts".
    pub fn path(&self, p: &str) -> Val<'a> {
        p.split('.').fold(*self, |v, k| match (v.kind(), k.parse::<usize>()) {
            (LIST, Ok(i)) => v.at(i),
            _ => v.get(k),
        })
    }
}

impl core::fmt::Debug for Val<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.kind() {
            FALSE | TRUE => write!(f, "{}", self.bool()),
            NUM => write!(f, "{}", self.num()),
            STR => write!(f, "{:?}", self.str()),
            LIST => f.debug_list().entries(self.iter()).finish(),
            MAP => f.debug_map().entries(self.entries()).finish(),
            _ => f.write_str("null"),
        }
    }
}

fn split(b: &[u8]) -> Option<(Val<'_>, &[u8])> {
    let n = size(b)?;
    Some((Val { b: &b[..n] }, &b[n..]))
}

fn key(b: &[u8]) -> Option<(&str, &[u8])> {
    let n = u32_at(b, 0)? as usize;
    let s = b.get(4..4usize.checked_add(n)?)?;
    Some((core::str::from_utf8(s).ok()?, &b[4 + n..]))
}

pub struct Iter<'a> {
    b: &'a [u8],
    map: bool,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Val<'a>;
    fn next(&mut self) -> Option<Val<'a>> {
        let mut b = self.b;
        if self.map {
            b = key(b)?.1;
        }
        let (v, rest) = split(b)?;
        self.b = rest;
        Some(v)
    }
}

pub struct Entries<'a> {
    b: &'a [u8],
}

impl<'a> Iterator for Entries<'a> {
    type Item = (&'a str, Val<'a>);
    fn next(&mut self) -> Option<(&'a str, Val<'a>)> {
        let (k, b) = key(self.b)?;
        let (v, rest) = split(b)?;
        self.b = rest;
        Some((k, v))
    }
}

#[cfg(feature = "alloc")]
pub use w::*;

#[cfg(feature = "alloc")]
mod w {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Builds an encoded value. Containers are opened with `list`/`map` and
    /// closed with `end`; inside a map, call `key` before each value.
    #[derive(Default)]
    pub struct Writer {
        pub buf: Vec<u8>,
        /// Open containers: (header offset, item count).
        open: Vec<(usize, u32)>,
    }

    impl Writer {
        pub fn new() -> Writer {
            Writer::default()
        }
        fn item(&mut self) {
            if let Some(top) = self.open.last_mut() {
                top.1 += 1;
            }
        }
        pub fn null(&mut self) -> &mut Self {
            self.item();
            self.buf.push(NULL);
            self
        }
        pub fn bool(&mut self, v: bool) -> &mut Self {
            self.item();
            self.buf.push(if v { TRUE } else { FALSE });
            self
        }
        pub fn num(&mut self, v: f64) -> &mut Self {
            self.item();
            self.buf.push(NUM);
            self.buf.extend_from_slice(&v.to_le_bytes());
            self
        }
        pub fn str(&mut self, v: &str) -> &mut Self {
            self.item();
            self.buf.push(STR);
            self.buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
            self.buf.extend_from_slice(v.as_bytes());
            self
        }
        /// Copies an already-encoded value.
        pub fn val(&mut self, v: Val) -> &mut Self {
            if v.raw().is_empty() {
                return self.null();
            }
            self.item();
            self.buf.extend_from_slice(v.raw());
            self
        }
        fn open(&mut self, kind: u8) -> &mut Self {
            self.item();
            self.open.push((self.buf.len(), 0));
            self.buf.push(kind);
            self.buf.extend_from_slice(&[0; 8]);
            self
        }
        pub fn list(&mut self) -> &mut Self {
            self.open(LIST)
        }
        pub fn map(&mut self) -> &mut Self {
            self.open(MAP)
        }
        /// A map key; the next value is its value.
        pub fn key(&mut self, k: &str) -> &mut Self {
            self.buf.extend_from_slice(&(k.len() as u32).to_le_bytes());
            self.buf.extend_from_slice(k.as_bytes());
            self
        }
        /// Closes the innermost list or map.
        pub fn end(&mut self) -> &mut Self {
            if let Some((at, n)) = self.open.pop() {
                let size = (self.buf.len() - at - 9) as u32;
                self.buf[at + 1..at + 5].copy_from_slice(&n.to_le_bytes());
                self.buf[at + 5..at + 9].copy_from_slice(&size.to_le_bytes());
            }
            self
        }
        /// Closes anything still open and returns the bytes.
        pub fn finish(mut self) -> Vec<u8> {
            while !self.open.is_empty() {
                self.end();
            }
            self.buf
        }
    }

    /// Parses JSON into the encoding. None if it isn't valid JSON.
    pub fn from_json(src: &[u8]) -> Option<Vec<u8>> {
        let mut p = P { s: src, i: 0, w: Writer::new() };
        p.ws();
        p.value(0)?;
        p.ws();
        (p.i == src.len()).then(|| p.w.finish())
    }

    /// Writes a value as compact JSON. Integral numbers print without a
    /// fraction; NaN and infinities print as null.
    pub fn to_json(v: Val) -> String {
        let mut s = String::new();
        json_into(&mut s, v);
        s
    }

    pub fn json_into(out: &mut String, v: Val) {
        match v.kind() {
            FALSE => out.push_str("false"),
            TRUE => out.push_str("true"),
            NUM => num_into(out, v.num()),
            STR => quote_into(out, v.str()),
            LIST => {
                out.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    json_into(out, x);
                }
                out.push(']');
            }
            MAP => {
                out.push('{');
                for (i, (k, x)) in v.entries().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote_into(out, k);
                    out.push(':');
                    json_into(out, x);
                }
                out.push('}');
            }
            _ => out.push_str("null"),
        }
    }

    fn digits_into(out: &mut String, mut v: u64) {
        let mut b = [0u8; 20];
        let mut i = b.len();
        loop {
            i -= 1;
            b[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for &c in &b[i..] {
            out.push(c as char);
        }
    }

    /// Appends a number as JSON, without `core::fmt` (which would add about
    /// 20 KB to an app). Whole numbers are exact up to 1e15; others keep 15
    /// significant digits.
    pub fn num_into(out: &mut String, n: f64) {
        if !n.is_finite() {
            return out.push_str("null");
        }
        if n < 0.0 {
            out.push('-');
        }
        let mut a = if n < 0.0 { -n } else { n };
        if a < 1e15 && a == (a as u64) as f64 {
            return digits_into(out, a as u64);
        }
        // Scale to 1 <= a < 10 and remember the power of ten.
        let mut e = 0i32;
        while a >= 1e16 {
            a /= 1e16;
            e += 16;
        }
        while a < 1e-15 {
            a *= 1e16;
            e -= 16;
        }
        while a >= 10.0 {
            a /= 10.0;
            e += 1;
        }
        while a < 1.0 {
            a *= 10.0;
            e -= 1;
        }
        let mut d = (a * 1e14 + 0.5) as u64;
        if d >= 1_000_000_000_000_000 {
            d /= 10;
            e += 1;
        }
        let mut dig = [0u8; 15];
        for i in (0..15).rev() {
            dig[i] = b'0' + (d % 10) as u8;
            d /= 10;
        }
        let mut len = 15;
        while len > 1 && dig[len - 1] == b'0' {
            len -= 1;
        }
        let dig = &dig[..len];
        let put = |out: &mut String, b: &[u8]| b.iter().for_each(|&c| out.push(c as char));
        if (-5..15).contains(&e) {
            if e < 0 {
                out.push_str("0.");
                for _ in 0..(-e - 1) {
                    out.push('0');
                }
                put(out, dig);
            } else {
                let whole = (e as usize + 1).min(len);
                put(out, &dig[..whole]);
                for _ in whole..(e as usize + 1) {
                    out.push('0');
                }
                if len > whole {
                    out.push('.');
                    put(out, &dig[whole..]);
                }
            }
        } else {
            put(out, &dig[..1]);
            if len > 1 {
                out.push('.');
                put(out, &dig[1..]);
            }
            out.push('e');
            if e < 0 {
                out.push('-');
            }
            digits_into(out, e.unsigned_abs() as u64);
        }
    }

    /// Appends `s` as a quoted JSON string.
    pub fn quote_into(o: &mut String, s: &str) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        o.push('"');
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    o.push_str("\\u00");
                    o.push(HEX[(c as usize) >> 4] as char);
                    o.push(HEX[(c as usize) & 15] as char);
                }
                c => o.push(c),
            }
        }
        o.push('"');
    }

    struct P<'a> {
        s: &'a [u8],
        i: usize,
        w: Writer,
    }

    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
        }
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }
        fn eat(&mut self, c: u8) -> bool {
            self.ws();
            let hit = self.peek() == Some(c);
            self.i += hit as usize;
            hit
        }
        fn lit(&mut self, w: &[u8]) -> Option<()> {
            self.s[self.i..].starts_with(w).then(|| self.i += w.len())
        }

        fn value(&mut self, depth: usize) -> Option<()> {
            if depth > 256 {
                return None;
            }
            match self.peek()? {
                b'{' => {
                    self.i += 1;
                    self.w.map();
                    if !self.eat(b'}') {
                        loop {
                            self.ws();
                            if self.peek() != Some(b'"') {
                                return None;
                            }
                            let k = self.string()?;
                            self.w.key(&k);
                            if !self.eat(b':') {
                                return None;
                            }
                            self.ws();
                            self.value(depth + 1)?;
                            if self.eat(b'}') {
                                break;
                            }
                            if !self.eat(b',') {
                                return None;
                            }
                        }
                    }
                    self.w.end();
                }
                b'[' => {
                    self.i += 1;
                    self.w.list();
                    if !self.eat(b']') {
                        loop {
                            self.ws();
                            self.value(depth + 1)?;
                            if self.eat(b']') {
                                break;
                            }
                            if !self.eat(b',') {
                                return None;
                            }
                        }
                    }
                    self.w.end();
                }
                b'"' => {
                    let s = self.string()?;
                    self.w.str(&s);
                }
                b't' => {
                    self.lit(b"true")?;
                    self.w.bool(true);
                }
                b'f' => {
                    self.lit(b"false")?;
                    self.w.bool(false);
                }
                b'n' => {
                    self.lit(b"null")?;
                    self.w.null();
                }
                b'-' | b'0'..=b'9' => {
                    let st = self.i;
                    while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                        self.i += 1;
                    }
                    let t = core::str::from_utf8(&self.s[st..self.i]).ok()?;
                    self.w.num(t.parse().ok()?);
                }
                _ => return None,
            }
            Some(())
        }

        fn hex4(&mut self) -> Option<u32> {
            let t = core::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
            let v = u32::from_str_radix(t, 16).ok()?;
            self.i += 4;
            Some(v)
        }

        fn string(&mut self) -> Option<String> {
            self.i += 1; // opening quote
            let mut out: Vec<u8> = Vec::new();
            loop {
                let c = *self.s.get(self.i)?;
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let e = *self.s.get(self.i)?;
                        self.i += 1;
                        let ch = match e {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'b' => '\u{8}',
                            b'f' => '\u{c}',
                            b'u' => {
                                let mut u = self.hex4()?;
                                if (0xd800..0xdc00).contains(&u) && self.s[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    u = 0x10000 + ((u - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff);
                                }
                                char::from_u32(u).unwrap_or('\u{fffd}')
                            }
                            c => c as char,
                        };
                        let mut b = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                    }
                    c => out.push(c),
                }
            }
            Some(match String::from_utf8(out) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into(),
            })
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use alloc::string::String;

    #[test]
    fn numbers_print_without_fmt() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (42.0, "42"),
            (-7.0, "-7"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (3.14159, "3.14159"),
            (1234.5678, "1234.5678"),
            (0.1 + 0.2, "0.3"),
            (0.00012, "0.00012"),
            (1e21, "1e21"),
            (1.5e-9, "1.5e-9"),
            (123456789012345.0, "123456789012345"),
            (1e15, "1e15"),
            (f64::NAN, "null"),
        ];
        for (n, want) in cases {
            let mut s = String::new();
            num_into(&mut s, *n);
            assert_eq!(&s, want, "{n}");
            if n.is_finite() {
                let back: f64 = s.parse().unwrap();
                assert!((back - n).abs() <= n.abs() * 1e-14, "{n} -> {s}");
            }
        }
    }

    #[test]
    fn round_trip() {
        let src = r#"{"ok":true,"n":-1.5,"s":"a\"\u00e9\ud83d\ude00","l":[1,null,{"x":[]}],"e":{}}"#.as_bytes();
        let b = from_json(src).unwrap();
        let v = Val::new(&b);
        assert!(v.get("ok").bool());
        assert_eq!(v.get("n").num(), -1.5);
        assert_eq!(v.get("s").str(), "a\"é😀");
        assert_eq!(v.get("l").len(), 3);
        assert!(v.path("l.1").is_null());
        assert_eq!(v.path("l.2.x").kind(), LIST);
        assert!(v.path("l.9.x").is_null());
        assert!(v.get("missing").get("deeper").is_null());
        assert_eq!(to_json(v), r#"{"ok":true,"n":-1.5,"s":"a\"é😀","l":[1,null,{"x":[]}],"e":{}}"#);
        assert!(from_json(b"[1,]").is_none());
        assert!(from_json(b"{} x").is_none());
        // Truncated input reads as null, never panics.
        for i in 0..b.len() {
            let _ = to_json(Val::new(&b[..i]));
        }
    }
}
