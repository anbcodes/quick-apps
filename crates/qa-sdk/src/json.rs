//! JSON, parsed by the runtime into structured values (see `Val`).
//!
//! ```text
//! let doc = json::parse(body)?;
//! let v = doc.root();
//! for m in v.get("messages").iter() {
//!     let ts = m.get("ts").str();
//! }
//! ```

use super::*;
pub use qa_proto::val::{quote_into, Entries, Iter, Val, Writer};

/// Encoded value bytes that own their buffer.
#[derive(Clone, Default)]
pub struct Doc {
    pub bytes: Vec<u8>,
}

impl Doc {
    pub fn root(&self) -> Val<'_> {
        Val::new(&self.bytes)
    }
}

impl From<Writer> for Doc {
    fn from(w: Writer) -> Doc {
        Doc { bytes: w.finish() }
    }
}

/// Parses JSON. None if it isn't valid JSON.
pub fn parse(src: &[u8]) -> Option<Doc> {
    #[cfg(target_arch = "wasm32")]
    {
        // Encoded values are usually a bit larger than their JSON.
        let mut buf: Vec<u8> = Vec::with_capacity(src.len() + src.len() / 2 + 64);
        loop {
            let mut needed = 0u32;
            let n = unsafe { sys::json_parse(src.as_ptr(), src.len(), buf.as_mut_ptr(), buf.capacity(), &mut needed) };
            if n == err::TOO_SMALL {
                buf.reserve(needed as usize);
                continue;
            }
            if n < 0 {
                return None;
            }
            unsafe { buf.set_len(n as usize) };
            return Some(Doc { bytes: buf });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    qa_proto::val::from_json(src).map(|bytes| Doc { bytes })
}

/// A value as compact JSON.
pub fn stringify(v: Val) -> String {
    qa_proto::val::to_json(v)
}

/// A string as a quoted JSON literal.
pub fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    quote_into(&mut o, s);
    o
}
