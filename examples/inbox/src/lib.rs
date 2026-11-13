//! Receives messages from other apps, and can send one.
#![no_std]
use qa_sdk::*;

struct Inbox {
    list: Node,
    to: Node,
    send: Node,
    target: String,
    result: Node,
}

impl App for Inbox {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(10.0).rows(&[auto(), auto(), auto(), fr(1.0)]).root();
        ui.text("Inbox").style("title").add_to(root);
        let bar = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).add_to(root);
        let to = ui.input().placeholder("Send to (manifest URL or app id)").add_to(bar);
        let send = ui.button("Send hello").add_to(bar);
        let result = ui.text("").variant("muted").add_to(root);
        let scroll = ui.scroll().stick_end(true).add_to(root);
        let list = ui.grid().gap(6.0).add_to(scroll);
        Inbox { list, to, send, target: String::new(), result }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Change(id, v) if id == self.to => self.target = String::from(v),
            Event::Click(id) if id == self.send => {
                let r = msg::send(&self.target, b"hello from inbox");
                let s = match r {
                    0 => "delivered",
                    1 => "queued in its inbox",
                    err::DENIED => "denied",
                    err::NOT_FOUND => "no such app",
                    _ => "failed",
                };
                ui.set_text(self.result, s);
            }
            Event::Message { from, data } => {
                let row = ui.grid().variant("card").pad(8.0).gap(2.0).add_to(self.list);
                ui.text(from).style("caption").variant("muted").add_to(row);
                ui.text(core::str::from_utf8(data).unwrap_or("(binary)")).add_to(row);
                ui.scroll_to(row);
            }
            Event::Activate => ui.set_text(self.result, "activated"),
            _ => {}
        }
    }
}

app!(Inbox);
