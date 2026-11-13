//! Network, messages between apps and opening links (docs/io.md).

pub mod http;
pub mod ws;

use crate::host::Services;
use crate::pkg::{app_id, data_home, AppCtx};
use qa_proto::{err, ev, Reader, Sink};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HTTP: usize = 8;
const INBOX_MAX_MSGS: usize = 100;
const INBOX_MAX_BYTES: u64 = 4 << 20;

pub fn sock_dir() -> PathBuf {
    // The temp folder differs between a terminal and a launched app, and sockets have short paths.
    #[cfg(target_os = "macos")]
    return PathBuf::from(format!("/tmp/quick-apps-{}", unsafe { libc::getuid() }));
    #[cfg(not(target_os = "macos"))]
    {
        let rt = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        rt.join("quick-apps")
    }
}

pub fn sock_path(id: &str) -> PathBuf {
    sock_dir().join(format!("{id}.sock"))
}

/// An app id from a manifest URL or an id.
pub fn target_id(to: &str) -> String {
    if to.len() == 16 && to.bytes().all(|b| b.is_ascii_hexdigit()) {
        to.to_ascii_lowercase()
    } else {
        app_id(to)
    }
}

/// Asks a running instance to raise its window. True if one answered.
pub fn activate_running(id: &str) -> bool {
    match UnixStream::connect(sock_path(id)) {
        Ok(mut s) => s.write_all(&[1]).is_ok(),
        Err(_) => false,
    }
}

fn message_event(from: &str, data: &[u8]) -> Vec<u8> {
    let mut b = vec![ev::MESSAGE];
    b.str(from);
    b.bytes(data);
    b
}

/// Takes every message queued in `ctx`'s inbox, oldest first.
pub fn drain_inbox(ctx: &AppCtx) -> Vec<Vec<u8>> {
    let dir = ctx.home.join("inbox");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        if let Ok(b) = std::fs::read(&f) {
            let mut r = Reader::new(&b);
            if let (Some(from), Some(data)) = (r.str(), r.bytes()) {
                out.push(message_event(from, data));
            }
        }
        let _ = std::fs::remove_file(&f);
    }
    out
}

fn inbox_put(id: &str, from: &str, data: &[u8]) -> i32 {
    let home = data_home().join("apps").join(id);
    if !home.exists() {
        return err::NOT_FOUND;
    }
    let dir = home.join("inbox");
    if std::fs::create_dir_all(&dir).is_err() {
        return err::IO;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut b = Vec::new();
    b.str(from);
    b.bytes(data);
    if std::fs::write(dir.join(format!("{now:024}.msg")), b).is_err() {
        return err::IO;
    }
    // Drop the oldest beyond the limits.
    let mut files: Vec<(PathBuf, u64)> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| (e.path(), e.metadata().map(|m| m.len()).unwrap_or(0))).collect())
        .unwrap_or_default();
    files.sort();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let mut count = files.len();
    for (p, len) in files {
        if count <= INBOX_MAX_MSGS && total <= INBOX_MAX_BYTES {
            break;
        }
        let _ = std::fs::remove_file(p);
        count -= 1;
        total -= len;
    }
    1
}

pub struct Net {
    me: String,
    efd: RawFd,
    done: Arc<Mutex<Vec<Vec<u8>>>>,
    running: Arc<AtomicUsize>,
    queue: VecDeque<(u32, http::Req)>,
    cancelled: Arc<Mutex<HashSet<u32>>>,
    ws: HashMap<u32, (Sender<ws::WsOut>, RawFd)>,
    listener: Option<UnixListener>,
    sock: Option<PathBuf>,
    /// URL prefix rewrites (`--map from=to`), applied to HTTP and WebSocket.
    pub map: Vec<(String, String)>,
    /// Print links instead of opening them (`qa shot`).
    pub headless: bool,
}

impl Net {
    /// `listen` binds the app's socket (single instance and incoming messages).
    pub fn new(ctx: &AppCtx, listen: bool) -> Net {
        let efd = crate::sys::new_wake();
        let mut n = Net {
            me: ctx.source.clone(),
            efd,
            done: Arc::default(),
            running: Arc::default(),
            queue: VecDeque::new(),
            cancelled: Arc::default(),
            ws: HashMap::new(),
            listener: None,
            sock: None,
            map: Vec::new(),
            headless: false,
        };
        if listen {
            let _ = std::fs::create_dir_all(sock_dir());
            let p = sock_path(&ctx.id);
            // A socket nobody answers on is stale.
            if UnixStream::connect(&p).is_err() {
                let _ = std::fs::remove_file(&p);
            }
            if let Ok(l) = UnixListener::bind(&p) {
                let _ = l.set_nonblocking(true);
                n.listener = Some(l);
                n.sock = Some(p);
            }
        }
        n
    }

    fn wake(efd: RawFd) {
        crate::sys::wake(efd);
    }

    fn start_http(&mut self, id: u32, req: http::Req) {
        self.running.fetch_add(1, Ordering::SeqCst);
        let (done, running, cancelled, efd) = (self.done.clone(), self.running.clone(), self.cancelled.clone(), self.efd);
        std::thread::spawn(move || {
            let r = http::fetch(&req);
            running.fetch_sub(1, Ordering::SeqCst);
            if !cancelled.lock().unwrap().remove(&id) {
                done.lock().unwrap().push(http::encode_event(id, r));
            }
            Net::wake(efd);
        });
    }

    fn accept(&mut self) {
        let Some(l) = &self.listener else { return };
        while let Ok((mut s, _)) = l.accept() {
            let (done, efd) = (self.done.clone(), self.efd);
            std::thread::spawn(move || {
                let _ = s.set_nonblocking(false);
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut b = Vec::new();
                if s.take((1 << 20) + 4096).read_to_end(&mut b).is_err() {
                    return;
                }
                let ev = match b.first() {
                    Some(1) => Some(vec![ev::ACTIVATE]),
                    Some(2) => {
                        let mut r = Reader::new(&b[1..]);
                        match (r.str(), r.bytes()) {
                            (Some(from), Some(data)) => Some(message_event(from, data)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(e) = ev {
                    done.lock().unwrap().push(e);
                    Net::wake(efd);
                }
            });
        }
    }
}

impl Net {
    fn remap(&self, url: &str) -> String {
        for (from, to) in &self.map {
            if let Some(rest) = url.strip_prefix(from.as_str()) {
                return format!("{to}{rest}");
            }
        }
        url.to_string()
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        if let Some(p) = &self.sock {
            let _ = std::fs::remove_file(p);
        }
        for (tx, fd) in self.ws.values() {
            let _ = tx.send(ws::WsOut::Close(1001));
            Net::wake(*fd);
        }
    }
}

impl Services for Net {
    fn http(&mut self, id: u32, req: Vec<u8>) {
        let Some(mut r) = http::Req::decode(&req) else {
            self.done.lock().unwrap().push(http::encode_event(id, Err("malformed request".into())));
            return Net::wake(self.efd);
        };
        r.url = self.remap(&r.url);
        if self.running.load(Ordering::SeqCst) >= MAX_HTTP {
            self.queue.push_back((id, r));
        } else {
            self.start_http(id, r);
        }
    }

    fn http_cancel(&mut self, id: u32) {
        self.queue.retain(|(i, _)| *i != id);
        self.cancelled.lock().unwrap().insert(id);
    }

    fn ws_open(&mut self, id: u32, req: Vec<u8>) {
        let mut r = Reader::new(&req);
        let url = self.remap(r.str().unwrap_or(""));
        let headers: Vec<(String, String)> =
            r.str().unwrap_or("").lines().filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        let (tx, rx) = mpsc::channel();
        let wake = crate::sys::new_wake();
        self.ws.insert(id, (tx, wake));
        let (done, efd) = (self.done.clone(), self.efd);
        std::thread::spawn(move || {
            let emit = |kind: u8, data: Vec<u8>| {
                let mut b = vec![ev::WS];
                b.u32(id);
                b.u8(kind);
                b.bytes(&data);
                done.lock().unwrap().push(b);
                Net::wake(efd);
            };
            ws::run(&url, &headers, rx, wake, &emit);
            unsafe { libc::close(wake) };
        });
    }

    fn ws_send(&mut self, id: u32, kind: u32, data: Vec<u8>) {
        if let Some((tx, fd)) = self.ws.get(&id) {
            let m = if kind == 2 { ws::WsOut::Binary(data) } else { ws::WsOut::Text(data) };
            let _ = tx.send(m);
            Net::wake(*fd);
        }
    }

    fn ws_close(&mut self, id: u32, code: u32) {
        if let Some((tx, fd)) = self.ws.remove(&id) {
            let _ = tx.send(ws::WsOut::Close(code as u16));
            Net::wake(fd);
        }
    }

    fn send_msg(&mut self, to: &str, data: &[u8]) -> i32 {
        let id = target_id(to);
        let mut b = vec![2u8];
        b.str(&self.me);
        b.bytes(data);
        if let Ok(mut s) = UnixStream::connect(sock_path(&id)) {
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
            return if s.write_all(&b).is_ok() { 0 } else { err::IO };
        }
        inbox_put(&id, &self.me, data)
    }

    fn open_url(&mut self, url: &str) {
        if self.headless {
            return eprintln!("qa: open_url {url}");
        }
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&url).stdin(std::process::Stdio::null()).status();
        });
    }

    fn fds(&self) -> Vec<RawFd> {
        let mut v = vec![self.efd];
        if let Some(l) = &self.listener {
            v.push(l.as_raw_fd());
        }
        v
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        crate::sys::drain(self.efd);
        self.accept();
        while self.running.load(Ordering::SeqCst) < MAX_HTTP {
            let Some((id, r)) = self.queue.pop_front() else { break };
            self.start_http(id, r);
        }
        let evs = std::mem::take(&mut *self.done.lock().unwrap());
        // Closed sockets are forgotten.
        for e in &evs {
            if e.first() == Some(&ev::WS) && e.len() > 5 && (e[5] == ws::CLOSED || e[5] == ws::ERROR) {
                let id = u32::from_le_bytes([e[1], e[2], e[3], e[4]]);
                self.ws.remove(&id);
            }
        }
        evs
    }
}
