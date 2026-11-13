//! URL and form encoding, header lookup, and a request queue.

use super::*;
use alloc_crate::collections::VecDeque;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes everything but `A-Z a-z 0-9 - _ . ~`.
pub fn url_encode_into(out: &mut String, v: &str) {
    for &b in v.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
}

pub fn url_encode(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    url_encode_into(&mut s, v);
    s
}

/// Decodes %XX escapes and '+' (as a space).
pub fn url_decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let hex = |c: u8| (c as char).to_digit(16);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `application/x-www-form-urlencoded`, also usable as a query string.
pub fn form(params: &[(&str, &str)]) -> String {
    let mut s = String::new();
    for (k, v) in params {
        if !s.is_empty() {
            s.push('&');
        }
        url_encode_into(&mut s, k);
        s.push('=');
        url_encode_into(&mut s, v);
    }
    s
}

/// A header's value from "Name: value\n" lines (case-insensitive name).
pub fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// A finished request from a `Client`.
pub struct Response<'a, T> {
    pub tag: T,
    /// 0 when the request failed (no connection, denied, timed out).
    pub status: u16,
    pub headers: &'a str,
    pub body: &'a [u8],
}

impl<T> Response<'_, T> {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        header(self.headers, name)
    }
    pub fn json(&self) -> Option<json::Doc> {
        json::parse(self.body)
    }
}

struct Pending<T> {
    method: &'static str,
    url: String,
    headers: String,
    body: Vec<u8>,
    tag: T,
    tries: u8,
}

impl<T> Pending<T> {
    fn same(&self, method: &str, url: &str, body: &[u8]) -> bool {
        self.method == method && self.url == url && self.body == body
    }
}

/// Queues HTTP requests, keeps at most `max` in flight, drops duplicates of
/// a request that is still pending, retries 429 and 503 responses after
/// their Retry-After, and hands each response back with the tag it was sent
/// with. Pass every event to `handle`.
pub struct Client<T> {
    /// Sent with every request, e.g. "Authorization: Bearer …\n".
    pub headers: String,
    /// Requests in flight at once (the runtime allows 8 per app).
    pub max: usize,
    /// Attempts for a rate-limited request before its 429 is returned.
    pub tries: u8,
    queue: VecDeque<Pending<T>>,
    flying: Vec<(u32, Pending<T>)>,
    waiting: Vec<(u32, Pending<T>)>,
}

impl<T> Default for Client<T> {
    fn default() -> Self {
        Client::new()
    }
}

impl<T> Client<T> {
    pub fn new() -> Self {
        Client { headers: String::new(), max: 6, tries: 4, queue: VecDeque::new(), flying: Vec::new(), waiting: Vec::new() }
    }

    /// Queues a request. Returns false if the same method, URL and body
    /// is already pending.
    pub fn send(&mut self, method: &'static str, url: &str, headers: &str, body: &[u8], tag: T) -> bool {
        let dup = self.queue.iter().chain(self.flying.iter().map(|f| &f.1)).chain(self.waiting.iter().map(|w| &w.1)).any(|p| p.same(method, url, body));
        if dup {
            return false;
        }
        self.queue.push_back(Pending { method, url: String::from(url), headers: String::from(headers), body: body.to_vec(), tag, tries: 0 });
        self.pump();
        true
    }
    pub fn get(&mut self, url: &str, tag: T) -> bool {
        self.send("GET", url, "", &[], tag)
    }
    /// POSTs `params` as a form.
    pub fn post_form(&mut self, url: &str, params: &[(&str, &str)], tag: T) -> bool {
        self.send("POST", url, "Content-Type: application/x-www-form-urlencoded\n", form(params).as_bytes(), tag)
    }
    /// POSTs a JSON body.
    pub fn post_json(&mut self, url: &str, body: &str, tag: T) -> bool {
        self.send("POST", url, "Content-Type: application/json; charset=utf-8\n", body.as_bytes(), tag)
    }

    /// Requests queued, in flight or waiting to retry.
    pub fn pending(&self) -> usize {
        self.queue.len() + self.flying.len() + self.waiting.len()
    }
    /// Whether any pending request's tag matches.
    pub fn has(&self, f: impl Fn(&T) -> bool) -> bool {
        self.queue.iter().map(|p| &p.tag).chain(self.flying.iter().map(|f| &f.1.tag)).chain(self.waiting.iter().map(|w| &w.1.tag)).any(f)
    }
    /// Drops pending requests whose tag matches (cancelling those in flight).
    pub fn cancel(&mut self, f: impl Fn(&T) -> bool) {
        self.queue.retain(|p| !f(&p.tag));
        self.flying.retain(|(req, p)| {
            let hit = f(&p.tag);
            if hit {
                net::cancel(*req);
            }
            !hit
        });
        self.waiting.retain(|(t, p)| {
            let hit = f(&p.tag);
            if hit {
                timer_cancel(*t);
            }
            !hit
        });
        self.pump();
    }

    fn pump(&mut self) {
        while self.flying.len() < self.max.max(1) {
            let Some(p) = self.queue.pop_front() else { break };
            let mut h = self.headers.clone();
            if !h.is_empty() && !h.ends_with('\n') {
                h.push('\n');
            }
            h.push_str(&p.headers);
            let req = request(&Request { method: p.method, url: &p.url, headers: &h, body: &p.body, timeout_ms: 0, max_body: 0 });
            self.flying.push((req, p));
        }
    }

    /// Feeds an event. Returns the response when it finishes one of this
    /// client's requests; retries and the client's own timers return None.
    pub fn handle<'a>(&mut self, ev: &Event<'a>) -> Option<Response<'a, T>> {
        match *ev {
            Event::Http { req, status, headers, body } => {
                let i = self.flying.iter().position(|f| f.0 == req)?;
                let (_, mut p) = self.flying.remove(i);
                if matches!(status, 429 | 503) && p.tries + 1 < self.tries {
                    p.tries += 1;
                    let secs = header(headers, "retry-after").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1 << p.tries);
                    let t = timer(secs.clamp(1, 120) * 1000);
                    self.waiting.push((t, p));
                    self.pump();
                    return None;
                }
                self.pump();
                Some(Response { tag: p.tag, status, headers, body })
            }
            Event::Timer(t) => {
                let i = self.waiting.iter().position(|w| w.0 == t)?;
                let (_, p) = self.waiting.remove(i);
                self.queue.push_front(p);
                self.pump();
                None
            }
            _ => None,
        }
    }
}
