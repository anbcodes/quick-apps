//! `qa demo-todo`: a native test harness that drives the UI only through
//! patches, the same bytes a wasm app would send (milestone M1).

use crate::gfx::NoPixmaps;
use crate::shell::Driver;
use crate::ui::{Ui, UiEvent};
use qa_proto::{kind, prop, Sink, Track, Value};

struct Item {
    row: u32,
    check: u32,
    label: u32,
    del: u32,
    done: bool,
}

pub struct Todo {
    next: u32,
    input: u32,
    add: u32,
    list: u32,
    count: u32,
    clear: u32,
    items: Vec<Item>,
    text: String,
}

fn str_prop(p: &mut Vec<u8>, id: u32, pr: u16, s: &str) {
    p.op_set(id, pr);
    p.value(&Value::Str(s));
}
fn int_prop(p: &mut Vec<u8>, id: u32, pr: u16, v: i32) {
    p.op_set(id, pr);
    p.value(&Value::I32(v));
}
fn f_prop(p: &mut Vec<u8>, id: u32, pr: u16, v: f32) {
    p.op_set(id, pr);
    p.value(&Value::F32(v));
}

impl Todo {
    pub fn new() -> Todo {
        Todo { next: 1, input: 0, add: 0, list: 0, count: 0, clear: 0, items: Vec::new(), text: String::new() }
    }

    fn node(&mut self, p: &mut Vec<u8>, k: u8) -> u32 {
        let id = self.next;
        self.next += 1;
        p.op_create(id, k);
        id
    }

    fn add_item(&mut self, p: &mut Vec<u8>, text: &str) {
        let row = self.node(p, kind::GRID);
        p.op_set(row, prop::grid::COLS);
        p.tracks(&[Track::auto(), Track::fr(1.0), Track::auto()]);
        str_prop(p, row, prop::VARIANT, "card");
        p.op_set(row, prop::PAD);
        p.value(&Value::Edges([6.0, 8.0, 6.0, 10.0]));
        int_prop(p, row, prop::ALIGN_Y, 0);
        let check = self.node(p, kind::CHECKBOX);
        int_prop(p, check, prop::ALIGN_Y, 2);
        let label = self.node(p, kind::TEXT);
        str_prop(p, label, prop::text::TEXT, text);
        int_prop(p, label, prop::ALIGN_Y, 2);
        let del = self.node(p, kind::BUTTON);
        str_prop(p, del, prop::button::LABEL, "Delete");
        str_prop(p, del, prop::VARIANT, "flat");
        int_prop(p, del, prop::PRIORITY, -1);
        p.op_append(row, check);
        p.op_append(row, label);
        p.op_append(row, del);
        p.op_append(self.list, row);
        p.op_scroll_to(row);
        self.items.push(Item { row, check, label, del, done: false });
    }

    fn update_count(&self, p: &mut Vec<u8>) {
        let left = self.items.iter().filter(|i| !i.done).count();
        let s = match left {
            1 => "1 item left".to_string(),
            n => format!("{n} items left"),
        };
        str_prop(p, self.count, prop::text::TEXT, &s);
    }
}

impl Driver for Todo {
    fn start(&mut self, ui: &mut Ui) {
        let mut p = Vec::new();
        let root = self.node(&mut p, kind::GRID);
        p.op_set(root, prop::PAD);
        p.value(&Value::Edges([16.0; 4]));
        f_prop(&mut p, root, prop::grid::GAP, 12.0);
        p.op_set(root, prop::grid::ROWS);
        p.tracks(&[Track::auto(), Track::auto(), Track::fr(1.0), Track::auto()]);
        let title = self.node(&mut p, kind::TEXT);
        str_prop(&mut p, title, prop::text::TEXT, "Todo");
        str_prop(&mut p, title, prop::text::STYLE, "title");
        let bar = self.node(&mut p, kind::GRID);
        p.op_set(bar, prop::grid::COLS);
        p.tracks(&[Track::fr(1.0), Track::auto()]);
        self.input = self.node(&mut p, kind::INPUT);
        str_prop(&mut p, self.input, prop::input::PLACEHOLDER, "What needs doing?");
        self.add = self.node(&mut p, kind::BUTTON);
        str_prop(&mut p, self.add, prop::button::LABEL, "Add");
        str_prop(&mut p, self.add, prop::VARIANT, "primary");
        p.op_append(bar, self.input);
        p.op_append(bar, self.add);
        let scroll = self.node(&mut p, kind::SCROLL);
        self.list = self.node(&mut p, kind::GRID);
        f_prop(&mut p, self.list, prop::grid::GAP, 6.0);
        p.op_append(scroll, self.list);
        let footer = self.node(&mut p, kind::GRID);
        p.op_set(footer, prop::grid::COLS);
        p.tracks(&[Track::fr(1.0), Track::auto()]);
        self.count = self.node(&mut p, kind::TEXT);
        str_prop(&mut p, self.count, prop::VARIANT, "muted");
        int_prop(&mut p, self.count, prop::ALIGN_Y, 2);
        self.clear = self.node(&mut p, kind::BUTTON);
        str_prop(&mut p, self.clear, prop::button::LABEL, "Clear done");
        p.op_append(footer, self.count);
        p.op_append(footer, self.clear);
        for c in [title, bar, scroll, footer] {
            p.op_append(root, c);
        }
        p.op_root(root);
        for t in ["Water the plants", "Write the M1 harness", "Measure damage rects while typing"] {
            self.add_item(&mut p, t);
        }
        self.update_count(&mut p);
        p.op_focus(self.input);
        if let Err(e) = ui.apply_patch(&p) {
            eprintln!("todo: patch error at {}: {}", e.offset, e.msg);
        }
    }

    fn events(&mut self, ui: &mut Ui, evs: Vec<UiEvent>) {
        let mut p = Vec::new();
        for e in evs {
            match e {
                UiEvent::Change(id, v) if id == self.input => self.text = v,
                UiEvent::Submit(id, _) if id == self.input => self.submit(&mut p),
                UiEvent::Click(id) if id == self.add => self.submit(&mut p),
                UiEvent::Click(id) if id == self.clear => {
                    for it in self.items.iter().filter(|i| i.done) {
                        p.op_destroy(it.row);
                    }
                    self.items.retain(|i| !i.done);
                    self.update_count(&mut p);
                }
                UiEvent::Click(id) => {
                    if let Some(i) = self.items.iter().position(|it| it.del == id) {
                        p.op_destroy(self.items[i].row);
                        self.items.remove(i);
                        self.update_count(&mut p);
                    }
                }
                UiEvent::Toggle(id, on) => {
                    if let Some(it) = self.items.iter_mut().find(|it| it.check == id) {
                        it.done = on;
                        str_prop(&mut p, it.label, prop::VARIANT, if on { "muted" } else { "" });
                    }
                    self.update_count(&mut p);
                }
                _ => {}
            }
        }
        if !p.is_empty() {
            if let Err(e) = ui.apply_patch(&p) {
                eprintln!("todo: patch error at {}: {}", e.offset, e.msg);
            }
        }
    }

    fn pixmaps(&self) -> &dyn crate::gfx::PixmapSource {
        &NoPixmaps
    }
}

impl Todo {
    fn submit(&mut self, p: &mut Vec<u8>) {
        let t = self.text.trim().to_string();
        if t.is_empty() {
            return;
        }
        self.add_item(p, &t);
        self.text.clear();
        str_prop(p, self.input, prop::input::VALUE, "");
        self.update_count(p);
    }
}
