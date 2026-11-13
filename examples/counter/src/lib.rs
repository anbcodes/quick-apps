#![no_std]
use qa_sdk::*;

struct Counter {
    n: i32,
    label: Node,
    plus: Node,
    reset: Node,
}

impl App for Counter {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(12.0).root();
        let label = ui.text("0").style("title").align_x(CENTER).add_to(root);
        let row = ui.grid().cols(&[fr(1.0), fr(1.0)]).gap(8.0).add_to(root);
        let plus = ui.button("+1").variant("primary").add_to(row);
        let reset = ui.button("Reset").add_to(row);
        Counter { n: 0, label, plus, reset }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Click(id) if id == self.plus => self.n += 1,
            Event::Click(id) if id == self.reset => self.n = 0,
            _ => return,
        }
        ui.set_text(self.label, itoa(self.n as i64).as_str());
    }
}

app!(Counter);
