//! Every built-in component on two tabs.
#![no_std]
use qa_sdk::*;

struct Widgets {
    tabs: Node,
    pages: [Node; 2],
    select: Node,
    chosen: Node,
    bar: Node,
    more: Node,
    value: f32,
}

const OPTIONS: &str = "Apples\nBananas\nCherries\nDates";

impl App for Widgets {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(12.0).rows(&[auto(), auto(), fr(1.0)]).root();
        ui.text("Widgets").style("title").add_to(root);
        let tabs = ui.tabs("Controls\nMedia").add_to(root);
        let stack = ui.grid().add_to(root);

        let p1 = ui.grid().gap(10.0).at(0, 0).add_to(stack);
        let row = ui.grid().cols(&[px(200.0), fr(1.0)]).gap(10.0).add_to(p1);
        let select = ui.select(OPTIONS).placeholder("Pick a fruit").add_to(row);
        let chosen = ui.text("Nothing picked").variant("muted").align_y(CENTER).add_to(row);
        ui.checkbox("A checkbox").add_to(p1);
        ui.checkbox("Disabled and checked").checked(true).disabled(true).add_to(p1);
        let prow = ui.grid().cols(&[fr(1.0), auto()]).gap(10.0).add_to(p1);
        let bar = ui.progress(0.3).align_y(CENTER).add_to(prow);
        let more = ui.button("More").add_to(prow);
        ui.progress(-1.0).add_to(p1);
        ui.divider().add_to(p1);
        ui.input().multiline(true).lines(3).placeholder("A multiline input").add_to(p1);
        let brow = ui.grid().cols(&[auto(), auto(), auto(), fr(1.0)]).gap(8.0).add_to(p1);
        ui.button("Primary").variant("primary").add_to(brow);
        ui.button("Danger").variant("danger").add_to(brow);
        ui.button("Disabled").disabled(true).add_to(brow);

        let p2 = ui.grid().cols(&[fr(1.0), fr(1.0)]).gap(12.0).at(0, 0).hidden(true).add_to(stack);
        ui.image("assets/logo.png").min_h(120.0).add_to(p2);
        let canvas = ui.canvas(32, 32).fit(0).min_h(120.0).add_to(p2);
        let mut px = Vec::with_capacity(32 * 32 * 4);
        for y in 0..32u32 {
            for x in 0..32u32 {
                let on = (x / 4 + y / 4) % 2 == 0;
                px.extend_from_slice(&[if on { 207 } else { 255 }, (x * 8) as u8, (y * 8) as u8, 255]);
            }
        }
        ui.canvas_put(canvas, 0, 0, 32, 32, &px);
        Widgets { tabs, pages: [p1, p2], select, chosen, bar, more, value: 0.3 }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Select(id, i) if id == self.tabs => {
                ui.hide(self.pages[0], i != 0);
                ui.hide(self.pages[1], i != 1);
            }
            Event::Select(id, i) if id == self.select => {
                let name = OPTIONS.split('\n').nth(i as usize).unwrap_or("?");
                ui.set_text(self.chosen, &cat(&["You picked ", name]));
            }
            Event::Click(id) if id == self.more => {
                self.value = if self.value >= 1.0 { 0.0 } else { self.value + 0.1 };
                ui.set_f(self.bar, prop::progress::VALUE, self.value);
            }
            _ => {}
        }
    }
}

app!(Widgets);
