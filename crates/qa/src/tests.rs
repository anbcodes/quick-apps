//! Headless tests: the UI driven through patches and synthetic input.

use crate::gfx::glyph::GlyphCache;
use crate::gfx::raster::Canvas;
use crate::gfx::{render, NoPixmaps, RectF};
use crate::shell::{merge, Driver};
use crate::text::{self, Fonts};
use crate::todo::Todo;
use crate::ui::layout::{size_tracks, TCell};
use crate::ui::{theme::Theme, Ui, UiEvent};
use qa_proto::Track;

fn ui() -> Ui {
    let mut fonts = Fonts::new();
    fonts.find_first(text::DEFAULT_SANS).expect("a system font");
    let mut ui = Ui::new(fonts, Theme::new());
    ui.set_window(600, 480, 1.0);
    ui
}

fn settle(ui: &mut Ui) {
    ui.layout();
    ui.damage.clear();
}

#[test]
fn typing_damages_only_the_input() {
    let mut ui = ui();
    let mut d = Todo::new();
    d.start(&mut ui);
    settle(&mut ui);
    let input = ui.focus;
    assert_ne!(input, 0, "the input has focus");
    let r = ui.node(input).unwrap().rect;
    for ch in ["h", "e", "l", "l", "o"] {
        ui.key(ch.as_bytes()[0] as u32, ch, 0);
        let evs = ui.take_events();
        d.events(&mut ui, evs);
        ui.layout();
        assert!(!ui.damage.is_empty());
        for dmg in &ui.damage {
            // Damage rects carry a 4 px margin for focus rings.
            assert!(dmg.x >= r.x - 4.0 && dmg.y >= r.y - 4.0 && dmg.x + dmg.w <= r.x + r.w + 4.0 && dmg.y + dmg.h <= r.y + r.h + 4.0, "{dmg:?} outside {r:?}");
        }
        ui.damage.clear();
    }
}

#[test]
fn submit_adds_a_row_and_clears_input() {
    let mut ui = ui();
    let mut d = Todo::new();
    d.start(&mut ui);
    settle(&mut ui);
    let before = ui.nodes.len();
    for ch in "milk".chars() {
        ui.key(ch as u32, &ch.to_string(), 0);
    }
    ui.key(0xff0d, "\r", 0);
    let evs = ui.take_events();
    assert!(evs.iter().any(|e| matches!(e, UiEvent::Submit(_, v) if v == "milk")));
    d.events(&mut ui, evs);
    ui.layout();
    assert_eq!(ui.nodes.len(), before + 4);
    let input = ui.focus;
    match &ui.nodes[&input].st {
        crate::ui::State::Input(e) => assert_eq!(e.text, ""),
        _ => panic!(),
    }
}

#[test]
fn clicking_a_checkbox_toggles_it() {
    let mut ui = ui();
    let mut d = Todo::new();
    d.start(&mut ui);
    settle(&mut ui);
    let cb = *ui.nodes.iter().find(|(_, n)| n.kind == qa_proto::kind::CHECKBOX).unwrap().0;
    let r = ui.node(cb).unwrap().rect;
    let (x, y) = (r.x + 5.0, r.y + r.h / 2.0);
    ui.pointer_move(x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, true, x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, false, x, y);
    assert!(ui.take_events().contains(&UiEvent::Toggle(cb, true)));
}

#[test]
fn renders_without_panicking() {
    let mut ui = ui();
    let mut d = Todo::new();
    d.start(&mut ui);
    ui.layout();
    let list = ui.paint(RectF::new(0.0, 0.0, 600.0, 480.0));
    let mut px = vec![0u32; 600 * 480];
    let mut c = Canvas::new(&mut px, 600, 480, 600);
    let mut cache = GlyphCache::new();
    render(&list, &mut c, crate::platform::Rect::new(0, 0, 600, 480), &ui.fonts, &mut cache, &NoPixmaps);
    assert!(px.iter().any(|&p| p != 0xffffffff));
}

#[test]
fn low_priority_tracks_hide_first() {
    // sidebar(220, min 140) | content fr(1, min 240) | inspector 280 (priority -1)
    let specs = [Track::px(220.0).min(140.0), Track::fr(1.0).min(240.0), Track::px(280.0)];
    let cells = [
        TCell { start: 0, span: 1, min: 0.0, pref: 0.0, prio: 0 },
        TCell { start: 1, span: 1, min: 0.0, pref: 0.0, prio: 1 },
        TCell { start: 2, span: 1, min: 0.0, pref: 0.0, prio: -1 },
    ];
    let wide = size_tracks(&specs, 3, &cells, 1000.0, 0.0);
    assert_eq!(wide.hidden, vec![false; 3]);
    assert_eq!(wide.sizes, vec![220.0, 500.0, 280.0]);
    let mid = size_tracks(&specs, 3, &cells, 700.0, 0.0);
    assert_eq!(mid.hidden, vec![false, false, true]);
    assert_eq!(mid.sizes[0], 220.0);
    assert_eq!(mid.sizes[1], 480.0);
    let narrow = size_tracks(&specs, 3, &cells, 420.0, 0.0);
    assert!(narrow.hidden[2]);
    assert_eq!(narrow.sizes[0], 180.0);
    assert_eq!(narrow.sizes[1], 240.0);
    let tiny = size_tracks(&specs, 3, &cells, 300.0, 0.0);
    assert_eq!(tiny.sizes[0], 140.0);
    assert_eq!(tiny.sizes[1], 240.0);
}

#[test]
fn fr_tracks_share_surplus() {
    let specs = [Track::fr(1.0), Track::fr(2.0), Track::px(100.0)];
    let s = size_tracks(&specs, 3, &[], 400.0, 0.0);
    assert_eq!(s.sizes, vec![100.0, 200.0, 100.0]);
    let specs = [Track::fr(1.0).max(50.0), Track::fr(1.0)];
    let s = size_tracks(&specs, 2, &[], 300.0, 0.0);
    assert_eq!(s.sizes, vec![50.0, 250.0]);
}

#[test]
fn damage_merges() {
    use crate::platform::Rect;
    let mut v = vec![Rect::new(0, 0, 10, 10), Rect::new(5, 5, 10, 10)];
    merge(&mut v);
    assert_eq!(v, vec![Rect::new(0, 0, 15, 15)]);
    let mut v = vec![Rect::new(0, 0, 10, 10), Rect::new(500, 500, 10, 10)];
    merge(&mut v);
    assert_eq!(v.len(), 2);
}

#[test]
fn bad_patches_report_offsets() {
    let mut ui = ui();
    let mut p = Vec::new();
    use qa_proto::Sink;
    p.op_create(1, qa_proto::kind::GRID);
    p.op_append(1, 99);
    let e = ui.apply_patch(&p).unwrap_err();
    assert_eq!(e.offset, 6);
}

/// Waits for the app thread and applies what it sent.
fn pump(d: &mut crate::host::WasmDriver, ui: &mut Ui) {
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        d.poll(ui);
    }
}

#[test]
fn counter_wasm_counts() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter/target/wasm32-unknown-unknown/release/counter.wasm");
    let Ok(wasm) = std::fs::read(path) else {
        eprintln!("skipped: build examples/counter first");
        return;
    };
    let mut ui = ui();
    let ctx = crate::pkg::AppCtx::new("test://counter", "counter", None);
    let mut d = crate::host::WasmDriver::new(wasm, ctx);
    d.start(&mut ui);
    pump(&mut d, &mut ui);
    assert!(d.error.is_none(), "{:?}", d.error);
    ui.layout();
    let plus = *ui.nodes.iter().find(|(_, n)| n.kind == qa_proto::kind::BUTTON && n.s(qa_proto::prop::button::LABEL) == "+1").unwrap().0;
    d.events(&mut ui, vec![UiEvent::Click(plus), UiEvent::Click(plus), UiEvent::Click(plus)]);
    pump(&mut d, &mut ui);
    let label = ui.nodes.values().find(|n| n.kind == qa_proto::kind::TEXT).unwrap();
    assert_eq!(label.s(qa_proto::prop::text::TEXT), "3");
}

fn sandbox() -> Option<(crate::host::WasmDriver, Ui)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sandbox/target/wasm32-unknown-unknown/release/sandbox.wasm");
    let wasm = std::fs::read(path).ok()?;
    let mut ui = ui();
    let mut ctx = crate::pkg::AppCtx::new(&format!("test://sandbox/{}", std::process::id()), "Sandbox", None);
    ctx.home = std::env::temp_dir().join(format!("qa-sandbox-{}", std::process::id()));
    let mut d = crate::host::WasmDriver::new(wasm, ctx);
    d.start(&mut ui);
    pump(&mut d, &mut ui);
    Some((d, ui))
}

fn button(ui: &Ui, label: &str) -> u32 {
    *ui.nodes.iter().find(|(_, n)| n.kind == qa_proto::kind::BUTTON && n.s(qa_proto::prop::button::LABEL) == label).unwrap().0
}

fn log_text(ui: &Ui) -> String {
    ui.nodes.values().filter(|n| n.kind == qa_proto::kind::TEXT && n.s(qa_proto::prop::text::STYLE) == "mono").map(|n| n.s(qa_proto::prop::text::TEXT).to_string()).collect()
}

#[test]
fn sandbox_paths_cannot_escape() {
    let Some((mut d, mut ui)) = sandbox() else { return };
    let b = button(&ui, "Open ../secret");
    d.events(&mut ui, vec![UiEvent::Click(b)]);
    let b = button(&ui, "Write + read");
    d.events(&mut ui, vec![UiEvent::Click(b)]);
    pump(&mut d, &mut ui);
    let log = log_text(&ui);
    assert!(log.contains("open ../secret -> INVALID"), "{log}");
    assert!(log.contains("open /etc/passwd -> INVALID"), "{log}");
    assert!(log.contains("read back: hello from the sandbox"), "{log}");
}

#[test]
fn memory_hog_prompts() {
    let Some((mut d, mut ui)) = sandbox() else { return };
    let b = button(&ui, "Hog memory");
    d.events(&mut ui, vec![UiEvent::Click(b)]);
    pump(&mut d, &mut ui);
    let o = d.overlay_ui().expect("a prompt is showing");
    let texts: String = o.nodes.values().map(|n| n.s(qa_proto::prop::text::TEXT).to_string() + "|").collect();
    assert!(texts.contains("more than 50 MB"), "{texts}");
    let deny = button(o, "Deny");
    d.overlay_events(&mut ui, vec![UiEvent::Click(deny)]);
    pump(&mut d, &mut ui);
    // Denied growth makes the allocation fail, which stops the app.
    assert!(d.error.is_some());
}

#[test]
#[ignore = "takes ~9 s; run with --ignored"]
fn runaway_loop_is_throttled() {
    let Some((mut d, mut ui)) = sandbox() else { return };
    let cpu = || {
        let s = std::fs::read_to_string("/proc/self/stat").unwrap();
        let f: Vec<&str> = s.rsplit(')').next().unwrap().split_whitespace().collect();
        (f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap()) as f64 / 100.0
    };
    let c0 = cpu();
    let t0 = std::time::Instant::now();
    let b = button(&ui, "Spin forever");
    d.events(&mut ui, vec![UiEvent::Click(b)]);
    std::thread::sleep(std::time::Duration::from_secs(9));
    d.poll(&mut ui);
    let used = cpu() - c0;
    let wall = t0.elapsed().as_secs_f64();
    eprintln!("spin: {used:.2}s CPU in {wall:.2}s");
    // 5 s of CPU per 10 s window: the loop gets 5 s, then waits.
    assert!(used < 5.6 && used > 4.8, "{used}");
    assert!(d.overlay_ui().is_some(), "not responding prompt");
    let o = d.overlay_ui().unwrap();
    let stop = button(o, "Stop");
    d.overlay_events(&mut ui, vec![UiEvent::Click(stop)]);
    pump(&mut d, &mut ui);
    assert!(d.error.as_deref().unwrap_or("").contains("stopped"));
}

/// Writes premultiplied ARGB pixels as an uncompressed PNG (for looking at test renders).
pub fn write_png(path: &str, px: &[u32], w: usize, h: usize) {
    std::fs::write(path, crate::gfx::png::encode(px, w, h)).unwrap();
}

/// Renders a tree (and optionally an overlay tree) to a PNG when QA_SHOT_DIR is set.
pub fn shot(name: &str, ui: &mut Ui, over: Option<&mut Ui>) {
    let Some(dir) = std::env::var_os("QA_SHOT_DIR") else { return };
    ui.layout();
    let (w, h) = ((ui.win_w * ui.scale) as i32, (ui.win_h * ui.scale) as i32);
    let full = RectF::new(0.0, 0.0, ui.win_w, ui.win_h);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut px, w, h, w as usize);
    let list = ui.paint(full);
    render(&list, &mut c, crate::platform::Rect::new(0, 0, w, h), &ui.fonts, &mut GlyphCache::new(), &NoPixmaps);
    if let Some(o) = over {
        o.match_window(ui);
        o.layout();
        let l = o.paint(full);
        render(&l, &mut c, crate::platform::Rect::new(0, 0, w, h), &o.fonts, &mut GlyphCache::new(), &NoPixmaps);
    }
    let p = std::path::Path::new(&dir).join(format!("{name}.png"));
    write_png(p.to_str().unwrap(), &px, w as usize, h as usize);
}

#[test]
fn picker_renders() {
    let mut ui = ui();
    let mut d = Todo::new();
    d.start(&mut ui);
    let mut o = crate::prompt::Overlay::new("Notes", Theme::new());
    o.pick(1, crate::prompt::PickMode::Open { patterns: vec!["*.md".into()] });
    assert!(o.active());
    shot("picker", &mut ui, Some(&mut o.ui));
    let esc = UiEvent::Key { node: 0, sym: 0xff1b, mods: 0, text: String::new() };
    let done = o.events(vec![esc]);
    assert!(matches!(done.as_slice(), [crate::prompt::Done::Picked { path: None, .. }]));
}

fn example(name: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("{}/../../examples/{name}/target/wasm32-unknown-unknown/release/{name}.wasm", env!("CARGO_MANIFEST_DIR"))).ok()
}

fn temp_ctx(name: &str) -> crate::pkg::AppCtx {
    let mut ctx = crate::pkg::AppCtx::new(&format!("test://{name}/{}", std::process::id()), name, None);
    ctx.home = std::env::temp_dir().join(format!("qa-{name}-{}", std::process::id()));
    ctx
}

#[test]
#[ignore = "needs the network"]
fn fetch_app_gets_json_after_the_internet_prompt() {
    let Some(wasm) = example("fetch") else { return };
    let mut ui = ui();
    let ctx = temp_ctx("fetch");
    let mut d = crate::host::WasmDriver::new(wasm, ctx.clone());
    d.services = Box::new(crate::io::Net::new(&ctx, false));
    d.start(&mut ui);
    pump(&mut d, &mut ui);
    let go = button(&ui, "Fetch");
    d.events(&mut ui, vec![UiEvent::Click(go)]);
    pump(&mut d, &mut ui);
    let o = d.overlay_ui().expect("internet prompt");
    let once = button(o, "Allow once");
    d.overlay_events(&mut ui, vec![UiEvent::Click(once)]);
    let mut status = String::new();
    for _ in 0..40 {
        pump(&mut d, &mut ui);
        status = ui.nodes.values().filter(|n| n.kind == qa_proto::kind::TEXT).map(|n| n.s(qa_proto::prop::text::TEXT).to_string()).find(|s| s.starts_with("HTTP") || s.starts_with("Failed")).unwrap_or_default();
        if !status.is_empty() {
            break;
        }
    }
    assert!(status.starts_with("HTTP 200"), "{status}");
    let body: String = ui.nodes.values().filter(|n| n.s(qa_proto::prop::text::STYLE) == "mono").map(|n| n.s(qa_proto::prop::text::TEXT).to_string()).collect();
    assert!(body.contains("\"full_name\""), "{}", &body[..body.len().min(200)]);
}

#[test]
fn messages_reach_running_apps_and_inboxes() {
    use crate::host::Services;
    let a = temp_ctx("sender");
    let mut b = crate::pkg::AppCtx::new(&format!("test://receiver/{}", std::process::id()), "receiver", None);
    // The inbox lives under the real data home, so use a throwaway id there.
    let _ = std::fs::create_dir_all(&b.home);
    let mut na = crate::io::Net::new(&a, false);
    {
        let mut nb = crate::io::Net::new(&b, true);
        assert_eq!(na.send_msg(&b.source, b"live"), 0);
        let mut got = Vec::new();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            got.extend(nb.poll());
            if !got.is_empty() {
                break;
            }
        }
        let mut r = qa_proto::Reader::new(&got[0][1..]);
        assert_eq!(got[0][0], qa_proto::ev::MESSAGE);
        assert_eq!(r.str(), Some(a.source.as_str()));
        assert_eq!(r.bytes(), Some(&b"live"[..]));
    }
    // Not running: queued.
    assert_eq!(na.send_msg(&b.id, b"later"), 1);
    let q = crate::io::drain_inbox(&b);
    assert_eq!(q.len(), 1);
    b.home = b.home.clone();
    let _ = std::fs::remove_dir_all(&b.home);
}

#[test]
fn gzip_bodies_decompress() {
    let data = b"hello hello hello hello".repeat(10);
    let deflated = miniz_oxide::deflate::compress_to_vec(&data, 6);
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend_from_slice(&deflated);
    gz.extend_from_slice(&[0; 8]);
    let raw = crate::io::http::gzip_payload(&gz).unwrap();
    assert_eq!(miniz_oxide::inflate::decompress_to_vec(raw).unwrap(), data);
}

#[test]
fn widgets_render() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/widgets");
    let Some(wasm) = example("widgets") else { return };
    let mut ui = ui();
    let mut ctx = temp_ctx("widgets");
    ctx.bundle = Some(std::path::PathBuf::from(dir));
    let mut d = crate::host::WasmDriver::new(wasm, ctx);
    d.start(&mut ui);
    pump(&mut d, &mut ui);
    assert!(d.error.is_none(), "{:?}", d.error);
    assert!(d.pix.canvases.len() == 1);
    shot("widgets1", &mut ui, None);
    let tabs = *ui.nodes.iter().find(|(_, n)| n.kind == qa_proto::kind::TABS).unwrap().0;
    d.events(&mut ui, vec![UiEvent::Select(tabs, 1)]);
    pump(&mut d, &mut ui);
    assert_eq!(d.pix.images.len(), 1, "logo decoded");
    assert_eq!(ui.image_sizes.values().next(), Some(&(192.0, 128.0)));
    // Render with pixmaps through the real path.
    ui.layout();
    let (w, h) = (600, 480);
    let mut px = vec![0u32; w * h];
    let mut c = Canvas::new(&mut px, w as i32, h as i32, w);
    let list = ui.paint(RectF::new(0.0, 0.0, 600.0, 480.0));
    use crate::shell::Driver as _;
    render(&list, &mut c, crate::platform::Rect::new(0, 0, w as i32, h as i32), &ui.fonts, &mut GlyphCache::new(), d.pixmaps());
    if let Some(dir) = std::env::var_os("QA_SHOT_DIR") {
        write_png(std::path::Path::new(&dir).join("widgets2.png").to_str().unwrap(), &px, w, h);
    }
}

#[test]
fn text_fits_its_own_measured_width() {
    // Words with kerning pairs ("Ye", "To", "AV") must not wrap at the width
    // `measure` reports, or an auto track sized to it breaks the last letter off.
    let mut fonts = Fonts::new();
    let f = fonts.find_first(text::DEFAULT_SANS).expect("a system font");
    for word in ["Yesterday", "Today", "AVATAR", "Wave"] {
        let w = fonts.measure(f, 12.0, word);
        let lines = text::wrap(&fonts, f, 12.0, word, w);
        assert_eq!(lines.len(), 1, "{word} wrapped at its own width {w}");
    }
}

/// Applies ops written by `f` and lays out.
fn patch(ui: &mut Ui, f: impl FnOnce(&mut Vec<u8>)) {
    let mut p = Vec::new();
    f(&mut p);
    ui.apply_patch(&p).unwrap();
    ui.layout();
}

fn set_i(p: &mut Vec<u8>, id: u32, prop: u16, v: i32) {
    use qa_proto::Sink;
    p.op_set(id, prop);
    p.value(&qa_proto::Value::I32(v));
}

#[test]
fn replace_keeps_the_position() {
    use qa_proto::{kind, Sink};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::GRID);
        for i in 2..=4 {
            p.op_create(i, kind::TEXT);
            p.op_append(1, i);
        }
        p.op_create(5, kind::TEXT);
        p.op_root(1);
        p.op_replace(3, 5);
    });
    assert_eq!(ui.nodes[&1].children, vec![2, 5, 4]);
    assert!(!ui.nodes.contains_key(&3));
    // Replacing the root makes the new node the root.
    patch(&mut ui, |p| {
        p.op_create(6, kind::GRID);
        p.op_replace(1, 6);
    });
    assert_eq!(ui.root, 6);
    assert!(!ui.nodes.contains_key(&2));
    // A node can't replace its own ancestor.
    let mut p = Vec::new();
    p.op_create(7, kind::TEXT);
    p.op_append(6, 7);
    p.op_replace(6, 7);
    assert!(ui.apply_patch(&p).is_err());
}

#[test]
fn events_carry_the_node_tag() {
    use qa_proto::{ev, kind, prop, Reader, Sink};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::BUTTON);
        p.op_root(1);
        set_i(p, 1, prop::TAG, 42);
    });
    assert_eq!(ui.tag_of(1), 42);
    let b = crate::host::encode::ui_event(&UiEvent::Change(1, "x".into()), ui.tag_of(1));
    let mut r = Reader::new(&b);
    assert_eq!((r.u8(), r.u32(), r.str(), r.i32()), (Some(ev::CHANGE), Some(1), Some("x"), Some(42)));
    // So does every other node event.
    let b = crate::host::encode::ui_event(&UiEvent::FocusOut(1), 42);
    let mut r = Reader::new(&b);
    assert_eq!((r.u8(), r.u32(), r.i32()), (Some(ev::FOCUS_OUT), Some(1), Some(42)));
}

fn click(ui: &mut Ui, id: u32) {
    let r = ui.node(id).unwrap().rect;
    let (x, y) = (r.x + 5.0, r.y + r.h / 2.0);
    ui.pointer_move(x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, true, x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, false, x, y);
}

/// A column (1) holding three buttons (2, 3, 4).
fn three_buttons() -> Ui {
    use qa_proto::{kind, Sink};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::GRID);
        for i in 2..=4 {
            p.op_create(i, kind::BUTTON);
            p.op_append(1, i);
        }
        p.op_root(1);
    });
    ui
}

#[test]
fn disabling_a_grid_disables_what_is_inside() {
    use qa_proto::prop;
    let mut ui = three_buttons();
    patch(&mut ui, |p| set_i(p, 3, prop::DISABLED, 1));
    click(&mut ui, 3);
    click(&mut ui, 2);
    assert_eq!(ui.take_events(), vec![UiEvent::Click(2)]);
    assert_eq!(ui.focus_order(), vec![2, 4]);
    patch(&mut ui, |p| set_i(p, 1, prop::DISABLED, 1));
    click(&mut ui, 2);
    assert!(ui.take_events().is_empty());
    assert!(ui.focus_order().is_empty());
}

#[test]
fn focus_moves_on_when_its_node_is_destroyed() {
    use qa_proto::Sink;
    let mut ui = three_buttons();
    ui.set_focus(3, true);
    patch(&mut ui, |p| p.op_destroy(3));
    assert_eq!(ui.focus, 4);
    assert!(ui.focus_visible);
    // The last one hands focus back to the one before it.
    patch(&mut ui, |p| p.op_destroy(4));
    assert_eq!(ui.focus, 2);
    patch(&mut ui, |p| p.op_destroy(2));
    assert_eq!(ui.focus, 0);
}

#[test]
fn two_quick_clicks_send_a_double_click() {
    let mut ui = three_buttons();
    click(&mut ui, 2);
    click(&mut ui, 2);
    assert_eq!(ui.take_events(), vec![UiEvent::Click(2), UiEvent::Click(2), UiEvent::DoubleClick(2)]);
    // A third click starts over, and clicks on different nodes don't pair up.
    click(&mut ui, 2);
    click(&mut ui, 3);
    assert_eq!(ui.take_events(), vec![UiEvent::Click(2), UiEvent::Click(3)]);
}

#[test]
fn checkbox_labels_wrap() {
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::GRID);
        p.op_create(2, kind::CHECKBOX);
        p.op_set(2, prop::checkbox::LABEL);
        p.value(&Value::Str("a label that is far too long to fit on one line of a narrow window, so it wraps"));
        p.op_set(2, prop::ALIGN_Y);
        p.value(&Value::I32(1));
        p.op_append(1, 2);
        p.op_root(1);
    });
    ui.set_window(200, 480, 1.0);
    ui.layout();
    let tall = ui.node(2).unwrap().rect.h;
    patch(&mut ui, |p| set_i(p, 2, prop::checkbox::WRAP, 1));
    let short = ui.node(2).unwrap().rect.h;
    assert!(tall >= short * 3.0, "{tall} vs {short}");
}

#[test]
fn flow_grids_wrap() {
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::GRID);
        set_i(p, 1, prop::grid::FLOW, 1);
        p.op_set(1, prop::grid::GAP);
        p.value(&Value::F32(4.0));
        p.op_set(1, prop::MAX_W);
        p.value(&Value::F32(250.0));
        for i in 0..6 {
            p.op_create(10 + i, kind::BUTTON);
            p.op_set(10 + i, prop::button::LABEL);
            p.value(&Value::Str("chip"));
            p.op_append(1, 10 + i);
        }
        p.op_create(2, kind::GRID);
        p.op_append(2, 1);
        p.op_root(2);
    });
    let r: Vec<RectF> = (10..16).map(|i| ui.nodes[&i].rect).collect();
    // Same width, left to right, then a new line.
    assert!(r[1].x > r[0].x && r[1].y == r[0].y);
    let wrapped = r.iter().position(|x| x.y > r[0].y).expect("a second line");
    assert!(wrapped > 1 && wrapped < 6);
    assert_eq!(r[wrapped].x, r[0].x);
    assert!(r[wrapped].y >= r[0].y + r[0].h + 4.0 - 0.01);
    assert!(r.iter().all(|x| x.x + x.w <= ui.nodes[&1].rect.x + 250.0 + 0.01));
    let outer = ui.nodes[&1].rect;
    assert!((outer.y + outer.h - (r[5].y + r[5].h)).abs() < 0.01, "the grid is as tall as its lines");
}

#[test]
fn link_spans_send_link_events() {
    use qa_proto::{kind, prop, span, Sink, Value};
    let mut ui = ui();
    // "see docs here": span 0 is bold "see", span 1 links "docs".
    let mut spans = Vec::new();
    for (a, b, f) in [(0u32, 3u32, span::BOLD), (4, 8, span::LINK)] {
        spans.u32(a);
        spans.u32(b);
        spans.u8(f);
        spans.str("");
    }
    patch(&mut ui, |p| {
        p.op_create(1, kind::TEXT);
        p.op_set(1, prop::text::TEXT);
        p.value(&Value::Str("see docs here"));
        p.op_set(1, prop::text::SPANS);
        p.value(&Value::Bytes(&spans));
        p.op_root(1);
    });
    let r = ui.nodes[&1].rect;
    let ts = ui.text_style(1, "");
    let x_docs = r.x + ui.fonts.measure(ts.font, ts.size, "see d");
    let y = r.y + ts.line_h / 2.0;
    assert_eq!(ui.link_at(1, x_docs, y), Some(1));
    assert_eq!(ui.link_at(1, r.x + 2.0, y), None);
    for (x, want) in [(x_docs, true), (r.x + 2.0, false)] {
        ui.pointer_move(x, y);
        ui.pointer_button(crate::platform::BUTTON_LEFT, true, x, y);
        ui.pointer_button(crate::platform::BUTTON_LEFT, false, x, y);
        let evs = ui.take_events();
        assert_eq!(evs.contains(&UiEvent::Link(1, 1)), want, "{evs:?}");
    }
    shot("spans", &mut ui, None);
}

#[test]
fn missing_fallback_fonts_are_skipped() {
    let mut f = Fonts::new();
    let sans = f.find_first(text::DEFAULT_SANS).expect("a system font");
    f.set_fallback(&["no-such-font.ttf", text::DEFAULT_SANS[0], text::DEFAULT_SANS[0]]);
    assert!(f.fallback.len() <= 1);
    assert!(f.fallback.iter().all(|&id| id == sans || f.fonts[id as usize].face.tables().glyf.is_some() || f.fonts[id as usize].face.tables().cff.is_some()));
}

#[test]
fn run_opts_parse() {
    let mut a: Vec<String> = ["app", "--map", "https://x/=http://y/", "--grant", "internet", "click:Go"].iter().map(|s| s.to_string()).collect();
    let o = crate::shot::RunOpts::take(&mut a).unwrap();
    assert_eq!(a, vec!["app", "click:Go"]);
    assert_eq!(o.map, vec![("https://x/".to_string(), "http://y/".to_string())]);
    assert_eq!(o.grants, vec![crate::host::grants::Grant::Internet]);
}

#[test]
fn sandbox_json_secrets_and_tags() {
    let Some((mut d, mut ui)) = sandbox() else { return };
    let b = button(&ui, "JSON + secrets");
    d.events(&mut ui, vec![UiEvent::Click(b)]);
    pump(&mut d, &mut ui);
    let log = log_text(&ui);
    assert!(log.contains("json: b=xé n=-2.5 len=2 bad=rejected tag=7"), "{log}");
    assert!(log.contains("secret: set -> ok, read back: s3cret\tvalue, removed: yes"), "{log}");
    assert!(log.contains("utc offset: "), "{log}");
}

#[test]
fn text_selects_by_default_and_copies() {
    use crate::platform::BUTTON_LEFT;
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = ui();
    let mut p = Vec::new();
    p.op_create(1, kind::GRID);
    p.op_create(2, kind::TEXT);
    p.op_set(2, prop::text::TEXT);
    p.value(&Value::Str("hello brave world"));
    // A clickable row holding text, and text that opted out.
    p.op_create(3, kind::GRID);
    p.op_set(3, prop::grid::CLICKABLE);
    p.value(&Value::I32(1));
    p.op_create(4, kind::TEXT);
    p.op_set(4, prop::text::TEXT);
    p.value(&Value::Str("row label"));
    p.op_create(5, kind::TEXT);
    p.op_set(5, prop::text::TEXT);
    p.value(&Value::Str("not selectable"));
    p.op_set(5, prop::text::SELECTABLE);
    p.value(&Value::I32(0));
    p.op_append(3, 4);
    for c in [2, 3, 5] {
        p.op_append(1, c);
    }
    p.op_root(1);
    ui.apply_patch(&p).unwrap();
    settle(&mut ui);
    let r = ui.node(2).unwrap().rect;
    let y = r.y + r.h / 2.0;

    // Drag from the start across most of the text.
    ui.pointer_button(BUTTON_LEFT, true, r.x + 1.0, y);
    ui.pointer_move(r.x + 40.0, y);
    ui.pointer_move(r.x + 400.0, y);
    ui.pointer_button(BUTTON_LEFT, false, r.x + 400.0, y);
    assert_eq!(ui.selected_text().as_deref(), Some("hello brave world"));
    ui.key(0x63, "c", crate::ui::edit::CTRL);
    assert_eq!(ui.clipboard_out.take().as_deref(), Some("hello brave world"));

    // Double-click selects a word; a click elsewhere clears it.
    let ts = ui.text_style(2, "");
    let wx = r.x + ui.fonts.measure(ts.font, ts.size, "hello br");
    for _ in 0..2 {
        ui.pointer_button(BUTTON_LEFT, true, wx, y);
        ui.pointer_button(BUTTON_LEFT, false, wx, y);
    }
    assert_eq!(ui.selected_text().as_deref(), Some("brave"));

    // A plain click on text in a clickable row clicks it; a drag selects instead.
    let rr = ui.node(4).unwrap().rect;
    let ry = rr.y + rr.h / 2.0;
    ui.pointer_button(BUTTON_LEFT, true, rr.x + 2.0, ry);
    ui.pointer_button(BUTTON_LEFT, false, rr.x + 2.0, ry);
    assert!(ui.selected_text().is_none());
    assert!(ui.take_events().iter().any(|e| matches!(e, UiEvent::Click(3))));
    ui.pointer_button(BUTTON_LEFT, true, rr.x + 1.0, ry);
    ui.pointer_move(rr.x + 300.0, ry);
    ui.pointer_button(BUTTON_LEFT, false, rr.x + 300.0, ry);
    assert_eq!(ui.selected_text().as_deref(), Some("row label"));
    assert!(!ui.take_events().iter().any(|e| matches!(e, UiEvent::Click(3))));

    // Opted out: a drag does nothing.
    let nr = ui.node(5).unwrap().rect;
    let ny = nr.y + nr.h / 2.0;
    ui.pointer_button(BUTTON_LEFT, true, nr.x + 1.0, ny);
    ui.pointer_move(nr.x + 300.0, ny);
    ui.pointer_button(BUTTON_LEFT, false, nr.x + 300.0, ny);
    assert!(ui.selected_text().is_none());
}

/// A scroll (1) holding a column (2) of `n` buttons (100..), as the root.
fn long_list(n: u32) -> Ui {
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::SCROLL);
        p.op_create(2, kind::GRID);
        p.op_append(1, 2);
        for i in 0..n {
            p.op_create(100 + i, kind::BUTTON);
            p.op_set(100 + i, prop::button::LABEL);
            p.value(&Value::Str("row"));
            p.op_append(2, 100 + i);
        }
        p.op_root(1);
    });
    ui
}

fn scroll_y(ui: &Ui, id: u32) -> f32 {
    match &ui.nodes[&id].st {
        crate::ui::State::Scroll(s) => s.y,
        _ => panic!("not a scroll"),
    }
}

#[test]
fn rows_added_above_the_view_keep_it_in_place() {
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = long_list(60);
    ui.take_events();
    assert!(ui.scroll_by(1, 0.0, 700.0));
    let probe = (100..160).find(|i| ui.nodes[i].rect.y >= 0.0).unwrap();
    let before = ui.nodes[&probe].rect.y;
    patch(&mut ui, |p| {
        for i in 0..5 {
            p.op_create(200 + i, kind::BUTTON);
            p.op_set(200 + i, prop::button::LABEL);
            p.value(&Value::Str("older"));
            p.op_insert(2, 200 + i, 100);
        }
    });
    assert!((ui.nodes[&probe].rect.y - before).abs() < 0.5, "{} moved to {}", before, ui.nodes[&probe].rect.y);
    assert!(scroll_y(&ui, 1) > 700.0);
    // A row growing above the view doesn't move it either.
    patch(&mut ui, |p| {
        p.op_set(101, prop::MIN_H);
        p.value(&Value::F32(200.0));
    });
    assert!((ui.nodes[&probe].rect.y - before).abs() < 0.5);
}

#[test]
fn scrolls_report_near_start_and_near_end() {
    let mut ui = long_list(60);
    let evs = ui.take_events();
    assert!(evs.contains(&UiEvent::NearStart(1)), "{evs:?}");
    assert!(!evs.contains(&UiEvent::NearEnd(1)));
    ui.scroll_by(1, 0.0, 1e9);
    assert!(ui.take_events().contains(&UiEvent::NearEnd(1)));
    // Once only, until the user leaves the start and comes back.
    ui.scroll_by(1, 0.0, -1e9);
    assert!(ui.take_events().contains(&UiEvent::NearStart(1)));
    ui.scroll_by(1, 0.0, -10.0);
    assert!(ui.take_events().is_empty());
}

#[test]
fn stick_end_follows_new_rows() {
    use qa_proto::{kind, prop, Sink};
    let mut ui = long_list(60);
    patch(&mut ui, |p| set_i(p, 1, prop::scroll::STICK_END, 1));
    ui.scroll_by(1, 0.0, 1e9);
    let end = scroll_y(&ui, 1);
    patch(&mut ui, |p| {
        p.op_create(300, kind::BUTTON);
        p.op_append(2, 300);
    });
    assert!(scroll_y(&ui, 1) > end);
    let r = ui.nodes[&300].rect;
    assert!(r.y + r.h <= 480.5);
}

#[test]
fn wheel_notches_scroll_smoothly() {
    let mut ui = long_list(60);
    ui.pointer_move(100.0, 100.0);
    ui.wheel(0.0, 56.0, true);
    assert_eq!(scroll_y(&ui, 1), 0.0, "moves over the next frames, not at once");
    let t0 = ui.scroll_clock + 16;
    let mut t = t0;
    let mut ys = Vec::new();
    while ui.step_scrolls(t) {
        ys.push(scroll_y(&ui, 1));
        t += 16;
        assert!(t < t0 + 2000, "never settles");
    }
    assert!((scroll_y(&ui, 1) - 56.0).abs() < 0.01);
    assert!(ys.windows(2).all(|w| w[1] >= w[0]));
    assert!(ys.len() > 3);
    // Touchpad scrolling follows the fingers.
    ui.wheel(0.0, 10.0, false);
    assert!((scroll_y(&ui, 1) - 66.0).abs() < 0.01);
}

/// A button (2) in a column (1) whose context menu is popup 10 holding menu 11.
fn with_menu(items: &str) -> Ui {
    use qa_proto::{kind, prop, Sink, Value};
    let mut ui = ui();
    patch(&mut ui, |p| {
        p.op_create(1, kind::GRID);
        p.op_create(2, kind::BUTTON);
        p.op_set(2, prop::button::LABEL);
        p.value(&Value::Str("target"));
        set_i(p, 2, prop::CONTEXT_MENU, 10);
        p.op_append(1, 2);
        p.op_create(3, kind::BUTTON);
        p.op_append(1, 3);
        p.op_root(1);
        p.op_create(10, kind::POPUP);
        p.op_create(11, kind::MENU);
        p.op_set(11, prop::menu::ITEMS);
        p.value(&Value::Str(items));
        p.op_append(10, 11);
    });
    ui
}

fn right_click(ui: &mut Ui, x: f32, y: f32) {
    ui.pointer_move(x, y);
    ui.pointer_button(crate::platform::BUTTON_RIGHT, true, x, y);
    ui.pointer_button(crate::platform::BUTTON_RIGHT, false, x, y);
}

#[test]
fn right_click_opens_the_context_menu_and_a_choice_closes_it() {
    let mut ui = with_menu("Reply\n-\n!Edit\nDelete\tDel");
    let r = ui.nodes[&2].rect;
    right_click(&mut ui, r.x + 10.0, r.y + 15.0);
    ui.layout();
    assert!(ui.is_open(10));
    assert_eq!(ui.take_events(), vec![UiEvent::ContextMenu { node: 2, x: r.x + 10.0, y: r.y + 15.0 }]);
    // It opens at the pointer, and the menu has focus.
    let pr = ui.nodes[&10].rect;
    assert_eq!((pr.x, pr.y), (r.x + 10.0, r.y + 15.0));
    assert_eq!(ui.focus, 11);
    // Disabled items and separators can't be chosen; Delete can.
    let rects = ui.menu_rects(11);
    let (_, edit, ok) = rects[2];
    assert!(!ok);
    click_at(&mut ui, edit.x + 5.0, edit.y + 3.0);
    assert!(ui.is_open(10));
    let (_, del, _) = rects[3];
    click_at(&mut ui, del.x + 5.0, del.y + 3.0);
    assert!(!ui.is_open(10));
    assert_eq!(ui.take_events(), vec![UiEvent::Select(11, 3), UiEvent::Dismiss(10)]);
}

fn click_at(ui: &mut Ui, x: f32, y: f32) {
    ui.pointer_move(x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, true, x, y);
    ui.pointer_button(crate::platform::BUTTON_LEFT, false, x, y);
}

#[test]
fn a_click_outside_or_escape_closes_a_popup() {
    use qa_proto::prop;
    let mut ui = with_menu("One\nTwo");
    let r = ui.nodes[&2].rect;
    right_click(&mut ui, r.x + 10.0, r.y + 5.0);
    ui.layout();
    ui.take_events();
    // The click that closes it doesn't reach the button under it.
    let b = ui.nodes[&3].rect;
    click_at(&mut ui, b.x + b.w - 5.0, b.y + b.h / 2.0);
    assert!(!ui.is_open(10));
    assert_eq!(ui.take_events(), vec![UiEvent::Dismiss(10)]);
    assert_eq!(ui.nodes[&10].i(prop::popup::OPEN, 0), 0);
    // The app opens it next to a node; Esc closes it and focus goes back.
    ui.set_focus(3, true);
    let mut p = Vec::new();
    set_i(&mut p, 10, prop::popup::ANCHOR, 2);
    set_i(&mut p, 10, prop::popup::OPEN, 1);
    ui.apply_patch(&p).unwrap();
    ui.layout();
    let pr = ui.nodes[&10].rect;
    // Below it, as wide as it but kept 6 px inside the window.
    assert!(pr.y >= r.y + r.h && pr.x == 6.0 && pr.w == 588.0, "{pr:?} under {r:?}");
    assert_eq!(ui.focus, 11);
    ui.key(crate::ui::edit::keys::ESCAPE, "", 0);
    assert!(!ui.is_open(10));
    assert_eq!(ui.focus, 3);
    assert_eq!(ui.take_events(), vec![UiEvent::Dismiss(10)]);
}

#[test]
fn menus_work_from_the_keyboard() {
    use crate::ui::edit::{keys, SHIFT};
    let mut ui = with_menu("!Gone\nCopy\n-\nPaste\nQuit");
    ui.set_focus(2, true);
    ui.key(keys::F10, "", SHIFT);
    ui.layout();
    assert!(ui.is_open(10));
    assert_eq!(ui.take_events().len(), 1);
    // The first enabled item is highlighted; Down skips the separator.
    assert_eq!(ui.menu_hl(11), 1);
    ui.key(keys::DOWN, "", 0);
    assert_eq!(ui.menu_hl(11), 3);
    ui.key('q' as u32, "q", 0);
    assert_eq!(ui.menu_hl(11), 4);
    ui.key(keys::RETURN, "\r", 0);
    assert_eq!(ui.take_events(), vec![UiEvent::Select(11, 4), UiEvent::Dismiss(10)]);
    assert_eq!(ui.focus, 2);
}

#[test]
fn popups_must_stay_detached() {
    use qa_proto::{kind, Sink};
    let mut ui = with_menu("x");
    let mut p = Vec::new();
    p.op_append(1, 10);
    assert!(ui.apply_patch(&p).is_err());
    // Destroying an open popup closes it.
    let r = ui.nodes[&2].rect;
    right_click(&mut ui, r.x + 10.0, r.y + 5.0);
    let mut p = Vec::new();
    p.op_destroy(10);
    ui.apply_patch(&p).unwrap();
    ui.layout();
    assert!(ui.popups.is_empty());
    let _ = kind::POPUP;
}

#[test]
fn rows_added_under_a_header_at_the_top_keep_the_view() {
    use qa_proto::{kind, prop, Sink, Value};
    // As in a chat: a "load older" button stays first, older rows go in after it.
    let mut ui = long_list(60);
    patch(&mut ui, |p| {
        p.op_create(99, kind::BUTTON);
        p.op_insert(2, 99, 100);
    });
    ui.scroll_by(1, 0.0, -1e9);
    assert_eq!(scroll_y(&ui, 1), 0.0);
    let probe = 105;
    let before = ui.nodes[&probe].rect.y;
    patch(&mut ui, |p| {
        for i in 0..10 {
            p.op_create(400 + i, kind::BUTTON);
            p.op_set(400 + i, prop::button::LABEL);
            p.value(&Value::Str("older"));
            p.op_insert(2, 400 + i, 100);
        }
    });
    assert!((ui.nodes[&probe].rect.y - before).abs() < 0.5, "{before} → {}", ui.nodes[&probe].rect.y);
    assert!(scroll_y(&ui, 1) > 0.0);
}
