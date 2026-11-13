//! Runtime-owned overlays: the permission prompt band and the file picker.
//!
//! They are a second UI tree, built from the same native components and
//! styled by the user's theme, drawn over the app. The app cannot see,
//! draw over or click them.

use crate::gfx::Color;
use crate::text::Fonts;
use crate::ui::theme::Theme;
use crate::ui::{Ui, UiEvent};
use qa_proto::{kind, prop, Sink, Track, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

/// What a prompt is for; the driver acts on the answer.
pub enum PKind {
    Ask(std::sync::mpsc::Sender<usize>),
    Request { req: u32, kind: u32, arg: Vec<u8> },
    NotResponding,
    Stopped,
    /// Network calls are waiting for the internet grant.
    Internet,
    /// A newer version is on the server.
    Update,
    Message { to: String, data: Vec<u8>, reply: std::sync::mpsc::Sender<i32> },
}

pub struct Prompt {
    pub text: String,
    pub buttons: Vec<String>,
    /// The answer for Esc.
    pub cancel: usize,
    pub kind: PKind,
}

#[derive(Clone, Debug)]
pub enum PickMode {
    Open { patterns: Vec<String> },
    Save { name: String },
    Folder { write: bool },
}

pub enum Done {
    Prompt(Prompt, usize),
    /// `path` is None when cancelled.
    Picked { req: u32, mode: PickMode, path: Option<PathBuf>, remember: bool },
}

enum Item {
    Prompt(Prompt),
    Pick { req: u32, mode: PickMode },
}

struct Picker {
    req: u32,
    mode: PickMode,
    dir: PathBuf,
    list: u32,
    path_label: u32,
    up: u32,
    cancel: u32,
    ok: u32,
    name_input: u32,
    remember: u32,
    rows: HashMap<u32, (String, bool)>,
    selected: Option<String>,
    last_click: (u32, std::time::Instant),
    name: String,
}

pub struct Overlay {
    pub ui: Ui,
    app: String,
    queue: VecDeque<Item>,
    current: Option<Prompt>,
    buttons: Vec<u32>,
    picker: Option<Picker>,
    next: u32,
    pub changed: bool,
}

fn set_s(p: &mut Vec<u8>, id: u32, pr: u16, s: &str) {
    p.op_set(id, pr);
    p.value(&Value::Str(s));
}
fn set_i(p: &mut Vec<u8>, id: u32, pr: u16, v: i32) {
    p.op_set(id, pr);
    p.value(&Value::I32(v));
}
fn set_f(p: &mut Vec<u8>, id: u32, pr: u16, v: f32) {
    p.op_set(id, pr);
    p.value(&Value::F32(v));
}
fn pad(p: &mut Vec<u8>, id: u32, e: [f32; 4]) {
    p.op_set(id, prop::PAD);
    p.value(&Value::Edges(e));
}
fn ctx_c(p: &mut Vec<u8>, id: u32, k: &str, c: Color) {
    p.op_ctx(id, k);
    let v = ((c.r as u32) << 24) | ((c.g as u32) << 16) | ((c.b as u32) << 8) | c.a as u32;
    p.value(&Value::Color(v));
}
fn ctx_f(p: &mut Vec<u8>, id: u32, k: &str, f: f32) {
    p.op_ctx(id, k);
    p.value(&Value::F32(f));
}

fn matches(name: &str, pats: &[String]) -> bool {
    if pats.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    pats.iter().any(|p| {
        let p = p.trim().to_lowercase();
        if p == "*" || p.is_empty() {
            true
        } else if let Some(ext) = p.strip_prefix('*') {
            lower.ends_with(ext)
        } else {
            lower == p
        }
    })
}

impl Overlay {
    pub fn new(app: &str, theme: Theme) -> Overlay {
        let mut fonts = Fonts::new();
        let _ = fonts.find_first(crate::text::DEFAULT_SANS);
        let mut ui = Ui::new(fonts, theme);
        ui.transparent = true;
        Overlay { ui, app: app.to_string(), queue: VecDeque::new(), current: None, buttons: Vec::new(), picker: None, next: 1, changed: false }
    }

    pub fn active(&self) -> bool {
        self.current.is_some() || self.picker.is_some()
    }

    pub fn has_prompt_kind(&self, f: impl Fn(&PKind) -> bool) -> bool {
        self.current.as_ref().is_some_and(|p| f(&p.kind))
            || self.queue.iter().any(|i| matches!(i, Item::Prompt(p) if f(&p.kind)))
    }

    pub fn push(&mut self, p: Prompt) {
        self.queue.push_back(Item::Prompt(p));
        self.show_next();
    }

    pub fn pick(&mut self, req: u32, mode: PickMode) {
        self.queue.push_back(Item::Pick { req, mode });
        self.show_next();
    }

    /// Withdraws prompts matching `f` (e.g. "not responding" once the app answers).
    pub fn withdraw(&mut self, f: impl Fn(&PKind) -> bool) {
        self.queue.retain(|i| !matches!(i, Item::Prompt(p) if f(&p.kind)));
        if self.current.as_ref().is_some_and(|p| f(&p.kind)) {
            self.current = None;
            self.clear();
            self.show_next();
        }
    }

    fn id(&mut self, p: &mut Vec<u8>, k: u8) -> u32 {
        let id = self.next;
        self.next += 1;
        p.op_create(id, k);
        id
    }

    fn clear(&mut self) {
        self.ui.reset();
        self.buttons.clear();
        self.changed = true;
    }

    fn apply(&mut self, p: &[u8]) {
        if let Err(e) = self.ui.apply_patch(p) {
            eprintln!("qa: overlay patch error at {}: {}", e.offset, e.msg);
        }
    }

    fn root(&mut self, p: &mut Vec<u8>) -> u32 {
        let root = self.id(p, kind::GRID);
        let surface = self.ui.color(0, &["color.surface"]);
        let border = self.ui.color(0, &["color.border"]);
        ctx_c(p, root, "surface.band.bg", surface);
        ctx_c(p, root, "surface.band.border", border);
        ctx_f(p, root, "surface.band.radius", 0.0);
        ctx_c(p, root, "surface.dim.bg", Color::rgba(0, 0, 0, 0x55));
        ctx_f(p, root, "surface.dim.radius", 0.0);
        p.op_root(root);
        root
    }

    fn show_next(&mut self) {
        if self.active() {
            return;
        }
        let Some(item) = self.queue.pop_front() else { return };
        self.clear();
        let mut p = Vec::new();
        match item {
            Item::Prompt(pr) => {
                let root = self.root(&mut p);
                set_f(&mut p, root, prop::grid::GAP, 0.0);
                p.op_set(root, prop::grid::ROWS);
                p.tracks(&[Track::auto(), Track::fr(1.0)]);
                let band = self.id(&mut p, kind::GRID);
                set_s(&mut p, band, prop::VARIANT, "band");
                pad(&mut p, band, [12.0, 16.0, 12.0, 16.0]);
                let mut cols = vec![Track::fr(1.0)];
                cols.extend(pr.buttons.iter().map(|_| Track::auto()));
                p.op_set(band, prop::grid::COLS);
                p.tracks(&cols);
                let msg = self.id(&mut p, kind::GRID);
                set_f(&mut p, msg, prop::grid::GAP, 2.0);
                set_i(&mut p, msg, prop::ALIGN_Y, 2);
                let name = self.id(&mut p, kind::TEXT);
                set_s(&mut p, name, prop::text::TEXT, &self.app);
                set_s(&mut p, name, prop::text::STYLE, "heading");
                let text = self.id(&mut p, kind::TEXT);
                set_s(&mut p, text, prop::text::TEXT, &pr.text);
                p.op_append(msg, name);
                p.op_append(msg, text);
                p.op_append(band, msg);
                for (i, b) in pr.buttons.iter().enumerate() {
                    let id = self.id(&mut p, kind::BUTTON);
                    set_s(&mut p, id, prop::button::LABEL, b);
                    set_i(&mut p, id, prop::ALIGN_Y, 2);
                    if i == 0 {
                        set_s(&mut p, id, prop::VARIANT, "primary");
                    }
                    p.op_append(band, id);
                    self.buttons.push(id);
                }
                let dim = self.id(&mut p, kind::GRID);
                set_s(&mut p, dim, prop::VARIANT, "dim");
                p.op_append(root, band);
                p.op_append(root, dim);
                self.current = Some(pr);
            }
            Item::Pick { req, mode } => {
                let root = self.root(&mut p);
                set_s(&mut p, root, prop::VARIANT, "dim");
                pad(&mut p, root, [24.0; 4]);
                let card = self.id(&mut p, kind::GRID);
                set_s(&mut p, card, prop::VARIANT, "card");
                ctx_c(&mut p, card, "surface.card.bg", self.ui.color(0, &["color.bg"]));
                pad(&mut p, card, [16.0; 4]);
                set_f(&mut p, card, prop::grid::GAP, 10.0);
                p.op_set(card, prop::grid::ROWS);
                p.tracks(&[Track::auto(), Track::auto(), Track::fr(1.0), Track::auto(), Track::auto()]);
                set_f(&mut p, card, prop::MIN_W, 360.0);
                set_f(&mut p, card, prop::MAX_W, 680.0);
                set_f(&mut p, card, prop::MIN_H, 380.0);
                set_f(&mut p, card, prop::MAX_H, 560.0);
                set_i(&mut p, card, prop::ALIGN_X, 2);
                set_i(&mut p, card, prop::ALIGN_Y, 2);
                let title = self.id(&mut p, kind::TEXT);
                let (t, okl) = match &mode {
                    PickMode::Open { .. } => ("Open a file", "Open"),
                    PickMode::Save { .. } => ("Save as", "Save"),
                    PickMode::Folder { .. } => ("Choose a folder", "Choose"),
                };
                set_s(&mut p, title, prop::text::TEXT, &format!("{t} for {}", self.app));
                set_s(&mut p, title, prop::text::STYLE, "heading");
                let pathrow = self.id(&mut p, kind::GRID);
                p.op_set(pathrow, prop::grid::COLS);
                p.tracks(&[Track::auto(), Track::fr(1.0)]);
                let up = self.id(&mut p, kind::BUTTON);
                set_s(&mut p, up, prop::button::LABEL, "Up");
                let path_label = self.id(&mut p, kind::TEXT);
                set_i(&mut p, path_label, prop::text::WRAP, 1);
                set_i(&mut p, path_label, prop::ALIGN_Y, 2);
                set_s(&mut p, path_label, prop::VARIANT, "muted");
                p.op_append(pathrow, up);
                p.op_append(pathrow, path_label);
                let scroll = self.id(&mut p, kind::SCROLL);
                let list = self.id(&mut p, kind::GRID);
                set_f(&mut p, list, prop::grid::GAP, 0.0);
                p.op_append(scroll, list);
                let extra = self.id(&mut p, kind::GRID);
                let (mut name_input, mut remember, mut name) = (0, 0, String::new());
                match &mode {
                    PickMode::Save { name: n } => {
                        name_input = self.id(&mut p, kind::INPUT);
                        set_s(&mut p, name_input, prop::input::PLACEHOLDER, "File name");
                        set_s(&mut p, name_input, prop::input::VALUE, n);
                        p.op_append(extra, name_input);
                        name = n.clone();
                    }
                    PickMode::Folder { write } => {
                        remember = self.id(&mut p, kind::CHECKBOX);
                        let l = if *write { "Remember: let it read and change this folder next time" } else { "Remember: let it read this folder next time" };
                        set_s(&mut p, remember, prop::checkbox::LABEL, l);
                        p.op_append(extra, remember);
                    }
                    _ => {}
                }
                let btns = self.id(&mut p, kind::GRID);
                p.op_set(btns, prop::grid::COLS);
                p.tracks(&[Track::fr(1.0), Track::auto(), Track::auto()]);
                let spacer = self.id(&mut p, kind::TEXT);
                let cancel = self.id(&mut p, kind::BUTTON);
                set_s(&mut p, cancel, prop::button::LABEL, "Cancel");
                let ok = self.id(&mut p, kind::BUTTON);
                set_s(&mut p, ok, prop::button::LABEL, okl);
                set_s(&mut p, ok, prop::VARIANT, "primary");
                for c in [spacer, cancel, ok] {
                    p.op_append(btns, c);
                }
                for c in [title, pathrow, scroll, extra, btns] {
                    p.op_append(card, c);
                }
                p.op_append(root, card);
                if name_input != 0 {
                    p.op_focus(name_input);
                }
                let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
                self.picker = Some(Picker {
                    req,
                    mode,
                    dir,
                    list,
                    path_label,
                    up,
                    cancel,
                    ok,
                    name_input,
                    remember,
                    rows: HashMap::new(),
                    selected: None,
                    last_click: (0, std::time::Instant::now()),
                    name,
                });
            }
        }
        self.apply(&p);
        if self.picker.is_some() {
            self.fill_list();
        }
        self.changed = true;
    }

    fn fill_list(&mut self) {
        let Some(pk) = self.picker.as_mut() else { return };
        let mut p = Vec::new();
        for id in pk.rows.keys() {
            p.op_destroy(*id);
        }
        pk.rows.clear();
        pk.selected = None;
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(&pk.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().into_owned();
                        if n.starts_with('.') {
                            return None;
                        }
                        let dir = e.path().is_dir();
                        Some((n, dir))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.retain(|(n, d)| {
            *d || match &pk.mode {
                PickMode::Open { patterns } => matches(n, patterns),
                PickMode::Save { .. } => true,
                PickMode::Folder { .. } => false,
            }
        });
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        entries.truncate(2000);
        let (list, label) = (pk.list, pk.path_label);
        let dir_s = pk.dir.display().to_string();
        let mut rows = Vec::new();
        for (n, d) in entries {
            let id = self.next;
            self.next += 1;
            p.op_create(id, kind::GRID);
            set_i(&mut p, id, prop::grid::CLICKABLE, 1);
            pad(&mut p, id, [5.0, 8.0, 5.0, 8.0]);
            let t = self.next;
            self.next += 1;
            p.op_create(t, kind::TEXT);
            set_s(&mut p, t, prop::text::TEXT, &if d { format!("{n}/") } else { n.clone() });
            set_i(&mut p, t, prop::text::WRAP, 1);
            p.op_append(id, t);
            p.op_append(list, id);
            rows.push((id, (n, d)));
        }
        set_s(&mut p, label, prop::text::TEXT, &dir_s);
        let pk = self.picker.as_mut().unwrap();
        pk.rows.extend(rows);
        self.apply(&p);
        self.changed = true;
    }

    fn finish_pick(&mut self, path: Option<PathBuf>) -> Done {
        let pk = self.picker.take().unwrap();
        let remember = pk.remember != 0 && self.ui.node(pk.remember).is_some_and(|n| n.i(prop::checkbox::CHECKED, 0) != 0);
        self.clear();
        let d = Done::Picked { req: pk.req, mode: pk.mode, path, remember };
        self.show_next();
        d
    }

    fn answer(&mut self, i: usize) -> Done {
        let pr = self.current.take().unwrap();
        self.clear();
        let d = Done::Prompt(pr, i);
        self.show_next();
        d
    }

    /// Handles the overlay's own UI events. Returns finished prompts and picks.
    pub fn events(&mut self, evs: Vec<UiEvent>) -> Vec<Done> {
        let mut out = Vec::new();
        for e in evs {
            if self.current.is_some() {
                match e {
                    UiEvent::Click(id) => {
                        if let Some(i) = self.buttons.iter().position(|&b| b == id) {
                            out.push(self.answer(i));
                        }
                    }
                    UiEvent::Key { sym: 0xff1b, .. } => {
                        let c = self.current.as_ref().unwrap().cancel;
                        out.push(self.answer(c));
                    }
                    _ => {}
                }
                continue;
            }
            let Some(pk) = self.picker.as_mut() else { continue };
            match e {
                UiEvent::Key { sym: 0xff1b, .. } => out.push(self.finish_pick(None)),
                UiEvent::Click(id) if id == pk.cancel => out.push(self.finish_pick(None)),
                UiEvent::Click(id) if id == pk.up => {
                    if let Some(parent) = pk.dir.parent() {
                        pk.dir = parent.to_path_buf();
                        self.fill_list();
                    }
                }
                UiEvent::Change(id, v) if id == pk.name_input => pk.name = v,
                UiEvent::Click(id) if id == pk.ok => {
                    if let Some(r) = self.ok_result() {
                        out.push(self.finish_pick(Some(r)));
                    }
                }
                UiEvent::Submit(id, _) if id == pk.name_input => {
                    if let Some(r) = self.ok_result() {
                        out.push(self.finish_pick(Some(r)));
                    }
                }
                UiEvent::Click(id) => {
                    let Some((name, dir)) = pk.rows.get(&id).cloned() else { continue };
                    if dir {
                        pk.dir = pk.dir.join(&name);
                        self.fill_list();
                        continue;
                    }
                    let now = std::time::Instant::now();
                    let double = pk.last_click.0 == id && now.duration_since(pk.last_click.1).as_millis() < 450;
                    pk.last_click = (id, now);
                    let mut p = Vec::new();
                    for (rid, (n, _)) in &pk.rows {
                        if Some(n) == pk.selected.as_ref() {
                            set_i(&mut p, *rid, prop::grid::SELECTED, 0);
                        }
                    }
                    set_i(&mut p, id, prop::grid::SELECTED, 1);
                    pk.selected = Some(name.clone());
                    if pk.name_input != 0 {
                        set_s(&mut p, pk.name_input, prop::input::VALUE, &name);
                        pk.name = name.clone();
                    }
                    self.apply(&p);
                    if double {
                        if let Some(r) = self.ok_result() {
                            out.push(self.finish_pick(Some(r)));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn ok_result(&self) -> Option<PathBuf> {
        let pk = self.picker.as_ref()?;
        match &pk.mode {
            PickMode::Open { .. } => pk.selected.as_ref().map(|n| pk.dir.join(n)),
            PickMode::Save { .. } => {
                let n = pk.name.trim();
                (!n.is_empty() && !n.contains('/')).then(|| pk.dir.join(n))
            }
            PickMode::Folder { .. } => Some(pk.dir.clone()),
        }
    }
}
