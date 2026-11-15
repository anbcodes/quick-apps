//! Popups (an overlay layer above the tree) and menus.

use super::edit::keys;
use super::paint::{Align, Paint};
use super::{OpenPopup, State, Ui, UiEvent, Val};
use crate::gfx::{Color, RectF};
use qa_proto::{kind, prop};

/// One line of a menu's `items`.
pub struct Item<'a> {
    pub label: &'a str,
    pub hint: &'a str,
    pub sep: bool,
    pub disabled: bool,
    pub checked: bool,
}

pub fn parse_items(s: &str) -> Vec<Item<'_>> {
    s.split('\n')
        .map(|l| {
            if l.trim() == "-" {
                return Item { label: "", hint: "", sep: true, disabled: true, checked: false };
            }
            let mut l = l;
            let (mut disabled, mut checked) = (false, false);
            loop {
                if let Some(r) = l.strip_prefix('!') {
                    disabled = true;
                    l = r;
                } else if let Some(r) = l.strip_prefix('*') {
                    checked = true;
                    l = r;
                } else {
                    break;
                }
            }
            let (label, hint) = l.split_once('\t').unwrap_or((l, ""));
            Item { label, hint, sep: false, disabled, checked }
        })
        .collect()
}

/// Space between the popup edge and the window edge.
const MARGIN: f32 = 6.0;
/// Menu item padding.
const MENU_PX: f32 = 10.0;
const MENU_PY: f32 = 4.0;
const SEP_H: f32 = 9.0;

impl Ui {
    pub fn is_open(&self, id: u32) -> bool {
        self.popups.iter().any(|p| p.id == id)
    }

    /// Opens a popup on top of the others. `keyboard` shows the focus ring
    /// and highlights a menu's first item.
    pub fn open_popup(&mut self, id: u32, keyboard: bool) {
        if !self.nodes.get(&id).is_some_and(|n| n.kind == kind::POPUP) || self.is_open(id) {
            return;
        }
        if self.open_select != 0 {
            self.close_popup();
        }
        let prev_focus = self.focus;
        self.popups.push(OpenPopup { id, at: self.ptr, prev_focus, focus_in: Some(keyboard) });
        if let Some(n) = self.nodes.get_mut(&id) {
            match n.props.iter_mut().find(|(k, _)| *k == prop::popup::OPEN) {
                Some(p) => p.1 = Val::I(1),
                None => n.props.push((prop::popup::OPEN, Val::I(1))),
            }
        }
        // What was under the pointer no longer gets it.
        let (h, p) = (self.hover, self.press);
        self.hover = 0;
        self.press = 0;
        self.damage_node(h);
        self.damage_node(p);
        self.drag = None;
        self.text_press = None;
        self.invalidate(id);
        self.set_focus(0, false);
        self.damage_all();
    }

    /// Moves focus into popups that just opened: their first focusable node.
    fn focus_new_popups(&mut self) {
        let Some(top) = self.popups.last_mut() else { return };
        let Some(keyboard) = top.focus_in.take() else { return };
        let first = self.focus_order().first().copied().unwrap_or(0);
        if first != 0 {
            self.set_focus(first, keyboard);
            if keyboard {
                self.menu_move(first, 1, true);
            }
        }
    }

    /// Closes `id` and every popup opened after it. `notify` sends
    /// `Dismiss` for each (the runtime closed them, not the app).
    pub fn close_popups_from(&mut self, id: u32, notify: bool) {
        let Some(i) = self.popups.iter().position(|p| p.id == id) else { return };
        let closed: Vec<OpenPopup> = self.popups.drain(i..).collect();
        for p in closed.iter().rev() {
            if let Some(n) = self.nodes.get_mut(&p.id) {
                n.props.retain(|(k, _)| *k != prop::popup::OPEN);
            }
            if notify {
                self.out.push(UiEvent::Dismiss(p.id));
            }
        }
        // Focus goes back to where it was before the first closed popup opened.
        let back = closed[0].prev_focus;
        let inside = self.focus != 0 && closed.iter().any(|p| self.is_ancestor(p.id, self.focus));
        if inside || self.focus == 0 {
            let ok = self.nodes.contains_key(&back) && self.popup_root(back).map_or(true, |r| self.is_open(r));
            self.set_focus(if ok { back } else { 0 }, self.focus_visible);
        }
        self.hover = 0;
        self.press = 0;
        self.drag = None;
        let (x, y) = self.ptr;
        if x >= 0.0 {
            self.pointer_move(x, y);
        }
        self.damage_all();
    }

    pub fn close_all_popups(&mut self, notify: bool) {
        if let Some(p) = self.popups.first() {
            let id = p.id;
            self.close_popups_from(id, notify);
        }
    }

    /// The popup a node lives in, if any.
    pub fn popup_root(&self, mut id: u32) -> Option<u32> {
        while let Some(n) = self.nodes.get(&id) {
            if n.kind == kind::POPUP {
                return Some(id);
            }
            id = n.parent;
        }
        None
    }

    /// The topmost open popup.
    pub fn top_popup(&self) -> Option<u32> {
        self.popups.last().map(|p| p.id)
    }

    /// A right-click or the Menu key: opens the nearest `context_menu` on
    /// the path to `id`. Returns whether one opened.
    pub fn context_menu(&mut self, id: u32, x: f32, y: f32, keyboard: bool) -> bool {
        let mut cur = id;
        while let Some(n) = self.nodes.get(&cur) {
            let m = n.i(prop::CONTEXT_MENU, 0) as u32;
            if m != 0 && self.nodes.get(&m).is_some_and(|p| p.kind == kind::POPUP) {
                if self.is_open(m) {
                    self.close_popups_from(m, false);
                }
                self.out.push(UiEvent::ContextMenu { node: cur, x, y });
                let saved = self.ptr;
                self.ptr = (x, y);
                self.open_popup(m, keyboard);
                self.ptr = saved;
                return true;
            }
            cur = n.parent;
        }
        false
    }

    /// Places each open popup next to its anchor.
    pub(crate) fn place_popups(&mut self) {
        let ids: Vec<(u32, (f32, f32))> = self.popups.iter().map(|p| (p.id, p.at)).collect();
        for (id, at) in ids {
            if !self.nodes.contains_key(&id) {
                continue;
            }
            let n = &self.nodes[&id];
            let anchor = n.i(prop::popup::ANCHOR, 0) as u32;
            let place = n.i(prop::popup::PLACEMENT, 0);
            let a = match self.nodes.get(&anchor) {
                Some(an) if anchor != 0 => an.rect,
                _ => RectF::new(at.0, at.1, 0.0, 0.0),
            };
            let top = self.top_inset + MARGIN;
            let (ww, wh) = (self.win_w - 2.0 * MARGIN, self.win_h - top - MARGIN);
            // A dropdown is at least as wide as what opened it.
            let min = if anchor != 0 && place <= 1 { a.w } else { 0.0 };
            let w = self.pref_w(id).max(min).min(ww.max(0.0));
            let h = self.height(id, w).min(wh.max(0.0));
            let gap = if anchor != 0 { 4.0 } else { 0.0 };
            let fit_x = |x: f32| x.min(MARGIN + ww - w).max(MARGIN);
            let fit_y = |y: f32| y.min(top + wh - h).max(top);
            let (x, y) = match place {
                1 | 0 => {
                    let below = a.y + a.h + gap;
                    let above = a.y - gap - h;
                    let room_below = below + h <= top + wh;
                    let room_above = above >= top;
                    let y = if place == 0 { if room_below || !room_above { below } else { above } } else if room_above || !room_below { above } else { below };
                    (fit_x(a.x), fit_y(y))
                }
                _ => {
                    // Beside the anchor (submenus), or at the pointer.
                    let right = if anchor != 0 { a.x + a.w - 2.0 } else { a.x };
                    let left = if anchor != 0 { a.x - w + 2.0 } else { a.x - w };
                    let room_right = right + w <= MARGIN + ww;
                    let x = if place == 3 && left >= MARGIN || !room_right { left } else { right };
                    let y = if anchor != 0 { a.y - 4.0 } else if a.y + h <= top + wh { a.y } else { a.y - h };
                    (fit_x(x), fit_y(y))
                }
            };
            self.arrange_popup(id, RectF::new(x.round(), y.round(), w, h));
        }
        self.focus_new_popups();
    }

    pub(crate) fn paint_popups(&mut self, p: &mut Paint) {
        let ids: Vec<u32> = self.popups.iter().map(|p| p.id).collect();
        for id in ids {
            let Some(n) = self.nodes.get(&id) else { continue };
            let r = n.rect;
            let radius = self.num(id, &["popup.radius", "radius"], 6.0);
            let shadow = self.color_opt(id, &["popup.shadow"]).unwrap_or(Color::rgba(0, 0, 0, 34));
            p.fill(RectF::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h + 3.0), shadow.with_alpha(0.5), radius + 2.0);
            p.fill(RectF::new(r.x, r.y + 2.0, r.w, r.h + 2.0), shadow, radius + 1.0);
            let bg = self.color(id, &["popup.bg", "color.bg"]);
            let border = self.color(id, &["popup.border", "color.border"]);
            p.fill(r, bg, radius);
            let old = p.clip(r, radius);
            if let Some(c) = self.nodes[&id].children.first().copied() {
                self.paint_node(p, c);
            }
            p.unclip(old);
            p.stroke(r, border, radius, 1.0);
        }
    }

    // ---- Menus ----

    pub fn menu_row_h(&mut self, id: u32) -> f32 {
        self.text_style(id, "body").line_h + 2.0 * MENU_PY
    }

    pub fn menu_pref_w(&mut self, id: u32) -> f32 {
        let ts = self.text_style(id, "body");
        let items = self.nodes[&id].s(prop::menu::ITEMS).to_string();
        let items = parse_items(&items);
        let check = if items.iter().any(|i| i.checked) { ts.size + 6.0 } else { 0.0 };
        let mut w: f32 = 0.0;
        for i in &items {
            let lw = self.fonts.measure(ts.font, ts.size, i.label);
            let hw = if i.hint.is_empty() { 0.0 } else { 32.0 + self.fonts.measure(ts.font, ts.size, i.hint) };
            w = w.max(lw + hw);
        }
        (w + check + 2.0 * MENU_PX + 8.0).ceil().max(140.0)
    }

    pub fn menu_h(&mut self, id: u32) -> f32 {
        let row = self.menu_row_h(id);
        let s = self.nodes[&id].s(prop::menu::ITEMS);
        let items = parse_items(s);
        items.iter().map(|i| if i.sep { SEP_H } else { row }).sum::<f32>() + 8.0
    }

    /// Each item's rect: (line index, rect, item is selectable).
    pub fn menu_rects(&mut self, id: u32) -> Vec<(usize, RectF, bool)> {
        let row = self.menu_row_h(id);
        let r = self.nodes[&id].rect;
        let s = self.nodes[&id].s(prop::menu::ITEMS);
        let mut y = r.y + 4.0;
        let mut out = Vec::new();
        for (i, it) in parse_items(s).iter().enumerate() {
            let h = if it.sep { SEP_H } else { row };
            out.push((i, RectF::new(r.x + 4.0, y, r.w - 8.0, h), !it.sep && !it.disabled));
            y += h;
        }
        out
    }

    pub fn menu_hl(&self, id: u32) -> i32 {
        match self.nodes.get(&id).map(|n| &n.st) {
            Some(State::Menu(h)) => *h,
            _ => -1,
        }
    }

    pub fn set_menu_hl(&mut self, id: u32, hl: i32) {
        if let Some(n) = self.nodes.get_mut(&id) {
            if let State::Menu(h) = &mut n.st {
                if *h != hl {
                    *h = hl;
                    self.damage_node(id);
                }
            }
        }
    }

    /// The item under (x, y), if it can be chosen.
    pub fn menu_item_at(&mut self, id: u32, x: f32, y: f32) -> Option<i32> {
        self.menu_rects(id).into_iter().find(|(_, r, ok)| *ok && r.contains(x, y)).map(|(i, ..)| i as i32)
    }

    /// Moves the highlight `d` selectable items (from the start when `reset`).
    pub fn menu_move(&mut self, id: u32, d: i32, reset: bool) {
        if self.nodes.get(&id).map(|n| n.kind) != Some(kind::MENU) {
            return;
        }
        let ok: Vec<i32> = self.menu_rects(id).into_iter().filter(|r| r.2).map(|r| r.0 as i32).collect();
        if ok.is_empty() {
            return;
        }
        let cur = self.menu_hl(id);
        let pos = ok.iter().position(|&i| i == cur);
        let next = match (pos, reset) {
            (Some(p), false) => ok[(p as i32 + d).clamp(0, ok.len() as i32 - 1) as usize],
            _ if d < 0 => ok[ok.len() - 1],
            _ => ok[0],
        };
        self.set_menu_hl(id, next);
    }

    /// Chooses a menu item: sends `Select` and closes the popups it is in.
    pub fn menu_choose(&mut self, id: u32, i: i32) {
        let s = self.nodes[&id].s(prop::menu::ITEMS);
        let items = parse_items(s);
        if !items.get(i as usize).is_some_and(|it| !it.sep && !it.disabled) || self.is_disabled(id) {
            return;
        }
        self.out.push(UiEvent::Select(id, i));
        self.set_menu_hl(id, -1);
        if self.popup_root(id).is_some_and(|p| self.is_open(p)) {
            self.close_all_popups(true);
        }
    }

    /// A key for a focused menu. Returns whether it was used.
    pub fn menu_key(&mut self, id: u32, sym: u32, text: &str) -> bool {
        match sym {
            keys::UP => self.menu_move(id, -1, self.menu_hl(id) < 0),
            keys::DOWN => self.menu_move(id, 1, self.menu_hl(id) < 0),
            keys::HOME => self.menu_move(id, 1, true),
            keys::END => self.menu_move(id, -1, true),
            keys::RETURN | keys::KP_ENTER | keys::SPACE => {
                let hl = self.menu_hl(id);
                if hl >= 0 {
                    self.menu_choose(id, hl);
                }
            }
            _ => {
                // A letter jumps to the next item that starts with it.
                let Some(c) = text.chars().next().filter(|c| c.is_alphanumeric()) else { return false };
                let c = c.to_lowercase().next().unwrap_or(c);
                let s = self.nodes[&id].s(prop::menu::ITEMS).to_string();
                let items = parse_items(&s);
                let n = items.len() as i32;
                let start = self.menu_hl(id).max(-1);
                for k in 1..=n {
                    let i = (start + k).rem_euclid(n);
                    let it = &items[i as usize];
                    if !it.sep && !it.disabled && it.label.chars().next().and_then(|f| f.to_lowercase().next()) == Some(c) {
                        self.set_menu_hl(id, i);
                        break;
                    }
                }
            }
        }
        true
    }

    pub(crate) fn paint_menu(&mut self, p: &mut Paint, id: u32) {
        let ts = self.text_style(id, "body");
        let s = self.nodes[&id].s(prop::menu::ITEMS).to_string();
        let items = parse_items(&s);
        let rects = self.menu_rects(id);
        let hl = self.menu_hl(id);
        let radius = (self.num(id, &["popup.radius", "radius"], 6.0) - 2.0).max(0.0);
        let check = if items.iter().any(|i| i.checked) { ts.size + 6.0 } else { 0.0 };
        let fg = self.color(id, &["color.fg"]);
        let muted = self.color(id, &["color.muted"]);
        let border = self.color(id, &["color.border"]);
        let off = self.is_disabled(id);
        for ((i, r, _), it) in rects.iter().zip(&items) {
            if it.sep {
                p.fill(RectF::new(r.x + 4.0, r.y + (r.h / 2.0).floor(), r.w - 8.0, 1.0), border, 0.0);
                continue;
            }
            if *i as i32 == hl && !it.disabled && !off {
                let c = self.color(id, &["menu.highlight", "color.selection"]);
                p.fill(*r, c, radius);
            }
            let c = if it.disabled { muted } else { fg };
            let x = r.x + MENU_PX - 4.0;
            if it.checked {
                let ac = self.color(id, &["color.accent"]);
                self.draw_text(p, ts, ac, x, r.y + MENU_PY, check, Align::Start, "✓", false);
            }
            let hw = if it.hint.is_empty() { 0.0 } else { self.fonts.measure(ts.font, ts.size, it.hint) + 16.0 };
            self.draw_text(p, ts, c, x + check, r.y + MENU_PY, (r.w - 2.0 * (MENU_PX - 4.0) - check - hw).max(0.0), Align::Start, it.label, true);
            if !it.hint.is_empty() {
                self.draw_text(p, ts, muted, r.x, r.y + MENU_PY, r.w - (MENU_PX - 4.0), Align::End, it.hint, false);
            }
        }
        if self.focus == id && self.focus_visible && hl < 0 {
            let r = self.nodes[&id].rect;
            let c = self.color(id, &["color.focus"]);
            p.stroke(RectF::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0), c, radius, 2.0);
        }
    }
}
