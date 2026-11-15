//! `qa shot`: run an app headless, perform scripted steps, write a PNG and a
//! dump of the UI tree. Also the `--map` / `--grant` options it shares with
//! `qa dev`.

use crate::gfx::glyph::GlyphCache;
use crate::gfx::raster::Canvas;
use crate::gfx::{render, RectF};
use crate::host::grants::Grant;
use crate::host::WasmDriver;
use crate::pkg::AppCtx;
use crate::shell::Driver;
use crate::ui::{theme::Theme, Ui};
use qa_proto::{kind, prop};
use std::fmt::Write;
use std::path::Path;
use std::time::{Duration, Instant};

pub const USAGE: &str = "usage: qa shot <dir|wasm|app|qa:url> [options] [step...]

Runs the app without a window, performs the steps, then writes a PNG and,
next to it, a .txt dump of the UI tree. A qa: link shows the install prompt
that `qa open` would (clicking Install really installs).

options:
  --out <file.png>    where to write (default shot.png)
  --size <WxH>        window size in logical px (default 800x600)
  --scale <s>         pixel scale (default 1)
  --data <dir>        use this folder as the app's home instead of its own
  --map <from>=<to>   rewrite request URLs that start with <from>
  --grant <grant>     grant for this run only: internet, cpu, folder rw <path>, …

steps (run in order; each waits for the app to settle):
  click:<text>        click the button, link, tab, menu item or node showing <text>
  rclick:<text>       right-click it (opens its context menu)
  hover:<text>        move the pointer over it
  wheel:<px>          scroll under the pointer (negative scrolls up)
  select:<text>       drag-select <text> inside a text node
  type:<text>         type into the focused input
  key:<name>          Enter, Tab, Escape, Backspace, Up, Down, Left, Right,
                      PageUp, PageDown, Home, End, a letter; prefix ctrl+ / shift+
  wait:<ms>           let timers and requests run
  shot:<file.png>     write an extra screenshot now
";

/// Options shared by `qa dev` and `qa shot`.
#[derive(Default)]
pub struct RunOpts {
    pub map: Vec<(String, String)>,
    pub grants: Vec<Grant>,
}

impl RunOpts {
    /// Takes `--map` and `--grant` out of `args`.
    pub fn take(args: &mut Vec<String>) -> Result<RunOpts, String> {
        let mut o = RunOpts::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--map" | "--grant" if i + 1 >= args.len() => return Err(format!("{} needs a value", args[i])),
                "--map" => {
                    let v = args.remove(i + 1);
                    let (a, b) = v.split_once('=').ok_or(format!("--map {v}: expected from=to"))?;
                    o.map.push((a.to_string(), b.to_string()));
                    args.remove(i);
                }
                "--grant" => {
                    let v = args.remove(i + 1);
                    o.grants.push(Grant::parse(&v).ok_or(format!("--grant {v}: not a grant"))?);
                    args.remove(i);
                }
                _ => i += 1,
            }
        }
        Ok(o)
    }

    /// Sets up services and session grants on a driver.
    pub fn apply(&self, d: &mut WasmDriver, listen: bool) {
        let mut net = crate::io::Net::new(&d.ctx, listen);
        net.map = self.map.clone();
        net.headless = !listen;
        d.services = Box::new(net);
        d.grants.session = self.grants.clone();
    }
}

fn opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    if i + 1 >= args.len() {
        return None;
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

/// The module and context for a crate folder, app folder, .wasm or installed app.
fn resolve(arg: &str) -> Result<(Vec<u8>, AppCtx), String> {
    let p = Path::new(arg);
    if p.join("Cargo.toml").exists() {
        let dir = std::fs::canonicalize(p).map_err(|e| format!("{arg}: {e}"))?;
        let wasm = crate::dev::build(&dir)?;
        let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("app").to_string();
        // The same context as `qa dev`, so both see the same data.
        let ctx = AppCtx::new(&format!("file://{}", dir.display()), &name, Some(dir));
        return Ok((wasm, ctx));
    }
    if p.exists() {
        let file = if p.is_dir() { p.join("app.wasm") } else { p.to_path_buf() };
        let wasm = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
        return Ok((wasm, AppCtx::local(p, name)));
    }
    let ctx = crate::cli::find(arg)?;
    let file = ctx.home.join("bundle/app.wasm");
    let wasm = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((wasm, ctx))
}

struct Run {
    d: Box<dyn Driver>,
    ui: Ui,
}

impl Run {
    /// Polls the app for `ms`, forwarding UI events and laying out.
    fn settle(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            self.d.poll(&mut self.ui);
            self.ui.layout();
            let evs = self.ui.take_events();
            if !evs.is_empty() {
                self.d.events(&mut self.ui, evs);
                continue;
            }
            if Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn click_at(&mut self, x: f32, y: f32) {
        self.press_at(crate::platform::BUTTON_LEFT, x, y);
    }

    fn press_at(&mut self, button: u32, x: f32, y: f32) {
        self.ui.pointer_move(x, y);
        self.ui.pointer_button(button, true, x, y);
        self.ui.pointer_button(button, false, x, y);
        let evs = self.ui.take_events();
        self.d.events(&mut self.ui, evs);
    }

    /// Where to click to hit the node showing `text`.
    fn find(&mut self, text: &str) -> Option<(f32, f32)> {
        let order = tree_order(&self.ui);
        for exact in [true, false] {
            let m = |s: &str| if exact { s == text } else { s.contains(text) };
            for &id in &order {
                let n = &self.ui.nodes[&id];
                let r = n.rect;
                if r.w <= 0.0 || r.h <= 0.0 {
                    continue;
                }
                let center = Some((r.x + r.w / 2.0, r.y + r.h / 2.0));
                let hit = match n.kind {
                    kind::BUTTON => m(n.s(prop::button::LABEL)) || m(n.s(prop::TOOLTIP)),
                    kind::CHECKBOX => m(n.s(prop::checkbox::LABEL)),
                    kind::INPUT => m(n.s(prop::input::PLACEHOLDER)) || (!n.s(prop::input::VALUE).is_empty() && m(n.s(prop::input::VALUE))),
                    kind::SELECT => m(n.s(prop::select::PLACEHOLDER)),
                    kind::MENU => {
                        let s = n.s(prop::menu::ITEMS).to_string();
                        let items = crate::ui::popup::parse_items(&s);
                        if let Some(i) = items.iter().position(|it| !it.sep && m(it.label)) {
                            let rects = self.ui.menu_rects(id);
                            return rects.get(i).map(|(_, r, _)| (r.x + r.w / 2.0, r.y + r.h / 2.0));
                        }
                        false
                    }
                    kind::TABS => {
                        let labels: Vec<String> = n.s(prop::tabs::LABELS).split('\n').map(String::from).collect();
                        if let Some(i) = labels.iter().position(|l| m(l)) {
                            let rects = self.ui.tab_rects(id);
                            return rects.get(i).map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0));
                        }
                        false
                    }
                    kind::TEXT => {
                        let t = n.s(prop::text::TEXT).to_string();
                        // A link span showing the text wins over the whole node.
                        for sp in self.ui.spans(id) {
                            if sp.flags & qa_proto::span::LINK != 0 && t.get(sp.start..sp.end).is_some_and(m) {
                                if let Some(p) = self.link_point(id, sp.start) {
                                    return Some(p);
                                }
                            }
                        }
                        m(&t)
                    }
                    _ => m(n.s(prop::TOOLTIP)),
                };
                if hit {
                    return center;
                }
            }
        }
        None
    }

    /// A point where a press lands at byte offset `at` of a text node.
    fn index_point(&mut self, id: u32, at: usize) -> Option<(f32, f32)> {
        let r = self.ui.nodes[&id].rect;
        let mut y = r.y + 2.0;
        while y < r.y + r.h {
            let mut x = r.x;
            while x < r.x + r.w + 2.0 {
                if self.ui.text_index_at(id, x, y) == at {
                    return Some((x, y));
                }
                x += 1.0;
            }
            y += 4.0;
        }
        None
    }

    /// Selects `want` in the first text node that contains it, by dragging.
    fn select(&mut self, want: &str) -> Result<(), String> {
        let order = tree_order(&self.ui);
        let (id, at) = order
            .iter()
            .filter(|id| self.ui.nodes[id].kind == kind::TEXT)
            .find_map(|&id| self.ui.nodes[&id].s(prop::text::TEXT).find(want).map(|i| (id, i)))
            .ok_or(format!("select: no text contains {want:?}"))?;
        let a = self.index_point(id, at).ok_or("select: can't find where the text starts")?;
        let b = self.index_point(id, at + want.len()).ok_or("select: can't find where the text ends")?;
        let ui = &mut self.ui;
        ui.pointer_move(a.0, a.1);
        ui.pointer_button(crate::platform::BUTTON_LEFT, true, a.0, a.1);
        ui.pointer_move((a.0 + b.0) / 2.0, b.1);
        ui.pointer_move(b.0, b.1);
        ui.pointer_button(crate::platform::BUTTON_LEFT, false, b.0, b.1);
        Ok(())
    }

    /// A point over byte `at` of a text node, found by probing.
    fn link_point(&mut self, id: u32, at: usize) -> Option<(f32, f32)> {
        let r = self.ui.nodes[&id].rect;
        let mut y = r.y + 2.0;
        while y < r.y + r.h {
            let mut x = r.x + 1.0;
            while x < r.x + r.w {
                if self.ui.link_at(id, x, y).is_some() {
                    let spans = self.ui.spans(id);
                    let i = self.ui.link_at(id, x, y)? as usize;
                    if spans.get(i).is_some_and(|s| s.start <= at && at < s.end) {
                        return Some((x + 1.0, y));
                    }
                }
                x += 2.0;
            }
            y += 4.0;
        }
        None
    }

    fn step(&mut self, s: &str, settle: u64) -> Result<(), String> {
        let (cmd, arg) = s.split_once(':').ok_or(format!("{s}: expected <step>:<arg>"))?;
        match cmd {
            "click" => {
                let (x, y) = self.find(arg).ok_or(format!("click: nothing shows {arg:?}"))?;
                self.click_at(x, y);
            }
            "dblclick" => {
                let (x, y) = self.find(arg).ok_or(format!("dblclick: nothing shows {arg:?}"))?;
                self.click_at(x, y);
                self.click_at(x, y);
            }
            "rclick" => {
                let (x, y) = self.find(arg).ok_or(format!("rclick: nothing shows {arg:?}"))?;
                self.press_at(crate::platform::BUTTON_RIGHT, x, y);
            }
            "hover" => {
                let (x, y) = self.find(arg).ok_or(format!("hover: nothing shows {arg:?}"))?;
                self.ui.pointer_move(x, y);
            }
            "wheel" => {
                let d: f32 = arg.parse().map_err(|_| format!("wheel: {arg:?} is not a number"))?;
                self.ui.wheel(0.0, d, false);
            }
            "select" => self.select(arg)?,
            "type" => {
                for ch in arg.chars() {
                    self.ui.key(ch as u32, &ch.to_string(), 0);
                }
            }
            "key" => {
                let (sym, text, mods) = key(arg).ok_or(format!("key: unknown key {arg:?}"))?;
                self.ui.key(sym, &text, mods);
            }
            "wait" => {
                let ms = arg.parse().map_err(|_| format!("wait: {arg:?} is not a number"))?;
                self.settle(ms);
                return Ok(());
            }
            "shot" => {
                self.write(arg)?;
                return Ok(());
            }
            _ => return Err(format!("unknown step {cmd:?}")),
        }
        let evs = self.ui.take_events();
        self.d.events(&mut self.ui, evs);
        self.settle(settle);
        Ok(())
    }

    fn write(&mut self, out: &str) -> Result<(), String> {
        self.ui.layout();
        let ui = &mut self.ui;
        let (w, h) = ((ui.win_w * ui.scale).round() as i32, (ui.win_h * ui.scale).round() as i32);
        let full = RectF::new(0.0, 0.0, ui.win_w, ui.win_h);
        let mut px = vec![0u32; (w * h) as usize];
        let mut c = Canvas::new(&mut px, w, h, w as usize);
        let list = ui.paint(full);
        let mut cache = GlyphCache::new();
        render(&list, &mut c, crate::platform::Rect::new(0, 0, w, h), &ui.fonts, &mut cache, self.d.pixmaps());
        let mut extra = Vec::new();
        self.d.overlay(ui, &mut extra);
        render(&extra, &mut c, crate::platform::Rect::new(0, 0, w, h), &ui.fonts, &mut cache, self.d.pixmaps());
        let mut tree = dump(ui);
        if let Some(o) = self.d.overlay_ui() {
            o.match_window(ui);
            o.layout();
            let l = o.paint(full);
            render(&l, &mut c, crate::platform::Rect::new(0, 0, w, h), &o.fonts, &mut GlyphCache::new(), &crate::gfx::NoPixmaps);
            tree.push_str("\n# overlay (a runtime prompt)\n");
            tree.push_str(&dump(o));
        }
        if let Some(e) = self.d.error() {
            let _ = writeln!(tree, "\n# app error: {e}");
        }
        std::fs::write(out, crate::gfx::png::encode(&px, w as usize, h as usize)).map_err(|e| format!("{out}: {e}"))?;
        let txt = Path::new(out).with_extension("txt");
        std::fs::write(&txt, tree).map_err(|e| format!("{}: {e}", txt.display()))?;
        eprintln!("qa shot: wrote {out} and {}", txt.display());
        Ok(())
    }
}

fn key(name: &str) -> Option<(u32, String, u8)> {
    let mut mods = 0;
    let mut rest = name;
    loop {
        if let Some(r) = rest.strip_prefix("ctrl+") {
            mods |= crate::ui::edit::CTRL;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("shift+") {
            mods |= crate::ui::edit::SHIFT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt+") {
            mods |= crate::ui::edit::ALT;
            rest = r;
        } else {
            break;
        }
    }
    let (sym, text) = match rest {
        "Enter" | "Return" => (0xff0d, "\r"),
        "Tab" => (0xff09, "\t"),
        "Escape" | "Esc" => (0xff1b, ""),
        "Backspace" => (0xff08, ""),
        "Delete" => (0xffff, ""),
        "Up" => (0xff52, ""),
        "Down" => (0xff54, ""),
        "Left" => (0xff51, ""),
        "Right" => (0xff53, ""),
        "PageUp" => (0xff55, ""),
        "PageDown" => (0xff56, ""),
        "Home" => (0xff50, ""),
        "End" => (0xff57, ""),
        "Space" => (0x20, " "),
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            let text = if mods & crate::ui::edit::CTRL != 0 { String::new() } else { c.to_string() };
            return Some((c.to_ascii_lowercase() as u32, text, mods));
        }
        _ => return None,
    };
    Some((sym, text.to_string(), mods))
}

fn tree_order(ui: &Ui) -> Vec<u32> {
    let mut out = Vec::new();
    // Open popups first: they are on top.
    let mut stack = vec![ui.root];
    stack.extend(ui.popups.iter().map(|p| p.id));
    while let Some(id) = stack.pop() {
        let Some(n) = ui.nodes.get(&id) else { continue };
        if n.hidden() {
            continue;
        }
        out.push(id);
        stack.extend(n.children.iter().rev());
    }
    out
}

fn kind_name(k: u8) -> &'static str {
    match k {
        kind::GRID => "grid",
        kind::TEXT => "text",
        kind::IMAGE => "image",
        kind::CANVAS => "canvas",
        kind::SCROLL => "scroll",
        kind::BUTTON => "button",
        kind::INPUT => "input",
        kind::CHECKBOX => "checkbox",
        kind::SELECT => "select",
        kind::TABS => "tabs",
        kind::PROGRESS => "progress",
        kind::DIVIDER => "divider",
        kind::POPUP => "popup",
        kind::MENU => "menu",
        _ => "?",
    }
}

/// One line per visible node: kind, id, what it shows, its rect.
pub fn dump(ui: &Ui) -> String {
    fn walk(ui: &Ui, id: u32, depth: usize, out: &mut String) {
        let Some(n) = ui.nodes.get(&id) else { return };
        if n.hidden() {
            return;
        }
        let r = n.rect;
        let _ = write!(out, "{}{} #{id}", "  ".repeat(depth), kind_name(n.kind));
        let shown = match n.kind {
            kind::TEXT => n.s(prop::text::TEXT),
            kind::BUTTON => n.s(prop::button::LABEL),
            kind::CHECKBOX => n.s(prop::checkbox::LABEL),
            kind::INPUT => n.s(prop::input::VALUE),
            kind::TABS => n.s(prop::tabs::LABELS),
            kind::IMAGE => n.s(prop::image::SRC),
            kind::MENU => n.s(prop::menu::ITEMS),
            _ => "",
        };
        if !shown.is_empty() {
            let mut s: String = shown.chars().take(120).collect::<String>().replace('\n', "⏎");
            if shown.chars().count() > 120 {
                s.push('…');
            }
            let _ = write!(out, " {s:?}");
        }
        let extra: [(&str, String); 5] = [
            ("variant", n.s(prop::VARIANT).to_string()),
            ("tag", match n.i(prop::TAG, 0) { 0 => String::new(), t => t.to_string() }),
            ("placeholder", if n.kind == kind::INPUT { n.s(prop::input::PLACEHOLDER).to_string() } else { String::new() }),
            ("selected", match n.kind { kind::SELECT | kind::TABS => n.i(prop::select::SELECTED, -1).to_string(), _ => String::new() }),
            ("checked", if n.kind == kind::CHECKBOX { (n.i(prop::checkbox::CHECKED, 0) != 0).to_string() } else { String::new() }),
        ];
        for (k, v) in extra.iter().filter(|(_, v)| !v.is_empty()) {
            let _ = write!(out, " {k}={v:?}");
        }
        if ui.focus == id {
            out.push_str(" focused");
        }
        let _ = writeln!(out, " [{:.0},{:.0} {:.0}x{:.0}]", r.x, r.y, r.w, r.h);
        for &c in &n.children {
            walk(ui, c, depth + 1, out);
        }
    }
    let mut out = String::new();
    walk(ui, ui.root, 0, &mut out);
    for p in &ui.popups {
        out.push_str("open ");
        walk(ui, p.id, 0, &mut out);
    }
    out
}

pub fn run(args: &[String]) -> i32 {
    let mut args = args.to_vec();
    let fail = |e: String| {
        eprintln!("qa shot: {e}");
        1
    };
    let opts = match RunOpts::take(&mut args) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let out = opt(&mut args, "--out").unwrap_or_else(|| "shot.png".into());
    let size = opt(&mut args, "--size").unwrap_or_else(|| "800x600".into());
    let scale: f32 = opt(&mut args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let data = opt(&mut args, "--data");
    let Some((w, h)) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))) else {
        return fail(format!("--size {size}: expected WxH"));
    };
    if args.is_empty() || args[0].starts_with('-') {
        eprint!("{USAGE}");
        return 2;
    }
    let Some(fonts) = crate::load_fonts() else { return 1 };
    let (d, theme_id): (Box<dyn Driver>, Option<String>) = if args[0].starts_with("qa:") {
        (Box::new(crate::open::Prompt::new(&crate::open::link_url(&args[0]))), None)
    } else {
        let (wasm, mut ctx) = match resolve(&args[0]) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        if let Some(d) = data {
            let _ = std::fs::create_dir_all(&d);
            ctx.home = std::fs::canonicalize(&d).unwrap_or_else(|_| d.into());
        }
        let id = ctx.id.clone();
        let mut d = WasmDriver::new(wasm, ctx);
        opts.apply(&mut d, false);
        (Box::new(d), Some(id))
    };
    let mut ui = Ui::new(fonts, Theme::load(theme_id.as_deref()));
    ui.set_window((w * scale) as i32, (h * scale) as i32, scale);
    let mut run = Run { d, ui };
    run.d.start(&mut run.ui);
    run.d.resize(&mut run.ui, w, h, scale);
    run.settle(500);
    for s in &args[1..] {
        if let Err(e) = run.step(s, 300) {
            let _ = run.write(&out);
            return fail(e);
        }
    }
    match run.write(&out) {
        Ok(()) if run.d.error().is_some() => 1,
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}
