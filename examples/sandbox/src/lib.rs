//! Exercises the sandbox: files, prompts, pickers and limits.
#![no_std]
use qa_sdk::*;

struct Sandbox {
    log: Node,
    lines: Vec<String>,
    spin: Node,
    hog: Node,
    escape: Node,
    file: Node,
    open: Node,
    folder: Node,
    net: Node,
    data: Node,
    hogged: Vec<Vec<u8>>,
}

fn code(n: i32) -> &'static str {
    match n {
        err::DENIED => "DENIED",
        err::NOT_FOUND => "NOT_FOUND",
        err::EXISTS => "EXISTS",
        err::INVALID => "INVALID",
        err::IO => "IO",
        err::LIMIT => "LIMIT",
        err::BAD_HANDLE => "BAD_HANDLE",
        _ => "ok",
    }
}

impl Sandbox {
    fn say(&mut self, ui: &mut Ui, s: String) {
        self.lines.push(s);
        if self.lines.len() > 12 {
            self.lines.remove(0);
        }
        let mut all = String::new();
        for l in &self.lines {
            all.push_str(l);
            all.push('\n');
        }
        ui.set_text(self.log, all.trim_end());
    }
}

impl App for Sandbox {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().pad(16.0).gap(12.0).rows(&[auto(), auto(), fr(1.0)]).root();
        ui.text("Sandbox").style("title").add_to(root);
        let row = ui.grid().cols(&[auto(), auto(), auto(), auto()]).gap(8.0).add_to(root);
        let spin = ui.button("Spin forever").variant("danger").add_to(row);
        let hog = ui.button("Hog memory").add_to(row);
        let escape = ui.button("Open ../secret").add_to(row);
        let file = ui.button("Write + read").add_to(row);
        let open = ui.button("Open file…").add_to(row);
        let folder = ui.button("Pick folder…").add_to(row);
        let net = ui.button("Ask internet").add_to(row);
        let data = ui.button("JSON + secrets").tag(7).add_to(row);
        let scroll = ui.scroll().add_to(root);
        let log = ui.text("").style("mono").add_to(scroll);
        Sandbox { log, lines: Vec::new(), spin, hog, escape, file, open, folder, net, data, hogged: Vec::new() }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Click(id) if id == self.spin => {
                let mut x: u64 = 1;
                loop {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    if x == 0 {
                        break;
                    }
                }
            }
            Event::Click(id) if id == self.hog => {
                for _ in 0..30 {
                    self.hogged.push(vec![1u8; 10 << 20]);
                }
                let n = self.hogged.len() as i64 * 10;
                self.say(ui, cat(&["holding ", &itoa(n), " MB"]));
            }
            Event::Click(id) if id == self.escape => {
                let r = fs::open(fs::APP_DIR, "../secret", fs::READ).err().unwrap_or(0);
                self.say(ui, cat(&["open ../secret -> ", code(r)]));
                let r = fs::open(fs::APP_DIR, "/etc/passwd", fs::READ).err().unwrap_or(0);
                self.say(ui, cat(&["open /etc/passwd -> ", code(r)]));
            }
            Event::Click(id) if id == self.data => {
                let tag = ui.tag();
                let doc = json::parse(br#"{"a":[1,{"b":"x\u00e9"}],"n":-2.5}"#);
                let v = doc.as_ref().map(|d| d.root()).unwrap_or_default();
                let bad = json::parse(b"{nope").is_none();
                self.say(ui, cat(&["json: b=", v.path("a.1.b").str(), " n=", &ftoa(v.get("n").num() as f32, 1), " len=", &itoa(v.get("a").len() as i64), " bad=", if bad { "rejected" } else { "accepted" }, " tag=", &itoa(tag as i64)]));
                let w = secret::set("token", "s3cret\tvalue");
                let back = secret::get("token").unwrap_or_default();
                secret::set("token", "");
                let gone = secret::get("token").is_none();
                self.say(ui, cat(&["secret: set -> ", code(w), ", read back: ", &back, ", removed: ", if gone { "yes" } else { "no" }]));
                self.say(ui, cat(&["utc offset: ", &itoa(env().utc_offset as i64)]));
            }
            Event::Click(id) if id == self.file => {
                let w = fs::write(fs::APP_DIR, "hello.txt", b"hello from the sandbox").err().unwrap_or(0);
                let r = fs::read(fs::APP_DIR, "hello.txt").unwrap_or_default();
                let s = String::from_utf8(r).unwrap_or_default();
                self.say(ui, cat(&["write -> ", code(w), ", read back: ", &s]));
            }
            Event::Click(id) if id == self.open => {
                request::open_file("*.md;*.txt;*.toml");
            }
            Event::Click(id) if id == self.folder => {
                request::pick_folder(false);
            }
            Event::Click(id) if id == self.net => {
                request::internet();
            }
            Event::RequestDone { req, result, info } => {
                let mut extra = String::new();
                if result > 1 {
                    // A file handle: read its first bytes; a folder: list it.
                    if let Ok(b) = fs::read(result, "") {
                        extra = cat(&[" (", &itoa(b.len() as i64), " bytes)"]);
                    } else if let Ok(l) = fs::list(result, "") {
                        extra = cat(&[" (", &itoa(l.len() as i64), " entries)"]);
                    }
                }
                self.say(ui, cat(&["request ", &itoa(req as i64), " -> ", &itoa(result as i64), " ", info, &extra]));
            }
            _ => {}
        }
    }
}

app!(Sandbox);
