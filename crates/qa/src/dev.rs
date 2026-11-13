//! `qa dev <dir>`: build a Rust app crate, run it, rebuild and reload on change.

use crate::gfx::{Cmd, PixmapSource};
use crate::host::WasmDriver;
use crate::shell::Driver;
use crate::ui::{Ui, UiEvent};
use std::ffi::CString;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Builds the crate in `dir` for wasm and returns the module bytes.
pub fn build(dir: &Path) -> Result<Vec<u8>, String> {
    eprintln!("qa dev: building {}", dir.display());
    let out = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !out.status.success() {
        return Err("build failed (details in the terminal)".into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut wasm = None;
    for line in stdout.lines().filter(|l| l.contains("\"compiler-artifact\"")) {
        if let Some(i) = line.find("\"filenames\":[") {
            for part in line[i + 13..].split('"') {
                if part.ends_with(".wasm") {
                    wasm = Some(PathBuf::from(part));
                }
            }
        }
    }
    let path = wasm.ok_or("cargo produced no .wasm (is crate-type = [\"cdylib\"] set?)")?;
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

pub struct Dev {
    dir: PathBuf,
    inner: WasmDriver,
    ifd: RawFd,
    pending: Option<Instant>,
    build_error: Option<String>,
}

fn watch(ifd: RawFd, p: &Path, recursive: bool) {
    let Ok(c) = CString::new(p.as_os_str().as_encoded_bytes()) else { return };
    let mask = libc::IN_CLOSE_WRITE | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_TO;
    if unsafe { libc::inotify_add_watch(ifd, c.as_ptr(), mask) } < 0 {
        return;
    }
    if recursive {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    watch(ifd, &e.path(), true);
                }
            }
        }
    }
}

impl Dev {
    pub fn new(dir: PathBuf, wasm: Vec<u8>, name: &str, opts: &crate::shot::RunOpts) -> Dev {
        let ifd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        watch(ifd, &dir.join("src"), true);
        watch(ifd, &dir.join("assets"), true);
        watch(ifd, &dir, false);
        let ctx = crate::pkg::AppCtx::new(&format!("file://{}", dir.display()), name, Some(dir.clone()));
        let mut inner = WasmDriver::new(wasm, ctx);
        inner.dev = true;
        opts.apply(&mut inner, true);
        Dev { dir, inner, ifd, pending: None, build_error: None }
    }

    fn rebuild(&mut self, ui: &mut Ui) {
        match build(&self.dir) {
            Ok(w) => {
                eprintln!("qa dev: reloaded ({} bytes)", w.len());
                self.build_error = None;
                self.inner.restart(ui, Some(w));
            }
            Err(e) => {
                eprintln!("qa dev: {e}");
                self.build_error = Some(e);
                ui.damage_all();
            }
        }
    }
}

impl Driver for Dev {
    fn fds(&self) -> Vec<RawFd> {
        let mut v = self.inner.fds();
        v.push(self.ifd);
        v
    }
    fn timeout_ms(&mut self) -> Option<i32> {
        let a = self.pending.map(|t| DEBOUNCE.saturating_sub(t.elapsed()).as_millis() as i32 + 1);
        match (a, self.inner.timeout_ms()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    fn start(&mut self, ui: &mut Ui) {
        self.inner.start(ui);
    }
    fn events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        self.inner.events(ui, evs);
    }
    fn resize(&mut self, ui: &mut Ui, w: f32, h: f32, s: f32) {
        self.inner.resize(ui, w, h, s);
    }
    fn poll(&mut self, ui: &mut Ui) {
        let mut buf = [0u8; 4096];
        let mut changed = false;
        while unsafe { libc::read(self.ifd, buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {
            changed = true;
        }
        if changed {
            self.pending = Some(Instant::now());
        }
        if self.pending.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            self.pending = None;
            // New folders under src/ need watches too.
            watch(self.ifd, &self.dir.join("src"), true);
            self.rebuild(ui);
        }
        self.inner.poll(ui);
    }
    fn close(&mut self, ui: &mut Ui) -> bool {
        self.inner.close(ui)
    }
    fn take_title(&mut self) -> Option<String> {
        self.inner.take_title()
    }
    fn quit(&self) -> bool {
        self.inner.quit()
    }
    fn pixmaps(&self) -> &dyn PixmapSource {
        self.inner.pixmaps()
    }
    fn overlay_ui(&mut self) -> Option<&mut Ui> {
        self.inner.overlay_ui()
    }
    fn overlay_events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        self.inner.overlay_events(ui, evs)
    }
    fn pasted(&mut self, ui: &mut Ui, text: String) {
        self.inner.pasted(ui, text)
    }
    fn theme_changed(&mut self) {
        self.inner.theme_changed()
    }
    fn overlay(&mut self, ui: &mut Ui, list: &mut Vec<Cmd>) {
        if let Some(e) = &self.build_error {
            let msg = format!("qa dev: {e}");
            ui.band(list, &msg, true);
        } else {
            self.inner.overlay(ui, list);
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let mut args = args.to_vec();
    let opts = match crate::shot::RunOpts::take(&mut args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("qa dev: {e}");
            return 2;
        }
    };
    let [dir] = args.as_slice() else {
        eprintln!("usage: qa dev <dir> [--map from=to]... [--grant grant]...");
        return 2;
    };
    let dir = match std::fs::canonicalize(dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("qa dev: {dir}: {e}");
            return 1;
        }
    };
    let wasm = match build(&dir) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("qa dev: {e}");
            return 1;
        }
    };
    let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("app").to_string();
    let d = Dev::new(dir, wasm, &name, &opts);
    let app_id = format!("qa-{}", d.inner.ctx.id);
    crate::run_native(d, &name, &app_id)
}
