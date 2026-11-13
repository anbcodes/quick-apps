//! The app thread: one wasmi instance, its imports, timers and limits.

use super::encode;
use super::fs::{Fs, Handle};
use qa_proto::err;
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wasmi::{Caller, Config, Engine, Error, Linker, Memory, Module, Store, TypedFunc, TypedResumableCall};
use wasmi_core::{LimiterError, ResourceLimiter};

/// Instructions per fuel slice. Between slices the CPU limit is checked.
pub const SLICE: u64 = 1_000_000;
const CPU_WINDOW: Duration = Duration::from_secs(10);
/// CPU time allowed per window: 50% of one core.
const CPU_BUDGET: Duration = Duration::from_secs(5);
const THROTTLE_PROMPT_AFTER: Duration = Duration::from_secs(10);

pub enum ToApp {
    Event(Vec<u8>),
    /// A request finished. A handle, if any, is added before the event is sent.
    Done { req: u32, result: i32, info: String, handle: Option<Handle> },
    SetMemLimit(usize),
    SetCpuUnlimited,
    Stop,
}

/// Why the app thread blocks for an answer.
#[derive(Clone, Debug)]
pub enum Ask {
    /// Linear memory wants to grow past the budget.
    Memory { limit: usize, desired: usize },
    /// The app has been throttled for a while.
    Cpu,
}

pub enum FromApp {
    /// The result of one call. Also an acknowledgement that the call ended.
    Out(Out),
    /// Blocks the app thread until the user answers with a button index.
    Ask(Ask, Sender<usize>),
    /// An async request (`request` import).
    Request { req: u32, kind: u32, arg: Vec<u8> },
    /// A message to another app; blocks for the result code.
    Msg { to: String, data: Vec<u8>, reply: Sender<i32> },
}

#[derive(Default)]
pub struct Out {
    pub patch: Vec<u8>,
    pub title: Option<String>,
    pub quit: bool,
    pub error: Option<String>,
    pub clipboard: Option<String>,
    pub canvas: Vec<(u32, u32, u32, u32, u32, Vec<u8>)>,
    pub open_url: Option<String>,
    pub http: Vec<(u32, Vec<u8>)>,
    pub http_cancel: Vec<u32>,
    pub ws_open: Vec<(u32, Vec<u8>)>,
    pub ws_send: Vec<(u32, u32, Vec<u8>)>,
    pub ws_close: Vec<(u32, u32)>,
}

impl Out {
    pub fn is_empty(&self) -> bool {
        self.patch.is_empty()
            && self.title.is_none()
            && !self.quit
            && self.error.is_none()
            && self.clipboard.is_none()
            && self.canvas.is_empty()
            && self.open_url.is_none()
            && self.http.is_empty()
            && self.http_cancel.is_empty()
            && self.ws_open.is_empty()
            && self.ws_send.is_empty()
            && self.ws_close.is_empty()
    }
}

/// State the main thread and the app thread both see.
pub struct Shared {
    /// Logical window w, h and scale.
    pub win: Mutex<[f32; 3]>,
    /// Written whenever the app thread sends something.
    pub efd: RawFd,
    pub name: String,
    /// Set by the main thread to stop a running call at the next fuel slice.
    pub stop: AtomicBool,
    /// The app's secrets file.
    pub secrets: std::path::PathBuf,
}

impl Shared {
    pub fn wake(&self) {
        let one = 1u64;
        unsafe { libc::write(self.efd, &one as *const u64 as *const _, 8) };
    }
}

struct Timer {
    id: u32,
    due: Instant,
    every: Option<Duration>,
}

/// Sliding-window CPU accounting for the app thread.
pub struct CpuMeter {
    samples: VecDeque<(Instant, Duration)>,
    pub throttled: Duration,
    pub unlimited: bool,
    asked: bool,
}

fn thread_cpu() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

impl CpuMeter {
    fn new(unlimited: bool) -> CpuMeter {
        let mut samples = VecDeque::new();
        samples.push_back((Instant::now(), thread_cpu()));
        CpuMeter { samples, throttled: Duration::ZERO, unlimited, asked: false }
    }

    /// CPU used in the last window. Time before the thread started counts as idle.
    fn used(&mut self) -> Duration {
        let now = Instant::now();
        let cpu = thread_cpu();
        self.samples.push_back((now, cpu));
        let start = now.checked_sub(CPU_WINDOW).unwrap_or(now);
        while self.samples.len() > 2 && self.samples[1].0 <= start {
            self.samples.pop_front();
        }
        let (t0, c0) = self.samples[0];
        let base = if t0 < start && self.samples.len() > 1 {
            let (t1, c1) = self.samples[1];
            let span = (t1 - t0).as_secs_f64().max(1e-9);
            let f = ((start - t0).as_secs_f64() / span).clamp(0.0, 1.0);
            c0 + (c1.saturating_sub(c0)).mul_f64(f)
        } else {
            c0
        };
        // Keep the deque short under heavy load.
        if self.samples.len() > 4096 {
            let keep = self.samples.split_off(self.samples.len() - 2048);
            let first = self.samples[0];
            self.samples = keep;
            self.samples.push_front(first);
        }
        cpu.saturating_sub(base)
    }
}

pub struct Host {
    out: Out,
    timers: Vec<Timer>,
    next_timer: u32,
    next_req: u32,
    shared: Arc<Shared>,
    tx: Sender<FromApp>,
    pub mem_limit: usize,
    pub cpu: CpuMeter,
    pub fs: Fs,
    /// Set while handling a user input event (open_url, clipboard).
    pub user_event: bool,
}

impl Host {
    fn ask(&self, a: Ask) -> usize {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(FromApp::Ask(a, tx)).is_err() {
            return usize::MAX;
        }
        self.shared.wake();
        rx.recv().unwrap_or(usize::MAX)
    }
}

impl ResourceLimiter for Host {
    fn memory_growing(&mut self, _current: usize, desired: usize, _max: Option<usize>) -> Result<bool, LimiterError> {
        while desired > self.mem_limit {
            // Blocks this thread until the user answers. 0 = Allow.
            if self.ask(Ask::Memory { limit: self.mem_limit, desired }) != 0 {
                return Ok(false);
            }
            self.mem_limit = (self.mem_limit * 2).max(desired);
        }
        Ok(true)
    }
    fn table_growing(&mut self, _current: usize, desired: usize, _max: Option<usize>) -> Result<bool, LimiterError> {
        Ok(desired <= 100_000)
    }
    fn instances(&self) -> usize {
        1
    }
    fn tables(&self) -> usize {
        4
    }
    fn memories(&self) -> usize {
        1
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn mono_ms() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000
}

fn mem(c: &Caller<'_, Host>) -> Result<Memory, Error> {
    c.get_export("memory").and_then(|e| e.into_memory()).ok_or_else(|| Error::new("the app exports no memory"))
}

fn read(c: &Caller<'_, Host>, ptr: u32, len: u32) -> Result<Vec<u8>, Error> {
    let m = mem(c)?;
    let d = m.data(c);
    let (a, b) = (ptr as usize, ptr as usize + len as usize);
    d.get(a..b).map(|s| s.to_vec()).ok_or_else(|| Error::new("pointer out of bounds"))
}

fn read_str(c: &Caller<'_, Host>, ptr: u32, len: u32) -> Result<String, Error> {
    String::from_utf8(read(c, ptr, len)?).map_err(|_| Error::new("string is not UTF-8"))
}

fn write(c: &mut Caller<'_, Host>, ptr: u32, bytes: &[u8]) -> Result<(), Error> {
    let m = mem(c)?;
    let d = m.data_mut(c);
    let a = ptr as usize;
    d.get_mut(a..a + bytes.len()).ok_or_else(|| Error::new("pointer out of bounds"))?.copy_from_slice(bytes);
    Ok(())
}

fn define(l: &mut Linker<Host>) -> Result<(), Error> {
    const M: &str = "qa";
    l.func_wrap(M, "ui_patch", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<i32, Error> {
        let b = read(&c, ptr, len)?;
        c.data_mut().out.patch.extend_from_slice(&b);
        Ok(0)
    })?;
    l.func_wrap(M, "set_title", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<(), Error> {
        let s = read_str(&c, ptr, len)?;
        c.data_mut().out.title = Some(s);
        Ok(())
    })?;
    l.func_wrap(M, "quit", |mut c: Caller<'_, Host>| {
        c.data_mut().out.quit = true;
    })?;
    l.func_wrap(M, "env", |mut c: Caller<'_, Host>, out: u32, cap: u32| -> Result<i32, Error> {
        if cap < 21 {
            return Ok(err::TOO_SMALL);
        }
        let [w, h, s] = *c.data().shared.win.lock().unwrap();
        let mut b = Vec::with_capacity(21);
        for v in [w, h, w, h, s] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(0);
        // The local UTC offset in seconds, for callers with room for it.
        if cap >= 25 {
            b.extend_from_slice(&utc_offset().to_le_bytes());
        }
        write(&mut c, out, &b)?;
        Ok(b.len() as i32)
    })?;
    l.func_wrap(M, "json_parse", |mut c: Caller<'_, Host>, ptr: u32, len: u32, out: u32, cap: u32, needed: u32| -> Result<i32, Error> {
        let src = read(&c, ptr, len)?;
        let Some(b) = qa_proto::val::from_json(&src) else { return Ok(err::INVALID) };
        if b.len() > cap as usize {
            write(&mut c, needed, &(b.len() as u32).to_le_bytes())?;
            return Ok(err::TOO_SMALL);
        }
        write(&mut c, out, &b)?;
        Ok(b.len() as i32)
    })?;
    l.func_wrap(M, "secret_get", |mut c: Caller<'_, Host>, kp: u32, kl: u32, out: u32, cap: u32, needed: u32| -> Result<i32, Error> {
        let Ok(key) = read_str(&c, kp, kl) else { return Ok(err::INVALID) };
        let v = match super::secrets::get(&c.data().shared.secrets, &key) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        if v.len() > cap as usize {
            write(&mut c, needed, &(v.len() as u32).to_le_bytes())?;
            return Ok(err::TOO_SMALL);
        }
        write(&mut c, out, v.as_bytes())?;
        Ok(v.len() as i32)
    })?;
    l.func_wrap(M, "secret_set", |c: Caller<'_, Host>, kp: u32, kl: u32, vp: u32, vl: u32| -> Result<i32, Error> {
        let (Ok(key), Ok(val)) = (read_str(&c, kp, kl), read_str(&c, vp, vl)) else { return Ok(err::INVALID) };
        Ok(super::secrets::set(&c.data().shared.secrets, &key, &val))
    })?;
    l.func_wrap(M, "now_ms", || -> i64 { now_ms() })?;
    l.func_wrap(M, "mono_ms", || -> i64 { mono_ms() })?;
    l.func_wrap(M, "random", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<(), Error> {
        let mut b = vec![0u8; len as usize];
        let mut off = 0;
        while off < b.len() {
            let n = unsafe { libc::getrandom(b[off..].as_mut_ptr() as *mut _, b.len() - off, 0) };
            if n <= 0 {
                return Err(Error::new("getrandom failed"));
            }
            off += n as usize;
        }
        write(&mut c, ptr, &b)
    })?;
    l.func_wrap(M, "log", |c: Caller<'_, Host>, level: u32, ptr: u32, len: u32| -> Result<(), Error> {
        let s = read_str(&c, ptr, len)?;
        let lv = ["debug", "info", "warn", "error"].get(level as usize).copied().unwrap_or("info");
        eprintln!("[{}] {lv}: {s}", c.data().shared.name);
        Ok(())
    })?;
    l.func_wrap(M, "timer", |mut c: Caller<'_, Host>, ms: u32, repeat: u32| -> u32 {
        let h = c.data_mut();
        h.next_timer += 1;
        let id = h.next_timer;
        let d = Duration::from_millis(ms.max(4) as u64);
        h.timers.push(Timer { id, due: Instant::now() + d, every: (repeat != 0).then_some(d) });
        id
    })?;
    l.func_wrap(M, "timer_cancel", |mut c: Caller<'_, Host>, id: u32| {
        c.data_mut().timers.retain(|t| t.id != id);
    })?;
    l.func_wrap(M, "clipboard_write", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<i32, Error> {
        let s = read_str(&c, ptr, len)?;
        c.data_mut().out.clipboard = Some(s);
        Ok(0)
    })?;
    l.func_wrap(M, "open_url", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<i32, Error> {
        let s = read_str(&c, ptr, len)?;
        if !c.data().user_event {
            return Ok(err::DENIED);
        }
        if !(s.starts_with("https://") || s.starts_with("http://")) {
            return Ok(err::INVALID);
        }
        c.data_mut().out.open_url = Some(s);
        Ok(0)
    })?;
    l.func_wrap(M, "canvas_put", |mut c: Caller<'_, Host>, node: u32, x: u32, y: u32, w: u32, h: u32, ptr: u32| -> Result<i32, Error> {
        let len = w.checked_mul(h).and_then(|n| n.checked_mul(4)).ok_or_else(|| Error::new("canvas_put too large"))?;
        let b = read(&c, ptr, len)?;
        c.data_mut().out.canvas.push((node, x, y, w, h, b));
        Ok(0)
    })?;
    l.func_wrap(M, "request", |mut c: Caller<'_, Host>, kind: u32, ptr: u32, len: u32| -> Result<u32, Error> {
        let arg = read(&c, ptr, len)?;
        let h = c.data_mut();
        h.next_req += 1;
        let req = h.next_req;
        let _ = h.tx.send(FromApp::Request { req, kind, arg });
        h.shared.wake();
        Ok(req)
    })?;
    // Files.
    l.func_wrap(M, "fs_open", |c: Caller<'_, Host>, dir: i32, p: u32, pl: u32, flags: u32| -> Result<i32, Error> {
        let Ok(path) = read_str(&c, p, pl) else { return Ok(err::INVALID) };
        let mut c = c;
        Ok(c.data_mut().fs.open(dir, &path, flags))
    })?;
    l.func_wrap(M, "fs_read", |mut c: Caller<'_, Host>, fd: i32, ptr: u32, len: u32| -> Result<i32, Error> {
        let mut buf = vec![0u8; (len as usize).min(16 << 20)];
        let n = c.data().fs.read(fd, &mut buf);
        if n > 0 {
            write(&mut c, ptr, &buf[..n as usize])?;
        }
        Ok(n)
    })?;
    l.func_wrap(M, "fs_write", |c: Caller<'_, Host>, fd: i32, ptr: u32, len: u32| -> Result<i32, Error> {
        let b = read(&c, ptr, len)?;
        Ok(c.data().fs.write(fd, &b))
    })?;
    l.func_wrap(M, "fs_seek", |c: Caller<'_, Host>, fd: i32, off: i64, whence: u32| -> i64 { c.data().fs.seek(fd, off, whence) })?;
    l.func_wrap(M, "fs_close", |mut c: Caller<'_, Host>, fd: i32| -> i32 { c.data_mut().fs.close(fd) })?;
    l.func_wrap(M, "fs_stat", |mut c: Caller<'_, Host>, dir: i32, p: u32, pl: u32, out: u32| -> Result<i32, Error> {
        let Ok(path) = read_str(&c, p, pl) else { return Ok(err::INVALID) };
        match c.data().fs.stat(dir, &path) {
            Ok(b) => {
                write(&mut c, out, &b)?;
                Ok(0)
            }
            Err(e) => Ok(e),
        }
    })?;
    l.func_wrap(M, "fs_list", |mut c: Caller<'_, Host>, dir: i32, p: u32, pl: u32, out: u32, cap: u32, needed: u32| -> Result<i32, Error> {
        let Ok(path) = read_str(&c, p, pl) else { return Ok(err::INVALID) };
        match c.data().fs.list(dir, &path) {
            Ok(b) if b.len() > cap as usize => {
                write(&mut c, needed, &(b.len() as u32).to_le_bytes())?;
                Ok(err::TOO_SMALL)
            }
            Ok(b) => {
                write(&mut c, out, &b)?;
                Ok(b.len() as i32)
            }
            Err(e) => Ok(e),
        }
    })?;
    l.func_wrap(M, "fs_mkdir", |c: Caller<'_, Host>, dir: i32, p: u32, pl: u32| -> Result<i32, Error> {
        let Ok(path) = read_str(&c, p, pl) else { return Ok(err::INVALID) };
        Ok(c.data().fs.mkdir(dir, &path))
    })?;
    l.func_wrap(M, "fs_remove", |c: Caller<'_, Host>, dir: i32, p: u32, pl: u32| -> Result<i32, Error> {
        let Ok(path) = read_str(&c, p, pl) else { return Ok(err::INVALID) };
        Ok(c.data().fs.remove(dir, &path))
    })?;
    l.func_wrap(M, "fs_rename", |c: Caller<'_, Host>, d1: i32, p1: u32, l1: u32, d2: i32, p2: u32, l2: u32| -> Result<i32, Error> {
        let (Ok(a), Ok(b)) = (read_str(&c, p1, l1), read_str(&c, p2, l2)) else { return Ok(err::INVALID) };
        Ok(c.data().fs.rename(d1, &a, d2, &b))
    })?;
    // Network and messages: queued for the main thread, answered as events.
    l.func_wrap(M, "http", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<u32, Error> {
        let b = read(&c, ptr, len)?;
        let h = c.data_mut();
        h.next_req += 1;
        let id = h.next_req;
        h.out.http.push((id, b));
        Ok(id)
    })?;
    l.func_wrap(M, "http_cancel", |mut c: Caller<'_, Host>, req: u32| {
        c.data_mut().out.http_cancel.push(req);
    })?;
    l.func_wrap(M, "ws_open", |mut c: Caller<'_, Host>, ptr: u32, len: u32| -> Result<u32, Error> {
        let b = read(&c, ptr, len)?;
        let h = c.data_mut();
        h.next_req += 1;
        let id = h.next_req;
        h.out.ws_open.push((id, b));
        Ok(id)
    })?;
    l.func_wrap(M, "ws_send", |mut c: Caller<'_, Host>, ws: u32, kind: u32, ptr: u32, len: u32| -> Result<i32, Error> {
        let b = read(&c, ptr, len)?;
        c.data_mut().out.ws_send.push((ws, kind, b));
        Ok(0)
    })?;
    l.func_wrap(M, "ws_close", |mut c: Caller<'_, Host>, ws: u32, code: u32| {
        c.data_mut().out.ws_close.push((ws, code));
    })?;
    l.func_wrap(M, "msg_send", |mut c: Caller<'_, Host>, tp: u32, tl: u32, dp: u32, dl: u32| -> Result<i32, Error> {
        let Ok(to) = read_str(&c, tp, tl) else { return Ok(err::INVALID) };
        if dl > (1 << 20) {
            return Ok(err::LIMIT);
        }
        let data = read(&c, dp, dl)?;
        // Sending may need a prompt and a socket write: wait for the main thread's answer.
        let (tx, rx) = mpsc::channel();
        let h = c.data_mut();
        let _ = h.tx.send(FromApp::Msg { to, data, reply: tx });
        h.shared.wake();
        Ok(rx.recv().unwrap_or(err::IO))
    })?;
    Ok(())
}

pub struct Instance {
    store: Store<Host>,
    alloc: TypedFunc<u32, u32>,
    event: TypedFunc<(u32, u32), ()>,
    memory: Memory,
}

/// Checks the CPU budget between fuel slices, sleeping while over it.
fn throttle(store: &mut Store<Host>) -> Result<(), Error> {
    loop {
        let h = store.data_mut();
        if h.shared.stop.load(Ordering::Relaxed) {
            return Err(Error::new("stopped by the user"));
        }
        if h.cpu.unlimited {
            return Ok(());
        }
        if h.cpu.used() <= CPU_BUDGET {
            return Ok(());
        }
        let nap = Duration::from_millis(50);
        std::thread::sleep(nap);
        let h = store.data_mut();
        h.cpu.throttled += nap;
        if h.cpu.throttled >= THROTTLE_PROMPT_AFTER && !h.cpu.asked {
            h.cpu.asked = true;
            // 0 Allow (no limit), 1 Keep limited, 2 Stop.
            match h.ask(Ask::Cpu) {
                0 => h.cpu.unlimited = true,
                1 => {}
                _ => return Err(Error::new("stopped by the user")),
            }
        }
    }
}

/// Runs `f` in fuel slices so a long call can be throttled or stopped between them.
fn call<P, R>(store: &mut Store<Host>, f: &TypedFunc<P, R>, p: P) -> Result<R, Error>
where
    P: wasmi::WasmParams,
    R: wasmi::WasmResults,
{
    store.set_fuel(SLICE)?;
    let mut c = f.call_resumable(&mut *store, p)?;
    loop {
        match c {
            TypedResumableCall::Finished(r) => return Ok(r),
            TypedResumableCall::OutOfFuel(inv) => {
                throttle(store)?;
                store.set_fuel(SLICE)?;
                c = inv.resume(&mut *store)?;
            }
            TypedResumableCall::HostTrap(_) => return Err(Error::new("unexpected host trap")),
        }
    }
}

pub struct Setup {
    pub wasm: Vec<u8>,
    pub mem_limit: usize,
    pub cpu_unlimited: bool,
    pub fs: Fs,
}

fn instantiate(s: Setup, shared: Arc<Shared>, tx: Sender<FromApp>) -> Result<(Instance, TypedFunc<(), ()>), String> {
    let mut cfg = Config::default();
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg);
    let module = Module::new(&engine, &s.wasm).map_err(|e| format!("invalid app.wasm: {e}"))?;
    let host = Host {
        out: Out::default(),
        timers: Vec::new(),
        next_timer: 0,
        next_req: 0,
        shared,
        tx,
        mem_limit: s.mem_limit,
        cpu: CpuMeter::new(s.cpu_unlimited),
        fs: s.fs,
        user_event: false,
    };
    let mut store = Store::new(&engine, host);
    store.limiter(|h| h as &mut dyn ResourceLimiter);
    store.set_fuel(SLICE * 100).map_err(|e| e.to_string())?;
    let mut linker = Linker::new(&engine);
    define(&mut linker).map_err(|e| e.to_string())?;
    let inst = linker.instantiate_and_start(&mut store, &module).map_err(|e| format!("cannot start the app: {e}"))?;
    let memory = inst.get_memory(&store, "memory").ok_or("the app exports no `memory`")?;
    let alloc = inst.get_typed_func::<u32, u32>(&store, "qa_alloc").map_err(|e| format!("qa_alloc: {e}"))?;
    let event = inst.get_typed_func::<(u32, u32), ()>(&store, "qa_event").map_err(|e| format!("qa_event: {e}"))?;
    let start = inst.get_typed_func::<(), ()>(&store, "qa_start").map_err(|e| format!("qa_start: {e}"))?;
    Ok((Instance { store, alloc, event, memory }, start))
}

impl Instance {
    fn deliver(&mut self, ev: &[u8]) -> Result<(), Error> {
        use qa_proto::ev as e;
        self.store.data_mut().user_event = matches!(ev.first().copied(), Some(e::CLICK | e::KEY | e::SUBMIT | e::POINTER | e::LINK));
        let ptr = call(&mut self.store, &self.alloc, ev.len() as u32)?;
        let d = self.memory.data_mut(&mut self.store);
        let a = ptr as usize;
        d.get_mut(a..a + ev.len()).ok_or_else(|| Error::new("qa_alloc returned a bad pointer"))?.copy_from_slice(ev);
        let r = call(&mut self.store, &self.event, (ptr, ev.len() as u32));
        self.store.data_mut().user_event = false;
        r
    }

    fn take_out(&mut self) -> Out {
        std::mem::take(&mut self.store.data_mut().out)
    }
}

/// The app thread's body. Wakes the main loop when it ends.
pub fn run(setup: Setup, rx: Receiver<ToApp>, tx: Sender<FromApp>, shared: Arc<Shared>) {
    run_inner(setup, rx, tx, shared.clone());
    shared.wake();
}

fn run_inner(setup: Setup, rx: Receiver<ToApp>, tx: Sender<FromApp>, shared: Arc<Shared>) {
    let send = |o: Out| {
        let _ = tx.send(FromApp::Out(o));
        shared.wake();
    };
    let (mut inst, start) = match instantiate(setup, shared.clone(), tx.clone()) {
        Ok(v) => v,
        Err(e) => return send(Out { error: Some(e), ..Default::default() }),
    };
    if let Err(e) = call(&mut inst.store, &start, ()) {
        let mut o = inst.take_out();
        o.error = Some(format!("{e}"));
        return send(o);
    }
    send(inst.take_out());
    loop {
        let next = inst.store.data().timers.iter().map(|t| t.due).min();
        let msg = match next {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let result = match msg {
            Ok(ToApp::Event(ev)) => inst.deliver(&ev),
            Ok(ToApp::Done { req, result, info, handle }) => {
                let result = match handle {
                    Some(h) => inst.store.data_mut().fs.add_handle(h),
                    None => result,
                };
                inst.deliver(&encode::request_done(req, result, &info))
            }
            Ok(ToApp::SetMemLimit(n)) => {
                inst.store.data_mut().mem_limit = n;
                continue;
            }
            Ok(ToApp::SetCpuUnlimited) => {
                inst.store.data_mut().cpu.unlimited = true;
                continue;
            }
            Ok(ToApp::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let h = inst.store.data_mut();
                let mut due = Vec::new();
                for t in h.timers.iter_mut() {
                    if t.due <= now {
                        due.push(t.id);
                        if let Some(e) = t.every {
                            t.due = (t.due + e).max(now);
                        }
                    }
                }
                h.timers.retain(|t| t.every.is_some() || t.due > now);
                let mut r = Ok(());
                for id in due {
                    r = inst.deliver(&encode::timer(id));
                    if r.is_err() {
                        break;
                    }
                }
                r
            }
        };
        let mut o = inst.take_out();
        if let Err(e) = result {
            o.error = Some(format!("{e}"));
            return send(o);
        }
        // Always answer, so the main thread knows the call ended.
        send(o);
    }
}

/// The local zone's current offset from UTC, in seconds.
fn utc_offset() -> i32 {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i32
    }
}
