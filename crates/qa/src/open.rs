//! `qa open`: what a `qa:` link runs. An installed app just starts. Otherwise
//! a small runtime-owned window asks first, while the bundle downloads in the
//! background (docs/packaging.md).

use crate::gfx::{Pixmap, PixmapSource};
use crate::pkg::store::{self, Manifest, Staged};
use crate::shell::Driver;
use crate::ui::{Ui, UiEvent};
use qa_proto::{kind, prop, Sink, Track, Value};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// `qa:https://host/app.json` or `qa://host/app.json` → the manifest URL.
pub fn link_url(arg: &str) -> String {
    match arg.strip_prefix("qa:") {
        Some(r) if r.starts_with("//") => format!("https:{r}"),
        Some(r) => r.to_string(),
        None => arg.to_string(),
    }
}

pub fn open(arg: &str) -> i32 {
    let url = link_url(arg);
    let ctx = store::ctx_for(&url);
    if store::is_installed(&ctx) && store::desktop_path(&ctx).exists() {
        return crate::cli::run(&url);
    }
    let _ = store::register_handler();
    let mut d = Prompt::new(&url);
    let code = crate::run_native_sized(&mut d, "Install app", "qa-open", (460, 250), None);
    match d.outcome {
        Outcome::Run => crate::cli::run(&url),
        Outcome::Close => code,
    }
}

enum Msg {
    Manifest(Manifest),
    Staged(Staged),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outcome {
    Close,
    /// Close this window and start the app.
    Run,
}

#[derive(PartialEq, Debug)]
enum State {
    Loading,
    /// Waiting for the user. `click` remembers an Install / Run pressed before
    /// the download finished.
    Ask { click: Option<u32> },
    Installed,
    Failed,
}

pub struct Prompt {
    url: String,
    rx: Receiver<Msg>,
    state: State,
    m: Option<Manifest>,
    staged: Option<Staged>,
    pub outcome: Outcome,
    quit: bool,
    icon: Option<Pixmap>,
    // Nodes.
    img: u32,
    title: u32,
    meta: u32,
    desc: u32,
    status: u32,
    cancel: u32,
    run: u32,
    install: u32,
}

const IMG: u32 = 2;

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

fn human(n: u64) -> String {
    match n {
        0 => String::new(),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n => format!("{} KB", n.div_ceil(1024)),
    }
}

fn set_str(p: &mut Vec<u8>, id: u32, pr: u16, s: &str) {
    p.op_set(id, pr);
    p.value(&Value::Str(s));
}

fn set_int(p: &mut Vec<u8>, id: u32, pr: u16, v: i32) {
    p.op_set(id, pr);
    p.value(&Value::I32(v));
}

impl Prompt {
    pub fn new(url: &str) -> Prompt {
        let (tx, rx) = channel();
        let u = url.to_string();
        std::thread::spawn(move || {
            let r = store::fetch_manifest(&u).and_then(|(m, etag)| {
                let _ = tx.send(Msg::Manifest(m.clone()));
                store::stage(&u, m, etag)
            });
            let _ = tx.send(match r {
                Ok(s) => Msg::Staged(s),
                Err(e) => Msg::Failed(e),
            });
        });
        Prompt {
            url: url.to_string(),
            rx,
            state: State::Loading,
            m: None,
            staged: None,
            outcome: Outcome::Close,
            quit: false,
            icon: None,
            img: IMG,
            title: 0,
            meta: 0,
            desc: 0,
            status: 0,
            cancel: 0,
            run: 0,
            install: 0,
        }
    }

    fn apply(ui: &mut Ui, p: &[u8]) {
        if let Err(e) = ui.apply_patch(p) {
            eprintln!("qa open: patch error at {}: {}", e.offset, e.msg);
        }
    }

    fn name(&self) -> &str {
        self.m.as_ref().map(|m| m.name.as_str()).unwrap_or("this app")
    }

    /// Shows the buttons for the current state.
    fn buttons(&self, p: &mut Vec<u8>) {
        let (cancel, run, install) = match self.state {
            State::Loading | State::Ask { .. } => ("Cancel", "Just run it", "Install"),
            State::Installed => ("Close", "", "Open"),
            State::Failed => ("Close", "", ""),
        };
        for (id, label) in [(self.cancel, cancel), (self.run, run), (self.install, install)] {
            set_int(p, id, prop::HIDDEN, label.is_empty() as i32);
            if !label.is_empty() {
                set_str(p, id, prop::button::LABEL, label);
            }
        }
        let busy = matches!(self.state, State::Loading | State::Ask { click: Some(_) });
        set_int(p, self.run, prop::button::DISABLED, busy as i32);
        set_int(p, self.install, prop::button::DISABLED, busy as i32);
    }

    fn fail(&mut self, ui: &mut Ui, e: &str) {
        self.state = State::Failed;
        let mut p = Vec::new();
        if self.m.is_none() {
            set_str(&mut p, self.title, prop::text::TEXT, "Couldn't open this app");
        }
        set_str(&mut p, self.status, prop::text::TEXT, e);
        set_str(&mut p, self.status, prop::VARIANT, "danger");
        self.buttons(&mut p);
        p.op_focus(self.cancel);
        Self::apply(ui, &p);
    }

    fn got_manifest(&mut self, ui: &mut Ui, m: Manifest) {
        let mut p = Vec::new();
        set_str(&mut p, self.title, prop::text::TEXT, &m.name);
        set_str(&mut p, self.title, prop::VARIANT, "");
        let meta = [m.version.as_str(), &human(m.size), host(&self.url)].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
        set_str(&mut p, self.meta, prop::text::TEXT, &meta);
        set_str(&mut p, self.desc, prop::text::TEXT, &m.description);
        set_int(&mut p, self.desc, prop::HIDDEN, m.description.is_empty() as i32);
        let cmd = m.command_name().filter(|c| store::command_owner(c).is_none());
        let what = match cmd {
            Some(c) => format!("Install adds it to your app menu and the {c} command."),
            None => "Install adds it to your app menu.".to_string(),
        };
        set_str(&mut p, self.status, prop::text::TEXT, &what);
        self.m = Some(m);
        self.state = State::Ask { click: None };
        self.buttons(&mut p);
        p.op_focus(self.install);
        Self::apply(ui, &p);
    }

    fn got_bundle(&mut self, ui: &mut Ui, s: Staged) {
        if let Ok(b) = std::fs::read(s.dir.join("icon.png")) {
            if let Ok(pm) = crate::gfx::png::decode(&b) {
                ui.image_sizes.insert(self.img, (pm.w as f32, pm.h as f32));
                self.icon = Some(pm);
                let mut p = Vec::new();
                set_int(&mut p, self.img, prop::HIDDEN, 0);
                Self::apply(ui, &p);
            }
        }
        self.staged = Some(s);
        if let State::Ask { click: Some(id) } = self.state {
            self.state = State::Ask { click: None };
            self.clicked(ui, id);
        } else {
            let mut p = Vec::new();
            self.buttons(&mut p);
            Self::apply(ui, &p);
        }
    }

    fn clicked(&mut self, ui: &mut Ui, id: u32) {
        match self.state {
            _ if id == self.cancel => self.quit = true,
            State::Installed if id == self.install => {
                self.outcome = Outcome::Run;
                self.quit = true;
            }
            State::Ask { click: None } if id == self.run || id == self.install => {
                let Some(s) = self.staged.take() else {
                    // Still downloading: act when it lands.
                    self.state = State::Ask { click: Some(id) };
                    let mut p = Vec::new();
                    set_str(&mut p, self.status, prop::text::TEXT, "Downloading…");
                    self.buttons(&mut p);
                    Self::apply(ui, &p);
                    return;
                };
                let ctx = match store::commit(s) {
                    Ok(c) => c,
                    Err(e) => return self.fail(ui, &e),
                };
                if id == self.run {
                    self.outcome = Outcome::Run;
                    self.quit = true;
                    return;
                }
                let m = store::cached_manifest(&ctx).unwrap_or_else(|| self.m.clone().unwrap());
                if let Err(e) = store::write_desktop(&ctx, &m) {
                    return self.fail(ui, &format!("Couldn't add the menu entry: {e}"));
                }
                let msg = match store::link_command(&ctx, &m) {
                    Ok(Some(c)) => format!("{} is installed. Find it in your app menu, or type {c} in a terminal.", m.name),
                    _ => format!("{} is installed. Find it in your app menu.", m.name),
                };
                self.state = State::Installed;
                let mut p = Vec::new();
                set_str(&mut p, self.status, prop::text::TEXT, &msg);
                self.buttons(&mut p);
                p.op_focus(self.install);
                Self::apply(ui, &p);
            }
            _ => {}
        }
    }
}

impl Driver for Prompt {
    fn timeout_ms(&mut self) -> Option<i32> {
        (self.state == State::Loading || matches!(self.state, State::Ask { .. }) && self.staged.is_none()).then_some(30)
    }

    fn start(&mut self, ui: &mut Ui) {
        let mut p = Vec::new();
        let mut next = 1;
        let mut node = |p: &mut Vec<u8>, k: u8| {
            let id = next;
            next += 1;
            p.op_create(id, k);
            id
        };
        let root = node(&mut p, kind::GRID);
        p.op_set(root, prop::PAD);
        p.value(&Value::Edges([20.0; 4]));
        p.op_set(root, prop::grid::GAP);
        p.value(&Value::F32(16.0));
        p.op_set(root, prop::grid::ROWS);
        p.tracks(&[Track::fr(1.0), Track::auto()]);
        // The header: icon, then name, meta and description.
        let head = node(&mut p, kind::GRID);
        p.op_set(head, prop::grid::COLS);
        p.tracks(&[Track::auto(), Track::fr(1.0)]);
        p.op_set(head, prop::grid::GAP);
        p.value(&Value::F32(16.0));
        set_int(&mut p, head, prop::ALIGN_Y, 0);
        self.img = node(&mut p, kind::IMAGE);
        debug_assert_eq!(self.img, IMG);
        for pr in [prop::MIN_W, prop::MAX_W, prop::MIN_H, prop::MAX_H] {
            p.op_set(self.img, pr);
            p.value(&Value::F32(64.0));
        }
        set_int(&mut p, self.img, prop::HIDDEN, 1);
        set_int(&mut p, self.img, prop::ALIGN_Y, 0);
        let info = node(&mut p, kind::GRID);
        p.op_set(info, prop::grid::GAP);
        p.value(&Value::F32(4.0));
        set_int(&mut p, info, prop::ALIGN_Y, 0);
        self.title = node(&mut p, kind::TEXT);
        set_str(&mut p, self.title, prop::text::TEXT, "Loading…");
        set_str(&mut p, self.title, prop::text::STYLE, "title");
        set_str(&mut p, self.title, prop::VARIANT, "muted");
        self.meta = node(&mut p, kind::TEXT);
        set_str(&mut p, self.meta, prop::text::TEXT, host(&self.url));
        set_str(&mut p, self.meta, prop::VARIANT, "muted");
        self.desc = node(&mut p, kind::TEXT);
        set_int(&mut p, self.desc, prop::HIDDEN, 1);
        self.status = node(&mut p, kind::TEXT);
        set_str(&mut p, self.status, prop::text::TEXT, "");
        for c in [self.title, self.meta, self.desc, self.status] {
            p.op_append(info, c);
        }
        p.op_append(head, self.img);
        p.op_append(head, info);
        // Buttons, right-aligned.
        let bar = node(&mut p, kind::GRID);
        p.op_set(bar, prop::grid::COLS);
        p.tracks(&[Track::fr(1.0), Track::auto(), Track::auto(), Track::auto()]);
        p.op_set(bar, prop::grid::GAP);
        p.value(&Value::F32(8.0));
        let spacer = node(&mut p, kind::GRID);
        self.cancel = node(&mut p, kind::BUTTON);
        self.run = node(&mut p, kind::BUTTON);
        self.install = node(&mut p, kind::BUTTON);
        set_str(&mut p, self.install, prop::VARIANT, "primary");
        for c in [spacer, self.cancel, self.run, self.install] {
            p.op_append(bar, c);
        }
        p.op_append(root, head);
        p.op_append(root, bar);
        p.op_root(root);
        self.buttons(&mut p);
        p.op_focus(self.cancel);
        Self::apply(ui, &p);
    }

    fn poll(&mut self, ui: &mut Ui) {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Manifest(m)) => self.got_manifest(ui, m),
                Ok(Msg::Staged(s)) => self.got_bundle(ui, s),
                Ok(Msg::Failed(e)) => self.fail(ui, &e),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        for e in evs {
            match e {
                UiEvent::Click(id) => self.clicked(ui, id),
                UiEvent::Key { sym: 0xff1b, .. } => self.quit = true,
                _ => {}
            }
        }
    }

    fn quit(&self) -> bool {
        self.quit
    }

    fn pixmaps(&self) -> &dyn PixmapSource {
        self
    }
}

impl PixmapSource for Prompt {
    fn image(&self, id: u32) -> Option<&Pixmap> {
        self.icon.as_ref().filter(|_| id == self.img)
    }
    fn canvas(&self, _: u32) -> Option<&Pixmap> {
        None
    }
}
