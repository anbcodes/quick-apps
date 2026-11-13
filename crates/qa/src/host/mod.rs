//! The wasm host: runs an app on its own thread and connects it to the UI,
//! the user's prompts and the grants file.

pub mod app;
pub mod encode;
pub mod fs;
pub mod grants;
mod secrets;

use crate::gfx::{Cmd, Pixmap, PixmapSource};
use crate::pkg::AppCtx;
use crate::prompt::{Done, Overlay, PKind, PickMode, Prompt};
use crate::shell::Driver;
use crate::ui::{Ui, UiEvent};
use app::{Ask, FromApp, Out, Setup, Shared, ToApp};
use grants::{Grant, Grants};
use qa_proto::{err, ev, Reader};
use std::collections::HashMap;
use std::os::fd::{OwnedFd, RawFd};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_MEM_LIMIT: usize = 50 << 20;
const NOT_RESPONDING: Duration = Duration::from_secs(5);

pub mod req {
    pub const OPEN_FILE: u32 = 1;
    pub const SAVE_FILE: u32 = 2;
    pub const PICK_FOLDER: u32 = 3;
    pub const FOLDER: u32 = 4;
    pub const INTERNET: u32 = 5;
    pub const MEMORY: u32 = 6;
    pub const CPU: u32 = 7;
    pub const CLIPBOARD_READ: u32 = 8;
    pub const MESSAGE: u32 = 9;
}

#[derive(Default)]
pub struct Pixmaps {
    pub images: HashMap<u32, Pixmap>,
    pub canvases: HashMap<u32, Pixmap>,
}

impl PixmapSource for Pixmaps {
    fn image(&self, id: u32) -> Option<&Pixmap> {
        self.images.get(&id)
    }
    fn canvas(&self, id: u32) -> Option<&Pixmap> {
        self.canvases.get(&id)
    }
}

/// Network and message services the driver hands work to (milestone M4).
pub trait Services {
    fn http(&mut self, _id: u32, _req: Vec<u8>) {}
    fn http_cancel(&mut self, _id: u32) {}
    fn ws_open(&mut self, _id: u32, _req: Vec<u8>) {}
    fn ws_send(&mut self, _id: u32, _kind: u32, _data: Vec<u8>) {}
    fn ws_close(&mut self, _id: u32, _code: u32) {}
    /// Delivers a message; returns 0 sent, 1 queued, or an error code.
    fn send_msg(&mut self, _to: &str, _data: &[u8]) -> i32 {
        err::IO
    }
    fn open_url(&mut self, _url: &str) {}
    fn fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
    /// Finished network events for the app.
    fn poll(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

struct NoServices;
impl Services for NoServices {}

pub struct WasmDriver {
    wasm: Vec<u8>,
    pub name: String,
    pub ctx: AppCtx,
    pub grants: Grants,
    tx: Option<Sender<ToApp>>,
    rx: Option<Receiver<FromApp>>,
    shared: Arc<Shared>,
    efd: RawFd,
    title: Option<String>,
    quit: bool,
    /// Why the app stopped.
    pub error: Option<String>,
    closing: Option<Instant>,
    pub pix: Pixmaps,
    pub mem_limit: usize,
    cpu_unlimited: bool,
    internet_once: bool,
    /// Calls sent to the app and not yet answered, and since when.
    in_flight: u32,
    busy_since: Option<Instant>,
    /// Print patch errors with offsets (qa dev).
    pub dev: bool,
    pub overlay: Option<Overlay>,
    pub services: Box<dyn Services>,
    /// Events that must wait until the app is running (inbox messages).
    pub after_start: Vec<Vec<u8>>,
    /// Network calls waiting for the internet prompt: (is_ws, id, request).
    net_wait: Vec<(bool, u32, Vec<u8>)>,
    update_rx: Option<Receiver<UpdateMsg>>,
    update_m: Option<crate::pkg::store::Manifest>,
    /// The update is downloaded; waiting for the app to handle Close.
    pending_update: Option<Instant>,
}

pub enum UpdateMsg {
    Available(crate::pkg::store::Manifest),
    Ready,
    Failed(String),
}

fn mb(n: usize) -> String {
    format!("{} MB", (n + (1 << 19)) >> 20)
}

impl WasmDriver {
    pub fn new(wasm: Vec<u8>, ctx: AppCtx) -> WasmDriver {
        let efd = crate::sys::new_wake();
        let shared = Arc::new(Shared { win: Mutex::new([0.0; 3]), efd, name: ctx.name.clone(), stop: Default::default(), secrets: ctx.home.join("secrets") });
        let grants = Grants::load(ctx.grants_path());
        let mem_limit = grants.memory().map(|m| m as usize).unwrap_or(DEFAULT_MEM_LIMIT).max(DEFAULT_MEM_LIMIT);
        let cpu_unlimited = grants.cpu();
        WasmDriver {
            wasm,
            name: ctx.name.clone(),
            ctx: ctx.clone(),
            grants,
            tx: None,
            rx: None,
            shared,
            efd,
            title: Some(ctx.name.clone()),
            quit: false,
            error: None,
            closing: None,
            pix: Pixmaps::default(),
            mem_limit,
            cpu_unlimited,
            internet_once: false,
            in_flight: 0,
            busy_since: None,
            dev: false,
            overlay: None,
            services: Box::new(NoServices),
            after_start: Vec::new(),
            net_wait: Vec::new(),
            update_rx: None,
            update_m: None,
            pending_update: None,
        }
    }

    /// Checks the manifest URL for a newer version in the background.
    pub fn check_for_update(&mut self) {
        let (tx, rx) = mpsc::channel();
        let (ctx, efd) = (self.ctx.clone(), self.efd);
        std::thread::spawn(move || {
            // Offline or a fetch error is silent.
            if let Ok(Some(m)) = crate::pkg::store::check_update(&ctx, Duration::from_secs(5)) {
                let _ = tx.send(UpdateMsg::Available(m));
                crate::sys::wake(efd);
            }
        });
        self.update_rx = Some(rx);
    }

    fn poll_update(&mut self, ui: &mut Ui) {
        let msgs: Vec<UpdateMsg> = match &self.update_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for m in msgs {
            match m {
                UpdateMsg::Available(m) => {
                    let text = format!("{} is available (you have {})", m.version, crate::pkg::store::cached_manifest(&self.ctx).map(|c| c.version).unwrap_or_default());
                    self.update_m = Some(m);
                    let p = Prompt { text, buttons: vec!["Update".into(), "Later".into(), "Skip this version".into()], cancel: 1, kind: PKind::Update };
                    self.overlay(ui).push(p);
                }
                UpdateMsg::Ready => {
                    self.send(encode::simple(ev::CLOSE));
                    self.pending_update = Some(Instant::now());
                }
                UpdateMsg::Failed(e) => {
                    eprintln!("qa: update failed: {e}");
                }
            }
        }
        if let Some(t) = self.pending_update {
            if self.in_flight == 0 || t.elapsed() > Duration::from_secs(2) {
                self.pending_update = None;
                match crate::pkg::store::apply_update(&self.ctx).and_then(|m| {
                    let w = std::fs::read(self.ctx.home.join("bundle/app.wasm")).map_err(|e| e.to_string())?;
                    Ok((m, w))
                }) {
                    Ok((m, wasm)) => {
                        eprintln!("qa: updated {} to {}", m.name, m.version);
                        self.restart(ui, Some(wasm));
                    }
                    Err(e) => eprintln!("qa: update failed: {e}"),
                }
            }
        }
    }

    fn overlay(&mut self, ui: &Ui) -> &mut Overlay {
        if self.overlay.is_none() {
            let mut o = Overlay::new(&self.name, crate::ui::theme::Theme::load(Some(&self.ctx.id)));
            o.ui.match_window(ui);
            self.overlay = Some(o);
        }
        self.overlay.as_mut().unwrap()
    }

    fn spawn(&mut self, ui: &mut Ui) {
        *self.shared.win.lock().unwrap() = [ui.win_w, ui.win_h, ui.scale];
        self.shared.stop.store(false, Ordering::Relaxed);
        let data = self.ctx.data_dir();
        let _ = std::fs::create_dir_all(&data);
        let data_fd = fs::open_dir(&data).ok();
        let bundle_fd = self.ctx.bundle.as_ref().and_then(|b| fs::open_dir(b).ok());
        let setup = Setup { wasm: self.wasm.clone(), mem_limit: self.mem_limit, cpu_unlimited: self.cpu_unlimited, fs: fs::Fs::new(data_fd, bundle_fd) };
        let (tx, arx) = mpsc::channel();
        let (atx, rx) = mpsc::channel();
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("app".into())
            .stack_size(8 << 20)
            .spawn(move || app::run(setup, arx, atx, shared))
            .expect("spawn app thread");
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.error = None;
        // qa_start counts as a call in flight.
        self.in_flight = 1;
        self.busy_since = Some(Instant::now());
        for e in std::mem::take(&mut self.after_start) {
            self.send(e);
        }
    }

    fn to_app(&mut self, m: ToApp) {
        let counts = matches!(m, ToApp::Event(_) | ToApp::Done { .. });
        if let Some(tx) = &self.tx {
            if tx.send(m).is_err() {
                self.tx = None;
                return;
            }
            if counts {
                if self.in_flight == 0 {
                    self.busy_since = Some(Instant::now());
                }
                self.in_flight += 1;
            }
        }
    }

    pub fn send(&mut self, ev: Vec<u8>) {
        self.to_app(ToApp::Event(ev));
    }

    fn done(&mut self, req: u32, result: i32, info: &str, handle: Option<fs::Handle>) {
        self.to_app(ToApp::Done { req, result, info: info.to_string(), handle });
    }

    /// Replaces the app: stops the old instance, clears the tree, starts `wasm`.
    pub fn restart(&mut self, ui: &mut Ui, wasm: Option<Vec<u8>>) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(ToApp::Stop);
        }
        self.rx = None;
        // The old thread may still be finishing; give the new one its own stop flag.
        let efd = self.efd;
        self.shared = Arc::new(Shared { win: Mutex::new([0.0; 3]), efd, name: self.name.clone(), stop: Default::default(), secrets: self.ctx.home.join("secrets") });
        if let Some(w) = wasm {
            self.wasm = w;
        }
        if let Some(o) = self.overlay.as_mut() {
            o.withdraw(|k| matches!(k, PKind::Stopped | PKind::NotResponding | PKind::Ask(_)));
        }
        ui.reset();
        self.pix = Pixmaps::default();
        self.spawn(ui);
    }

    fn fail(&mut self, ui: &mut Ui, msg: String) {
        eprintln!("qa: {}: {msg}", self.name);
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(ToApp::Stop);
        }
        self.in_flight = 0;
        self.busy_since = None;
        self.error = Some(msg.clone());
        let o = self.overlay(ui);
        o.withdraw(|k| matches!(k, PKind::NotResponding));
        o.push(Prompt {
            text: format!("stopped: {msg}"),
            buttons: vec!["Restart".into(), "Close".into()],
            cancel: 1,
            kind: PKind::Stopped,
        });
        ui.damage_all();
    }

    fn apply(&mut self, ui: &mut Ui, o: Out) {
        if !o.patch.is_empty() {
            if let Err(e) = ui.apply_patch(&o.patch) {
                let msg = if self.dev { format!("bad patch at byte {}: {}", e.offset, e.msg) } else { format!("bad patch: {}", e.msg) };
                return self.fail(ui, msg);
            }
        }
        if !ui.image_dirty.is_empty() {
            self.load_images(ui);
        }
        if let Some(t) = o.title {
            self.title = Some(t);
        }
        if let Some(c) = o.clipboard {
            ui.clipboard_out = Some(c);
        }
        for (node, x, y, w, h, rgba) in o.canvas {
            self.canvas_put(ui, node, x, y, w, h, &rgba);
        }
        if let Some(u) = o.open_url {
            self.services.open_url(&u);
        }
        let net_ok = self.grants.internet() || self.internet_once;
        for (id, r) in o.http {
            if net_ok {
                self.services.http(id, r);
            } else {
                self.net_wait.push((false, id, r));
            }
        }
        for id in o.http_cancel {
            self.net_wait.retain(|(ws, i, _)| *ws || *i != id);
            self.services.http_cancel(id);
        }
        for (id, r) in o.ws_open {
            if net_ok {
                self.services.ws_open(id, r);
            } else {
                self.net_wait.push((true, id, r));
            }
        }
        if !self.net_wait.is_empty() && !self.overlay(ui).has_prompt_kind(|k| matches!(k, PKind::Internet)) {
            self.ask(ui, "wants to use the internet".into(), &["Allow", "Allow once", "Deny"], PKind::Internet);
        }
        for (id, k, d) in o.ws_send {
            self.services.ws_send(id, k, d);
        }
        for (id, c) in o.ws_close {
            self.services.ws_close(id, c);
        }
        if o.quit {
            self.quit = true;
        }
        if let Some(e) = o.error {
            self.fail(ui, e);
        }
    }

    /// Decodes images whose source changed. Missing or bad images log and stay empty.
    fn load_images(&mut self, ui: &mut Ui) {
        use qa_proto::prop::image;
        self.pix.images.retain(|id, _| ui.node(*id).is_some());
        self.pix.canvases.retain(|id, _| ui.node(*id).is_some());
        for id in std::mem::take(&mut ui.image_dirty) {
            let Some(n) = ui.node(id) else { continue };
            let bytes: Result<Vec<u8>, String> = match n.get(image::DATA) {
                Some(crate::ui::Val::B(b)) => Ok(b.clone()),
                _ => {
                    let src = n.s(image::SRC).to_string();
                    let (base, rel) = match src.strip_prefix("app:") {
                        Some(r) => (Some(self.ctx.data_dir()), r.to_string()),
                        None => (self.ctx.bundle.clone(), src.clone()),
                    };
                    match base {
                        _ if src.is_empty() => Err(String::new()),
                        Some(b) if fs::valid(&rel) => std::fs::read(b.join(&rel)).map_err(|e| format!("{src}: {e}")),
                        _ => Err(format!("{src}: not allowed")),
                    }
                }
            };
            let r = bytes.and_then(|b| crate::gfx::png::decode(&b));
            match r {
                Ok(pm) => {
                    ui.image_sizes.insert(id, (pm.w as f32, pm.h as f32));
                    self.pix.images.insert(id, pm);
                }
                Err(e) => {
                    if !e.is_empty() {
                        eprintln!("qa: {}: image: {e}", self.name);
                    }
                    ui.image_sizes.remove(&id);
                    self.pix.images.remove(&id);
                }
            }
            ui.invalidate(id);
            ui.damage_node(id);
        }
    }

    fn canvas_put(&mut self, ui: &mut Ui, node: u32, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) {
        let Some(n) = ui.node(node) else { return };
        if n.kind != qa_proto::kind::CANVAS {
            return;
        }
        let pw = n.i(qa_proto::prop::canvas::PX_W, 0).max(0) as u32;
        let ph = n.i(qa_proto::prop::canvas::PX_H, 0).max(0) as u32;
        let pm = self.pix.canvases.entry(node).or_insert_with(|| Pixmap { w: 0, h: 0, px: Vec::new() });
        if pm.w as u32 != pw || pm.h as u32 != ph {
            *pm = Pixmap { w: pw as i32, h: ph as i32, px: vec![0; (pw * ph) as usize] };
        }
        for row in 0..h {
            let ty = y + row;
            if ty >= ph {
                break;
            }
            for col in 0..w {
                let tx = x + col;
                if tx >= pw {
                    break;
                }
                let i = ((row * w + col) * 4) as usize;
                let c = crate::gfx::Color::rgba(rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]);
                pm.px[(ty * pw + tx) as usize] = c.premul();
            }
        }
        ui.damage_node(node);
    }

    fn ask(&mut self, ui: &mut Ui, text: String, buttons: &[&str], kind: PKind) {
        let cancel = buttons.len() - 1;
        let p = Prompt { text, buttons: buttons.iter().map(|s| s.to_string()).collect(), cancel, kind };
        self.overlay(ui).push(p);
    }

    fn open_folder(&mut self, req: u32, path: &str, write: bool) {
        match fs::open_dir(std::path::Path::new(path)) {
            Ok(fd) => self.done(req, 0, path, Some(fs::Handle { fd, kind: fs::HKind::Dir { write } })),
            Err(_) => self.done(req, err::NOT_FOUND, path, None),
        }
    }

    fn handle_request(&mut self, ui: &mut Ui, r: u32, kind: u32, arg: Vec<u8>) {
        let mut rd = Reader::new(&arg);
        match kind {
            req::OPEN_FILE => {
                let f = rd.str().unwrap_or("");
                let patterns = f.split(';').filter(|s| !s.trim().is_empty()).map(|s| s.to_string()).collect();
                self.overlay(ui).pick(r, PickMode::Open { patterns });
            }
            req::SAVE_FILE => {
                let name = rd.str().unwrap_or("").to_string();
                self.overlay(ui).pick(r, PickMode::Save { name });
            }
            req::PICK_FOLDER => {
                let write = rd.u8().unwrap_or(0) == 1;
                self.overlay(ui).pick(r, PickMode::Folder { write });
            }
            req::FOLDER => {
                let write = rd.u8().unwrap_or(0) == 1;
                let path = rd.str().unwrap_or("").trim_end_matches('/').to_string();
                if !path.starts_with('/') {
                    return self.done(r, err::INVALID, "", None);
                }
                if self.grants.folder(&path, write) {
                    return self.open_folder(r, &path, write);
                }
                let what = if write { "read and change files in" } else { "read files in" };
                self.ask(ui, format!("wants to {what} {path}"), &["Allow", "Allow once", "Deny"], PKind::Request { req: r, kind, arg });
            }
            req::INTERNET => {
                if self.grants.internet() || self.internet_once {
                    return self.done(r, 0, "", None);
                }
                self.ask(ui, "wants to use the internet".into(), &["Allow", "Allow once", "Deny"], PKind::Request { req: r, kind, arg });
            }
            req::MEMORY => {
                let bytes = rd.u64().unwrap_or(0) as usize;
                if bytes <= self.mem_limit {
                    return self.done(r, 0, "", None);
                }
                self.ask(ui, format!("wants to use up to {} of memory", mb(bytes)), &["Allow", "Deny"], PKind::Request { req: r, kind, arg });
            }
            req::CPU => {
                if self.cpu_unlimited {
                    return self.done(r, 0, "", None);
                }
                self.ask(ui, "wants to use more than half of a CPU core".into(), &["Allow", "Deny"], PKind::Request { req: r, kind, arg });
            }
            req::CLIPBOARD_READ => {
                if ui.last_input.elapsed() < Duration::from_secs(1) {
                    ui.app_paste.push(r);
                } else {
                    self.ask(ui, "wants to read your clipboard".into(), &["Allow once", "Deny"], PKind::Request { req: r, kind, arg });
                }
            }
            req::MESSAGE => {
                let to = rd.str().unwrap_or("").to_string();
                if self.grants.message(&to) {
                    return self.done(r, 0, "", None);
                }
                self.ask(ui, format!("wants to send data to {to}"), &["Allow", "Deny"], PKind::Request { req: r, kind, arg });
            }
            _ => self.done(r, err::INVALID, "", None),
        }
    }

    fn answered(&mut self, ui: &mut Ui, pr: Prompt, i: usize) {
        let allow = i == 0;
        let once = pr.buttons.get(i).is_some_and(|b| b == "Allow once");
        match pr.kind {
            PKind::Ask(reply) => {
                let _ = reply.send(i);
            }
            PKind::Update => {
                let Some(m) = self.update_m.take() else { return };
                match i {
                    0 => {
                        let (tx, rx) = mpsc::channel();
                        let (ctx, efd) = (self.ctx.clone(), self.efd);
                        std::thread::spawn(move || {
                            let r = crate::pkg::store::download_update(&ctx, &m);
                            let _ = tx.send(match r {
                                Ok(()) => UpdateMsg::Ready,
                                Err(e) => UpdateMsg::Failed(e),
                            });
                            crate::sys::wake(efd);
                        });
                        self.update_rx = Some(rx);
                    }
                    2 => crate::pkg::store::skip_version(&self.ctx, &m.version),
                    _ => {}
                }
            }
            PKind::Internet => {
                let waiting = std::mem::take(&mut self.net_wait);
                if allow || once {
                    if allow {
                        self.grants.add(Grant::Internet);
                    } else {
                        self.internet_once = true;
                    }
                    for (ws, id, r) in waiting {
                        if ws {
                            self.services.ws_open(id, r);
                        } else {
                            self.services.http(id, r);
                        }
                    }
                } else {
                    for (ws, id, _) in waiting {
                        let e = if ws {
                            let mut b = vec![ev::WS];
                            qa_proto::Sink::u32(&mut b, id);
                            b.push(crate::io::ws::ERROR);
                            qa_proto::Sink::bytes(&mut b, b"denied");
                            b
                        } else {
                            crate::io::http::encode_event(id, Err("denied".into()))
                        };
                        self.send(e);
                    }
                }
            }
            PKind::NotResponding => {
                if i == 0 {
                    self.busy_since = Some(Instant::now());
                } else {
                    self.shared.stop.store(true, Ordering::Relaxed);
                }
            }
            PKind::Stopped => {
                if i == 0 {
                    self.restart(ui, None);
                } else {
                    self.quit = true;
                }
            }
            PKind::Message { to, data, reply } => {
                if allow {
                    self.grants.add(Grant::Message(to.clone()));
                    let _ = reply.send(self.services.send_msg(&to, &data));
                } else {
                    let _ = reply.send(err::DENIED);
                }
            }
            PKind::Request { req: r, kind, arg } => {
                let mut rd = Reader::new(&arg);
                if !(allow || once) {
                    return self.done(r, err::DENIED, "", None);
                }
                match kind {
                    req::FOLDER => {
                        let write = rd.u8().unwrap_or(0) == 1;
                        let path = rd.str().unwrap_or("").trim_end_matches('/').to_string();
                        if allow {
                            self.grants.add(Grant::Folder { write, path: path.clone() });
                        }
                        self.open_folder(r, &path, write);
                    }
                    req::INTERNET => {
                        if allow {
                            self.grants.add(Grant::Internet);
                        } else {
                            self.internet_once = true;
                        }
                        self.done(r, 0, "", None);
                    }
                    req::MEMORY => {
                        let bytes = rd.u64().unwrap_or(0) as usize;
                        self.mem_limit = self.mem_limit.max(bytes);
                        self.grants.add(Grant::Memory(self.mem_limit as u64));
                        self.to_app(ToApp::SetMemLimit(self.mem_limit));
                        self.done(r, 0, "", None);
                    }
                    req::CPU => {
                        self.cpu_unlimited = true;
                        self.grants.add(Grant::Cpu);
                        self.to_app(ToApp::SetCpuUnlimited);
                        self.done(r, 0, "", None);
                    }
                    req::CLIPBOARD_READ => ui.app_paste.push(r),
                    req::MESSAGE => {
                        let to = rd.str().unwrap_or("").to_string();
                        self.grants.add(Grant::Message(to));
                        self.done(r, 0, "", None);
                    }
                    _ => self.done(r, err::INVALID, "", None),
                }
            }
        }
    }

    fn picked(&mut self, r: u32, mode: PickMode, path: Option<std::path::PathBuf>, remember: bool) {
        let Some(path) = path else { return self.done(r, err::DENIED, "", None) };
        let shown = path.display().to_string();
        let opened: std::io::Result<(OwnedFd, fs::HKind)> = match &mode {
            PickMode::Open { .. } => std::fs::File::open(&path).map(|f| (f.into(), fs::HKind::File { read: true, write: false })),
            PickMode::Save { .. } => std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .map(|f| (f.into(), fs::HKind::File { read: false, write: true })),
            PickMode::Folder { write } => fs::open_dir(&path).map(|fd| (fd, fs::HKind::Dir { write: *write })),
        };
        match opened {
            Ok((fd, kind)) => {
                if let (PickMode::Folder { write }, true) = (&mode, remember) {
                    self.grants.add(Grant::Folder { write: *write, path: shown.clone() });
                }
                let info = match mode {
                    PickMode::Folder { .. } => shown,
                    _ => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                };
                self.done(r, 0, &info, Some(fs::Handle { fd, kind }));
            }
            Err(_) => self.done(r, err::IO, "", None),
        }
    }
}

impl Driver for WasmDriver {
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn fds(&self) -> Vec<RawFd> {
        let mut v = vec![self.efd];
        v.extend(self.services.fds());
        v
    }

    fn timeout_ms(&mut self) -> Option<i32> {
        let mut t: Option<Duration> = None;
        if let Some(c) = self.closing {
            let left = Duration::from_secs(2).saturating_sub(c.elapsed());
            if left.is_zero() {
                self.quit = true;
            }
            t = Some(left);
        }
        if let Some(b) = self.busy_since {
            let left = NOT_RESPONDING.saturating_sub(b.elapsed());
            t = Some(t.map_or(left, |x| x.min(left)));
        }
        t.map(|d| d.as_millis() as i32 + 1)
    }

    fn start(&mut self, ui: &mut Ui) {
        self.spawn(ui);
    }

    fn events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        for e in evs {
            if let UiEvent::Key { sym: 0x72, mods: 2, .. } = e {
                if self.error.is_some() {
                    self.restart(ui, None);
                    continue;
                }
            }
            self.send(encode::ui_event(&e, ui.tag_of(e.node())));
        }
    }

    fn resize(&mut self, ui: &mut Ui, w: f32, h: f32, scale: f32) {
        *self.shared.win.lock().unwrap() = [w, h, scale];
        if let Some(o) = self.overlay.as_mut() {
            o.ui.match_window(ui);
        }
        self.send(encode::resize(w, h, scale));
    }

    fn poll(&mut self, ui: &mut Ui) {
        crate::sys::drain(self.efd);
        loop {
            let Some(rx) = &self.rx else { break };
            match rx.try_recv() {
                Ok(FromApp::Out(o)) => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    self.busy_since = if self.in_flight > 0 { Some(Instant::now()) } else { None };
                    if let Some(ov) = self.overlay.as_mut() {
                        ov.withdraw(|k| matches!(k, PKind::NotResponding));
                    }
                    self.apply(ui, o);
                }
                Ok(FromApp::Ask(a, reply)) => {
                    // The app thread is blocked on purpose; that is not "not responding".
                    self.busy_since = None;
                    let (text, buttons): (String, &[&str]) = match a {
                        Ask::Memory { limit, desired } => (format!("wants to use more than {} of memory ({} asked)", mb(limit), mb(desired)), &["Allow", "Deny"]),
                        Ask::Cpu => ("is using a lot of CPU and is being slowed down".into(), &["Allow", "Keep limited", "Stop"]),
                    };
                    let buttons: Vec<String> = buttons.iter().map(|s| s.to_string()).collect();
                    let cancel = 1;
                    self.overlay(ui).push(Prompt { text, buttons, cancel, kind: PKind::Ask(reply) });
                }
                Ok(FromApp::Request { req, kind, arg }) => self.handle_request(ui, req, kind, arg),
                Ok(FromApp::Msg { to, data, reply }) => {
                    if self.grants.message(&to) {
                        let _ = reply.send(self.services.send_msg(&to, &data));
                    } else {
                        let text = format!("wants to send data to {to}");
                        self.ask(ui, text, &["Allow", "Deny"], PKind::Message { to, data, reply });
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.tx = None;
                    self.in_flight = 0;
                    self.busy_since = None;
                    if self.closing.is_some() {
                        self.quit = true;
                    }
                }
            }
        }
        for e in self.services.poll() {
            self.send(e);
        }
        self.poll_update(ui);
        if let Some(b) = self.busy_since {
            if b.elapsed() >= NOT_RESPONDING && self.error.is_none() && self.closing.is_none() {
                let o = self.overlay(ui);
                if !o.has_prompt_kind(|k| matches!(k, PKind::NotResponding)) {
                    o.push(Prompt { text: "is not responding".into(), buttons: vec!["Wait".into(), "Stop".into()], cancel: 0, kind: PKind::NotResponding });
                }
            }
        }
    }

    fn close(&mut self, _ui: &mut Ui) -> bool {
        if self.closing.is_some() || self.tx.is_none() {
            return true;
        }
        self.closing = Some(Instant::now());
        self.send(encode::simple(ev::CLOSE));
        if let Some(tx) = &self.tx {
            let _ = tx.send(ToApp::Stop);
        }
        false
    }

    fn take_title(&mut self) -> Option<String> {
        self.title.take()
    }

    fn quit(&self) -> bool {
        self.quit
    }

    fn pixmaps(&self) -> &dyn PixmapSource {
        &self.pix
    }

    fn overlay_ui(&mut self) -> Option<&mut Ui> {
        self.overlay.as_mut().filter(|o| o.active()).map(|o| &mut o.ui)
    }

    fn overlay_events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        let Some(o) = self.overlay.as_mut() else { return };
        for d in o.events(evs) {
            match d {
                Done::Prompt(p, i) => self.answered(ui, p, i),
                Done::Picked { req, mode, path, remember } => self.picked(req, mode, path, remember),
            }
        }
        ui.damage_all();
    }

    fn theme_changed(&mut self) {
        if let Some(o) = self.overlay.as_mut() {
            o.ui.theme.reload();
            o.ui.invalidate_all();
            o.ui.damage_all();
        }
    }

    fn pasted(&mut self, ui: &mut Ui, text: String) {
        for r in std::mem::take(&mut ui.app_paste) {
            self.done(r, 0, &text, None);
        }
    }
}
