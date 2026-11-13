//! Blocking HTTP/1.1 with redirects, gzip/deflate and a body limit, over mhttp's TLS.

use mhttp::{connect, read_head, Url};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

pub struct Req {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub max_body: usize,
}

pub struct Resp {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

impl Req {
    /// Decodes the `http` import's argument (docs/io.md).
    pub fn decode(b: &[u8]) -> Option<Req> {
        let mut r = qa_proto::Reader::new(b);
        let method = r.str()?.to_ascii_uppercase();
        let url = r.str()?.to_string();
        let headers = r
            .str()?
            .lines()
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        let body = r.bytes()?.to_vec();
        let t = r.u32().unwrap_or(0);
        let m = r.u32().unwrap_or(0);
        Some(Req {
            method: if method.is_empty() { "GET".into() } else { method },
            url,
            headers,
            body,
            timeout: Duration::from_millis(if t == 0 { 30_000 } else { t as u64 }),
            max_body: if m == 0 { 8 << 20 } else { m as usize },
        })
    }
}

fn host_header(u: &Url) -> String {
    let default = if u.tls { 443 } else { 80 };
    if u.port == default {
        u.host.clone()
    } else {
        format!("{}:{}", u.host, u.port)
    }
}

fn read_body(r: &mut impl BufRead, hdrs: &[(String, String)], method: &str, status: u16, max: usize) -> Result<Vec<u8>, String> {
    let get = |k: &str| hdrs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
    let mut body = Vec::new();
    let too_big = || format!("response body larger than {max} bytes");
    if method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(body);
    }
    if get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        let mut line = String::new();
        loop {
            line.clear();
            r.read_line(&mut line).map_err(|e| e.to_string())?;
            let n = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| "bad chunk size".to_string())?;
            if n == 0 {
                break;
            }
            if body.len() + n > max {
                return Err(too_big());
            }
            let at = body.len();
            body.resize(at + n, 0);
            r.read_exact(&mut body[at..]).map_err(|e| e.to_string())?;
            line.clear();
            r.read_line(&mut line).map_err(|e| e.to_string())?;
        }
    } else if let Some(n) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if n > max {
            return Err(too_big());
        }
        body.resize(n, 0);
        r.read_exact(&mut body).map_err(|e| e.to_string())?;
    } else {
        r.take(max as u64 + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
        if body.len() > max {
            return Err(too_big());
        }
    }
    Ok(body)
}

fn decompress(enc: Option<&str>, body: Vec<u8>, max: usize) -> Result<Vec<u8>, String> {
    use miniz_oxide::inflate::{decompress_to_vec_with_limit, decompress_to_vec_zlib_with_limit};
    match enc.map(|e| e.trim().to_ascii_lowercase()) {
        Some(e) if e == "gzip" || e == "x-gzip" => {
            let raw = gzip_payload(&body).ok_or("bad gzip header")?;
            decompress_to_vec_with_limit(raw, max).map_err(|_| "bad gzip body or too large".into())
        }
        Some(e) if e == "deflate" => decompress_to_vec_zlib_with_limit(&body, max)
            .or_else(|_| decompress_to_vec_with_limit(&body, max))
            .map_err(|_| "bad deflate body or too large".into()),
        _ => Ok(body),
    }
}

/// The deflate stream inside a gzip member.
pub fn gzip_payload(b: &[u8]) -> Option<&[u8]> {
    if b.len() < 18 || b[0] != 0x1f || b[1] != 0x8b || b[2] != 8 {
        return None;
    }
    let flags = b[3];
    let mut i = 10;
    if flags & 4 != 0 {
        let n = u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
        i += 2 + n;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            while *b.get(i)? != 0 {
                i += 1;
            }
            i += 1;
        }
    }
    if flags & 2 != 0 {
        i += 2;
    }
    b.get(i..b.len().checked_sub(8)?)
}

fn resolve(base: &Url, loc: &str) -> String {
    if loc.contains("://") {
        return loc.to_string();
    }
    let scheme = if base.tls { "https" } else { "http" };
    let hp = host_header(base);
    if loc.starts_with('/') {
        format!("{scheme}://{hp}{loc}")
    } else {
        let dir = base.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        format!("{scheme}://{hp}{dir}/{loc}")
    }
}

pub fn fetch(req: &Req) -> Result<Resp, String> {
    let mut url = req.url.clone();
    let mut method = req.method.clone();
    let mut body = req.body.clone();
    let origin = Url::parse(&url)?.host;
    for _ in 0..=5 {
        let u = Url::parse(&url)?;
        let mut s = connect(&u, req.timeout)?;
        let mut head = format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: quick-apps/{}\r\nAccept-Encoding: gzip, deflate\r\nConnection: close\r\n",
            u.path,
            host_header(&u),
            env!("CARGO_PKG_VERSION")
        );
        if !body.is_empty() || matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
            head += &format!("Content-Length: {}\r\n", body.len());
        }
        for (k, v) in &req.headers {
            let lk = k.to_ascii_lowercase();
            if matches!(lk.as_str(), "host" | "content-length" | "connection" | "transfer-encoding" | "accept-encoding") {
                continue;
            }
            // Credentials do not follow a redirect to another host.
            if u.host != origin && matches!(lk.as_str(), "authorization" | "cookie") {
                continue;
            }
            if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
                return Err("header contains a line break".into());
            }
            head += &format!("{k}: {v}\r\n");
        }
        head += "\r\n";
        let mut out = head.into_bytes();
        out.extend_from_slice(&body);
        s.write_all(&out).and_then(|_| s.flush()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(s);
        let (status, hdrs) = read_head(&mut r).map_err(|e| e.to_string())?;
        let get = |k: &str| hdrs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if let Some(loc) = get("location") {
                url = resolve(&u, &loc);
                if status == 303 || ((status == 301 || status == 302) && method == "POST") {
                    method = "GET".into();
                    body.clear();
                }
                continue;
            }
        }
        let raw = read_body(&mut r, &hdrs, &method, status, req.max_body)?;
        let data = decompress(get("content-encoding").as_deref(), raw, req.max_body)?;
        let mut hs = String::new();
        for (k, v) in &hdrs {
            hs.push_str(k);
            hs.push_str(": ");
            hs.push_str(v);
            hs.push('\n');
        }
        return Ok(Resp { status, headers: hs, body: data });
    }
    Err("too many redirects".into())
}

pub fn encode_event(req: u32, r: Result<Resp, String>) -> Vec<u8> {
    use qa_proto::Sink;
    let mut b = vec![qa_proto::ev::HTTP];
    b.u32(req);
    match r {
        Ok(resp) => {
            b.u16(resp.status);
            b.str(&resp.headers);
            b.bytes(&resp.body);
        }
        Err(e) => {
            b.u16(0);
            b.str("");
            b.bytes(e.as_bytes());
        }
    }
    b
}
