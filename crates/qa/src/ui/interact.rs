//! Pointer and keyboard handling, focus, and the built-in behaviour of components.

use super::edit::{keys, EditResult, CTRL, SHIFT};
use super::{State, Ui, UiEvent};
use crate::gfx::RectF;
use crate::platform::{Cursor, BUTTON_LEFT};
use qa_proto::{kind, prop};

pub(crate) enum Drag {
    /// Selecting text in an input.
    Select(u32),
    /// Dragging a scrollbar thumb: (scroll id, grab offset).
    Thumb(u32, f32),
    /// Pointer captured by a canvas.
    Canvas(u32),
    /// Selecting text in a text node.
    Text(u32),
}

/// How far the pointer moves before a press on text becomes a selection.
const DRAG_SLOP: f32 = 4.0;

/// The largest char boundary in `s` at or before `i`.
pub(crate) fn floor_char(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The word around byte offset `i`.
fn word_at(s: &str, i: usize) -> (usize, usize) {
    let i = floor_char(s, i);
    let is_word = super::edit::is_word;
    let a = s[..i].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(i, |(j, _)| j);
    let b = s[i..].char_indices().find(|(_, c)| !is_word(*c)).map_or(s.len(), |(j, _)| i + j);
    (a, b)
}

fn now_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

impl Ui {
    /// The path from the root to the deepest visible node under (x, y).
    pub fn hit_path(&self, x: f32, y: f32) -> Vec<u32> {
        let mut path = Vec::new();
        let mut cur = self.root;
        loop {
            let Some(n) = self.nodes.get(&cur) else { break };
            if n.hidden() || !n.rect.contains(x, y) {
                break;
            }
            path.push(cur);
            // Topmost (last painted) child first.
            let next = n.children.iter().rev().copied().find(|c| {
                self.nodes.get(c).is_some_and(|cn| !cn.hidden() && cn.rect.contains(x, y))
            });
            match next {
                Some(c) => cur = c,
                None => break,
            }
        }
        path
    }

    fn interactive(&self, id: u32) -> bool {
        let n = &self.nodes[&id];
        match n.kind {
            kind::BUTTON | kind::INPUT | kind::CHECKBOX | kind::SELECT | kind::TABS | kind::CANVAS => true,
            kind::GRID => n.i(prop::grid::CLICKABLE, 0) != 0,
            kind::TEXT => false,
            _ => false,
        }
    }

    fn target_at(&mut self, x: f32, y: f32) -> u32 {
        let path = self.hit_path(x, y);
        for &id in path.iter().rev() {
            if self.interactive(id) || (self.nodes[&id].kind == kind::TEXT && self.link_at(id, x, y).is_some()) {
                return id;
            }
        }
        0
    }

    fn scroll_at(&self, x: f32, y: f32, vertical: bool) -> Option<u32> {
        let path = self.hit_path(x, y);
        path.iter().rev().copied().find(|id| {
            let n = &self.nodes[id];
            if n.kind != kind::SCROLL {
                return false;
            }
            let axis = n.i(prop::scroll::AXIS, 0);
            match &n.st {
                State::Scroll(s) => {
                    if vertical {
                        axis != 1 && s.content_h > n.rect.h + 0.5
                    } else {
                        axis != 0 && s.content_w > n.rect.w + 0.5
                    }
                }
                _ => false,
            }
        })
    }

    fn update_hover(&mut self, x: f32, y: f32) {
        let t = self.target_at(x, y);
        if t != self.hover {
            let old = self.hover;
            self.hover = t;
            self.damage_node(old);
            self.damage_node(t);
        }
        let cursor = match self.nodes.get(&t) {
            None if self.selectable_text_at(x, y).is_some() => Cursor::Text,
            Some(n) if n.disabled() => Cursor::Default,
            Some(n) if n.kind == kind::INPUT => Cursor::Text,
            Some(n) if n.kind == kind::CANVAS => Cursor::Default,
            Some(_) => Cursor::Pointer,
            None => Cursor::Default,
        };
        self.cursor = cursor;
    }

    fn scroll_hover_damage(&mut self, ox: f32, oy: f32, x: f32, y: f32) {
        // Scrollbars show while the pointer is over a scroll.
        let ids: Vec<u32> = self.nodes.iter().filter(|(_, n)| n.kind == kind::SCROLL && !n.hidden()).map(|(id, _)| *id).collect();
        for id in ids {
            let r = self.nodes[&id].rect;
            if r.contains(ox, oy) != r.contains(x, y) {
                self.damage_rect(r);
            }
        }
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        let (ox, oy) = self.ptr;
        self.ptr = (x, y);
        if self.open_select != 0 {
            if let Some((r, row, opts)) = self.popup_geometry() {
                let i = if r.contains(x, y) { ((y - r.y - 4.0) / row).floor() as i32 } else { -1 };
                let i = if i >= 0 && (i as usize) < opts.len() { i } else { -1 };
                if i != self.popup_hl && i >= 0 {
                    self.popup_hl = i;
                    self.damage_rect(r);
                }
            }
            self.cursor = Cursor::Default;
            return;
        }
        self.scroll_hover_damage(ox, oy, x, y);
        match self.drag {
            Some(Drag::Select(id)) => {
                if let Some(i) = self.input_index_at(id, x, y) {
                    let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
                    if e.caret != i {
                        e.caret = i;
                        self.damage_node(id);
                    }
                }
                return;
            }
            Some(Drag::Thumb(id, grab)) => {
                let n = &self.nodes[&id];
                let r = n.rect;
                if let State::Scroll(s) = &n.st {
                    let th = (r.h * r.h / s.content_h).max(24.0).min(r.h);
                    let frac = ((y - grab - r.y) / (r.h - th).max(1.0)).clamp(0.0, 1.0);
                    let target = frac * (s.content_h - r.h);
                    let dy = target - s.y;
                    self.scroll_by(id, 0.0, dy);
                }
                return;
            }
            Some(Drag::Canvas(id)) => {
                self.canvas_pointer(id, 2, x, y, 0.0, 0.0);
                return;
            }
            Some(Drag::Text(id)) => {
                if let Some((sid, anchor, _)) = self.text_sel.filter(|s| s.0 == id) {
                    let i = self.text_index_at(id, x, y);
                    self.text_sel = Some((sid, anchor, i));
                    self.damage_node(id);
                }
                self.cursor = Cursor::Text;
                return;
            }
            None => {}
        }
        if let Some((id, i, px, py)) = self.text_press {
            if (x - px).abs() > DRAG_SLOP || (y - py).abs() > DRAG_SLOP {
                // The press becomes a selection, so it won't click, and
                // doesn't count toward a double-click.
                self.text_press = None;
                self.last_click = (0, 0, 0);
                let p = std::mem::take(&mut self.press);
                self.damage_node(p);
                let j = self.text_index_at(id, x, y);
                self.text_sel = Some((id, i, j));
                self.drag = Some(Drag::Text(id));
                self.damage_node(id);
                self.cursor = Cursor::Text;
                return;
            }
        }
        self.update_hover(x, y);
        if let Some(n) = self.nodes.get(&self.hover) {
            if n.kind == kind::CANVAS {
                let h = self.hover;
                self.canvas_pointer(h, 2, x, y, 0.0, 0.0);
            }
        }
    }

    pub fn pointer_leave(&mut self) {
        let (ox, oy) = self.ptr;
        self.ptr = (-1.0, -1.0);
        self.scroll_hover_damage(ox, oy, -1.0, -1.0);
        if let Some(n) = self.nodes.get(&self.hover) {
            if n.kind == kind::CANVAS {
                let h = self.hover;
                self.canvas_pointer(h, 4, ox, oy, 0.0, 0.0);
            }
        }
        let old = self.hover;
        self.hover = 0;
        self.damage_node(old);
    }

    fn canvas_pointer(&mut self, id: u32, k: u8, x: f32, y: f32, dx: f32, dy: f32) {
        let n = &self.nodes[&id];
        let r = n.rect;
        let (pw, ph) = (n.i(prop::canvas::PX_W, 0) as f32, n.i(prop::canvas::PX_H, 0) as f32);
        let (cx, cy) = if r.w > 0.0 && r.h > 0.0 { ((x - r.x) * pw / r.w, (y - r.y) * ph / r.h) } else { (0.0, 0.0) };
        let buttons = if self.press == id { 1 } else { 0 };
        self.out.push(UiEvent::Pointer { node: id, kind: k, x: cx, y: cy, buttons, dx, dy });
    }

    /// The byte offset in an input's text nearest to (x, y).
    fn input_index_at(&mut self, id: u32, x: f32, y: f32) -> Option<usize> {
        let r = self.nodes[&id].rect;
        let ts = self.text_style(id, "body");
        let (px, py) = self.ctl_pad(id);
        let n = &self.nodes[&id];
        let multiline = n.i(prop::input::MULTILINE, 0) != 0;
        let secret = n.i(prop::input::SECRET, 0) != 0;
        let State::Input(e) = &n.st else { return None };
        let text = e.text.clone();
        let (sx, sy) = (e.scroll_x, e.scroll_y);
        let inner_w = (r.w - 2.0 * px).max(1.0);
        let shown: String = if secret { text.chars().map(|c| if c == '\n' { c } else { '•' }).collect() } else { text.clone() };
        let lines = if multiline { crate::text::wrap(&self.fonts, ts.font, ts.size, &shown, inner_w) } else { vec![(0, shown.len())] };
        let li = (((y - r.y - py + sy) / ts.line_h).floor().max(0.0) as usize).min(lines.len() - 1);
        let (a, b) = lines[li];
        let stops = self.fonts.caret_stops(ts.font, ts.size, &shown[a..b]);
        let lx = x - r.x - px + sx;
        let mut best = 0;
        for (i, s) in stops.iter().enumerate() {
            if (s - lx).abs() < (stops[best] - lx).abs() {
                best = i;
            }
        }
        // Char index within the line, then back to a byte offset in the real text.
        let line_chars_before = shown[..a].chars().count();
        let ci = line_chars_before + best.min(shown[a..b].chars().count());
        Some(text.char_indices().nth(ci).map(|(i, _)| i).unwrap_or(text.len()))
    }

    pub fn pointer_button(&mut self, button: u32, pressed: bool, x: f32, y: f32) {
        self.ptr = (x, y);
        self.last_input = std::time::Instant::now();
        if self.open_select != 0 {
            if pressed {
                if let Some((r, row, opts)) = self.popup_geometry() {
                    let id = self.open_select;
                    if r.contains(x, y) {
                        let i = ((y - r.y - 4.0) / row).floor() as i32;
                        if i >= 0 && (i as usize) < opts.len() {
                            self.choose_option(id, i);
                        }
                    } else {
                        self.close_popup();
                    }
                }
            }
            return;
        }
        if pressed {
            // A press on a scrollbar thumb drags it.
            if let Some(sid) = self.scroll_at(x, y, true) {
                let r = self.nodes[&sid].rect;
                if x >= r.x + r.w - 10.0 {
                    if let State::Scroll(s) = &self.nodes[&sid].st {
                        let th = (r.h * r.h / s.content_h).max(24.0).min(r.h);
                        let ty = r.y + (r.h - th) * (s.y / (s.content_h - r.h));
                        let grab = if y >= ty && y <= ty + th { y - ty } else { th / 2.0 };
                        self.drag = Some(Drag::Thumb(sid, grab));
                        self.pointer_move(x, y);
                        return;
                    }
                }
            }
            if let Some((old, ..)) = self.text_sel.take() {
                self.damage_node(old);
            }
            self.text_press = None;
            if button == BUTTON_LEFT {
                if let Some(tn) = self.selectable_text_at(x, y) {
                    let i = self.text_index_at(tn, x, y);
                    let now = now_ms();
                    let (lid, lt, count) = self.last_click;
                    let count = if lid == tn && now - lt < 400 { count + 1 } else { 1 };
                    self.last_click = (tn, now, count);
                    let text = self.nodes[&tn].s(prop::text::TEXT);
                    match count {
                        1 => self.text_press = Some((tn, i, x, y)),
                        2 => {
                            let (a, b) = word_at(text, i);
                            self.text_sel = Some((tn, a, b));
                        }
                        _ => self.text_sel = Some((tn, 0, text.len())),
                    }
                    if count > 1 {
                        self.drag = Some(Drag::Text(tn));
                        self.damage_node(tn);
                    }
                }
            }
            let t = self.target_at(x, y);
            if t == 0 {
                if self.focus != 0 {
                    self.set_focus(0, false);
                }
                return;
            }
            if self.nodes[&t].disabled() {
                return;
            }
            let k = self.nodes[&t].kind;
            if button == BUTTON_LEFT || k == kind::CANVAS {
                self.press = t;
                if self.nodes[&t].focusable() {
                    self.set_focus(t, false);
                }
                self.damage_node(t);
            }
            match k {
                kind::INPUT if button == BUTTON_LEFT => {
                    if let Some(i) = self.input_index_at(t, x, y) {
                        let now = now_ms();
                        let (lid, lt, count) = self.last_click;
                        let count = if lid == t && now - lt < 400 { count + 1 } else { 1 };
                        self.last_click = (t, now, count);
                        let shift = self.mods & SHIFT != 0;
                        let e = self.nodes.get_mut(&t).unwrap().edit().unwrap();
                        match count {
                            2 => {
                                let (a, b) = e.word_at(i);
                                e.anchor = a;
                                e.caret = b;
                            }
                            n if n >= 3 => {
                                e.anchor = 0;
                                e.caret = e.text.len();
                            }
                            _ => {
                                e.caret = i;
                                if !shift {
                                    e.anchor = i;
                                }
                            }
                        }
                        self.drag = Some(Drag::Select(t));
                    }
                }
                kind::TABS if button == BUTTON_LEFT => {
                    let rects = self.tab_rects(t);
                    if let Some(i) = rects.iter().position(|r| r.contains(x, y)) {
                        let cur = self.nodes[&t].i(prop::tabs::SELECTED, 0);
                        if cur != i as i32 {
                            let _ = self.set_prop(t, prop::tabs::SELECTED, super::Val::I(i as i32));
                            self.out.push(UiEvent::Select(t, i as i32));
                        }
                    }
                }
                kind::CANVAS => {
                    self.drag = Some(Drag::Canvas(t));
                    self.canvas_pointer(t, 0, x, y, 0.0, 0.0);
                }
                _ => {}
            }
        } else {
            let drag = self.drag.take();
            self.text_press = None;
            if let Some(Drag::Text(_)) = drag {
                self.press = 0;
                self.update_hover(x, y);
                return;
            }
            if let Some(Drag::Canvas(id)) = drag {
                self.canvas_pointer(id, 1, x, y, 0.0, 0.0);
                self.press = 0;
                self.update_hover(x, y);
                return;
            }
            let p = self.press;
            self.press = 0;
            if p == 0 {
                self.update_hover(x, y);
                return;
            }
            self.damage_node(p);
            let t = self.target_at(x, y);
            if t == p && button == BUTTON_LEFT {
                if self.nodes[&p].kind == kind::TEXT {
                    if let Some(i) = self.link_at(p, x, y) {
                        self.out.push(UiEvent::Link(p, i));
                    }
                } else {
                    self.activate(p, false);
                }
            }
            self.update_hover(x, y);
        }
    }

    /// The topmost selectable text node under (x, y).
    pub(crate) fn selectable_text_at(&self, x: f32, y: f32) -> Option<u32> {
        let path = self.hit_path(x, y);
        let id = *path.last()?;
        let n = &self.nodes[&id];
        (n.kind == kind::TEXT && n.i(prop::text::SELECTABLE, 1) != 0 && !n.s(prop::text::TEXT).is_empty()).then_some(id)
    }

    /// The selected text, if any, clamped to the node's current text.
    pub fn selected_text(&self) -> Option<String> {
        let (id, a, b) = self.text_sel?;
        let text = self.nodes.get(&id)?.s(prop::text::TEXT);
        let (a, b) = (floor_char(text, a.min(b)), floor_char(text, a.max(b)));
        (a < b).then(|| text[a..b].to_string())
    }

    /// Clicks a button, clickable grid, checkbox or select.
    fn activate(&mut self, id: u32, keyboard: bool) {
        let n = &self.nodes[&id];
        if n.disabled() {
            return;
        }
        match n.kind {
            kind::BUTTON | kind::GRID => self.out.push(UiEvent::Click(id)),
            kind::CHECKBOX => {
                let v = n.i(prop::checkbox::CHECKED, 0) == 0;
                let _ = self.set_prop(id, prop::checkbox::CHECKED, super::Val::I(v as i32));
                self.out.push(UiEvent::Toggle(id, v));
            }
            kind::SELECT => {
                self.open_select = id;
                // The list is drawn over other nodes, so nothing else marks its area.
                self.damage_all();
            }
            _ => {}
        }
        let _ = keyboard;
    }

    pub fn wheel(&mut self, dx: f32, dy: f32) {
        let (x, y) = self.ptr;
        if let Some(n) = self.nodes.get(&self.hover) {
            if n.kind == kind::CANVAS {
                let h = self.hover;
                self.canvas_pointer(h, 3, x, y, dx, dy);
                return;
            }
        }
        // Shift turns vertical wheels into horizontal scrolling.
        let (dx, dy) = if self.mods & SHIFT != 0 && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
        let mut moved = false;
        if dy != 0.0 {
            if let Some(s) = self.scroll_at(x, y, true) {
                moved |= self.scroll_by(s, 0.0, dy);
            }
        }
        if dx != 0.0 {
            if let Some(s) = self.scroll_at(x, y, false) {
                moved |= self.scroll_by(s, dx, 0.0);
            }
        }
        if moved {
            self.update_hover(x, y);
        }
    }

    pub fn paste(&mut self, text: String) {
        let id = std::mem::take(&mut self.paste_into);
        if id == 0 || !self.nodes.contains_key(&id) {
            return;
        }
        let multiline = self.nodes[&id].i(prop::input::MULTILINE, 0) != 0;
        let t = if multiline { text.replace("\r\n", "\n") } else { text.replace(['\r', '\n'], " ") };
        let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
        e.insert(&t, false);
        self.input_changed(id);
    }

    fn input_changed(&mut self, id: u32) {
        let v = match &self.nodes[&id].st {
            State::Input(e) => e.text.clone(),
            _ => return,
        };
        // Keep the VALUE prop in sync so a SET of the same text is a no-op.
        if let Some(n) = self.nodes.get_mut(&id) {
            match n.props.iter_mut().find(|(k, _)| *k == prop::input::VALUE) {
                Some(p) => p.1 = super::Val::S(v.clone()),
                None => n.props.push((prop::input::VALUE, super::Val::S(v.clone()))),
            }
        }
        self.damage_node(id);
        self.out.push(UiEvent::Change(id, v));
        let multiline = self.nodes[&id].i(prop::input::MULTILINE, 0) != 0;
        if multiline {
            // Height does not depend on text (fixed `lines`), so only paint changes.
        }
    }

    fn move_focus(&mut self, back: bool) {
        let order = self.focus_order();
        if order.is_empty() {
            return;
        }
        let i = order.iter().position(|&f| f == self.focus);
        let next = match (i, back) {
            (None, false) => order[0],
            (None, true) => order[order.len() - 1],
            (Some(i), false) => order[(i + 1) % order.len()],
            (Some(i), true) => order[(i + order.len() - 1) % order.len()],
        };
        self.set_focus(next, true);
        self.request_scroll_to(next);
    }

    /// A key press. Returns true when the runtime used it.
    pub fn key(&mut self, sym: u32, text: &str, mods: u8) {
        self.last_input = std::time::Instant::now();
        let ctrl = mods & CTRL != 0;
        if ctrl && (sym == 0x71 || sym == 0x51) {
            self.quit = true;
            return;
        }
        if self.open_select != 0 {
            self.select_popup_key(sym);
            return;
        }
        let input_focused = self.nodes.get(&self.focus).is_some_and(|n| n.kind == kind::INPUT);
        if ctrl && (sym == 0x63 || sym == 0x43) && !input_focused {
            if let Some(s) = self.selected_text() {
                self.clipboard_out = Some(s);
                return;
            }
        }
        let f = self.focus;
        if f != 0 && self.nodes.contains_key(&f) && !self.nodes[&f].disabled() {
            let k = self.nodes[&f].kind;
            match k {
                kind::INPUT => {
                    let multiline = self.nodes[&f].i(prop::input::MULTILINE, 0) != 0;
                    if sym == keys::TAB || sym == keys::ISO_LEFT_TAB || sym == keys::ESCAPE || (ctrl && !is_edit_ctrl(sym)) && sym != keys::RETURN {
                        // Fall through to traversal / the app.
                    } else if multiline && (sym == keys::UP || sym == keys::DOWN) && mods & CTRL == 0 {
                        self.input_vertical(f, sym == keys::UP, mods & SHIFT != 0);
                        self.damage_node(f);
                        return;
                    } else if !multiline && (sym == keys::UP || sym == keys::DOWN || sym == keys::PAGE_UP || sym == keys::PAGE_DOWN) {
                        // Not an input key: give it to the app.
                    } else {
                        let r = self.nodes.get_mut(&f).unwrap().edit().unwrap().key(sym, text, mods, multiline);
                        match r {
                            EditResult::Ignored => {}
                            EditResult::Moved => {
                                self.damage_node(f);
                                return;
                            }
                            EditResult::Changed => {
                                self.input_changed(f);
                                return;
                            }
                            EditResult::Submit => {
                                let v = self.nodes.get_mut(&f).unwrap().edit().unwrap().text.clone();
                                self.out.push(UiEvent::Submit(f, v));
                                return;
                            }
                            EditResult::Copy(s) => {
                                if self.nodes[&f].i(prop::input::SECRET, 0) == 0 {
                                    self.clipboard_out = Some(s);
                                }
                                return;
                            }
                            EditResult::Cut(s) => {
                                if self.nodes[&f].i(prop::input::SECRET, 0) == 0 {
                                    self.clipboard_out = Some(s);
                                }
                                self.input_changed(f);
                                return;
                            }
                            EditResult::Paste => {
                                self.paste_into = f;
                                return;
                            }
                        }
                    }
                }
                kind::BUTTON | kind::GRID | kind::CHECKBOX | kind::SELECT => {
                    if (sym == keys::RETURN || sym == keys::KP_ENTER || sym == keys::SPACE) && mods & (CTRL | 4) == 0 {
                        if k == kind::CHECKBOX && sym != keys::SPACE {
                            // Enter on a checkbox goes to the app.
                        } else {
                            self.activate(f, true);
                            return;
                        }
                    }
                }
                kind::TABS => {
                    if sym == keys::LEFT || sym == keys::RIGHT {
                        let n = self.nodes[&f].s(prop::tabs::LABELS).split('\n').count() as i32;
                        let cur = self.nodes[&f].i(prop::tabs::SELECTED, 0);
                        let next = if sym == keys::LEFT { (cur - 1).max(0) } else { (cur + 1).min(n - 1) };
                        if next != cur {
                            let _ = self.set_prop(f, prop::tabs::SELECTED, super::Val::I(next));
                            self.out.push(UiEvent::Select(f, next));
                        }
                        return;
                    }
                }
                _ => {}
            }
        }
        // Scroll keys go to the scroll that holds the focus, or the one under the pointer.
        if matches!(sym, keys::PAGE_UP | keys::PAGE_DOWN) || (f == 0 && matches!(sym, keys::HOME | keys::END | keys::UP | keys::DOWN)) {
            let sid = self.scroll_for_keys();
            if let Some(s) = sid {
                let h = self.nodes[&s].rect.h;
                let dy = match sym {
                    keys::PAGE_UP => -h * 0.9,
                    keys::PAGE_DOWN => h * 0.9,
                    keys::HOME => -f32::INFINITY,
                    keys::END => f32::INFINITY,
                    keys::UP => -40.0,
                    _ => 40.0,
                };
                let dy = if dy.is_infinite() { dy.signum() * 1e9 } else { dy };
                if self.scroll_by(s, 0.0, dy) {
                    return;
                }
            }
        }
        if sym == keys::TAB || sym == keys::ISO_LEFT_TAB {
            self.move_focus(sym == keys::ISO_LEFT_TAB || mods & SHIFT != 0);
            return;
        }
        self.out.push(UiEvent::Key { node: f, sym, mods, text: text.to_string() });
    }

    fn scroll_for_keys(&self) -> Option<u32> {
        let mut cur = self.focus;
        while let Some(n) = self.nodes.get(&cur) {
            if n.kind == kind::SCROLL {
                return Some(cur);
            }
            cur = n.parent;
        }
        let (x, y) = self.ptr;
        self.scroll_at(x, y, true).or_else(|| {
            // The first vertical scroll in the tree.
            let mut stack = vec![self.root];
            while let Some(id) = stack.pop() {
                let n = self.nodes.get(&id)?;
                if n.kind == kind::SCROLL && !n.hidden() {
                    return Some(id);
                }
                stack.extend(n.children.iter().rev());
            }
            None
        })
    }

    fn input_vertical(&mut self, id: u32, up: bool, select: bool) {
        let r = self.nodes[&id].rect;
        let ts = self.text_style(id, "body");
        let (px, _) = self.ctl_pad(id);
        let e = match &self.nodes[&id].st {
            State::Input(e) => e,
            _ => return,
        };
        let text = e.text.clone();
        let caret = e.caret;
        let goal = e.goal_x;
        let lines = crate::text::wrap(&self.fonts, ts.font, ts.size, &text, (r.w - 2.0 * px).max(1.0));
        let li = lines.iter().rposition(|&(a, _)| a <= caret).unwrap_or(0);
        let (a, b) = lines[li];
        let stops = self.fonts.caret_stops(ts.font, ts.size, &text[a..b]);
        let cx = goal.unwrap_or_else(|| stops.get(text[a..caret.min(b)].chars().count()).copied().unwrap_or(0.0));
        let target = if up { li.checked_sub(1) } else if li + 1 < lines.len() { Some(li + 1) } else { None };
        let new = match target {
            None => {
                if up {
                    0
                } else {
                    text.len()
                }
            }
            Some(t) => {
                let (a, b) = lines[t];
                let line = &text[a..b];
                let stops = self.fonts.caret_stops(ts.font, ts.size, line);
                let mut best = 0;
                for (i, s) in stops.iter().enumerate() {
                    if (s - cx).abs() < (stops[best] - cx).abs() {
                        best = i;
                    }
                }
                let best = best.min(line.trim_end_matches('\n').chars().count());
                a + line.char_indices().nth(best).map(|(i, _)| i).unwrap_or(line.len())
            }
        };
        let e = self.nodes.get_mut(&id).unwrap().edit().unwrap();
        e.caret = new;
        if !select {
            e.anchor = new;
        }
        e.goal_x = Some(cx);
    }

    fn select_popup_key(&mut self, sym: u32) {
        let id = self.open_select;
        let n = self.nodes[&id].s(prop::select::OPTIONS).split('\n').count() as i32;
        let cur = self.nodes[&id].i(prop::select::SELECTED, -1);
        let hl = if self.popup_hl < 0 { cur.max(0) } else { self.popup_hl };
        match sym {
            keys::ESCAPE => self.close_popup(),
            keys::UP => self.popup_hl = (hl - 1).max(0),
            keys::DOWN => self.popup_hl = (hl + 1).min(n - 1),
            keys::RETURN | keys::KP_ENTER | keys::SPACE => {
                self.choose_option(id, hl);
            }
            _ => {}
        }
        self.damage_all();
    }

    pub fn choose_option(&mut self, id: u32, i: i32) {
        let cur = self.nodes[&id].i(prop::select::SELECTED, -1);
        if i != cur && i >= 0 {
            let _ = self.set_prop(id, prop::select::SELECTED, super::Val::I(i));
            self.out.push(UiEvent::Select(id, i));
        }
        self.close_popup();
    }

    pub fn close_popup(&mut self) {
        self.open_select = 0;
        self.popup_hl = -1;
        self.damage_all();
    }

    /// The popup list for an open select, in logical px: (rect, row height, options).
    pub fn popup_geometry(&mut self) -> Option<(RectF, f32, Vec<String>)> {
        let id = self.open_select;
        if id == 0 || !self.nodes.contains_key(&id) {
            return None;
        }
        let r = self.nodes[&id].rect;
        let ts = self.text_style(id, "body");
        let opts: Vec<String> = self.nodes[&id].s(prop::select::OPTIONS).split('\n').map(|s| s.to_string()).collect();
        let row = ts.line_h + 8.0;
        let h = (row * opts.len() as f32 + 8.0).min(self.win_h - 16.0);
        let mut y = r.y + r.h + 4.0;
        if y + h > self.win_h - 8.0 {
            y = (r.y - 4.0 - h).max(8.0);
        }
        Some((RectF::new(r.x, y, r.w.max(120.0), h), row, opts))
    }
}

fn is_edit_ctrl(sym: u32) -> bool {
    let s = if (0x41..=0x5a).contains(&sym) { sym + 0x20 } else { sym };
    matches!(s, 0x61 | 0x65 | 0x6b | 0x75 | 0x77 | 0x63 | 0x78 | 0x76 | 0x7a | 0x79)
        || matches!(sym, keys::LEFT | keys::RIGHT | keys::BACKSPACE | keys::DELETE | keys::HOME | keys::END | keys::RETURN | keys::KP_ENTER)
}
