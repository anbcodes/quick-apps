//! Fetches JSON over HTTPS and shows it.
#![no_std]
use qa_sdk::*;

struct Fetch {
    url: Node,
    go: Node,
    status: Node,
    body: Node,
    busy: Node,
    req: u32,
    text: String,
}

impl App for Fetch {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(10.0).rows(&[auto(), auto(), auto(), auto(), fr(1.0)]).root();
        ui.text("Fetch").style("title").add_to(root);
        let bar = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).add_to(root);
        let start = "https://api.github.com/repos/rust-lang/rust";
        let url = ui.input().value(start).add_to(bar);
        let go = ui.button("Fetch").variant("primary").add_to(bar);
        let busy = ui.progress(-1.0).hidden(true).add_to(root);
        let status = ui.text("Press Fetch").variant("muted").add_to(root);
        let scroll = ui.scroll().add_to(root);
        let body = ui.text("").style("mono").add_to(scroll);
        Fetch { url, go, status, body, busy, req: 0, text: String::from(start) }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Change(id, v) if id == self.url => self.text = String::from(v),
            Event::Click(id) if id == self.go => self.fetch(ui),
            Event::Submit(id, _) if id == self.url => self.fetch(ui),
            Event::Http { req, status, body, .. } if req == self.req => {
                ui.hide(self.busy, true);
                if status == 0 {
                    ui.set_text(self.status, &cat(&["Failed: ", core::str::from_utf8(body).unwrap_or("?")]));
                    ui.set_text(self.body, "");
                    return;
                }
                ui.set_text(self.status, &cat(&["HTTP ", &itoa(status as i64), " · ", &itoa(body.len() as i64), " bytes"]));
                let s = core::str::from_utf8(body).unwrap_or("(binary)");
                let cut = s.char_indices().nth(4000).map(|(i, _)| i).unwrap_or(s.len());
                ui.set_text(self.body, &s[..cut]);
            }
            _ => {}
        }
    }
}

impl Fetch {
    fn fetch(&mut self, ui: &mut Ui) {
        self.req = net::get(&self.text);
        ui.hide(self.busy, false);
        ui.set_text(self.status, "Loading…");
    }
}

app!(Fetch);
