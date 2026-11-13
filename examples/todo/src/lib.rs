//! A todo list saved in the app's folder. Shows rows with tags, filtering
//! with `hide`, editing a row in place, and saving as JSON.
#![no_std]
use qa_sdk::*;

const FILE: &str = "todos.json";

struct Item {
    id: i32,
    text: String,
    done: bool,
    row: Node,
    /// The row's checkbox, or the input that stands in for it while editing.
    cell: Node,
    editing: bool,
}

struct Todo {
    items: Vec<Item>,
    next_id: i32,
    filter: i32,
    input: Node,
    add: Node,
    tabs: Node,
    list: Node,
    empty: Node,
    left: Node,
    clear: Node,
}

/// A row's checkbox. It carries the item's id as its tag.
fn checkbox(ui: &mut Ui, id: i32, text: &str, done: bool) -> Node {
    ui.checkbox(text).checked(done).variant(if done { "muted" } else { "" }).tag(id).align_y(CENTER).detached()
}

impl Todo {
    fn push(&mut self, ui: &mut Ui, text: &str, done: bool) {
        let id = self.next_id;
        self.next_id += 1;
        let row = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).add_to(self.list);
        let cell = checkbox(ui, id, text, done);
        ui.append(row, cell);
        ui.button("×").variant("flat").tooltip("Delete").tag(id).align_y(START).add_to(row);
        self.items.push(Item { id, text: String::from(text), done, row, cell, editing: false });
    }

    fn add(&mut self, ui: &mut Ui) {
        let text = String::from(ui.value(self.input).trim());
        if !text.is_empty() {
            self.push(ui, &text, false);
            self.changed(ui);
        }
        ui.set_value(self.input, "");
        ui.focus(self.input);
    }

    /// Swaps a row's checkbox for an input holding its text.
    fn edit(&mut self, ui: &mut Ui, i: usize) {
        let it = &mut self.items[i];
        let input = ui.input().value(&it.text).tag(it.id).detached();
        ui.replace(it.cell, input);
        ui.focus(input);
        it.cell = input;
        it.editing = true;
    }

    /// Swaps the checkbox back, keeping the new text unless it is empty.
    fn end_edit(&mut self, ui: &mut Ui, i: usize, keep: bool) {
        let it = &mut self.items[i];
        let text = ui.value(it.cell).trim();
        if keep && !text.is_empty() {
            it.text = String::from(text);
        }
        let cell = checkbox(ui, it.id, &it.text, it.done);
        ui.replace(it.cell, cell);
        it.cell = cell;
        it.editing = false;
        self.save();
    }

    /// Applies the filter, updates the footer and saves.
    fn changed(&mut self, ui: &mut Ui) {
        self.refresh(ui);
        self.save();
    }

    fn refresh(&mut self, ui: &mut Ui) {
        let (mut shown, mut left) = (0, 0);
        for it in &self.items {
            let show = match self.filter {
                1 => !it.done,
                2 => it.done,
                _ => true,
            };
            ui.hide(it.row, !show);
            shown += show as i32;
            left += !it.done as i32;
        }
        ui.hide(self.empty, shown > 0);
        ui.set_text(self.empty, if self.items.is_empty() { "Nothing to do yet." } else { "Nothing here." });
        ui.set_text(self.left, &cat(&[itoa(left as i64).as_str(), if left == 1 { " item left" } else { " items left" }]));
        ui.set_disabled(self.clear, left as usize == self.items.len());
    }

    fn save(&self) {
        let mut w = json::Writer::new();
        w.list();
        for it in &self.items {
            w.map().key("text").str(&it.text).key("done").bool(it.done).end();
        }
        let doc = json::Doc::from(w);
        if fs::write_atomic(fs::APP_DIR, FILE, json::stringify(doc.root()).as_bytes()).is_err() {
            log(2, "todo: could not save");
        }
    }
}

impl App for Todo {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(10.0).rows(&[auto(), auto(), auto(), fr(1.0), auto()]).root();
        ui.text("Todo").style("title").add_to(root);
        let top = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).add_to(root);
        let input = ui.input().placeholder("What needs doing?").add_to(top);
        let add = ui.button("Add").variant("primary").add_to(top);
        let tabs = ui.tabs("All\nActive\nDone").add_to(root);
        let scroll = ui.scroll().add_to(root);
        let list = ui.grid().gap(2.0).add_to(scroll);
        let empty = ui.text("").variant("muted").add_to(list);
        let foot = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).add_to(root);
        let left = ui.text("").variant("muted").align_y(CENTER).add_to(foot);
        let clear = ui.button("Clear done").add_to(foot);

        let mut app = Todo { items: Vec::new(), next_id: 1, filter: 0, input, add, tabs, list, empty, left, clear };
        if let Some(doc) = fs::read(fs::APP_DIR, FILE).ok().and_then(|b| json::parse(&b)) {
            for v in doc.root().iter() {
                app.push(ui, v.get("text").str(), v.get("done").bool());
            }
        }
        app.refresh(ui);
        ui.focus(input);
        app
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        // Rows find their item by tag; an input being edited by node.
        let tag = ui.tag();
        let by_tag = self.items.iter().position(|it| it.id == tag);
        let editing = |n: Node| self.items.iter().position(|it| it.editing && it.cell == n);
        match ev {
            Event::Submit(n, _) if n == self.input => self.add(ui),
            Event::Click(n) if n == self.add => self.add(ui),
            Event::Click(n) if n == self.clear => {
                for it in self.items.iter().filter(|it| it.done) {
                    ui.destroy(it.row);
                }
                self.items.retain(|it| !it.done);
                self.changed(ui);
            }
            // The only other buttons are the rows' delete buttons.
            Event::Click(_) => {
                if let Some(i) = by_tag {
                    ui.destroy(self.items.remove(i).row);
                    self.changed(ui);
                }
            }
            Event::Toggle(n, on) => {
                if let Some(i) = by_tag {
                    self.items[i].done = on;
                    ui.variant(n, if on { "muted" } else { "" });
                    self.changed(ui);
                }
            }
            Event::DoubleClick(n) => {
                if let Some(i) = by_tag.filter(|&i| self.items[i].cell == n) {
                    self.edit(ui, i);
                }
            }
            Event::Submit(n, _) | Event::FocusOut(n) => {
                if let Some(i) = editing(n) {
                    self.end_edit(ui, i, true);
                }
            }
            Event::Key { node, sym: key::ESCAPE, .. } => {
                if let Some(i) = editing(node) {
                    self.end_edit(ui, i, false);
                }
            }
            Event::Select(n, i) if n == self.tabs => {
                self.filter = i;
                self.refresh(ui);
            }
            _ => {}
        }
    }
}

app!(Todo);
