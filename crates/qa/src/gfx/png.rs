//! A PNG decoder: all standard color types and bit depths, non-interlaced and Adam7.

use super::{Color, Pixmap};

const MAX_PIXELS: u64 = 64 << 20;

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn unfilter(data: &[u8], w: usize, h: usize, bpp: usize, stride: usize, out: &mut Vec<u8>) -> Result<usize, String> {
    let need = (stride + 1) * h;
    if data.len() < need {
        return Err("truncated image data".into());
    }
    let start = out.len();
    out.resize(start + stride * h, 0);
    for y in 0..h {
        let f = data[y * (stride + 1)];
        let src = &data[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp { out[start + y * stride + x - bpp] } else { 0 };
            let b = if y > 0 { out[start + (y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 { out[start + (y - 1) * stride + x - bpp] } else { 0 };
            let v = match f {
                0 => src[x],
                1 => src[x].wrapping_add(a),
                2 => src[x].wrapping_add(b),
                3 => src[x].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => src[x].wrapping_add(paeth(a, b, c)),
                _ => return Err("bad filter".into()),
            };
            out[start + y * stride + x] = v;
        }
    }
    let _ = w;
    Ok(need)
}

pub fn decode(b: &[u8]) -> Result<Pixmap, String> {
    if b.len() < 8 || &b[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG".into());
    }
    let mut i = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let ty = &b[i + 4..i + 8];
        let d = b.get(i + 8..i + 8 + len).ok_or("truncated chunk")?;
        match ty {
            b"IHDR" => {
                if d.len() < 13 {
                    return Err("bad IHDR".into());
                }
                w = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize;
                h = u32::from_be_bytes([d[4], d[5], d[6], d[7]]) as usize;
                depth = d[8];
                ctype = d[9];
                interlace = d[12];
            }
            b"PLTE" => palette = d.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
            b"tRNS" => trns = d.to_vec(),
            b"IDAT" => idat.extend_from_slice(d),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    if w == 0 || h == 0 || (w as u64 * h as u64) > MAX_PIXELS {
        return Err("bad image size".into());
    }
    for (k, a) in trns.iter().enumerate() {
        if let Some(p) = palette.get_mut(k) {
            p[3] = *a;
        }
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err("bad color type".into()),
    };
    let bits = channels * depth as usize;
    let bpp = bits.div_ceil(8).max(1);
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, (w * h * 8 + h * 8) * 2).map_err(|_| "bad image data".to_string())?;
    let mut px = vec![0u32; w * h];
    let sample = |row: &[u8], x: usize, c: usize| -> u16 {
        match depth {
            16 => u16::from_be_bytes([row[(x * channels + c) * 2], row[(x * channels + c) * 2 + 1]]),
            8 => row[x * channels + c] as u16,
            d => {
                let bit = (x * channels + c) * d as usize;
                let v = (row[bit / 8] >> (8 - d as usize - bit % 8)) & ((1u16 << d) - 1) as u8;
                v as u16
            }
        }
    };
    let to8 = |v: u16| -> u8 {
        match depth {
            16 => (v >> 8) as u8,
            8 => v as u8,
            1 => (v * 255) as u8,
            2 => (v * 85) as u8,
            4 => (v * 17) as u8,
            _ => v as u8,
        }
    };
    let trns_gray = (trns.len() >= 2 && ctype == 0).then(|| u16::from_be_bytes([trns[0], trns[1]]));
    let trns_rgb = (trns.len() >= 6 && ctype == 2).then(|| [u16::from_be_bytes([trns[0], trns[1]]), u16::from_be_bytes([trns[2], trns[3]]), u16::from_be_bytes([trns[4], trns[5]])]);
    let mut put = |row: &[u8], rw: usize, y: usize, x0: usize, dx: usize| {
        for x in 0..rw {
            let c = match ctype {
                0 => {
                    let g = sample(row, x, 0);
                    let a = if Some(g) == trns_gray { 0 } else { 255 };
                    let g = to8(g);
                    Color::rgba(g, g, g, a)
                }
                2 => {
                    let rgb = [sample(row, x, 0), sample(row, x, 1), sample(row, x, 2)];
                    let a = if Some(rgb) == trns_rgb { 0 } else { 255 };
                    Color::rgba(to8(rgb[0]), to8(rgb[1]), to8(rgb[2]), a)
                }
                3 => {
                    let p = palette.get(sample(row, x, 0) as usize).copied().unwrap_or([0, 0, 0, 255]);
                    Color::rgba(p[0], p[1], p[2], p[3])
                }
                4 => {
                    let g = to8(sample(row, x, 0));
                    Color::rgba(g, g, g, to8(sample(row, x, 1)))
                }
                _ => Color::rgba(to8(sample(row, x, 0)), to8(sample(row, x, 1)), to8(sample(row, x, 2)), to8(sample(row, x, 3))),
            };
            px[y * w + x0 + x * dx] = c.premul();
        }
    };
    if interlace == 0 {
        let stride = (w * bits).div_ceil(8);
        let mut out = Vec::new();
        unfilter(&raw, w, h, bpp, stride, &mut out)?;
        for y in 0..h {
            put(&out[y * stride..(y + 1) * stride], w, y, 0, 1);
        }
    } else {
        let passes = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        let mut off = 0;
        for (x0, y0, dx, dy) in passes {
            let pw = (w + dx - 1 - x0) / dx;
            let ph = (h + dy - 1 - y0) / dy;
            if pw == 0 || ph == 0 {
                continue;
            }
            let stride = (pw * bits).div_ceil(8);
            let mut out = Vec::new();
            off += unfilter(&raw[off..], pw, ph, bpp, stride, &mut out)?;
            for r in 0..ph {
                let y = y0 + r * dy;
                put(&out[r * stride..(r + 1) * stride], pw, y, x0, dx);
            }
        }
    }
    Ok(Pixmap { w: w as i32, h: h as i32, px })
}

/// Encodes 0xRRGGBB pixels as an uncompressed (stored-deflate) RGB PNG.
pub fn encode(px: &[u32], w: usize, h: usize) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = px[y * w + x];
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &raw {
        a = (a + v as u32) % 65521;
        b = (b + a) % 65521;
    }
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut chunk = |t: &[u8], d: &[u8]| {
        out.extend_from_slice(&(d.len() as u32).to_be_bytes());
        let mut c = t.to_vec();
        c.extend_from_slice(d);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}
