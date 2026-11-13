//! Zip bundles: a reader that extracts safely and a writer for `qa pack`.

use std::path::Path;

pub const MAX_UNZIPPED: u64 = 64 << 20;

pub fn crc32(data: &[u8]) -> u32 {
    static mut TABLE: [u32; 256] = [0; 256];
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            unsafe { TABLE[i as usize] = c };
        }
    });
    let t = unsafe { &*std::ptr::addr_of!(TABLE) };
    let mut c = !0u32;
    for &b in data {
        c = t[((c ^ b as u32) & 255) as usize] ^ (c >> 8);
    }
    !c
}

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}
fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Reads every file entry. Rejects unsafe names and oversized archives.
pub fn read(zip: &[u8]) -> Result<Vec<Entry>, String> {
    let bad = || "not a valid zip file".to_string();
    // End of central directory: last 22..65557 bytes.
    let start = zip.len().saturating_sub(65557);
    let eocd = (start..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..].starts_with(&[0x50, 0x4b, 5, 6])).ok_or_else(bad)?;
    let count = u16le(zip, eocd + 10).ok_or_else(bad)? as usize;
    let mut p = u32le(zip, eocd + 16).ok_or_else(bad)? as usize;
    let mut out = Vec::new();
    let mut total = 0u64;
    for _ in 0..count {
        if u32le(zip, p) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let method = u16le(zip, p + 10).ok_or_else(bad)?;
        let crc = u32le(zip, p + 16).ok_or_else(bad)?;
        let csize = u32le(zip, p + 20).ok_or_else(bad)? as usize;
        let usize_ = u32le(zip, p + 24).ok_or_else(bad)? as u64;
        let nlen = u16le(zip, p + 28).ok_or_else(bad)? as usize;
        let xlen = u16le(zip, p + 30).ok_or_else(bad)? as usize;
        let clen = u16le(zip, p + 32).ok_or_else(bad)? as usize;
        let local = u32le(zip, p + 42).ok_or_else(bad)? as usize;
        let name = String::from_utf8(zip.get(p + 46..p + 46 + nlen).ok_or_else(bad)?.to_vec()).map_err(|_| bad())?;
        p += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue;
        }
        if name.starts_with('/') || name.contains('\\') || name.split('/').any(|c| c == ".." || c.is_empty()) {
            return Err(format!("unsafe path in bundle: {name}"));
        }
        total += usize_;
        if total > MAX_UNZIPPED {
            return Err("bundle is larger than 64 MB unzipped".into());
        }
        if u32le(zip, local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let lname = u16le(zip, local + 26).ok_or_else(bad)? as usize;
        let lextra = u16le(zip, local + 28).ok_or_else(bad)? as usize;
        let d0 = local + 30 + lname + lextra;
        let comp = zip.get(d0..d0 + csize).ok_or_else(bad)?;
        let data = match method {
            0 => comp.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(comp, usize_ as usize).map_err(|_| format!("{name}: bad deflate data"))?,
            m => return Err(format!("{name}: unsupported compression {m}")),
        };
        if data.len() as u64 != usize_ || crc32(&data) != crc {
            return Err(format!("{name}: checksum mismatch"));
        }
        out.push(Entry { name, data });
    }
    Ok(out)
}

/// Writes entries into `dir`, which must not exist yet.
pub fn extract(entries: &[Entry], dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for e in entries {
        let p = dir.join(&e.name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, &e.data).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}

/// Builds a zip, deflating entries when that makes them smaller.
pub fn write(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let crc = crc32(data);
        let def = miniz_oxide::deflate::compress_to_vec(data, 9);
        let (method, body): (u16, &[u8]) = if def.len() < data.len() { (8, &def) } else { (0, data) };
        let off = out.len() as u32;
        let mut h = Vec::new();
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&[0, 0, 0x21, 0]); // time/date: 1980-01-01
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&(body.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        let mut c = Vec::new();
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&h[4..30]);
        // comment length, disk, internal and external attributes
        c.extend_from_slice(&[0; 10]);
        c.extend_from_slice(&off.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&c);
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod t {
    #[test]
    fn roundtrip_and_reject() {
        let z = super::write(&[("app.wasm".into(), vec![1, 2, 3]), ("assets/a.txt".into(), b"hello hello hello hello hello".to_vec())]);
        let e = super::read(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "app.wasm");
        assert_eq!(e[0].data, vec![1, 2, 3]);
        assert_eq!(e[1].data, b"hello hello hello hello hello");
        let bad = super::write(&[("../x".into(), vec![1])]);
        assert!(super::read(&bad).is_err());
        assert_eq!(super::crc32(b"123456789"), 0xcbf4_3926);
    }
}
